use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use arrow_array::RecordBatch;
use cove_core::{
    profile::cove_o::{
        read_object_surface_from_bytes_with_pushdown_options, reconstruct_object_states,
        CoveObjectPropertyValue, CoveObjectReadOptions, CoveObjectReadWithPushdownOptions,
        CoveObjectReconstructionOptions, CoveObjectRecord, CoveObjectRedactionReadPolicy,
        CoveObjectState, CoveObjectSurface, CoveObjectTemporalCut,
        OBJECT_TYPE_FLAG_EVIDENCE_OBJECT, PROPERTY_FLAG_EVIDENCE_REF,
        PROPERTY_FLAG_MAPPING_RULE_REF,
    },
    reader::{validate_bytes, ValidationOptions},
    retained_bytes::RetainedBytes,
};
use cove_map::{projected_record_batches_from_cove_o_bytes, ProjectionBatchOptions};
use cove_map::{ProjectionFilter, ProjectionFilterLiteral, ProjectionFilterOp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    arrow_output::{
        execution_rows_to_json, record_batches_to_json_rows, record_batches_to_projection_rows,
    },
    association_opt::{association_row_matches_temporal, association_valid_at},
    evidence_opt::materialized_evidence_rows_for_plan,
    expr_eval::{
        distinct_count, eval_expr, eval_predicate, expr_collation_id, expr_logical_type,
        parse_decimal_value, stable_value_key, value_ordering_typed, EvalContext, ExactDecimal,
    },
    materialized::{
        hex, ExecutionRow, MaterializedAssociationRow, MaterializedChangeDetail,
        MaterializedChangeDiffKind, MaterializedEvidenceRow, MaterializedObjectRow,
        MaterializedProjectionRow, OutputGrain,
    },
    parse_resolve_and_plan_query,
    pushdown::{self, PushdownOptions, PushdownReport},
    AggregateDisclosurePolicy, AstAggregateName, AstChangeMode, AstCompareOp, AstHistoryMode,
    AstNullOrdering, AstOrderDirection, BuildLogicalPlanError, CoveOqlOutputMode,
    DatasetFileIdentity, DiagnosticSeverity, FallbackPolicy, LogicalPlanDiagnostic,
    ManifestDatasetMember, MetadataDisclosurePolicy, ParseOptions, PlanOptions, PlannedQuery,
    RedactionPolicy, ResolveOptions, ResolvedExpr, ResolvedLiteral, ResolvedLiteralValue,
    ResolvedMethodChain, ResolvedPath, ResolvedPredicate, ResolvedRoot, ResolvedTimeBound,
    ResourceBudgetPolicy, TemporalMode, TemporalRole, ValidatedFileIdentity, VisibilityPolicy,
};

#[derive(Debug, Clone)]
pub struct CoveOqlRetainedInput {
    bytes: RetainedBytes,
}

impl CoveOqlRetainedInput {
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes: RetainedBytes::from_vec(bytes),
        }
    }

    pub fn from_arc(bytes: Arc<Vec<u8>>) -> Self {
        Self {
            bytes: RetainedBytes::from_arc(bytes),
        }
    }

    pub fn from_retained(bytes: RetainedBytes) -> Self {
        Self { bytes }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub(crate) fn retained_bytes(&self) -> RetainedBytes {
        self.bytes.clone()
    }
}

#[derive(Debug, Clone)]
pub struct CoveOqlRetainedManifestMember {
    source: String,
    bytes: RetainedBytes,
}

impl CoveOqlRetainedManifestMember {
    pub fn from_vec(source: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            source: source.into(),
            bytes: RetainedBytes::from_vec(bytes),
        }
    }

    pub fn from_arc(source: impl Into<String>, bytes: Arc<Vec<u8>>) -> Self {
        Self {
            source: source.into(),
            bytes: RetainedBytes::from_arc(bytes),
        }
    }

    pub fn from_retained(source: impl Into<String>, bytes: RetainedBytes) -> Self {
        Self {
            source: source.into(),
            bytes,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn as_manifest_member(&self) -> ManifestDatasetMember<'_> {
        ManifestDatasetMember {
            source: &self.source,
            bytes: self.bytes.as_slice(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOptions {
    pub resource_budget: ResourceBudgetPolicy,
    pub batch_size: Option<usize>,
    pub emit_json_diagnostics: bool,
    pub mapping_path: Option<PathBuf>,
    pub allow_partial_results: bool,
    pub visibility_overlay: Option<VisibilityOverlay>,
    pub pushdown: PushdownOptions,
    pub execution_code_filecode_map: BTreeMap<u32, u64>,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            resource_budget: ResourceBudgetPolicy::default(),
            batch_size: None,
            emit_json_diagnostics: true,
            mapping_path: None,
            allow_partial_results: false,
            visibility_overlay: None,
            pushdown: PushdownOptions::default(),
            execution_code_filecode_map: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityOverlay {
    pub overlay_id: String,
    pub visible_goids: BTreeSet<String>,
    pub visible_record_ids: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAuthority {
    CoveMapMetadata,
    MaterializedEvidenceObjects,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAuthorityReport {
    pub source: ExecutionAuthoritySource,
    pub authoritative: bool,
    pub candidate_only: bool,
    pub residual_required: bool,
    pub materialized_fallback: bool,
    pub compared_with_materialized: bool,
    pub notes: Vec<String>,
}

impl ExecutionAuthorityReport {
    pub fn materialized_baseline(reason: impl Into<String>) -> Self {
        Self {
            source: ExecutionAuthoritySource::MaterializedBaseline,
            authoritative: true,
            candidate_only: false,
            residual_required: true,
            materialized_fallback: false,
            compared_with_materialized: false,
            notes: vec![reason.into()],
        }
    }

    pub fn exact_index_only(reason: impl Into<String>) -> Self {
        Self {
            source: ExecutionAuthoritySource::ExactIndexOnlyAnswer,
            authoritative: true,
            candidate_only: false,
            residual_required: false,
            materialized_fallback: false,
            compared_with_materialized: false,
            notes: vec![reason.into()],
        }
    }

    pub fn exact_zero_copy(reason: impl Into<String>) -> Self {
        Self {
            source: ExecutionAuthoritySource::ZeroCopyArrow,
            authoritative: true,
            candidate_only: false,
            residual_required: false,
            materialized_fallback: false,
            compared_with_materialized: false,
            notes: vec![reason.into()],
        }
    }

    pub fn exact_kernel(reason: impl Into<String>, residual_required: bool) -> Self {
        Self {
            source: ExecutionAuthoritySource::ExactOptimizedKernel,
            authoritative: true,
            candidate_only: false,
            residual_required,
            materialized_fallback: false,
            compared_with_materialized: false,
            notes: vec![reason.into()],
        }
    }

    pub fn physical_plan_only(reason: impl Into<String>) -> Self {
        Self {
            source: ExecutionAuthoritySource::PhysicalPlanOnly,
            authoritative: false,
            candidate_only: false,
            residual_required: false,
            materialized_fallback: false,
            compared_with_materialized: false,
            notes: vec![reason.into()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAuthoritySource {
    MaterializedBaseline,
    ExactOptimizedKernel,
    ExactIndexOnlyAnswer,
    ZeroCopyArrow,
    DataFusionProvider,
    PhysicalPlanOnly,
}

#[derive(Debug, Clone)]
pub struct ExecutedQuery {
    pub planned: PlannedQuery,
    pub result: CoveOqlExecutionResult,
    pub diagnostics: Vec<ExecutionDiagnostic>,
    pub row_counts: ExecutionRowCounts,
    pub output_fingerprint: String,
    pub pushdown_report: PushdownReport,
    pub evidence_authority: Option<EvidenceAuthority>,
    pub authority: ExecutionAuthorityReport,
}

impl ExecutedQuery {
    pub fn explain_json(&self) -> Value {
        crate::explain::executed_query_explain_json(self)
    }

    pub fn explain_text(&self) -> String {
        crate::render_explain_text(&self.explain_json())
    }
}

#[derive(Debug, Clone)]
pub struct CoveOqlResultStream {
    bytes: Vec<u8>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    executed: Option<ExecutedQuery>,
    batches: Vec<CoveOqlExecutionResult>,
    next_batch: usize,
    row_stream: Option<MaterializedRowStreamState>,
    blocking_reason: Option<String>,
    cancelled: bool,
}

impl CoveOqlResultStream {
    pub fn executed(&self) -> Option<&ExecutedQuery> {
        self.executed.as_ref()
    }

    pub fn is_blocking(&self) -> bool {
        self.blocking_reason.is_some()
    }

    pub fn blocking_reason(&self) -> Option<&str> {
        self.blocking_reason.as_deref()
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.batches.clear();
    }

    pub fn next_batch(&mut self) -> Result<Option<CoveOqlExecutionResult>, BuildExecutionError> {
        if self.blocking_reason.is_none() {
            return self.next_streaming_batch();
        }
        self.ensure_executed()?;
        let batch = self.batches.get(self.next_batch).cloned();
        if batch.is_some() {
            self.next_batch += 1;
        }
        Ok(batch)
    }

    pub fn finish(mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        if self.blocking_reason.is_none() {
            self.finish_streaming()
        } else {
            self.ensure_executed()?;
            self.executed
                .take()
                .ok_or_else(|| exec_error("E_STREAM_CANCELLED", "stream was cancelled", json!({})))
        }
    }

    fn next_streaming_batch(
        &mut self,
    ) -> Result<Option<CoveOqlExecutionResult>, BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if self.executed.is_some() {
            return Ok(None);
        }
        if self.row_stream.is_none() {
            self.row_stream = Some(MaterializedRowStreamState::new(
                &self.bytes,
                self.planned.clone(),
                self.options.clone(),
            )?);
        }
        let batch = self
            .row_stream
            .as_mut()
            .ok_or_else(|| exec_error("E_STREAM", "stream state was not initialized", json!({})))?
            .next_batch()?;
        Ok(batch)
    }

    fn finish_streaming(&mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if let Some(executed) = self.executed.take() {
            return Ok(executed);
        }
        if self.row_stream.is_none() {
            self.row_stream = Some(MaterializedRowStreamState::new(
                &self.bytes,
                self.planned.clone(),
                self.options.clone(),
            )?);
        }
        let state = self
            .row_stream
            .as_mut()
            .ok_or_else(|| exec_error("E_STREAM", "stream state was not initialized", json!({})))?;
        while state.next_batch()?.is_some() {}
        let executed = state.finish()?;
        self.executed = Some(executed.clone());
        Ok(executed)
    }

    fn ensure_executed(&mut self) -> Result<(), BuildExecutionError> {
        if self.cancelled {
            return Err(exec_error(
                "E_STREAM_CANCELLED",
                "stream was cancelled before completion",
                json!({}),
            ));
        }
        if self.executed.is_some() {
            return Ok(());
        }
        let mut executed =
            execute_planned_query(&self.bytes, self.planned.clone(), self.options.clone())?;
        if let Some(reason) = &self.blocking_reason {
            executed.diagnostics.push(exec_warning(
                "W_STREAM_BLOCKING_PLAN",
                format!("streaming plan is blocking: {reason}"),
                json!({ "blocking_reason": reason }),
            ));
        }
        let batch_size = if self.blocking_reason.is_some() {
            usize::MAX
        } else {
            self.options.batch_size.unwrap_or(usize::MAX).max(1)
        };
        self.batches = result_batches(&executed.result, batch_size);
        self.executed = Some(executed);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct MaterializedRowStreamState {
    planned: PlannedQuery,
    options: ExecutionOptions,
    started: Instant,
    source: MaterializedRowSource,
    next_input: usize,
    input_rows: usize,
    filtered_rows: usize,
    output_rows: usize,
    json_output: Vec<Value>,
    object_output: Vec<MaterializedObjectRow>,
    association_output: Vec<MaterializedAssociationRow>,
    evidence_output: Vec<MaterializedEvidenceRow>,
    finished: bool,
}

impl MaterializedRowStreamState {
    fn new(
        bytes: &[u8],
        planned: PlannedQuery,
        options: ExecutionOptions,
    ) -> Result<Self, BuildExecutionError> {
        let started = Instant::now();
        validate_security_scope(&planned, &options)?;
        validate_execution_grain(&planned)?;
        let mut source = object_backed_row_source(bytes, &planned, &options, started)?;
        let context = EvalContext::for_plan(&source.associations, &source.evidence_rows, &planned);
        sort_rows(&mut source.rows, &planned, &context)?;
        Ok(Self {
            planned,
            options,
            started,
            source,
            next_input: 0,
            input_rows: 0,
            filtered_rows: 0,
            output_rows: 0,
            json_output: Vec::new(),
            object_output: Vec::new(),
            association_output: Vec::new(),
            evidence_output: Vec::new(),
            finished: false,
        })
    }

    fn next_batch(&mut self) -> Result<Option<CoveOqlExecutionResult>, BuildExecutionError> {
        if self.finished {
            return Ok(None);
        }
        let batch_size = self.options.batch_size.unwrap_or(usize::MAX).max(1);
        let take = self
            .planned
            .resolved
            .method_chain
            .take
            .and_then(|take| usize::try_from(take).ok());
        let mut json_batch = Vec::new();
        let mut object_batch = Vec::new();
        let mut association_batch = Vec::new();
        let mut evidence_batch = Vec::new();
        while self.next_input < self.source.rows.len()
            && batch_len(
                &json_batch,
                &object_batch,
                &association_batch,
                &evidence_batch,
            ) < batch_size
        {
            if take.is_some_and(|take| self.output_rows >= take) {
                self.finished = true;
                break;
            }
            let row = self.source.rows[self.next_input].clone();
            self.next_input += 1;
            if !stream_row_visible(&row, &self.planned, &self.options) {
                continue;
            }
            self.input_rows += 1;
            let context = EvalContext::for_plan(
                &self.source.associations,
                &self.source.evidence_rows,
                &self.planned,
            );
            if !predicate_matches(&row, &self.planned, &context)? {
                continue;
            }
            self.filtered_rows += 1;
            let emitted = match (&self.planned.resolved.output_mode, row) {
                (CoveOqlOutputMode::JsonRows, row) => {
                    let value =
                        select_json_rows(std::slice::from_ref(&row), &self.planned, &context)?
                            .into_iter()
                            .next()
                            .unwrap_or(Value::Null);
                    json_batch.push(value.clone());
                    self.json_output.push(value);
                    true
                }
                (CoveOqlOutputMode::ObjectRows, ExecutionRow::Object(row)) => {
                    object_batch.push(row.clone());
                    self.object_output.push(row);
                    true
                }
                (CoveOqlOutputMode::AssociationRows, ExecutionRow::Association(row)) => {
                    association_batch.push(row.clone());
                    self.association_output.push(row);
                    true
                }
                (CoveOqlOutputMode::EvidenceRows, ExecutionRow::Evidence(row)) => {
                    evidence_batch.push(row.clone());
                    self.evidence_output.push(row);
                    true
                }
                _ => false,
            };
            if emitted {
                self.output_rows += 1;
            }
            check_time(&self.options.resource_budget, self.started)?;
        }
        if self.next_input >= self.source.rows.len() {
            self.finished = true;
        }
        if batch_len(
            &json_batch,
            &object_batch,
            &association_batch,
            &evidence_batch,
        ) == 0
        {
            return Ok(None);
        }
        Ok(Some(match &self.planned.resolved.output_mode {
            CoveOqlOutputMode::JsonRows => CoveOqlExecutionResult::JsonRows(json_batch),
            CoveOqlOutputMode::ObjectRows => CoveOqlExecutionResult::ObjectRows(object_batch),
            CoveOqlOutputMode::AssociationRows => {
                CoveOqlExecutionResult::AssociationRows(association_batch)
            }
            CoveOqlOutputMode::EvidenceRows => CoveOqlExecutionResult::EvidenceRows(evidence_batch),
            _ => unreachable!("non-streamable output modes are blocked before streaming"),
        }))
    }

    fn finish(&mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        while self.next_batch()?.is_some() {}
        let result = match &self.planned.resolved.output_mode {
            CoveOqlOutputMode::JsonRows => {
                CoveOqlExecutionResult::JsonRows(self.json_output.clone())
            }
            CoveOqlOutputMode::ObjectRows => {
                CoveOqlExecutionResult::ObjectRows(self.object_output.clone())
            }
            CoveOqlOutputMode::AssociationRows => {
                CoveOqlExecutionResult::AssociationRows(self.association_output.clone())
            }
            CoveOqlOutputMode::EvidenceRows => {
                CoveOqlExecutionResult::EvidenceRows(self.evidence_output.clone())
            }
            _ => unreachable!("non-streamable output modes are blocked before streaming"),
        };
        let row_counts = ExecutionRowCounts {
            input_rows: self.input_rows,
            filtered_rows: self.filtered_rows,
            output_rows: self.output_rows,
        };
        enforce_result_budgets(
            &result,
            &row_counts,
            &self.planned,
            &self.options,
            self.started,
        )?;
        let output_fingerprint = result_fingerprint(&result)?;
        let mut diagnostics = self
            .planned
            .diagnostics
            .iter()
            .cloned()
            .map(ExecutionDiagnostic::from)
            .collect::<Vec<_>>();
        diagnostics.push(exec_warning(
            "W_STREAM_BATCHED_EXECUTION",
            "streamable plan executed through materialized row-source batches; final summary was deferred until finish",
            json!({ "batch_size": self.options.batch_size }),
        ));
        self.source
            .pushdown_report
            .counters
            .rows_after_candidate_retain = row_counts.input_rows;
        Ok(ExecutedQuery {
            planned: self.planned.clone(),
            result,
            diagnostics,
            row_counts,
            output_fingerprint,
            pushdown_report: self.source.pushdown_report.clone(),
            evidence_authority: self.source.evidence_authority,
            authority: ExecutionAuthorityReport::materialized_baseline(
                "streamed materialized row-source execution produced the visible output",
            ),
        })
    }
}

fn batch_len(
    json_batch: &[Value],
    object_batch: &[MaterializedObjectRow],
    association_batch: &[MaterializedAssociationRow],
    evidence_batch: &[MaterializedEvidenceRow],
) -> usize {
    json_batch.len() + object_batch.len() + association_batch.len() + evidence_batch.len()
}

fn stream_row_visible(
    row: &ExecutionRow,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> bool {
    if !matches!(
        planned
            .resolved
            .operation_context
            .security
            .visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) {
        return true;
    }
    options
        .visibility_overlay
        .as_ref()
        .is_some_and(|overlay| row_visible_in_overlay(row, overlay))
}

impl Iterator for CoveOqlResultStream {
    type Item = Result<CoveOqlExecutionResult, BuildExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch().transpose()
    }
}

#[derive(Debug, Clone)]
pub enum CoveOqlExecutionResult {
    ObjectRows(Vec<MaterializedObjectRow>),
    AssociationRows(Vec<MaterializedAssociationRow>),
    EvidenceRows(Vec<MaterializedEvidenceRow>),
    ProjectionRows(Vec<MaterializedProjectionRow>),
    ArrowRecordBatches(Vec<RecordBatch>),
    JsonRows(Vec<Value>),
    ExplainJson(Value),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRowCounts {
    pub input_rows: usize,
    pub filtered_rows: usize,
    pub output_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub phase: String,
    pub safe_details: Value,
    pub redacted: bool,
}

#[derive(Debug, Clone)]
pub struct BuildExecutionError {
    pub diagnostics: Vec<ExecutionDiagnostic>,
    pub source: Option<String>,
}

impl BuildExecutionError {
    fn single(diagnostic: ExecutionDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            source: None,
        }
    }

    fn from_plan(error: BuildLogicalPlanError) -> Self {
        Self {
            diagnostics: error
                .diagnostics
                .into_iter()
                .map(ExecutionDiagnostic::from)
                .collect(),
            source: error.source,
        }
    }

    pub fn explain_json(&self) -> Value {
        crate::explain::error_explain_json(
            "execute",
            self.diagnostics
                .iter()
                .map(|diagnostic| {
                    json!({
                        "code": diagnostic.code,
                        "severity": crate::diagnostic_severity_name(diagnostic.severity),
                        "message": diagnostic.message,
                        "span": Value::Null,
                        "phase": diagnostic.phase,
                        "safe_details": diagnostic.safe_details,
                        "redacted": diagnostic.redacted,
                    })
                })
                .collect(),
            Vec::new(),
        )
    }
}

impl fmt::Display for BuildExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "Cove-OQL materialized execution failed")
        }
    }
}

impl Error for BuildExecutionError {}

impl From<LogicalPlanDiagnostic> for ExecutionDiagnostic {
    fn from(diagnostic: LogicalPlanDiagnostic) -> Self {
        Self {
            code: diagnostic.code,
            severity: diagnostic.severity,
            message: diagnostic.message,
            phase: diagnostic.phase,
            safe_details: diagnostic.safe_details,
            redacted: diagnostic.redacted,
        }
    }
}

pub fn parse_resolve_plan_and_execute_query(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    execution_options: ExecutionOptions,
    validation_options: ValidationOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let planned = parse_resolve_and_plan_query(
        bytes,
        text,
        parse_options,
        resolve_options,
        plan_options,
        validation_options,
    )
    .map_err(BuildExecutionError::from_plan)?;
    execute_planned_query(bytes, planned, execution_options)
}

pub fn execute_planned_query(
    bytes: &[u8],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let started = Instant::now();
    validate_security_scope(&planned, &options)?;
    validate_execution_grain(&planned)?;

    let mut diagnostics = planned
        .diagnostics
        .iter()
        .cloned()
        .map(ExecutionDiagnostic::from)
        .collect::<Vec<_>>();
    diagnostics.push(exec_warning(
        "W_MATERIALIZED_BASELINE",
        if options.pushdown.enabled {
            "conservative pushdown narrowed readback candidates; residual materialized checks remain the semantic authority"
        } else {
            "materialized baseline was executed with pushdown disabled"
        },
        json!({ "pushdown_enabled": options.pushdown.enabled }),
    ));
    if let Some(warning) = zero_copy_owned_fallback_warning(&planned) {
        diagnostics.push(warning);
    }

    let (result, row_counts, pushdown_report, evidence_authority) =
        match &planned.resolved.output_mode {
        CoveOqlOutputMode::ExplainJson => {
            let explain = planned.explain_json();
            (
                CoveOqlExecutionResult::ExplainJson(explain),
                ExecutionRowCounts::default(),
                PushdownReport::not_executed(&options.pushdown),
                None,
            )
        }
        CoveOqlOutputMode::DataFusionTableProvider => {
            return Err(exec_error(
                "E_UNSUPPORTED_OUTPUT",
                "DataFusion output is exposed through the Phase 3 registration helper, not execute_planned_query",
                json!({}),
            ))
        }
        _ => execute_rows(bytes, &planned, &options, started)?,
    };

    enforce_result_budgets(&result, &row_counts, &planned, &options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    Ok(ExecutedQuery {
        planned,
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report,
        evidence_authority,
        authority: ExecutionAuthorityReport::materialized_baseline(
            "materialized baseline execution produced the visible output",
        ),
    })
}

pub fn execute_planned_query_retained(
    input: CoveOqlRetainedInput,
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    execute_planned_query(input.as_slice(), planned, options)
}

pub fn execute_manifest_planned_query(
    members: &[ManifestDatasetMember<'_>],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let started = Instant::now();
    validate_security_scope(&planned, &options)?;
    validate_execution_output_mode(&planned)?;
    if matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::DataFusionTableProvider
    ) {
        return Err(exec_error(
            "E_UNSUPPORTED_OUTPUT",
            "manifest execution returns materialized OQL output; register an OQL DataFusion provider separately",
            json!({}),
        ));
    }
    let ordered_members = validate_manifest_execution_members(&planned, members)?;

    let mut diagnostics = planned
        .diagnostics
        .iter()
        .cloned()
        .map(ExecutionDiagnostic::from)
        .collect::<Vec<_>>();
    let (exact_bridge_count, inexact_bridge_count) =
        manifest_code_domain_bridge_counts(&planned.resolved.operation_context.dataset);
    let manifest_fallback_reason = if exact_bridge_count > 0 {
        "manifest member execution validated exact COVM code-domain bridge proofs, but this logical executor used the materialized OQL oracle across members because no manifest physical kernel path was selected"
    } else {
        "manifest member execution used the materialized OQL oracle across validated COVM members because cross-file coded acceleration requires exact bridge proofs and a manifest physical kernel path"
    };
    diagnostics.push(exec_warning(
        "W_MATERIALIZED_MANIFEST_BASELINE",
        manifest_fallback_reason,
        json!({
            "file_count": ordered_members.len(),
            "file_membership_fingerprint": planned.resolved.operation_context.dataset.file_membership_fingerprint.clone(),
            "exact_code_domain_bridge_count": exact_bridge_count,
            "inexact_code_domain_bridge_count": inexact_bridge_count,
            "fallback_boundary": if exact_bridge_count > 0 {
                "manifest_physical_kernel_not_selected"
            } else {
                "manifest_cross_file_bridge_not_exact"
            },
        }),
    ));
    if let Some(warning) = zero_copy_owned_fallback_warning(&planned) {
        diagnostics.push(warning);
    }

    let (result, row_counts, pushdown_report, evidence_authority) =
        match &planned.resolved.output_mode {
            CoveOqlOutputMode::ExplainJson => {
                let explain = planned.explain_json();
                (
                    CoveOqlExecutionResult::ExplainJson(explain),
                    ExecutionRowCounts::default(),
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                )
            }
            CoveOqlOutputMode::DataFusionTableProvider => unreachable!("handled above"),
            _ => execute_manifest_rows(&ordered_members, &planned, &options, started)?,
        };

    enforce_result_budgets(&result, &row_counts, &planned, &options, started)?;
    let output_fingerprint = result_fingerprint(&result)?;
    Ok(ExecutedQuery {
        planned,
        result,
        diagnostics,
        row_counts,
        output_fingerprint,
        pushdown_report,
        evidence_authority,
        authority: ExecutionAuthorityReport::materialized_baseline(
            "manifest materialized baseline execution produced the visible output",
        ),
    })
}

pub fn execute_manifest_planned_query_retained(
    members: &[CoveOqlRetainedManifestMember],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let borrowed = members
        .iter()
        .map(CoveOqlRetainedManifestMember::as_manifest_member)
        .collect::<Vec<_>>();
    execute_manifest_planned_query(&borrowed, planned, options)
}

fn manifest_code_domain_bridge_counts(dataset: &crate::DatasetScopeContext) -> (usize, usize) {
    let exact = dataset
        .code_domain_bridges
        .iter()
        .filter(|bridge| bridge.exact)
        .count();
    let inexact = dataset.code_domain_bridges.len().saturating_sub(exact);
    (exact, inexact)
}

pub fn execute_planned_query_stream(
    bytes: &[u8],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<CoveOqlResultStream, BuildExecutionError> {
    Ok(CoveOqlResultStream {
        bytes: bytes.to_vec(),
        blocking_reason: stream_blocking_reason(&planned),
        planned,
        options,
        executed: None,
        batches: Vec::new(),
        next_batch: 0,
        row_stream: None,
        cancelled: false,
    })
}

fn stream_blocking_reason(planned: &PlannedQuery) -> Option<String> {
    if matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::ArrowRecordBatch { .. }
            | CoveOqlOutputMode::ExplainJson
            | CoveOqlOutputMode::DataFusionTableProvider
            | CoveOqlOutputMode::ProjectionRows
    ) {
        return Some("output mode requires whole-result materialization".into());
    }
    if grouped_or_aggregate(planned) {
        return Some("aggregate execution requires complete input".into());
    }
    if planned.resolved.method_chain.order_by.is_some() {
        return Some("explicit orderBy requires full materialized sort".into());
    }
    if planned.resolved.method_chain.skip.is_some() {
        return Some("skip requires a stable global prefix".into());
    }
    if matches!(planned.resolved.root, ResolvedRoot::Projection(_)) {
        return Some("projection readback is materialized before streaming".into());
    }
    None
}

fn result_batches(
    result: &CoveOqlExecutionResult,
    batch_size: usize,
) -> Vec<CoveOqlExecutionResult> {
    match result {
        CoveOqlExecutionResult::ObjectRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::ObjectRows(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::AssociationRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::AssociationRows(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::EvidenceRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::EvidenceRows(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::ProjectionRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::ProjectionRows(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::ArrowRecordBatches(batches) => batches
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::ArrowRecordBatches(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::JsonRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveOqlExecutionResult::JsonRows(chunk.to_vec()))
            .collect(),
        CoveOqlExecutionResult::ExplainJson(value) => {
            vec![CoveOqlExecutionResult::ExplainJson(value.clone())]
        }
    }
}

fn execute_rows(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveOqlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    match &planned.resolved.root {
        ResolvedRoot::Projection(root) => {
            let (result, row_counts) =
                execute_projection_root(bytes, planned, options, started, &root.projection_id)?;
            let report = projection_readback_pushdown_report(planned, options, &row_counts);
            Ok((result, row_counts, report, None))
        }
        ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_) => {
            execute_object_backed_root(bytes, planned, options, started)
        }
    }
}

fn execute_object_backed_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveOqlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    let mut source = object_backed_row_source(bytes, planned, options, started)?;
    let evidence_authority = source.evidence_authority;
    let (result, row_counts) = finish_materialized_rows(
        source.rows,
        &source.associations,
        &source.evidence_rows,
        planned,
        options,
        started,
    )?;
    source.pushdown_report.counters.rows_after_candidate_retain = row_counts.input_rows;
    Ok((
        result,
        row_counts,
        source.pushdown_report,
        evidence_authority,
    ))
}

#[derive(Debug, Clone)]
struct MaterializedRowSource {
    rows: Vec<ExecutionRow>,
    associations: Vec<MaterializedAssociationRow>,
    evidence_rows: Vec<MaterializedEvidenceRow>,
    pushdown_report: PushdownReport,
    evidence_authority: Option<EvidenceAuthority>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManifestExecutionMemberRef<'a> {
    pub(crate) scope: DatasetFileIdentity,
    pub(crate) file: ValidatedFileIdentity,
    pub(crate) bytes: &'a [u8],
}

pub(crate) fn validate_manifest_execution_members<'a>(
    planned: &PlannedQuery,
    members: &'a [ManifestDatasetMember<'a>],
) -> Result<Vec<ManifestExecutionMemberRef<'a>>, BuildExecutionError> {
    let expected_files = &planned.resolved.operation_context.dataset.files;
    if expected_files.is_empty() {
        return Err(exec_error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            "manifest execution requires a resolved dataset scope with at least one member file",
            json!({}),
        ));
    }
    if expected_files.len() != members.len() {
        return Err(exec_error(
            "E_DATASET_MEMBER_MISMATCH",
            "manifest execution member count does not match the resolved dataset scope",
            json!({
                "expected_file_count": expected_files.len(),
                "provided_file_count": members.len(),
            }),
        ));
    }

    let mut ordered_expected = expected_files.clone();
    ordered_expected.sort_by_key(|file| file.ordinal);
    let mut used = vec![false; members.len()];
    let mut ordered = Vec::with_capacity(ordered_expected.len());
    for expected in ordered_expected {
        let Some((member_index, member)) = members
            .iter()
            .enumerate()
            .find(|(_, member)| member.source == expected.source)
        else {
            return Err(exec_error(
                "E_DATASET_MEMBER_MISMATCH",
                "manifest execution is missing a member file required by the resolved dataset scope",
                json!({ "source": expected.source }),
            ));
        };
        if used[member_index] {
            return Err(exec_error(
                "E_DATASET_MEMBER_MISMATCH",
                "manifest execution received a duplicate member source",
                json!({ "source": expected.source }),
            ));
        }
        used[member_index] = true;
        let validated = validate_bytes(member.bytes).map_err(|err| {
            exec_error(
                "E_DATASET_MEMBER_INVALID",
                format!(
                    "manifest execution member {} failed COVE validation: {err}",
                    member.source
                ),
                json!({ "source": member.source }),
            )
        })?;
        let file = ValidatedFileIdentity::from(&validated);
        validate_manifest_member_identity(&expected, &file)?;
        ordered.push(ManifestExecutionMemberRef {
            scope: expected,
            file,
            bytes: member.bytes,
        });
    }
    if let Some(member) = members
        .iter()
        .enumerate()
        .find(|(index, _)| !used[*index])
        .map(|(_, member)| member)
    {
        return Err(exec_error(
            "E_DATASET_MEMBER_MISMATCH",
            "manifest execution received a member file not present in the resolved dataset scope",
            json!({ "source": member.source }),
        ));
    }
    Ok(ordered)
}

fn validate_manifest_member_identity(
    expected: &DatasetFileIdentity,
    actual: &ValidatedFileIdentity,
) -> Result<(), BuildExecutionError> {
    if expected.file_id == actual.file_id
        && expected.file_len == actual.file_len
        && expected.footer_crc32c == actual.footer_crc32c
        && expected.primary_profile == actual.primary_profile
    {
        return Ok(());
    }
    Err(exec_error(
        "E_DATASET_MEMBER_STALE",
        "manifest execution member identity does not match the resolved dataset scope",
        json!({
            "source": expected.source,
            "expected": {
                "file_id": hex(&expected.file_id),
                "file_len": expected.file_len,
                "footer_crc32c": expected.footer_crc32c,
                "primary_profile": expected.primary_profile,
            },
            "actual": {
                "file_id": hex(&actual.file_id),
                "file_len": actual.file_len,
                "footer_crc32c": actual.footer_crc32c,
                "primary_profile": actual.primary_profile,
            },
        }),
    ))
}

fn execute_manifest_rows(
    members: &[ManifestExecutionMemberRef<'_>],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveOqlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    if matches!(
        planned.resolved.root,
        ResolvedRoot::Projection(_) | ResolvedRoot::Evidence(_)
    ) && (planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some())
    {
        return Err(incompatible_execution_grain(
            planned,
            "history and changes output grains require object or association roots",
        ));
    }

    let mut rows = Vec::new();
    let mut associations = Vec::new();
    let mut evidence_rows = Vec::new();
    let mut evidence_authorities = Vec::new();
    for member in members {
        check_time(&options.resource_budget, started)?;
        let member_plan = manifest_member_plan(planned, member);
        let file_id = hex(&member.scope.file_id);
        match &member_plan.resolved.root {
            ResolvedRoot::Projection(root) => {
                rows.extend(
                    projection_root_execution_rows(
                        member.bytes,
                        &member_plan,
                        options,
                        started,
                        &root.projection_id,
                    )?
                    .into_iter()
                    .map(|row| {
                        row.with_dataset_member(
                            member.scope.ordinal,
                            &member.scope.source,
                            file_id.clone(),
                        )
                    }),
                );
            }
            ResolvedRoot::Object(_) | ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_) => {
                let source =
                    object_backed_row_source(member.bytes, &member_plan, options, started)?;
                rows.extend(source.rows.into_iter().map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                }));
                associations.extend(source.associations.into_iter().map(|row| {
                    row.with_dataset_member(
                        member.scope.ordinal,
                        &member.scope.source,
                        file_id.clone(),
                    )
                }));
                evidence_rows.extend(source.evidence_rows.into_iter().map(|mut row| {
                    row.fields
                        .insert("dataset_file_ordinal".into(), json!(member.scope.ordinal));
                    row.fields.insert(
                        "dataset_file_source".into(),
                        Value::String(member.scope.source.clone()),
                    );
                    row.fields
                        .insert("dataset_file_id".into(), Value::String(file_id.clone()));
                    row
                }));
                if let Some(authority) = source.evidence_authority {
                    evidence_authorities.push(authority);
                }
            }
        }
    }

    let (result, row_counts) = finish_materialized_rows(
        rows,
        &associations,
        &evidence_rows,
        planned,
        options,
        started,
    )?;
    Ok((
        result,
        row_counts,
        manifest_materialized_pushdown_report(planned, options, members.len()),
        combined_evidence_authority(&evidence_authorities),
    ))
}

pub(crate) fn manifest_member_plan(
    planned: &PlannedQuery,
    member: &ManifestExecutionMemberRef<'_>,
) -> PlannedQuery {
    let mut member_plan = planned.clone();
    member_plan.resolved.operation_context.file = member.file.clone();
    member_plan.resolved.operation_context.dataset =
        crate::DatasetScopeContext::single_file_with_source(
            &member.file,
            &planned.resolved.operation_context.snapshot,
            &planned.resolved.operation_context.security,
            member.scope.source.clone(),
        );
    member_plan
}

fn manifest_materialized_pushdown_report(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    file_count: usize,
) -> PushdownReport {
    PushdownReport::not_applicable(
        &options.pushdown,
        format!(
            "manifest execution read {file_count} validated member files and applied global materialized OQL residual semantics for {} root",
            match planned.resolved.root {
                ResolvedRoot::Object(_) => "object",
                ResolvedRoot::Association(_) => "association",
                ResolvedRoot::Evidence(_) => "evidence",
                ResolvedRoot::Projection(_) => "projection",
            }
        ),
    )
}

pub(crate) fn combined_evidence_authority(
    authorities: &[EvidenceAuthority],
) -> Option<EvidenceAuthority> {
    let first = authorities.first().copied()?;
    if authorities.iter().all(|authority| *authority == first) {
        return Some(first);
    }
    Some(EvidenceAuthority::MaterializedEvidenceObjects)
}

fn object_backed_row_source(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<MaterializedRowSource, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let read_options = object_read_options(planned);
    let mut pushdown_plan = pushdown::pushdown_read_plan(planned, &options.pushdown);
    let read_result = read_object_surface_from_bytes_with_pushdown_options(
        bytes,
        &CoveObjectReadWithPushdownOptions {
            read: read_options,
            pushdown: pushdown_plan.read_options.clone(),
        },
    )
    .map_err(|err| {
        exec_error(
            "E_READBACK",
            format!("COVE-O materialized readback failed: {err}"),
            json!({}),
        )
    })?;
    pushdown_plan
        .report
        .merge_core_report(read_result.pushdown_report);
    let surface = read_result.surface;
    let states = object_states_for_temporal_context(&surface, planned)?;
    let associations = states
        .iter()
        .filter_map(MaterializedAssociationRow::from_state)
        .collect::<Vec<_>>();
    let (evidence_execution_rows, _) =
        materialized_evidence_rows_for_plan(planned, surface.evidence_index.as_ref());
    let evidence_authority = evidence_authority_for_rows(
        planned,
        surface.evidence_index.is_some(),
        !evidence_execution_rows.is_empty(),
    );
    let evidence_execution_rows = if evidence_execution_rows.is_empty()
        && matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
    {
        evidence_object_rows_from_states(&states)
    } else {
        evidence_execution_rows
    };
    let evidence_authority = evidence_authority.or_else(|| {
        evidence_authority_for_rows(
            planned,
            surface.evidence_index.is_some(),
            !evidence_execution_rows.is_empty(),
        )
    });
    require_evidence_catalog_or_objects(
        planned,
        &surface.evidence_index,
        &evidence_execution_rows,
    )?;
    let evidence_rows = evidence_execution_rows
        .iter()
        .filter_map(|row| match row {
            ExecutionRow::Evidence(row) => Some(row.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let context_associations = filter_association_context_rows(&associations, planned, options);
    let context_evidence_rows = filter_evidence_context_rows(&evidence_rows, planned, options);
    let all_rows = match &planned.resolved.root {
        ResolvedRoot::Object(root)
            if planned.resolved.method_chain.history.is_some()
                || planned.resolved.method_chain.changes.is_some() =>
        {
            temporal_object_rows(&surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Association(root)
            if planned.resolved.method_chain.history.is_some()
                || planned.resolved.method_chain.changes.is_some() =>
        {
            temporal_association_rows(&surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Object(root) => states
            .iter()
            .filter(|state| state.object_type_id == root.object_type_id)
            .map(MaterializedObjectRow::from_state)
            .map(|row| row.with_output_grain(OutputGrain::LatestState))
            .map(ExecutionRow::Object)
            .collect::<Vec<_>>(),
        ResolvedRoot::Association(root) => associations
            .iter()
            .filter(|association| association.object_type_id == root.object_type_id)
            .filter(|association| {
                association_row_matches_temporal(association, root, association_valid_at(planned))
            })
            .cloned()
            .map(ExecutionRow::Association)
            .collect::<Vec<_>>(),
        ResolvedRoot::Evidence(_) => evidence_execution_rows,
        ResolvedRoot::Projection(_) => unreachable!("projection handled separately"),
    };
    Ok(MaterializedRowSource {
        rows: all_rows,
        associations: context_associations,
        evidence_rows: context_evidence_rows,
        pushdown_report: pushdown_plan.report,
        evidence_authority,
    })
}

fn evidence_authority_for_rows(
    planned: &PlannedQuery,
    has_evidence_index: bool,
    has_evidence_rows: bool,
) -> Option<EvidenceAuthority> {
    if !matches!(planned.resolved.root, ResolvedRoot::Evidence(_)) {
        return None;
    }
    Some(if has_evidence_index {
        EvidenceAuthority::CoveMapMetadata
    } else if has_evidence_rows {
        EvidenceAuthority::MaterializedEvidenceObjects
    } else {
        EvidenceAuthority::Missing
    })
}

pub(crate) fn temporal_object_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    if let Some(changes) = &planned.resolved.method_chain.changes {
        return match changes.mode {
            AstChangeMode::Records => Ok(change_records(surface, planned)?
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .map(MaterializedObjectRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::ChangeRecord))
                .map(ExecutionRow::Object)
                .collect()),
            AstChangeMode::PropertyDiffs => {
                object_property_diff_rows(surface, planned, object_type_id)
            }
            AstChangeMode::StateTransitions => states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::ChangeStateTransition,
            ),
            AstChangeMode::FinalObjects => {
                final_object_rows_for_change_window(surface, planned, object_type_id)
            }
        };
    }
    match planned
        .resolved
        .method_chain
        .history
        .unwrap_or(AstHistoryMode::States)
    {
        AstHistoryMode::Records => Ok(history_records(surface, planned)
            .into_iter()
            .filter(|record| record.object_type_id == object_type_id)
            .filter(|record| include_record_tombstone(record, planned))
            .map(MaterializedObjectRow::from_record)
            .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
            .map(ExecutionRow::Object)
            .collect()),
        AstHistoryMode::States => {
            states_for_records(surface, planned, object_type_id, OutputGrain::HistoryState)
        }
        AstHistoryMode::RecordsAndStates => {
            let mut rows = history_records(surface, planned)
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .map(MaterializedObjectRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
                .map(ExecutionRow::Object)
                .collect::<Vec<_>>();
            rows.extend(states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::HistoryState,
            )?);
            Ok(rows)
        }
    }
}

pub(crate) fn temporal_association_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    if let Some(changes) = &planned.resolved.method_chain.changes {
        return match changes.mode {
            AstChangeMode::Records => Ok(change_records(surface, planned)?
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .filter_map(MaterializedAssociationRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::ChangeRecord))
                .map(ExecutionRow::Association)
                .collect()),
            AstChangeMode::PropertyDiffs => {
                association_property_diff_rows(surface, planned, object_type_id)
            }
            AstChangeMode::StateTransitions => association_states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::ChangeStateTransition,
            ),
            AstChangeMode::FinalObjects => {
                final_association_rows_for_change_window(surface, planned, object_type_id)
            }
        };
    }
    match planned
        .resolved
        .method_chain
        .history
        .unwrap_or(AstHistoryMode::States)
    {
        AstHistoryMode::Records => Ok(history_records(surface, planned)
            .into_iter()
            .filter(|record| record.object_type_id == object_type_id)
            .filter(|record| include_record_tombstone(record, planned))
            .filter_map(MaterializedAssociationRow::from_record)
            .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
            .map(ExecutionRow::Association)
            .collect()),
        AstHistoryMode::States => association_states_for_records(
            surface,
            planned,
            object_type_id,
            OutputGrain::HistoryState,
        ),
        AstHistoryMode::RecordsAndStates => {
            let mut rows = history_records(surface, planned)
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .filter_map(MaterializedAssociationRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
                .map(ExecutionRow::Association)
                .collect::<Vec<_>>();
            rows.extend(association_states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::HistoryState,
            )?);
            Ok(rows)
        }
    }
}

fn history_records<'a>(
    surface: &'a CoveObjectSurface,
    planned: &PlannedQuery,
) -> Vec<&'a CoveObjectRecord> {
    let branch_key = concrete_branch_key(planned);
    let mut records = surface
        .records
        .iter()
        .filter(|record| branch_key.is_none_or(|branch_key| record.branch_key == branch_key))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record_sort_key(record));
    records
}

fn object_states_for_temporal_context(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
) -> Result<Vec<CoveObjectState>, BuildExecutionError> {
    if let TemporalMode::AsOfTimestampMicros(timestamp) = planned.resolved.temporal.mode {
        if let Some(binding) = planned.resolved.temporal.role_binding.as_deref() {
            return role_bound_states_at(surface, planned, binding, timestamp);
        }
    }
    let reconstruction = reconstruction_options(planned)?;
    reconstruct_object_states(surface, &reconstruction).map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O object reconstruction failed: {err}"),
            json!({}),
        )
    })
}

pub(crate) fn role_bound_states_at(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    binding: &str,
    timestamp_micros: i64,
) -> Result<Vec<CoveObjectState>, BuildExecutionError> {
    let branch_key = concrete_branch_key(planned);
    let mut selected = BTreeMap::new();
    for record in &surface.records {
        if branch_key.is_some_and(|branch_key| record.branch_key != branch_key) {
            continue;
        }
        if !include_record_tombstone(record, planned) {
            continue;
        }
        let Some(value) = temporal_binding_value(record, binding)? else {
            continue;
        };
        if value > timestamp_micros {
            continue;
        }
        let key = (record.object_type_id, record.branch_key, record.goid);
        let replace = selected
            .get(&key)
            .is_none_or(|current: &&CoveObjectRecord| {
                record_sort_key(record) > record_sort_key(current)
            });
        if replace {
            selected.insert(key, record);
        }
    }
    let mut states = Vec::new();
    for record in selected.values() {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let reconstructed = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O role-bound reconstruction failed: {err}"),
                json!({ "binding": binding }),
            )
        })?;
        states.extend(
            reconstructed
                .into_iter()
                .filter(|state| state.object_type_id == record.object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id),
        );
    }
    states.sort_by_key(|state| {
        (
            state.object_type_id,
            state.branch_key,
            state.goid,
            state.timestamp_us,
            state.csn,
        )
    });
    Ok(states)
}

fn change_records<'a>(
    surface: &'a CoveObjectSurface,
    planned: &PlannedQuery,
) -> Result<Vec<&'a CoveObjectRecord>, BuildExecutionError> {
    let changes = planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .ok_or_else(|| exec_error("E_EXECUTION", "missing changes context", json!({})))?;
    let branch_key = concrete_branch_key(planned);
    let mut records = Vec::new();
    for record in &surface.records {
        if branch_key.is_some_and(|branch_key| record.branch_key != branch_key) {
            continue;
        }
        if record_in_half_open_bound(record, &changes.from, &changes.to)? {
            records.push(record);
        }
    }
    records.sort_by_key(|record| record_sort_key(record));
    Ok(records)
}

fn states_for_records(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
    output_grain: OutputGrain,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let records = if planned.resolved.method_chain.changes.is_some() {
        change_records(surface, planned)?
    } else {
        history_records(surface, planned)
    };
    let mut rows = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let states = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O history state reconstruction failed: {err}"),
                json!({}),
            )
        })?;
        rows.extend(
            states
                .iter()
                .filter(|state| state.object_type_id == object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id)
                .map(MaterializedObjectRow::from_state)
                .map(|row| row.with_output_grain(output_grain))
                .map(ExecutionRow::Object),
        );
    }
    Ok(rows)
}

fn association_states_for_records(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
    output_grain: OutputGrain,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let records = if planned.resolved.method_chain.changes.is_some() {
        change_records(surface, planned)?
    } else {
        history_records(surface, planned)
    };
    let mut rows = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let states = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O association history state reconstruction failed: {err}"),
                json!({}),
            )
        })?;
        rows.extend(
            states
                .iter()
                .filter(|state| state.object_type_id == object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id)
                .filter_map(MaterializedAssociationRow::from_state)
                .map(|row| row.with_output_grain(output_grain))
                .map(ExecutionRow::Association),
        );
    }
    Ok(rows)
}

fn object_property_diff_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let mut rows = Vec::new();
    for record in change_records(surface, planned)?
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let old_properties = previous_object_properties(surface, planned, record)?;
        let current_row = object_row_for_record_state(surface, planned, record)?
            .unwrap_or_else(|| MaterializedObjectRow::from_record(record))
            .with_output_grain(OutputGrain::ChangePropertyDiff);
        let new_properties =
            row_properties_by_id(&current_row.properties, &current_row.property_ids);
        for change in property_diffs(old_properties, new_properties) {
            rows.push(ExecutionRow::Object(
                current_row.clone().with_change(change),
            ));
        }
    }
    Ok(rows)
}

fn association_property_diff_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let mut rows = Vec::new();
    for record in change_records(surface, planned)?
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let Some(current_row) = association_row_for_record_state(surface, planned, record)?
            .or_else(|| MaterializedAssociationRow::from_record(record))
        else {
            continue;
        };
        let current_row = current_row.with_output_grain(OutputGrain::ChangePropertyDiff);
        let old_properties = previous_object_properties(surface, planned, record)?;
        let new_properties =
            row_properties_by_id(&current_row.properties, &current_row.property_ids);
        for change in property_diffs(old_properties, new_properties) {
            rows.push(ExecutionRow::Association(
                current_row.clone().with_change(change),
            ));
        }
    }
    Ok(rows)
}

fn previous_object_properties(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<BTreeMap<u32, (String, Value)>, BuildExecutionError> {
    if record.csn == 0 {
        return Ok(BTreeMap::new());
    }
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn.saturating_sub(1)),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O property diff previous-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states
        .iter()
        .find(|state| {
            state.object_type_id == record.object_type_id
                && state.branch_key == record.branch_key
                && state.goid == record.goid
        })
        .map(|state| properties_by_id(&state.properties))
        .unwrap_or_default())
}

fn object_row_for_record_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<MaterializedObjectRow>, BuildExecutionError> {
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O property diff current-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states
        .iter()
        .find(|state| {
            state.object_type_id == record.object_type_id
                && state.branch_key == record.branch_key
                && state.goid == record.goid
                && state.latest_record_id == record.record_id
        })
        .map(MaterializedObjectRow::from_state))
}

fn association_row_for_record_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<MaterializedAssociationRow>, BuildExecutionError> {
    Ok(object_row_state(surface, planned, record)?
        .as_ref()
        .and_then(MaterializedAssociationRow::from_state))
}

fn object_row_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<CoveObjectState>, BuildExecutionError> {
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O association property diff current-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states.into_iter().find(|state| {
        state.object_type_id == record.object_type_id
            && state.branch_key == record.branch_key
            && state.goid == record.goid
            && state.latest_record_id == record.record_id
    }))
}

fn properties_by_id(properties: &[CoveObjectPropertyValue]) -> BTreeMap<u32, (String, Value)> {
    properties
        .iter()
        .map(|property| {
            (
                property.property_id,
                (property.property_name.clone(), property.value.clone()),
            )
        })
        .collect()
}

fn row_properties_by_id(
    properties: &BTreeMap<String, Value>,
    property_ids: &BTreeMap<u32, String>,
) -> BTreeMap<u32, (String, Value)> {
    property_ids
        .iter()
        .filter_map(|(property_id, name)| {
            properties
                .get(name)
                .cloned()
                .map(|value| (*property_id, (name.clone(), value)))
        })
        .collect()
}

fn property_diffs(
    old_properties: BTreeMap<u32, (String, Value)>,
    new_properties: BTreeMap<u32, (String, Value)>,
) -> Vec<MaterializedChangeDetail> {
    let property_ids = old_properties
        .keys()
        .chain(new_properties.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    property_ids
        .into_iter()
        .filter_map(|property_id| {
            let old = old_properties.get(&property_id);
            let new = new_properties.get(&property_id);
            match (old, new) {
                (None, None) => None,
                (None, Some((name, new_value))) => Some(MaterializedChangeDetail {
                    property_id,
                    property_name: name.clone(),
                    old_value: Value::Null,
                    new_value: new_value.clone(),
                    diff_kind: MaterializedChangeDiffKind::Added,
                }),
                (Some((name, old_value)), None) => Some(MaterializedChangeDetail {
                    property_id,
                    property_name: name.clone(),
                    old_value: old_value.clone(),
                    new_value: Value::Null,
                    diff_kind: MaterializedChangeDiffKind::Removed,
                }),
                (Some((name, old_value)), Some((new_name, new_value))) => (old_value != new_value)
                    .then(|| MaterializedChangeDetail {
                        property_id,
                        property_name: new_name.clone().if_empty_then(name.clone()),
                        old_value: old_value.clone(),
                        new_value: new_value.clone(),
                        diff_kind: MaterializedChangeDiffKind::Changed,
                    }),
            }
        })
        .collect()
}

trait EmptyStringFallback {
    fn if_empty_then(self, fallback: String) -> String;
}

impl EmptyStringFallback for String {
    fn if_empty_then(self, fallback: String) -> String {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

fn final_object_rows_for_change_window(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let cut = change_to_reconstruction_cut(planned)?;
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: cut,
            branch_key: concrete_branch_key(planned),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O final change reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    let changed_keys = change_records(surface, planned)?
        .into_iter()
        .map(|record| (record.object_type_id, record.branch_key, record.goid))
        .collect::<BTreeSet<_>>();
    Ok(states
        .iter()
        .filter(|state| state.object_type_id == object_type_id)
        .filter(|state| {
            changed_keys.contains(&(state.object_type_id, state.branch_key, state.goid))
        })
        .map(MaterializedObjectRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::FinalObject))
        .map(ExecutionRow::Object)
        .collect())
}

fn final_association_rows_for_change_window(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let cut = change_to_reconstruction_cut(planned)?;
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: cut,
            branch_key: concrete_branch_key(planned),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O final association change reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    let changed_keys = change_records(surface, planned)?
        .into_iter()
        .map(|record| (record.object_type_id, record.branch_key, record.goid))
        .collect::<BTreeSet<_>>();
    Ok(states
        .iter()
        .filter(|state| state.object_type_id == object_type_id)
        .filter(|state| {
            changed_keys.contains(&(state.object_type_id, state.branch_key, state.goid))
        })
        .filter_map(MaterializedAssociationRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::FinalObject))
        .map(ExecutionRow::Association)
        .collect())
}

fn include_record_tombstone(record: &CoveObjectRecord, planned: &PlannedQuery) -> bool {
    planned.resolved.tombstone.include_tombstones
        || record.record_kind != cove_core::profile::cove_o::RecordKind::Tombstone
}

fn concrete_branch_key(planned: &PlannedQuery) -> Option<u64> {
    match planned.resolved.branch.selector {
        crate::BranchSelector::BranchKey(branch) => Some(branch),
        crate::BranchSelector::Default | crate::BranchSelector::RejectAmbiguous => None,
    }
}

fn record_sort_key(
    record: &CoveObjectRecord,
) -> (u32, u64, [u8; 16], i64, u64, u32, u32, [u8; 16]) {
    (
        record.object_type_id,
        record.branch_key,
        record.goid,
        record.timestamp_us,
        record.csn,
        record.segment_id,
        record.row_index,
        record.record_id,
    )
}

fn record_in_half_open_bound(
    record: &CoveObjectRecord,
    from: &ResolvedTimeBound,
    to: &ResolvedTimeBound,
) -> Result<bool, BuildExecutionError> {
    match (from, to) {
        (ResolvedTimeBound::Csn(from), ResolvedTimeBound::Csn(to)) => {
            Ok(record.csn >= *from && record.csn < *to)
        }
        (
            ResolvedTimeBound::TimestampMicros {
                role: from_role,
                binding: from_binding,
                timestamp_micros: from,
                ..
            },
            ResolvedTimeBound::TimestampMicros {
                role: to_role,
                binding: to_binding,
                timestamp_micros: to,
                ..
            },
        ) if from_role == to_role && *from_role == TemporalRole::CommitTime => {
            Ok(record.timestamp_us >= *from && record.timestamp_us < *to)
        }
        (
            ResolvedTimeBound::TimestampMicros {
                binding: from_binding,
                timestamp_micros: from,
                ..
            },
            ResolvedTimeBound::TimestampMicros {
                binding: to_binding,
                timestamp_micros: to,
                ..
            },
        ) => {
            if from_binding != to_binding {
                return Err(exec_error(
                    "E_UNSUPPORTED_TEMPORAL_ROLE",
                    "change windows must use matching temporal role bindings",
                    json!({}),
                ));
            }
            let Some(binding) = from_binding.as_deref() else {
                return Ok(record.timestamp_us >= *from && record.timestamp_us < *to);
            };
            let Some(value) = temporal_binding_value(record, binding)? else {
                return Ok(false);
            };
            Ok(value >= *from && value < *to)
        }
        _ => Err(exec_error(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            "change windows must use matching CSN or timestamp bound types",
            json!({}),
        )),
    }
}

fn temporal_binding_value(
    record: &CoveObjectRecord,
    binding: &str,
) -> Result<Option<i64>, BuildExecutionError> {
    let Some(property) = record
        .properties
        .iter()
        .find(|property| property.property_name == binding)
    else {
        return Ok(None);
    };
    match &property.value {
        Value::Number(number) => number.as_i64().map(Some).ok_or_else(|| {
            exec_error(
                "E_UNSUPPORTED_TEMPORAL_ROLE",
                "temporal role binding value must fit in timestamp micros",
                json!({ "binding": binding }),
            )
        }),
        Value::String(value) => {
            let (timestamp, _) = parse_execution_timestamp_micros(value)?;
            Ok(Some(timestamp))
        }
        Value::Null => Ok(None),
        _ => Err(exec_error(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            "temporal role binding value must be timestamp micros or RFC3339 text",
            json!({ "binding": binding }),
        )),
    }
}

fn parse_execution_timestamp_micros(value: &str) -> Result<(i64, String), BuildExecutionError> {
    let parsed = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| {
            exec_error(
                "E_LITERAL",
                "timestamp literal must be RFC3339 with explicit offset",
                json!({}),
            )
        })?;
    let micros = parsed.unix_timestamp_nanos() / 1_000;
    let micros = i64::try_from(micros).map_err(|_| {
        exec_error(
            "E_LITERAL",
            "timestamp literal is outside supported microsecond range",
            json!({}),
        )
    })?;
    let canonical = parsed
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|err| exec_error("E_LITERAL", err.to_string(), json!({})))?;
    Ok((micros, canonical))
}

fn change_to_reconstruction_cut(
    planned: &PlannedQuery,
) -> Result<CoveObjectTemporalCut, BuildExecutionError> {
    let changes = planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .ok_or_else(|| exec_error("E_EXECUTION", "missing changes context", json!({})))?;
    match &changes.to {
        ResolvedTimeBound::Csn(0) => Ok(CoveObjectTemporalCut::Csn(0)),
        ResolvedTimeBound::Csn(csn) => Ok(CoveObjectTemporalCut::Csn(csn.saturating_sub(1))),
        ResolvedTimeBound::TimestampMicros {
            role,
            timestamp_micros,
            ..
        } if *role == TemporalRole::CommitTime => Ok(CoveObjectTemporalCut::TimestampUs(
            timestamp_micros.saturating_sub(1),
        )),
        ResolvedTimeBound::TimestampMicros {
            timestamp_micros, ..
        } => Ok(CoveObjectTemporalCut::TimestampUs(
            timestamp_micros.saturating_sub(1),
        )),
    }
}

pub(crate) fn execute_projection_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<(CoveOqlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let pushed_filters = projection_filters_for_plan(planned);
    let output_columns = projection_output_columns_for_plan(planned);
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns,
        pushed_filters,
        batch_size: options.batch_size,
    };
    let batches = projected_record_batches_from_cove_o_bytes(
        bytes,
        options.mapping_path.as_deref(),
        projection_id,
        &projection_options,
    )
    .map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("COVE-MAP projection readback failed: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    if matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::ArrowRecordBatch { .. }
    ) && planned.resolved.method_chain.where_predicate.is_none()
        && planned.resolved.method_chain.order_by.is_none()
        && planned.resolved.method_chain.group_by.is_none()
        && planned.resolved.method_chain.skip.is_none()
        && planned.resolved.method_chain.take.is_none()
    {
        let row_count = batches.iter().map(RecordBatch::num_rows).sum();
        return Ok((
            CoveOqlExecutionResult::ArrowRecordBatches(batches),
            ExecutionRowCounts {
                input_rows: row_count,
                filtered_rows: row_count,
                output_rows: row_count,
            },
        ));
    }
    let rows = record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    let rows = rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    finish_materialized_rows(rows, &[], &[], planned, options, started)
}

pub(crate) fn projection_root_execution_rows(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let pushed_filters = projection_filters_for_plan(planned);
    let output_columns = projection_output_columns_for_plan(planned);
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns,
        pushed_filters,
        batch_size: options.batch_size,
    };
    let batches = projected_record_batches_from_cove_o_bytes(
        bytes,
        options.mapping_path.as_deref(),
        projection_id,
        &projection_options,
    )
    .map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("COVE-MAP projection readback failed: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    let rows = record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    Ok(rows.into_iter().map(ExecutionRow::Projection).collect())
}

pub(crate) fn projection_readback_pushdown_report(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    row_counts: &ExecutionRowCounts,
) -> PushdownReport {
    let output_columns = projection_output_columns_for_plan(planned);
    let pushed_filters = projection_filters_for_plan(planned);
    pushdown::projection_readback_report(
        planned,
        &options.pushdown,
        output_columns.as_deref(),
        pushed_filters.len(),
        row_counts.input_rows,
        row_counts.output_rows,
    )
}

fn projection_filters_for_plan(planned: &PlannedQuery) -> Vec<ProjectionFilter> {
    if !matches!(planned.resolved.root, ResolvedRoot::Projection(_)) {
        return Vec::new();
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Vec::new();
    };
    projection_filters_for_predicate(predicate).unwrap_or_default()
}

fn projection_output_columns_for_plan(planned: &PlannedQuery) -> Option<Vec<String>> {
    projection_output_columns_for_parts(&planned.resolved.root, &planned.resolved.method_chain)
}

fn projection_output_columns_for_parts(
    root: &ResolvedRoot,
    method_chain: &ResolvedMethodChain,
) -> Option<Vec<String>> {
    if !matches!(root, ResolvedRoot::Projection(_)) {
        return None;
    }
    let Some(select) = &method_chain.select else {
        return None;
    };
    let mut columns = BTreeSet::new();
    for item in select {
        collect_projection_expr_column(&item.expr, &mut columns);
    }
    if let Some(predicate) = &method_chain.where_predicate {
        collect_projection_predicate_columns(predicate, &mut columns);
    }
    if let Some(order) = &method_chain.order_by {
        collect_projection_expr_column(&order.expr, &mut columns);
    }
    if let Some(group_by) = &method_chain.group_by {
        for expr in group_by {
            collect_projection_expr_column(expr, &mut columns);
        }
    }
    (!columns.is_empty()).then(|| columns.into_iter().collect())
}

fn collect_projection_predicate_columns(
    predicate: &ResolvedPredicate,
    columns: &mut BTreeSet<String>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_projection_expr_column(left, columns);
            collect_projection_expr_column(right, columns);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_projection_expr_column(expr, columns),
        ResolvedPredicate::Not(inner) => collect_projection_predicate_columns(inner, columns),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_projection_predicate_columns(part, columns);
            }
        }
    }
}

fn collect_projection_expr_column(expr: &ResolvedExpr, columns: &mut BTreeSet<String>) {
    match expr {
        ResolvedExpr::Path(path) => {
            if path.projection_column.is_some() {
                columns.insert(projection_column(path));
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_projection_expr_column(arg, columns);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_projection_expr_column(arg, columns);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_projection_predicate_columns(predicate, columns);
            collect_projection_expr_column(then_expr, columns);
            collect_projection_expr_column(else_expr, columns);
        }
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) | ResolvedExpr::Literal(_) => {}
    }
}

fn projection_filters_for_predicate(
    predicate: &ResolvedPredicate,
) -> Option<Vec<ProjectionFilter>> {
    match predicate {
        ResolvedPredicate::And(parts) => {
            let mut out = Vec::new();
            for part in parts {
                out.extend(projection_filters_for_predicate(part)?);
            }
            Some(out)
        }
        ResolvedPredicate::Compare { left, op, right } => {
            projection_compare_filter(left, *op, right).map(|filter| vec![filter])
        }
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            Some(vec![ProjectionFilter::InList {
                column: projection_column(path),
                literals,
            }])
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let path = projection_path(expr)?;
            Some(vec![ProjectionFilter::IsNull {
                column: projection_column(path),
                negated: *negated,
            }])
        }
        ResolvedPredicate::BoolExpr(expr) => {
            projection_bool_filter(expr, true).map(|filter| vec![filter])
        }
        ResolvedPredicate::Not(inner) => projection_filters_for_negated_predicate(inner),
        ResolvedPredicate::Or(parts) => {
            projection_same_column_equality_or_filter(parts).map(|filter| vec![filter])
        }
        ResolvedPredicate::Exists(_) => None,
    }
}

fn projection_filters_for_negated_predicate(
    predicate: &ResolvedPredicate,
) -> Option<Vec<ProjectionFilter>> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            projection_compare_filter(left, negated_compare_op(*op), right)
                .map(|filter| vec![filter])
        }
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            if literals
                .iter()
                .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
            {
                return None;
            }
            Some(
                literals
                    .into_iter()
                    .map(|literal| ProjectionFilter::Compare {
                        column: projection_column(path),
                        op: ProjectionFilterOp::Ne,
                        literal,
                    })
                    .collect(),
            )
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let path = projection_path(expr)?;
            Some(vec![ProjectionFilter::IsNull {
                column: projection_column(path),
                negated: !*negated,
            }])
        }
        ResolvedPredicate::BoolExpr(expr) => {
            projection_bool_filter(expr, false).map(|filter| vec![filter])
        }
        ResolvedPredicate::Not(inner) => projection_filters_for_predicate(inner),
        ResolvedPredicate::And(_) | ResolvedPredicate::Or(_) | ResolvedPredicate::Exists(_) => None,
    }
}

struct ProjectionEqualitySetFilter {
    column: String,
    literals: Vec<ProjectionFilterLiteral>,
}

fn projection_same_column_equality_or_filter(
    parts: &[ResolvedPredicate],
) -> Option<ProjectionFilter> {
    let mut parts = parts.iter();
    let first = projection_single_equality_set_filter(parts.next()?)?;
    let mut column = first.column;
    let mut literals = first.literals;
    for part in parts {
        let filter = projection_single_equality_set_filter(part)?;
        if filter.column != column {
            return None;
        }
        column = filter.column;
        for literal in filter.literals {
            if matches!(literal, ProjectionFilterLiteral::Null) {
                return None;
            }
            if !literals.contains(&literal) {
                literals.push(literal);
            }
        }
    }
    if literals.is_empty()
        || literals
            .iter()
            .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
    {
        return None;
    }
    Some(ProjectionFilter::InList { column, literals })
}

fn projection_single_equality_set_filter(
    predicate: &ResolvedPredicate,
) -> Option<ProjectionEqualitySetFilter> {
    match predicate {
        ResolvedPredicate::Compare {
            left,
            op: AstCompareOp::Eq,
            right,
        } => match projection_compare_filter(left, AstCompareOp::Eq, right)? {
            ProjectionFilter::Compare {
                column,
                op: ProjectionFilterOp::Eq,
                literal,
            } if !matches!(literal, ProjectionFilterLiteral::Null) => {
                Some(ProjectionEqualitySetFilter {
                    column,
                    literals: vec![literal],
                })
            }
            _ => None,
        },
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            if literals.is_empty()
                || literals
                    .iter()
                    .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
            {
                return None;
            }
            Some(ProjectionEqualitySetFilter {
                column: projection_column(path),
                literals,
            })
        }
        ResolvedPredicate::Or(parts) => {
            let ProjectionFilter::InList { column, literals } =
                projection_same_column_equality_or_filter(parts)?
            else {
                return None;
            };
            Some(ProjectionEqualitySetFilter { column, literals })
        }
        _ => None,
    }
}

fn projection_bool_filter(expr: &ResolvedExpr, value: bool) -> Option<ProjectionFilter> {
    let path = projection_path(expr)?;
    matches!(path.logical_type.as_str(), "bool" | "boolean").then(|| ProjectionFilter::Compare {
        column: projection_column(path),
        op: ProjectionFilterOp::Eq,
        literal: ProjectionFilterLiteral::Boolean(value),
    })
}

fn projection_compare_filter(
    left: &ResolvedExpr,
    op: AstCompareOp,
    right: &ResolvedExpr,
) -> Option<ProjectionFilter> {
    if let (Some(path), ResolvedExpr::Literal(literal)) = (projection_path(left), right) {
        return projection_filter_op(op).and_then(|op| {
            Some(ProjectionFilter::Compare {
                column: projection_column(path),
                op,
                literal: projection_filter_literal(literal)?,
            })
        });
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) = (left, projection_path(right)) {
        return projection_filter_op(invert_compare_op(op)).and_then(|op| {
            Some(ProjectionFilter::Compare {
                column: projection_column(path),
                op,
                literal: projection_filter_literal(literal)?,
            })
        });
    }
    None
}

fn projection_path(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    let ResolvedExpr::Path(path) = expr else {
        return None;
    };
    path.projection_column.as_ref()?;
    Some(path)
}

fn projection_column(path: &ResolvedPath) -> String {
    path.projection_column
        .clone()
        .unwrap_or_else(|| path.display_name.clone())
}

fn projection_filter_op(op: AstCompareOp) -> Option<ProjectionFilterOp> {
    match op {
        AstCompareOp::Eq => Some(ProjectionFilterOp::Eq),
        AstCompareOp::Ne => Some(ProjectionFilterOp::Ne),
        AstCompareOp::Lt => Some(ProjectionFilterOp::Lt),
        AstCompareOp::Le => Some(ProjectionFilterOp::LtEq),
        AstCompareOp::Gt => Some(ProjectionFilterOp::Gt),
        AstCompareOp::Ge => Some(ProjectionFilterOp::GtEq),
    }
}

fn invert_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Eq,
        AstCompareOp::Ne => AstCompareOp::Ne,
        AstCompareOp::Lt => AstCompareOp::Gt,
        AstCompareOp::Le => AstCompareOp::Ge,
        AstCompareOp::Gt => AstCompareOp::Lt,
        AstCompareOp::Ge => AstCompareOp::Le,
    }
}

fn negated_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Ne,
        AstCompareOp::Ne => AstCompareOp::Eq,
        AstCompareOp::Lt => AstCompareOp::Ge,
        AstCompareOp::Le => AstCompareOp::Gt,
        AstCompareOp::Gt => AstCompareOp::Le,
        AstCompareOp::Ge => AstCompareOp::Lt,
    }
}

fn projection_filter_literal(literal: &ResolvedLiteral) -> Option<ProjectionFilterLiteral> {
    match &literal.typed_value {
        ResolvedLiteralValue::Null => Some(ProjectionFilterLiteral::Null),
        ResolvedLiteralValue::Boolean(value) => Some(ProjectionFilterLiteral::Boolean(*value)),
        ResolvedLiteralValue::SignedInteger(value) => Some(ProjectionFilterLiteral::Int64(*value)),
        ResolvedLiteralValue::UnsignedInteger(value) => {
            Some(ProjectionFilterLiteral::UInt64(*value))
        }
        ResolvedLiteralValue::BigInteger(_) => None,
        ResolvedLiteralValue::Decimal { canonical, .. } => canonical
            .parse::<f64>()
            .ok()
            .map(ProjectionFilterLiteral::Float64),
        ResolvedLiteralValue::String(value) => Some(ProjectionFilterLiteral::Utf8(value.clone())),
        ResolvedLiteralValue::TimestampMicros { micros, .. } => {
            Some(ProjectionFilterLiteral::Int64(*micros))
        }
        ResolvedLiteralValue::Uuid { canonical_hex, .. } => {
            Some(ProjectionFilterLiteral::Utf8(canonical_hex.clone()))
        }
        ResolvedLiteralValue::Binary {
            canonical_hex,
            bytes,
        } => Some(ProjectionFilterLiteral::Utf8(binary_materialized_value(
            bytes,
            canonical_hex,
        ))),
    }
}

fn binary_materialized_value(bytes: &[u8], canonical_hex: &str) -> String {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .unwrap_or_else(|_| canonical_hex.to_string())
}

pub(crate) fn finish_materialized_rows(
    rows: Vec<ExecutionRow>,
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[MaterializedEvidenceRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(CoveOqlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let context_associations = filter_association_context_rows(associations, planned, options);
    let context_evidence_rows = filter_evidence_context_rows(evidence_rows, planned, options);
    let context = EvalContext::for_plan(&context_associations, &context_evidence_rows, planned);
    let rows = apply_visibility_overlay(rows, planned, options);
    let rows = enforce_redaction_policy(rows, planned)?;
    let rows = enforce_materialized_branch_policy(rows, planned)?;
    let input_rows = rows.len();
    let mut rows = rows
        .into_iter()
        .filter_map(|row| match predicate_matches(&row, planned, &context) {
            Ok(true) => Some(Ok(row)),
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        })
        .collect::<Result<Vec<_>, BuildExecutionError>>()?;
    let filtered_rows = rows.len();
    check_time(&options.resource_budget, started)?;
    if grouped_or_aggregate(planned) {
        let json_rows = aggregate_rows(&rows, planned, &context, options)?;
        return finish_json_rows(
            json_rows,
            input_rows,
            filtered_rows,
            planned,
            options,
            started,
        );
    }
    sort_rows(&mut rows, planned, &context)?;
    let rows = apply_skip_take(rows, planned);
    let output_rows = rows.len();
    match &planned.resolved.output_mode {
        CoveOqlOutputMode::ObjectRows => Ok((
            CoveOqlExecutionResult::ObjectRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Object(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveOqlOutputMode::AssociationRows => Ok((
            CoveOqlExecutionResult::AssociationRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Association(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveOqlOutputMode::EvidenceRows => Ok((
            CoveOqlExecutionResult::EvidenceRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Evidence(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveOqlOutputMode::ProjectionRows => Ok((
            CoveOqlExecutionResult::ProjectionRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Projection(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveOqlOutputMode::JsonRows => {
            let json_rows = select_json_rows(&rows, planned, &context)?;
            finish_json_rows(
                json_rows,
                input_rows,
                filtered_rows,
                planned,
                options,
                started,
            )
        }
        CoveOqlOutputMode::ArrowRecordBatch { .. } => {
            let batch =
                crate::kernel_arrow::execution_rows_to_owned_record_batch(&rows, planned, &context)
                    .map_err(|err| {
                        exec_error(
                            "E_ARROW_OUTPUT",
                            format!("Arrow output materialization failed: {err}"),
                            json!({}),
                        )
                    })?;
            Ok((
                CoveOqlExecutionResult::ArrowRecordBatches(vec![batch]),
                ExecutionRowCounts {
                    input_rows,
                    filtered_rows,
                    output_rows,
                },
            ))
        }
        CoveOqlOutputMode::ExplainJson | CoveOqlOutputMode::DataFusionTableProvider => {
            unreachable!("handled before row execution")
        }
    }
}

fn enforce_materialized_branch_policy(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    if !matches!(
        planned.resolved.branch.selector,
        crate::BranchSelector::RejectAmbiguous
    ) {
        return Ok(rows);
    }
    let branches = rows
        .iter()
        .filter_map(execution_row_branch_key)
        .collect::<BTreeSet<_>>();
    if branches.len() > 1 {
        return Err(exec_error(
            "E_AMBIGUOUS_BRANCH",
            "branch(reject_ambiguous) matched multiple branch keys",
            json!({ "branch_count": branches.len() }),
        ));
    }
    Ok(rows)
}

fn execution_row_branch_key(row: &ExecutionRow) -> Option<u64> {
    match row {
        ExecutionRow::Object(row) => Some(row.branch_key),
        ExecutionRow::Association(row) => Some(row.branch_key),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => None,
    }
}

fn apply_visibility_overlay(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Vec<ExecutionRow> {
    if !matches!(
        planned
            .resolved
            .operation_context
            .security
            .visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) {
        return rows;
    }
    let Some(overlay) = &options.visibility_overlay else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|row| row_visible_in_overlay(row, overlay))
        .collect()
}

fn enforce_redaction_policy(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    match planned.resolved.operation_context.security.redaction_policy {
        RedactionPolicy::ProtectedValuesRedacted => Ok(rows),
        RedactionPolicy::RefuseProtectedValues => {
            if rows.iter().any(execution_row_has_redacted_values) {
                return Err(exec_error(
                    "E_SECURITY_DISCLOSURE_FORBIDDEN",
                    "redacted values are present but the active redaction policy refuses protected values",
                    json!({}),
                ));
            }
            Ok(rows)
        }
    }
}

fn execution_row_has_redacted_values(row: &ExecutionRow) -> bool {
    match row {
        ExecutionRow::Object(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Association(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => false,
    }
}

fn row_visible_in_overlay(row: &ExecutionRow, overlay: &VisibilityOverlay) -> bool {
    match row {
        ExecutionRow::Object(row) => {
            overlay.visible_goids.contains(&row.goid)
                || overlay.visible_record_ids.contains(&row.record_id)
        }
        ExecutionRow::Association(row) => {
            overlay.visible_goids.contains(&row.goid)
                || overlay.visible_record_ids.contains(&row.record_id)
        }
        ExecutionRow::Evidence(row) => evidence_row_visible_in_overlay(row, overlay),
        ExecutionRow::Projection(_) => false,
    }
}

fn filter_association_context_rows(
    rows: &[MaterializedAssociationRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Vec<MaterializedAssociationRow> {
    let Some(overlay) = active_visibility_overlay(planned, options) else {
        return rows.to_vec();
    };
    rows.iter()
        .filter(|row| association_row_visible_in_overlay(row, overlay))
        .cloned()
        .collect()
}

fn filter_evidence_context_rows(
    rows: &[MaterializedEvidenceRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Vec<MaterializedEvidenceRow> {
    let Some(overlay) = active_visibility_overlay(planned, options) else {
        return rows.to_vec();
    };
    rows.iter()
        .filter(|row| evidence_row_visible_in_overlay(row, overlay))
        .cloned()
        .collect()
}

fn active_visibility_overlay<'a>(
    planned: &PlannedQuery,
    options: &'a ExecutionOptions,
) -> Option<&'a VisibilityOverlay> {
    if !matches!(
        planned
            .resolved
            .operation_context
            .security
            .visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) {
        return None;
    }
    options.visibility_overlay.as_ref()
}

fn association_row_visible_in_overlay(
    row: &MaterializedAssociationRow,
    overlay: &VisibilityOverlay,
) -> bool {
    overlay.visible_goids.contains(&row.goid) || overlay.visible_record_ids.contains(&row.record_id)
}

fn evidence_row_visible_in_overlay(
    row: &MaterializedEvidenceRow,
    overlay: &VisibilityOverlay,
) -> bool {
    evidence_field_visible_in_overlay(
        row,
        &["evidence_id", "goid", "object_id"],
        &overlay.visible_goids,
    ) || evidence_field_visible_in_overlay(
        row,
        &["record_id", "latest_record_id"],
        &overlay.visible_record_ids,
    )
}

fn evidence_field_visible_in_overlay(
    row: &MaterializedEvidenceRow,
    field_names: &[&str],
    visible_ids: &BTreeSet<String>,
) -> bool {
    field_names.iter().any(|field| {
        row.fields
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|id| visible_ids.contains(id))
    })
}

pub(crate) fn finish_json_rows(
    json_rows: Vec<Value>,
    input_rows: usize,
    filtered_rows: usize,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(CoveOqlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let output_rows = json_rows.len();
    if matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::ArrowRecordBatch { .. }
    ) {
        let batch =
            crate::kernel_arrow::json_rows_to_owned_record_batch_for_plan(&json_rows, planned)
                .map_err(|err| {
                    exec_error(
                        "E_ARROW_OUTPUT",
                        format!("Arrow output materialization failed: {err}"),
                        json!({}),
                    )
                })?;
        Ok((
            CoveOqlExecutionResult::ArrowRecordBatches(vec![batch]),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        ))
    } else {
        Ok((
            CoveOqlExecutionResult::JsonRows(json_rows),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        ))
    }
}

pub(crate) fn predicate_matches(
    row: &ExecutionRow,
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<bool, BuildExecutionError> {
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(true);
    };
    eval_predicate(predicate, row, context).map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!("materialized predicate evaluation failed: {}", err.message),
            json!({}),
        )
    })
}

pub(crate) fn select_json_rows(
    rows: &[ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<Vec<Value>, BuildExecutionError> {
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(execution_rows_to_json(rows));
    };
    rows.iter()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for item in select {
                if matches!(item.expr, ResolvedExpr::AggregateCall { .. }) {
                    return Err(exec_error(
                        "E_EXPRESSION",
                        "aggregate expression reached row projection without aggregate execution",
                        json!({}),
                    ));
                }
                let key = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| output_name_for_expr(&item.expr));
                let value = eval_expr(&item.expr, row, context).map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!("materialized expression evaluation failed: {}", err.message),
                        json!({ "column": key }),
                    )
                })?;
                object.insert(key, value);
            }
            Ok(Value::Object(object))
        })
        .collect()
}

fn aggregate_rows(
    rows: &[ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
    options: &ExecutionOptions,
) -> Result<Vec<Value>, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact
        | AggregateDisclosurePolicy::AllowMaterializedOnly
        | AggregateDisclosurePolicy::AllowThresholded
        | AggregateDisclosurePolicy::AllowRedacted => {}
        AggregateDisclosurePolicy::Reject => {
            return Err(exec_error(
                "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
                "aggregate disclosure is rejected by the active security context",
                json!({}),
            ))
        }
    }
    let group_by = planned
        .resolved
        .method_chain
        .group_by
        .clone()
        .unwrap_or_default();
    let mut groups = BTreeMap::<String, Vec<&ExecutionRow>>::new();
    for row in rows {
        let key_values = group_by
            .iter()
            .map(|expr| eval_expr(expr, row, context))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("group expression evaluation failed: {}", err.message),
                    json!({}),
                )
            })?;
        let key = serde_json::to_string(&key_values).unwrap_or_else(|_| "[]".into());
        groups.entry(key).or_default().push(row);
    }
    if group_by.is_empty() {
        groups
            .entry("[]".into())
            .or_insert_with(|| rows.iter().collect());
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }
    let select = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .ok_or_else(|| {
            exec_error(
                "E_AGGREGATE",
                "aggregate query requires select output",
                json!({}),
            )
        })?;
    let mut out = Vec::new();
    for group_rows in groups.into_values() {
        let representative = group_rows.first().copied();
        let mut object = serde_json::Map::new();
        for item in select {
            let key = item
                .alias
                .clone()
                .unwrap_or_else(|| output_name_for_expr(&item.expr));
            let value = match &item.expr {
                ResolvedExpr::AggregateCall {
                    name, arg, star, ..
                } => aggregate_disclosed_value(
                    *name,
                    arg.as_deref(),
                    *star,
                    &group_rows,
                    context,
                    planned,
                )?,
                expr => match representative {
                    Some(row) => eval_expr(expr, row, context).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!("grouped expression evaluation failed: {}", err.message),
                            json!({ "column": key }),
                        )
                    })?,
                    None => Value::Null,
                },
            };
            object.insert(key, value);
        }
        out.push(Value::Object(object));
    }
    Ok(out)
}

fn aggregate_disclosed_value(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    star: bool,
    rows: &[&ExecutionRow],
    context: &EvalContext<'_>,
    planned: &PlannedQuery,
) -> Result<Value, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact
        | AggregateDisclosurePolicy::AllowMaterializedOnly => {
            aggregate_value(name, arg, star, rows, context)
        }
        AggregateDisclosurePolicy::AllowThresholded => {
            let threshold = planned
                .resolved
                .operation_context
                .security
                .aggregate_disclosure_threshold
                .unwrap_or(1);
            if rows.len() as u64 >= threshold {
                aggregate_value(name, arg, star, rows, context)
            } else {
                Ok(aggregate_policy_marker(
                    "thresholded",
                    "suppressed",
                    Some(threshold),
                ))
            }
        }
        AggregateDisclosurePolicy::AllowRedacted => {
            Ok(aggregate_policy_marker("redacted", "redacted", None))
        }
        AggregateDisclosurePolicy::Reject => Err(exec_error(
            "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
            "aggregate disclosure is rejected by the active security context",
            json!({}),
        )),
    }
}

fn aggregate_policy_marker(policy: &str, status: &str, threshold: Option<u64>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("policy".into(), Value::String(policy.into()));
    object.insert("status".into(), Value::String(status.into()));
    if let Some(threshold) = threshold {
        object.insert("threshold".into(), json!(threshold));
    }
    Value::Object(object)
}

fn aggregate_value(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    star: bool,
    rows: &[&ExecutionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    if !star {
        if let Some(ResolvedExpr::Association(association)) = arg {
            return Ok(match name {
                AstAggregateName::Count => json!(rows
                    .iter()
                    .map(|row| context.association_count_for(row, association) as u64)
                    .sum::<u64>()),
                AstAggregateName::Exists => Value::Bool(
                    rows.iter()
                        .any(|row| context.association_count_for(row, association) > 0),
                ),
                AstAggregateName::DistinctCount => {
                    let mut distinct = BTreeSet::new();
                    for row in rows {
                        distinct.extend(context.association_endpoint_keys_for(row, association));
                    }
                    json!(distinct.len() as u64)
                }
                _ => return Err(exec_error(
                    "E_AGGREGATE",
                    "association helper supports count, exists, and distinctCount aggregates only",
                    json!({}),
                )),
            });
        }
        if let Some(ResolvedExpr::Evidence(evidence)) = arg {
            return Ok(match name {
                AstAggregateName::Count => json!(rows
                    .iter()
                    .map(|row| context.evidence_count_for(row, evidence) as u64)
                    .sum::<u64>()),
                AstAggregateName::Exists => Value::Bool(
                    rows.iter()
                        .any(|row| context.evidence_count_for(row, evidence) > 0),
                ),
                AstAggregateName::DistinctCount => {
                    let mut distinct = BTreeSet::new();
                    for row in rows {
                        distinct.extend(context.evidence_identity_keys_for(row, evidence));
                    }
                    json!(distinct.len() as u64)
                }
                _ => {
                    return Err(exec_error(
                        "E_AGGREGATE",
                        "evidence helper supports count, exists, and distinctCount aggregates only",
                        json!({}),
                    ))
                }
            });
        }
    }
    let values = if star {
        Vec::new()
    } else if let Some(arg) = arg {
        rows.iter()
            .map(|row| eval_expr(arg, row, context))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("aggregate argument evaluation failed: {}", err.message),
                    json!({}),
                )
            })?
    } else {
        Vec::new()
    };
    Ok(match name {
        AstAggregateName::Count => {
            if star {
                json!(rows.len() as u64)
            } else {
                json!(values.iter().filter(|value| !value.is_null()).count() as u64)
            }
        }
        AstAggregateName::Exists => Value::Bool(if star {
            !rows.is_empty()
        } else {
            values.iter().any(|value| !value.is_null())
        }),
        AstAggregateName::DistinctCount => json!(distinct_count(
            values.into_iter().filter(|value| !value.is_null()),
            arg.and_then(expr_logical_type)
        )),
        AstAggregateName::Sum | AstAggregateName::Avg => numeric_aggregate(name, arg, &values)?,
        AstAggregateName::Min | AstAggregateName::Max => ordered_aggregate(name, arg, &values),
    })
}

fn numeric_aggregate(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    values: &[Value],
) -> Result<Value, BuildExecutionError> {
    let mut exact_sum: Option<ExactDecimal> = None;
    let mut float_sum = 0.0f64;
    let mut saw_float = false;
    let mut count = 0usize;
    let logical_type = arg.and_then(expr_logical_type);
    for value in values.iter().filter(|value| !value.is_null()) {
        let numeric_is_float = matches!(logical_type, Some("float32" | "float64"))
            || value
                .as_number()
                .is_some_and(|number| number.as_i64().is_none() && number.as_u64().is_none());
        if numeric_is_float {
            let Some(number) = value.as_f64() else {
                return Err(exec_error(
                    "E_AGGREGATE",
                    "sum/avg aggregate requires numeric materialized values",
                    json!({}),
                ));
            };
            saw_float = true;
            float_sum += number;
        } else if let Some(decimal) = parse_decimal_value(value) {
            exact_sum = Some(match exact_sum {
                Some(sum) => sum.checked_add(decimal).ok_or_else(|| {
                    exec_error(
                        "E_AGGREGATE",
                        "sum/avg aggregate overflowed decimal accumulator",
                        json!({}),
                    )
                })?,
                None => decimal,
            });
        } else {
            return Err(exec_error(
                "E_AGGREGATE",
                "sum/avg aggregate requires numeric materialized values",
                json!({}),
            ));
        }
        count += 1;
    }
    if count == 0 {
        return Ok(Value::Null);
    }
    match name {
        AstAggregateName::Sum if saw_float => Ok(json!(float_sum)),
        AstAggregateName::Avg if saw_float => Ok(json!(float_sum / count as f64)),
        AstAggregateName::Sum => exact_sum
            .ok_or_else(|| exec_error("E_AGGREGATE", "empty numeric accumulator", json!({})))?
            .to_json_sum()
            .map_err(|message| exec_error("E_AGGREGATE", message, json!({}))),
        AstAggregateName::Avg => exact_sum
            .ok_or_else(|| exec_error("E_AGGREGATE", "empty numeric accumulator", json!({})))?
            .checked_div_u64(count as u64)
            .ok_or_else(|| {
                exec_error(
                    "E_AGGREGATE",
                    "avg aggregate overflowed decimal accumulator",
                    json!({}),
                )
            })?
            .to_json_sum()
            .map_err(|message| exec_error("E_AGGREGATE", message, json!({}))),
        _ => unreachable!("guarded by caller"),
    }
}

fn ordered_aggregate(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    values: &[Value],
) -> Value {
    let mut selected: Option<Value> = None;
    let logical_type = arg.and_then(expr_logical_type);
    let collation_id = arg.and_then(expr_collation_id);
    for value in values.iter().filter(|value| !value.is_null()) {
        let replace = match &selected {
            None => true,
            Some(current) => value_ordering_typed(value, current, logical_type, collation_id)
                .is_some_and(|ordering| match name {
                    AstAggregateName::Min => ordering.is_lt(),
                    AstAggregateName::Max => ordering.is_gt(),
                    _ => false,
                }),
        };
        if replace {
            selected = Some(value.clone());
        }
    }
    selected.unwrap_or(Value::Null)
}

pub(crate) fn sort_rows(
    rows: &mut [ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<(), BuildExecutionError> {
    let Some(order) = &planned.resolved.method_chain.order_by else {
        if planned.logical_plan.default_ordering_applied {
            let fields = default_sort_fields(planned);
            rows.sort_by(|left, right| {
                compare_default_sort_fields(left, right, &fields)
                    .then_with(|| compare_manifest_member_order(left, right, planned))
                    .then_with(|| {
                        stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json()))
                    })
            });
        }
        return Ok(());
    };
    let logical_type = expr_logical_type(&order.expr);
    let collation_id = expr_collation_id(&order.expr);
    rows.sort_by(|left, right| {
        let left_value = eval_expr(&order.expr, left, context).unwrap_or(Value::Null);
        let right_value = eval_expr(&order.expr, right, context).unwrap_or(Value::Null);
        compare_sort_values(
            &left_value,
            &right_value,
            order.direction,
            order.nulls,
            logical_type,
            collation_id,
        )
        .then_with(|| compare_manifest_member_order(left, right, planned))
        .then_with(|| stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json())))
    });
    Ok(())
}

fn compare_manifest_member_order(
    left: &ExecutionRow,
    right: &ExecutionRow,
    planned: &PlannedQuery,
) -> Ordering {
    if planned.resolved.operation_context.dataset.files.len() <= 1 {
        return Ordering::Equal;
    }
    left.dataset_file_ordinal()
        .cmp(&right.dataset_file_ordinal())
}

fn compare_sort_values(
    left: &Value,
    right: &Value,
    direction: AstOrderDirection,
    nulls: AstNullOrdering,
    logical_type: Option<&str>,
    collation_id: Option<u16>,
) -> Ordering {
    let nulls = match nulls {
        AstNullOrdering::Default => match direction {
            AstOrderDirection::Asc => AstNullOrdering::NullsLast,
            AstOrderDirection::Desc => AstNullOrdering::NullsFirst,
        },
        explicit => explicit,
    };
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Less,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Greater,
        },
        (false, true) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Greater,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Less,
        },
        (false, false) => {
            let ordering = value_ordering_typed(left, right, logical_type, collation_id)
                .unwrap_or_else(|| stable_value_key(left).cmp(&stable_value_key(right)));
            match direction {
                AstOrderDirection::Asc => ordering,
                AstOrderDirection::Desc => ordering.reverse(),
            }
        }
    }
}

fn default_sort_fields(planned: &PlannedQuery) -> Vec<String> {
    planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::logical_plan::LogicalPlanNodeKind::Sort {
                keys,
                defaulted: true,
                ..
            } => Some(
                keys.iter()
                    .filter_map(|key| key.field.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn compare_default_sort_fields(
    left: &ExecutionRow,
    right: &ExecutionRow,
    fields: &[String],
) -> Ordering {
    for field in fields {
        let left_value = default_sort_field_value(left, field);
        let right_value = default_sort_field_value(right, field);
        let ordering = compare_sort_values(
            &left_value,
            &right_value,
            AstOrderDirection::Asc,
            AstNullOrdering::NullsLast,
            None,
            None,
        );
        if !ordering.is_eq() {
            return ordering;
        }
    }
    Ordering::Equal
}

fn default_sort_field_value(row: &ExecutionRow, field: &str) -> Value {
    if field == "dataset_file_ordinal" || field == "manifest_file_ordinal" {
        return row
            .dataset_file_ordinal()
            .map(|ordinal| json!(ordinal))
            .unwrap_or(Value::Null);
    }
    match row {
        ExecutionRow::Object(row) => match field {
            "object_type_id" => json!(row.object_type_id),
            "branch_key" => json!(row.branch_key),
            "goid" | "association_goid" => Value::String(row.goid.clone()),
            "timestamp_us" => json!(row.timestamp_us),
            "csn" => json!(row.csn),
            "record_id" => Value::String(row.record_id.clone()),
            "output_grain" => json!(row.output_grain),
            _ => Value::Null,
        },
        ExecutionRow::Association(row) => match field {
            "object_type_id" | "association_type_id" => json!(row.object_type_id),
            "branch_key" => json!(row.branch_key),
            "goid" | "association_goid" => Value::String(row.goid.clone()),
            "source_goid" => row
                .source_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "target_goid" => row
                .target_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "timestamp_us" => json!(row.timestamp_us),
            "csn" => json!(row.csn),
            "record_id" => Value::String(row.record_id.clone()),
            "output_grain" => json!(row.output_grain),
            _ => Value::Null,
        },
        ExecutionRow::Evidence(row) => evidence_default_sort_field_value(row, field),
        ExecutionRow::Projection(row) => row.values.get(field).cloned().unwrap_or(Value::Null),
    }
}

fn evidence_default_sort_field_value(row: &MaterializedEvidenceRow, field: &str) -> Value {
    row.fields
        .get(field)
        .or_else(|| match field {
            "target_id" => row.fields.get("output_object_id"),
            "source_system" => row.fields.get("source_id"),
            "evidence_id" => row.fields.get("assertion_id"),
            _ => None,
        })
        .cloned()
        .unwrap_or(Value::Null)
}

pub(crate) fn apply_skip_take(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
) -> Vec<ExecutionRow> {
    let skip = planned.resolved.method_chain.skip.unwrap_or(0) as usize;
    let take = planned
        .resolved
        .method_chain
        .take
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    rows.into_iter().skip(skip).take(take).collect()
}

fn require_evidence_catalog_or_objects(
    planned: &PlannedQuery,
    index: &Option<cove_core::profile::cove_map::MapEvidenceIndex>,
    rows: &[ExecutionRow],
) -> Result<(), BuildExecutionError> {
    if matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
        && index.is_none()
        && rows.is_empty()
    {
        return Err(exec_error(
            "E_EVIDENCE_CATALOG_REQUIRED",
            "evidence roots require embedded COVE-MAP evidence metadata or materialized COVE-O evidence objects",
            json!({}),
        ));
    }
    Ok(())
}

pub(crate) fn evidence_object_rows_from_states(states: &[CoveObjectState]) -> Vec<ExecutionRow> {
    states
        .iter()
        .filter(|state| state.object_type_flags & OBJECT_TYPE_FLAG_EVIDENCE_OBJECT != 0)
        .map(|state| {
            let mut fields = BTreeMap::new();
            fields.insert("evidence_id".into(), Value::String(hex(&state.goid)));
            fields.insert(
                "record_id".into(),
                Value::String(hex(&state.latest_record_id)),
            );
            fields.insert("object_type_id".into(), json!(state.object_type_id));
            fields.insert(
                "object_type_name".into(),
                Value::String(state.object_type_name.clone()),
            );
            fields.insert("branch_key".into(), json!(state.branch_key));
            fields.insert("timestamp_us".into(), json!(state.timestamp_us));
            fields.insert("csn".into(), json!(state.csn));
            fields.insert("grain".into(), Value::String("object".into()));
            for property in &state.properties {
                insert_evidence_property(&mut fields, property);
            }
            ExecutionRow::Evidence(MaterializedEvidenceRow { fields })
        })
        .collect()
}

fn insert_evidence_property(
    fields: &mut BTreeMap<String, Value>,
    property: &CoveObjectPropertyValue,
) {
    fields.insert(property.property_name.clone(), property.value.clone());
    if property.flags & PROPERTY_FLAG_EVIDENCE_REF != 0 {
        fields.insert("source_evidence_id".into(), property.value.clone());
    }
    if property.flags & PROPERTY_FLAG_MAPPING_RULE_REF != 0 {
        fields.insert("rule_id".into(), property.value.clone());
    }
}

pub(crate) fn object_read_options(planned: &PlannedQuery) -> CoveObjectReadOptions {
    CoveObjectReadOptions {
        requested_property_ids: planned.dependencies.property_ids.iter().copied().collect(),
        requested_property_names: planned
            .dependencies
            .temporal_role_bindings
            .iter()
            .cloned()
            .collect(),
        requested_object_type_names: planned
            .dependencies
            .object_type_names
            .iter()
            .cloned()
            .collect(),
        requested_evidence_metadata_keys: planned
            .dependencies
            .evidence_fields
            .iter()
            .filter_map(|field| {
                field
                    .strip_prefix("operation_metadata:")
                    .map(str::to_string)
            })
            .collect(),
        include_projection_catalog: true,
        include_function_registry: true,
        include_association_object_types: !planned.dependencies.association_type_ids.is_empty()
            || matches!(planned.resolved.root, ResolvedRoot::Association(_)),
        include_records: true,
        include_evidence_index: matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
            || !planned.dependencies.evidence_fields.is_empty(),
        redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
    }
}

pub(crate) fn reconstruction_options(
    planned: &PlannedQuery,
) -> Result<CoveObjectReconstructionOptions, BuildExecutionError> {
    let temporal_cut = match planned.resolved.temporal.mode {
        TemporalMode::Latest => CoveObjectTemporalCut::LatestCommitted,
        TemporalMode::AsOfCsn(csn) => CoveObjectTemporalCut::Csn(csn),
        TemporalMode::AsOfTimestampMicros(timestamp) => {
            if planned.resolved.temporal.role == TemporalRole::AssociationValidTime {
                CoveObjectTemporalCut::LatestCommitted
            } else if matches!(
                planned.resolved.temporal.role,
                TemporalRole::ValidTime
                    | TemporalRole::ObservedTime
                    | TemporalRole::SourceEventTime
            ) {
                CoveObjectTemporalCut::TimestampUs(timestamp)
            } else {
                if planned.resolved.temporal.role != TemporalRole::CommitTime {
                    return Err(exec_error(
                        "E_UNSUPPORTED_TEMPORAL_ROLE",
                        "Phase 3 materialized readback supports commit-time timestamp cuts only",
                        json!({}),
                    ));
                }
                CoveObjectTemporalCut::TimestampUs(timestamp)
            }
        }
        TemporalMode::HistoryRecords
        | TemporalMode::HistoryStates
        | TemporalMode::HistoryRecordsAndStates
        | TemporalMode::ChangesRecords
        | TemporalMode::ChangesStateTransitions
        | TemporalMode::ChangesPropertyDiffs
        | TemporalMode::ChangesFinalObjects => CoveObjectTemporalCut::LatestCommitted,
    };
    let branch_key = match planned.resolved.branch.selector {
        crate::BranchSelector::Default | crate::BranchSelector::RejectAmbiguous => None,
        crate::BranchSelector::BranchKey(branch) => Some(branch),
    };
    Ok(CoveObjectReconstructionOptions {
        temporal_cut,
        branch_key,
        include_tombstones: planned.resolved.tombstone.include_tombstones,
    })
}

pub(crate) fn validate_security_scope(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Result<(), BuildExecutionError> {
    if let VisibilityPolicy::ExternalOverlay(required_id) = &planned
        .resolved
        .operation_context
        .security
        .visibility_policy
    {
        let Some(overlay) = &options.visibility_overlay else {
            return Err(exec_error(
                "E_VISIBILITY_OVERLAY_UNAVAILABLE",
                "external visibility policy requires a matching execution overlay",
                json!({ "overlay_id": required_id }),
            ));
        };
        if &overlay.overlay_id != required_id {
            return Err(exec_error(
                "E_VISIBILITY_OVERLAY_MISMATCH",
                "external visibility overlay id does not match the resolved security policy",
                json!({ "required_overlay_id": required_id, "provided_overlay_id": overlay.overlay_id }),
            ));
        }
    }
    if zero_copy_arrow_requested(planned)
        && planned.resolved.operation_context.request.fallback_policy
            == FallbackPolicy::RejectOnFallback
    {
        let reason = zero_copy_owned_fallback_reason(planned);
        return Err(exec_error(
            "E_ZERO_COPY_UNSUPPORTED",
            "zero-copy Arrow output could not be proven by materialized execution; owned buffers would be required",
            json!({ "fallback_policy": "reject_on_fallback", "reason": reason }),
        ));
    }
    validate_association_evidence_disclosure(planned)?;
    Ok(())
}

pub(crate) fn zero_copy_owned_fallback_warning(
    planned: &PlannedQuery,
) -> Option<ExecutionDiagnostic> {
    if zero_copy_arrow_requested(planned) {
        let reason = zero_copy_owned_fallback_reason(planned);
        Some(exec_warning(
            "W_ZERO_COPY_MATERIALIZED_FALLBACK",
            "zero-copy Arrow output was requested, but Cove-OQL execution emitted owned materialized buffers",
            json!({ "fallback": "materialized_owned_arrow_buffers", "reason": reason }),
        ))
    } else {
        None
    }
}

fn zero_copy_owned_fallback_reason(planned: &PlannedQuery) -> String {
    if !matches!(planned.resolved.root, ResolvedRoot::Object(_)) {
        return "zero-copy retained object-page export supports object roots only".into();
    }
    if planned.resolved.method_chain.where_predicate.is_some() {
        return "filters require materialized residual evaluation before Arrow export".into();
    }
    if planned.resolved.method_chain.order_by.is_some() {
        return "sorting requires materialized row ordering before Arrow export".into();
    }
    if planned.resolved.method_chain.skip.is_some() || planned.resolved.method_chain.take.is_some()
    {
        return "paging requires materialized row selection before Arrow export".into();
    }
    if planned.resolved.method_chain.history.is_some() {
        return "history output is not eligible for retained object-page zero-copy export".into();
    }
    if planned.resolved.method_chain.changes.is_some() {
        return "changes output is not eligible for retained object-page zero-copy export".into();
    }
    if planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .is_none_or(|select| {
            select.is_empty()
                || select
                    .iter()
                    .any(|item| !matches!(item.expr, ResolvedExpr::Path(_)))
        })
    {
        return "zero-copy retained object-page export requires explicit direct property projections"
            .into();
    }
    "borrowed materialized execution cannot prove retained COVE-L page ownership, nullable page polarity, or no-null NumCode page compatibility; use retained physical execution with a validated zero-copy buffer map".into()
}

fn zero_copy_arrow_requested(planned: &PlannedQuery) -> bool {
    matches!(
        planned.resolved.output_mode,
        CoveOqlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true
        }
    )
}

fn validate_association_evidence_disclosure(
    planned: &PlannedQuery,
) -> Result<(), BuildExecutionError> {
    let allow_protected = planned
        .resolved
        .operation_context
        .security
        .metadata_disclosure_policy
        == MetadataDisclosurePolicy::AllowProtected;
    if allow_protected {
        return Ok(());
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        validate_predicate_disclosure(predicate, false)?;
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            validate_expr_disclosure(&item.expr)?;
        }
    }
    Ok(())
}

fn validate_predicate_disclosure(
    predicate: &ResolvedPredicate,
    negated: bool,
) -> Result<(), BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::Association(_)) if negated => Err(exec_error(
            "E_PROTECTED_ASSOCIATION_EXISTENCE",
            "association non-existence disclosure requires protected metadata disclosure permission",
            json!({}),
        )),
        ResolvedPredicate::Exists(ResolvedExpr::Evidence(_)) => Err(exec_error(
            "E_PROTECTED_EVIDENCE_EXISTENCE",
            "evidence existence disclosure requires protected metadata disclosure permission",
            json!({}),
        )),
        ResolvedPredicate::Not(inner) => validate_predicate_disclosure(inner, !negated),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                validate_predicate_disclosure(part, negated)?;
            }
            Ok(())
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            validate_expr_disclosure(left)?;
            validate_expr_disclosure(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr) => validate_expr_disclosure(expr),
        _ => Ok(()),
    }
}

fn validate_expr_disclosure(expr: &ResolvedExpr) -> Result<(), BuildExecutionError> {
    match expr {
        ResolvedExpr::AggregateCall {
            name,
            arg: Some(arg),
            ..
        } if matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) && matches!(arg.as_ref(), ResolvedExpr::Association(_)) =>
        {
            Err(exec_error(
                "E_PROTECTED_ASSOCIATION_EXISTENCE",
                "association count disclosure requires protected metadata disclosure permission",
                json!({}),
            ))
        }
        ResolvedExpr::AggregateCall {
            name,
            arg: Some(arg),
            ..
        } if matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) && matches!(arg.as_ref(), ResolvedExpr::Evidence(_)) =>
        {
            Err(exec_error(
                "E_PROTECTED_EVIDENCE_EXISTENCE",
                "evidence count disclosure requires protected metadata disclosure permission",
                json!({}),
            ))
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                validate_expr_disclosure(arg)?;
            }
            Ok(())
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                validate_expr_disclosure(arg)?;
            }
            Ok(())
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            validate_predicate_disclosure(predicate, false)?;
            validate_expr_disclosure(then_expr)?;
            validate_expr_disclosure(else_expr)
        }
        _ => Ok(()),
    }
}

pub(crate) fn validate_execution_grain(planned: &PlannedQuery) -> Result<(), BuildExecutionError> {
    validate_execution_output_mode(planned)?;
    if planned.resolved.operation_context.dataset.files.len() > 1
        && !matches!(
            planned.resolved.output_mode,
            CoveOqlOutputMode::ExplainJson | CoveOqlOutputMode::DataFusionTableProvider
        )
    {
        return Err(exec_error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            "multi-file dataset scopes require manifest-member execution; the single-input OQL executor refuses to run a manifest-scoped plan against one file",
            json!({
                "dataset_id": planned.resolved.operation_context.dataset.dataset_id.clone(),
                "manifest_id": planned.resolved.operation_context.dataset.manifest_id.clone(),
                "file_count": planned.resolved.operation_context.dataset.files.len(),
                "file_membership_fingerprint": planned.resolved.operation_context.dataset.file_membership_fingerprint.clone(),
            }),
        ));
    }

    if matches!(
        planned.resolved.root,
        ResolvedRoot::Projection(_) | ResolvedRoot::Evidence(_)
    ) && (planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some())
    {
        return Err(incompatible_execution_grain(
            planned,
            "history and changes output grains require object or association roots",
        ));
    }

    Ok(())
}

pub(crate) fn validate_execution_output_mode(
    planned: &PlannedQuery,
) -> Result<(), BuildExecutionError> {
    let output_valid = match (&planned.resolved.root, &planned.resolved.output_mode) {
        (_, CoveOqlOutputMode::JsonRows)
        | (_, CoveOqlOutputMode::ArrowRecordBatch { .. })
        | (_, CoveOqlOutputMode::ExplainJson) => true,
        (ResolvedRoot::Object(_), CoveOqlOutputMode::ObjectRows) => true,
        (ResolvedRoot::Association(_), CoveOqlOutputMode::AssociationRows) => true,
        (ResolvedRoot::Evidence(_), CoveOqlOutputMode::EvidenceRows) => true,
        (ResolvedRoot::Projection(_), CoveOqlOutputMode::ProjectionRows) => true,
        (_, CoveOqlOutputMode::DataFusionTableProvider) => true,
        _ => false,
    };
    if !output_valid {
        return Err(incompatible_execution_grain(
            planned,
            "output mode is incompatible with the resolved root kind",
        ));
    }
    Ok(())
}

fn incompatible_execution_grain(
    planned: &PlannedQuery,
    reason: &'static str,
) -> BuildExecutionError {
    exec_error(
        "E_EXECUTION_GRAIN",
        reason,
        json!({
            "root": execution_root_kind_name(&planned.resolved.root),
            "output_mode": planned.resolved.output_mode,
            "scan_grain": planned.logical_plan.context.scan_grain,
            "temporal_mode": planned.resolved.temporal.mode,
        }),
    )
}

fn execution_root_kind_name(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object",
        ResolvedRoot::Association(_) => "association",
        ResolvedRoot::Projection(_) => "projection",
        ResolvedRoot::Evidence(_) => "evidence",
    }
}

pub(crate) fn enforce_result_budgets(
    result: &CoveOqlExecutionResult,
    row_counts: &ExecutionRowCounts,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    if planned.resolved.method_chain.take.is_none()
        && row_counts.output_rows > options.resource_budget.maximum_rows_without_explicit_take
    {
        return Err(resource_error(
            "maximum_rows_without_explicit_take",
            row_counts.output_rows,
        ));
    }
    let decode_bytes = serde_json::to_vec(&result_json(result)?)
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))?
        .len();
    if decode_bytes > options.resource_budget.maximum_decode_bytes {
        return Err(resource_error("maximum_decode_bytes", decode_bytes));
    }
    let columns = max_output_columns(result)?;
    if columns > options.resource_budget.maximum_output_columns {
        return Err(resource_error("maximum_output_columns", columns));
    }
    Ok(())
}

pub(crate) fn check_time(
    budget: &ResourceBudgetPolicy,
    started: Instant,
) -> Result<(), BuildExecutionError> {
    if started.elapsed().as_millis() as u64 > budget.maximum_execution_time_ms {
        return Err(resource_error(
            "maximum_execution_time_ms",
            started.elapsed().as_millis() as usize,
        ));
    }
    Ok(())
}

pub(crate) fn result_fingerprint(
    result: &CoveOqlExecutionResult,
) -> Result<String, BuildExecutionError> {
    let value = result_json(result)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn result_json(result: &CoveOqlExecutionResult) -> Result<Value, BuildExecutionError> {
    Ok(match result {
        CoveOqlExecutionResult::ObjectRows(rows) => {
            Value::Array(rows.iter().map(MaterializedObjectRow::to_json).collect())
        }
        CoveOqlExecutionResult::AssociationRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedAssociationRow::to_json)
                .collect(),
        ),
        CoveOqlExecutionResult::EvidenceRows(rows) => {
            Value::Array(rows.iter().map(MaterializedEvidenceRow::to_json).collect())
        }
        CoveOqlExecutionResult::ProjectionRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedProjectionRow::to_json)
                .collect(),
        ),
        CoveOqlExecutionResult::ArrowRecordBatches(batches) => Value::Array(
            record_batches_to_json_rows(batches)
                .map_err(|err| exec_error("E_ARROW_OUTPUT", err, json!({})))?,
        ),
        CoveOqlExecutionResult::JsonRows(rows) => Value::Array(rows.clone()),
        CoveOqlExecutionResult::ExplainJson(value) => value.clone(),
    })
}

fn max_output_columns(result: &CoveOqlExecutionResult) -> Result<usize, BuildExecutionError> {
    let rows = match result {
        CoveOqlExecutionResult::ArrowRecordBatches(batches) => {
            return Ok(batches
                .iter()
                .map(|batch| batch.num_columns())
                .max()
                .unwrap_or_default())
        }
        _ => result_json(result)?,
    };
    Ok(rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_object().map(|object| object.len()).unwrap_or(1))
                .max()
                .unwrap_or_default()
        })
        .unwrap_or_default())
}

fn output_name_for_expr(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        ResolvedExpr::AggregateCall { name, .. } => aggregate_name(*name).into(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::Association(association) => association.type_name.clone(),
        ResolvedExpr::Evidence(_) => "evidence".into(),
        ResolvedExpr::Conditional { .. } => "if".into(),
    }
}

fn aggregate_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
    }
}

fn grouped_or_aggregate(planned: &PlannedQuery) -> bool {
    planned.resolved.method_chain.group_by.is_some()
        || planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .is_some_and(|select| select.iter().any(|item| contains_aggregate(&item.expr)))
}

fn contains_aggregate(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::AggregateCall { .. } => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(contains_aggregate),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_aggregate(predicate)
                || contains_aggregate(then_expr)
                || contains_aggregate(else_expr)
        }
        _ => false,
    }
}

fn predicate_contains_aggregate(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            contains_aggregate(left) || contains_aggregate(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => contains_aggregate(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_aggregate(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_aggregate)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use cove_core::{
        constants::{CoveLogicalType, CovePhysicalKind},
        profile::cove_o::{CoveObjectTombstoneStatus, RecordKind},
    };

    use crate::AstLiteral;

    use super::*;

    fn test_projection_root() -> ResolvedRoot {
        ResolvedRoot::Projection(crate::ResolvedProjectionRoot {
            projection_id: "people_projection".into(),
            mapping_id: "people-map".into(),
            mapping_version: "2026.05".into(),
            output_table: Some("people_projection".into()),
            row_grain: Some("one_row_per_object".into()),
            anchor: Some(crate::ResolvedProjectionAnchor {
                object_type: Some("Person".into()),
                association_type: None,
            }),
            temporal_mode: Some("latest_committed".into()),
            columns: vec![
                test_projection_column("active", "bool"),
                test_projection_column("name", "utf8"),
                test_projection_column("score", "float64"),
            ],
            assertion_ids: Vec::new(),
            multi_value_policy: Some("reject".into()),
            missing_policy: "null".into(),
            ordering: Vec::new(),
            evidence_policy: "none".into(),
            output_modes: vec!["json".into(), "arrow".into()],
            column_count: 3,
        })
    }

    fn test_projection_column(name: &str, logical_type: &str) -> crate::ResolvedProjectionColumn {
        crate::ResolvedProjectionColumn {
            name: name.into(),
            value: format!("property.{name}"),
            logical_type: Some(logical_type.into()),
            nested_shape: None,
            conflict_policy: "latest".into(),
            missing_policy: "null".into(),
            source_property_id: None,
        }
    }

    fn test_projection_path(column: &str, logical_type: &str) -> ResolvedExpr {
        ResolvedExpr::Path(ResolvedPath {
            display_name: column.into(),
            root_kind: crate::ResolvedPathRootKind::Projection,
            object_type_id: None,
            property_id: None,
            association_type_id: None,
            evidence_field_id: None,
            projection_id: Some("people_projection".into()),
            projection_column: Some(column.into()),
            system_field: None,
            logical_type: logical_type.into(),
            physical_kind: logical_type.into(),
            collation_id: None,
            nullable: true,
            null_policy: "cove_null_semantics_preserved".into(),
            temporal_role: None,
            code_domain_id: crate::CodeDomainId::Placeholder {
                root: "projection".into(),
                object_type_id: None,
                property_id: None,
                projection_id: Some("people_projection".into()),
                field: Some(column.into()),
            },
        })
    }

    fn test_bool_literal(value: bool) -> ResolvedExpr {
        ResolvedExpr::Literal(ResolvedLiteral {
            literal: AstLiteral::Boolean(value),
            logical_type: "bool".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Boolean(value),
            precision: None,
            scale: None,
        })
    }

    fn test_null_literal() -> ResolvedExpr {
        ResolvedExpr::Literal(ResolvedLiteral {
            literal: AstLiteral::Null,
            logical_type: "null".into(),
            canonical: "null".into(),
            typed_value: ResolvedLiteralValue::Null,
            precision: None,
            scale: None,
        })
    }

    fn test_eq_predicate(left: ResolvedExpr, right: ResolvedExpr) -> ResolvedPredicate {
        ResolvedPredicate::Compare {
            left,
            op: AstCompareOp::Eq,
            right,
        }
    }

    #[test]
    fn evidence_default_order_aliases_match_materialized_evidence_fields() {
        let row = MaterializedEvidenceRow {
            fields: BTreeMap::from([
                ("output_object_id".into(), json!("object-1")),
                ("source_id".into(), json!("crm.customers")),
                ("source_row_identity".into(), json!("customer_id=1")),
                ("assertion_id".into(), json!("assertion-1")),
            ]),
        };

        assert_eq!(
            evidence_default_sort_field_value(&row, "target_id"),
            json!("object-1")
        );
        assert_eq!(
            evidence_default_sort_field_value(&row, "source_system"),
            json!("crm.customers")
        );
        assert_eq!(
            evidence_default_sort_field_value(&row, "source_row_identity"),
            json!("customer_id=1")
        );
        assert_eq!(
            evidence_default_sort_field_value(&row, "evidence_id"),
            json!("assertion-1")
        );
    }

    fn test_function_expr(
        function_id: &str,
        arg: ResolvedExpr,
        logical_type: &str,
    ) -> ResolvedExpr {
        ResolvedExpr::FunctionCall {
            function_id: function_id.into(),
            deterministic: true,
            logical_type: logical_type.into(),
            physical_kind: logical_type.into(),
            contract: crate::ResolvedFunctionContract {
                function_id: function_id.into(),
                version: "1".into(),
                deterministic: true,
                dependency: "materialized".into(),
                execution_class: crate::FunctionExecutionClass::MaterializedOnly,
                unicode_or_collation_contract: None,
            },
            args: vec![arg],
        }
    }

    #[test]
    fn projection_output_columns_collect_residual_expression_inputs() {
        let root = test_projection_root();
        let method_chain = ResolvedMethodChain {
            select: Some(vec![crate::ResolvedSelectItem {
                alias: Some("display_name".into()),
                expr: test_function_expr("lower", test_projection_path("name", "utf8"), "utf8"),
            }]),
            where_predicate: Some(ResolvedPredicate::Not(Box::new(
                ResolvedPredicate::BoolExpr(test_projection_path("active", "bool")),
            ))),
            order_by: Some(crate::ResolvedOrderClause {
                expr: test_function_expr("lower", test_projection_path("name", "utf8"), "utf8"),
                direction: AstOrderDirection::Asc,
                nulls: AstNullOrdering::NullsLast,
                uses_default_ordering: false,
            }),
            group_by: Some(vec![test_projection_path("score", "float64")]),
            ..ResolvedMethodChain::default()
        };

        let output_columns = projection_output_columns_for_parts(&root, &method_chain).unwrap();

        assert_eq!(output_columns, vec!["active", "name", "score"]);
    }

    #[test]
    fn projection_same_column_or_lowers_to_single_in_list_filter() {
        let predicate = ResolvedPredicate::Or(vec![
            test_eq_predicate(
                test_projection_path("active", "bool"),
                test_bool_literal(true),
            ),
            ResolvedPredicate::InList {
                expr: test_projection_path("active", "bool"),
                values: vec![ResolvedLiteral {
                    literal: AstLiteral::Boolean(false),
                    logical_type: "bool".into(),
                    canonical: "false".into(),
                    typed_value: ResolvedLiteralValue::Boolean(false),
                    precision: None,
                    scale: None,
                }],
            },
        ]);

        let filters = projection_filters_for_predicate(&predicate).unwrap();

        assert_eq!(
            filters,
            vec![ProjectionFilter::InList {
                column: "active".into(),
                literals: vec![
                    ProjectionFilterLiteral::Boolean(true),
                    ProjectionFilterLiteral::Boolean(false)
                ],
            }]
        );
    }

    #[test]
    fn projection_or_with_null_literal_stays_residual() {
        let predicate = ResolvedPredicate::Or(vec![
            test_eq_predicate(
                test_projection_path("active", "bool"),
                test_bool_literal(true),
            ),
            test_eq_predicate(test_projection_path("active", "bool"), test_null_literal()),
        ]);

        assert!(projection_filters_for_predicate(&predicate).is_none());
    }

    #[test]
    fn projection_or_across_columns_stays_residual() {
        let predicate = ResolvedPredicate::Or(vec![
            test_eq_predicate(
                test_projection_path("active", "bool"),
                test_bool_literal(true),
            ),
            test_eq_predicate(
                test_projection_path("name", "utf8"),
                test_bool_literal(true),
            ),
        ]);

        assert!(projection_filters_for_predicate(&predicate).is_none());
    }

    #[test]
    fn evidence_object_rows_reuse_materialized_evidence_shape() {
        let states = vec![
            CoveObjectState {
                object_type_id: 9,
                object_type_name: "Evidence".into(),
                object_type_flags: OBJECT_TYPE_FLAG_EVIDENCE_OBJECT,
                branch_key: 3,
                goid: [1; 16],
                latest_record_id: [2; 16],
                latest_segment_id: 4,
                latest_row_index: 5,
                timestamp_us: 1_767_225_600_000_000,
                csn: 6,
                record_kind: RecordKind::Baseline,
                tombstone_status: CoveObjectTombstoneStatus::Live,
                association: None,
                properties: vec![
                    CoveObjectPropertyValue {
                        property_id: 1,
                        property_name: "source_id".into(),
                        logical_type: CoveLogicalType::Utf8,
                        physical_kind: CovePhysicalKind::VarBytes,
                        flags: 0,
                        value: json!("row-1"),
                        redacted: false,
                    },
                    CoveObjectPropertyValue {
                        property_id: 2,
                        property_name: "raw_evidence".into(),
                        logical_type: CoveLogicalType::Utf8,
                        physical_kind: CovePhysicalKind::VarBytes,
                        flags: PROPERTY_FLAG_EVIDENCE_REF,
                        value: json!("ev-source"),
                        redacted: false,
                    },
                    CoveObjectPropertyValue {
                        property_id: 3,
                        property_name: "mapping_rule".into(),
                        logical_type: CoveLogicalType::Utf8,
                        physical_kind: CovePhysicalKind::VarBytes,
                        flags: PROPERTY_FLAG_MAPPING_RULE_REF,
                        value: json!("rule-7"),
                        redacted: false,
                    },
                ],
            },
            CoveObjectState {
                object_type_id: 1,
                object_type_name: "Thing".into(),
                object_type_flags: 0,
                branch_key: 3,
                goid: [3; 16],
                latest_record_id: [4; 16],
                latest_segment_id: 4,
                latest_row_index: 6,
                timestamp_us: 1_767_225_600_000_001,
                csn: 7,
                record_kind: RecordKind::Baseline,
                tombstone_status: CoveObjectTombstoneStatus::Live,
                association: None,
                properties: Vec::new(),
            },
        ];

        let rows = evidence_object_rows_from_states(&states);
        assert_eq!(rows.len(), 1);
        let ExecutionRow::Evidence(row) = &rows[0] else {
            panic!("expected evidence row");
        };
        assert_eq!(row.fields["object_type_name"], json!("Evidence"));
        assert_eq!(row.fields["branch_key"], json!(3));
        assert_eq!(row.fields["source_id"], json!("row-1"));
        assert_eq!(row.fields["raw_evidence"], json!("ev-source"));
        assert_eq!(row.fields["source_evidence_id"], json!("ev-source"));
        assert_eq!(row.fields["mapping_rule"], json!("rule-7"));
        assert_eq!(row.fields["rule_id"], json!("rule-7"));
        assert_eq!(row.fields["grain"], json!("object"));
    }

    #[test]
    fn external_overlay_visibility_filters_helper_association_rows_by_association_identity() {
        let visible = MaterializedAssociationRow {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::AssociationState,
            change: None,
            object_type_id: 7,
            association_type: Some("CustomerPlacedOrder".into()),
            branch_key: 0,
            goid: "assoc-visible".into(),
            record_id: "assoc-record-visible".into(),
            source_goid: Some("person".into()),
            target_goid: Some("order".into()),
            timestamp_us: 0,
            csn: 1,
            record_kind: "baseline".into(),
            tombstone_status: "live".into(),
            properties: BTreeMap::new(),
            property_ids: BTreeMap::new(),
            redacted_properties: BTreeSet::new(),
        };
        let hidden = MaterializedAssociationRow {
            goid: "assoc-hidden".into(),
            record_id: "assoc-record-hidden".into(),
            ..visible.clone()
        };
        let overlay = VisibilityOverlay {
            overlay_id: "tenant-a".into(),
            visible_goids: BTreeSet::from(["assoc-visible".into()]),
            visible_record_ids: BTreeSet::new(),
        };

        assert!(association_row_visible_in_overlay(&visible, &overlay));
        assert!(!association_row_visible_in_overlay(&hidden, &overlay));
    }

    #[test]
    fn external_overlay_visibility_filters_helper_evidence_rows_by_evidence_identity() {
        let mut visible_fields = BTreeMap::new();
        visible_fields.insert("evidence_id".into(), json!("evidence-visible"));
        visible_fields.insert("output_object_id".into(), json!("object-visible"));
        let mut hidden_fields = BTreeMap::new();
        hidden_fields.insert("output_object_id".into(), json!("object-visible"));
        hidden_fields.insert("assertion_id".into(), json!("assertion-hidden"));
        let visible = MaterializedEvidenceRow {
            fields: visible_fields,
        };
        let hidden = MaterializedEvidenceRow {
            fields: hidden_fields,
        };
        let overlay = VisibilityOverlay {
            overlay_id: "tenant-a".into(),
            visible_goids: BTreeSet::from(["evidence-visible".into(), "object-visible".into()]),
            visible_record_ids: BTreeSet::new(),
        };

        assert!(evidence_row_visible_in_overlay(&visible, &overlay));
        assert!(!evidence_row_visible_in_overlay(&hidden, &overlay));
    }

    #[test]
    fn half_open_change_bounds_use_temporal_role_binding_value() {
        let record = CoveObjectRecord {
            object_type_id: 1,
            object_type_name: "Thing".into(),
            object_type_flags: 0,
            segment_id: 0,
            row_index: 0,
            timestamp_us: 5,
            csn: 10,
            branch_key: 0,
            goid: [1; 16],
            record_id: [2; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
            properties: vec![CoveObjectPropertyValue {
                property_id: 9,
                property_name: "source_event_time".into(),
                logical_type: CoveLogicalType::Int64,
                physical_kind: CovePhysicalKind::NumCode,
                flags: 0,
                value: json!(1_000),
                redacted: false,
            }],
            association: None,
        };
        let from = ResolvedTimeBound::TimestampMicros {
            role: TemporalRole::SourceEventTime,
            binding: Some("source_event_time".into()),
            timestamp_micros: 900,
            canonical_rfc3339: "n/a".into(),
        };
        let to = ResolvedTimeBound::TimestampMicros {
            role: TemporalRole::SourceEventTime,
            binding: Some("source_event_time".into()),
            timestamp_micros: 1_100,
            canonical_rfc3339: "n/a".into(),
        };

        assert!(record_in_half_open_bound(&record, &from, &to).unwrap());

        let late_from = ResolvedTimeBound::TimestampMicros {
            role: TemporalRole::SourceEventTime,
            binding: Some("source_event_time".into()),
            timestamp_micros: 1_100,
            canonical_rfc3339: "n/a".into(),
        };
        let late_to = ResolvedTimeBound::TimestampMicros {
            role: TemporalRole::SourceEventTime,
            binding: Some("source_event_time".into()),
            timestamp_micros: 1_200,
            canonical_rfc3339: "n/a".into(),
        };
        assert!(!record_in_half_open_bound(&record, &late_from, &late_to).unwrap());
    }
}

pub(crate) fn resource_error(limit: &str, actual: usize) -> BuildExecutionError {
    exec_error(
        "E_RESOURCE_BUDGET_EXCEEDED",
        format!("{limit} budget exceeded during execution"),
        json!({ "limit": limit, "actual": actual }),
    )
}

pub(crate) fn exec_error(
    code: &str,
    message: impl Into<String>,
    safe_details: Value,
) -> BuildExecutionError {
    BuildExecutionError::single(ExecutionDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: "execute".into(),
        safe_details,
        redacted: false,
    })
}

pub(crate) fn exec_warning(
    code: &str,
    message: impl Into<String>,
    safe_details: Value,
) -> ExecutionDiagnostic {
    ExecutionDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Warning,
        message: message.into(),
        phase: "execute".into(),
        safe_details,
        redacted: false,
    }
}
