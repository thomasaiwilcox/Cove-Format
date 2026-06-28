use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use arrow_array::{Array, RecordBatch};
use arrow_data::ArrayData;
use arrow_schema::DataType;
use cove_core::{
    profile::cove_o::{
        read_object_surface_from_bytes_with_pushdown_options, reconstruct_object_states,
        CoveObjectPropertyValue, CoveObjectReadOptions, CoveObjectReadPushdownOptions,
        CoveObjectReadWithPushdownOptions, CoveObjectReconstructionOptions, CoveObjectRecord,
        CoveObjectRedactionReadPolicy, CoveObjectState, CoveObjectSurface, CoveObjectTemporalCut,
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
        number_to_f64, parse_decimal_value, stable_value_key, value_ordering_typed, EvalContext,
        ExactDecimal,
    },
    materialized::{
        hex, window_function_key, ExecutionRow, MaterializedAssociationRow,
        MaterializedChangeDetail, MaterializedChangeDiffKind, MaterializedEvidenceRow,
        MaterializedObjectRow, MaterializedProjectionRow, OutputGrain,
        INTERNAL_PROJECTION_FIELD_PREFIX,
    },
    parse_resolve_and_plan_query,
    predicate::classify_predicate,
    pushdown::{self, PushdownOptions, PushdownReport},
    AggregateDisclosurePolicy, AstAggregateName, AstChangeMode, AstCompareOp, AstHistoryMode,
    AstNullOrdering, AstOrderDirection, BuildLogicalPlanError, CodeDomainId, CoveQlOutputMode,
    DatasetFileIdentity, DiagnosticSeverity, FallbackPolicy, GraphAlgorithmKind,
    GraphTraversalContract, GraphTraversalDistinctPolicy, GraphTraversalMode,
    LogicalPlanDiagnostic, ManifestDatasetMember, MetadataDisclosurePolicy, ParseOptions,
    PlanOptions, PlannedQuery, PredicateProofState, RedactionPolicy, ResolveOptions, ResolvedExpr,
    ResolvedLiteral, ResolvedLiteralValue, ResolvedMethodChain, ResolvedPath, ResolvedPathRootKind,
    ResolvedPredicate, ResolvedRoot, ResolvedTimeBound, ResourceBudgetPolicy,
    TableExecutionAuthority, TableJoinKind, TableLookupCardinality, TableLookupDuplicatePolicy,
    TableLookupUnmatchedPolicy, TemporalMode, TemporalRole, ValidatedFileIdentity,
    VisibilityPolicy,
};

const GRAPH_CURRENT_GOID_FIELD: &str = "__coveql_current_goid";
const GRAPH_PATH_START_GOID_FIELD: &str = "__coveql_path_start_goid";
const GRAPH_PATH_NODE_GOIDS_FIELD: &str = "__coveql_path_node_goids";
const GRAPH_PATH_EDGE_GOIDS_FIELD: &str = "__coveql_path_edge_goids";
const GRAPH_PATH_DEPTH_FIELD: &str = "__coveql_path_depth";

#[derive(Debug, Clone)]
pub struct CoveQlRetainedInput {
    bytes: RetainedBytes,
}

impl CoveQlRetainedInput {
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
pub struct CoveQlRetainedManifestMember {
    source: String,
    bytes: RetainedBytes,
}

impl CoveQlRetainedManifestMember {
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
    pub result: CoveQlExecutionResult,
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

    pub fn result_json(&self) -> Result<Value, BuildExecutionError> {
        result_json(&self.result)
    }
}

#[derive(Debug, Clone)]
pub struct CoveQlResultStream {
    bytes: Vec<u8>,
    planned: PlannedQuery,
    options: ExecutionOptions,
    executed: Option<ExecutedQuery>,
    batches: Vec<CoveQlExecutionResult>,
    next_batch: usize,
    row_stream: Option<MaterializedRowStreamState>,
    blocking_reason: Option<String>,
    cancelled: bool,
}

impl CoveQlResultStream {
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

    pub fn next_batch(&mut self) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
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
    ) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
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
        source.object_rows = filter_object_context_rows(&source.object_rows, &planned, &options);
        let context = EvalContext::for_plan_with_objects(
            &source.associations,
            &source.evidence_rows,
            &source.object_rows,
            &planned,
        );
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

    fn next_batch(&mut self) -> Result<Option<CoveQlExecutionResult>, BuildExecutionError> {
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
            let context = EvalContext::for_plan_with_objects(
                &self.source.associations,
                &self.source.evidence_rows,
                &self.source.object_rows,
                &self.planned,
            );
            if !predicate_matches(&row, &self.planned, &context)? {
                continue;
            }
            self.filtered_rows += 1;
            let emitted = match (&self.planned.resolved.output_mode, row) {
                (CoveQlOutputMode::JsonRows, row) => {
                    let value =
                        select_json_rows(std::slice::from_ref(&row), &self.planned, &context)?
                            .into_iter()
                            .next()
                            .unwrap_or(Value::Null);
                    json_batch.push(value.clone());
                    self.json_output.push(value);
                    true
                }
                (CoveQlOutputMode::ObjectRows, ExecutionRow::Object(row)) => {
                    object_batch.push(row.clone());
                    self.object_output.push(row);
                    true
                }
                (CoveQlOutputMode::AssociationRows, ExecutionRow::Association(row)) => {
                    association_batch.push(row.clone());
                    self.association_output.push(row);
                    true
                }
                (CoveQlOutputMode::EvidenceRows, ExecutionRow::Evidence(row)) => {
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
            CoveQlOutputMode::JsonRows => CoveQlExecutionResult::JsonRows(json_batch),
            CoveQlOutputMode::ObjectRows => CoveQlExecutionResult::ObjectRows(object_batch),
            CoveQlOutputMode::AssociationRows => {
                CoveQlExecutionResult::AssociationRows(association_batch)
            }
            CoveQlOutputMode::EvidenceRows => CoveQlExecutionResult::EvidenceRows(evidence_batch),
            _ => unreachable!("non-streamable output modes are blocked before streaming"),
        }))
    }

    fn finish(&mut self) -> Result<ExecutedQuery, BuildExecutionError> {
        while self.next_batch()?.is_some() {}
        let result = match &self.planned.resolved.output_mode {
            CoveQlOutputMode::JsonRows => CoveQlExecutionResult::JsonRows(self.json_output.clone()),
            CoveQlOutputMode::ObjectRows => {
                CoveQlExecutionResult::ObjectRows(self.object_output.clone())
            }
            CoveQlOutputMode::AssociationRows => {
                CoveQlExecutionResult::AssociationRows(self.association_output.clone())
            }
            CoveQlOutputMode::EvidenceRows => {
                CoveQlExecutionResult::EvidenceRows(self.evidence_output.clone())
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

impl Iterator for CoveQlResultStream {
    type Item = Result<CoveQlExecutionResult, BuildExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_batch().transpose()
    }
}

#[derive(Debug, Clone)]
pub enum CoveQlExecutionResult {
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
            write!(f, "CoveQL materialized execution failed")
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

pub fn parse_resolve_plan_and_execute_query_on_object_surface(
    planning_bytes: &[u8],
    surface: &CoveObjectSurface,
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    execution_options: ExecutionOptions,
    validation_options: ValidationOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let planned = parse_resolve_and_plan_query(
        planning_bytes,
        text,
        parse_options,
        resolve_options,
        plan_options,
        validation_options,
    )
    .map_err(BuildExecutionError::from_plan)?;
    execute_planned_query_on_object_surface(surface, planned, execution_options)
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
        CoveQlOutputMode::ExplainJson => {
            let explain = planned.explain_json();
            (
                CoveQlExecutionResult::ExplainJson(explain),
                ExecutionRowCounts::default(),
                PushdownReport::not_executed(&options.pushdown),
                None,
            )
        }
        CoveQlOutputMode::DataFusionTableProvider => {
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
    input: CoveQlRetainedInput,
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    execute_planned_query(input.as_slice(), planned, options)
}

pub fn execute_planned_query_on_object_surface(
    surface: &CoveObjectSurface,
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
        "W_MATERIALIZED_OBJECT_SURFACE_BASELINE",
        "validated COVE-O object surface execution produced the visible output; byte-level pushdown is not applied to pre-composed surfaces",
        json!({ "pushdown_enabled": options.pushdown.enabled }),
    ));
    if let Some(warning) = zero_copy_owned_fallback_warning(&planned) {
        diagnostics.push(warning);
    }

    let (result, row_counts, pushdown_report, evidence_authority) =
        match &planned.resolved.output_mode {
            CoveQlOutputMode::ExplainJson => {
                let explain = planned.explain_json();
                (
                    CoveQlExecutionResult::ExplainJson(explain),
                    ExecutionRowCounts::default(),
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                )
            }
            CoveQlOutputMode::DataFusionTableProvider => {
                return Err(exec_error(
                    "E_UNSUPPORTED_OUTPUT",
                    "DataFusion output is exposed through the Phase 3 registration helper, not object surface execution",
                    json!({}),
                ))
            }
            _ => execute_rows_on_object_surface(surface, &planned, &options, started)?,
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
            "validated COVE-O object surface execution produced the visible output without compacting the surface into snapshot bytes",
        ),
    })
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
        CoveQlOutputMode::DataFusionTableProvider
    ) {
        return Err(exec_error(
            "E_UNSUPPORTED_OUTPUT",
            "manifest execution returns materialized CoveQL output; register an CoveQL DataFusion provider separately",
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
        "manifest member execution validated exact COVM code-domain bridge proofs, but this logical executor used the materialized CoveQL oracle across members because no manifest physical kernel path was selected"
    } else {
        "manifest member execution used the materialized CoveQL oracle across validated COVM members because cross-file coded acceleration requires exact bridge proofs and a manifest physical kernel path"
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
            CoveQlOutputMode::ExplainJson => {
                let explain = planned.explain_json();
                (
                    CoveQlExecutionResult::ExplainJson(explain),
                    ExecutionRowCounts::default(),
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                )
            }
            CoveQlOutputMode::DataFusionTableProvider => unreachable!("handled above"),
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
    members: &[CoveQlRetainedManifestMember],
    planned: PlannedQuery,
    options: ExecutionOptions,
) -> Result<ExecutedQuery, BuildExecutionError> {
    let borrowed = members
        .iter()
        .map(CoveQlRetainedManifestMember::as_manifest_member)
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
) -> Result<CoveQlResultStream, BuildExecutionError> {
    Ok(CoveQlResultStream {
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
        CoveQlOutputMode::ArrowRecordBatch { .. }
            | CoveQlOutputMode::ExplainJson
            | CoveQlOutputMode::DataFusionTableProvider
            | CoveQlOutputMode::ProjectionRows
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

fn result_batches(result: &CoveQlExecutionResult, batch_size: usize) -> Vec<CoveQlExecutionResult> {
    match result {
        CoveQlExecutionResult::ObjectRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ObjectRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::AssociationRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::AssociationRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::EvidenceRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::EvidenceRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ProjectionRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ProjectionRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ArrowRecordBatches(batches) => batches
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::ArrowRecordBatches(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::JsonRows(rows) => rows
            .chunks(batch_size)
            .map(|chunk| CoveQlExecutionResult::JsonRows(chunk.to_vec()))
            .collect(),
        CoveQlExecutionResult::ExplainJson(value) => {
            vec![CoveQlExecutionResult::ExplainJson(value.clone())]
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
        CoveQlExecutionResult,
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
        ResolvedRoot::Table(root) => {
            let (result, row_counts) = if table_relation_context_required(planned) {
                execute_table_relation_root(bytes, planned, options, started, root)?
            } else if matches!(
                &root.execution_authority,
                TableExecutionAuthority::DeterministicProjection { .. }
            ) {
                execute_projection_root(
                    bytes,
                    planned,
                    options,
                    started,
                    &root.projection.projection_id,
                )?
            } else {
                execute_table_relation_root(bytes, planned, options, started, root)?
            };
            let report = projection_readback_pushdown_report(planned, options, &row_counts);
            Ok((result, row_counts, report, None))
        }
        ResolvedRoot::Object(_)
        | ResolvedRoot::Association(_)
        | ResolvedRoot::Edge(_)
        | ResolvedRoot::Evidence(_) => execute_object_backed_root(bytes, planned, options, started),
        ResolvedRoot::Node(root) => {
            if planned.resolved.method_chain.traversals.is_empty()
                && planned.resolved.method_chain.graph_algorithms.is_empty()
            {
                execute_object_backed_root(bytes, planned, options, started)
            } else {
                let (result, row_counts) =
                    execute_graph_traverse_root(bytes, planned, options, started, root)?;
                Ok((
                    result,
                    row_counts,
                    PushdownReport::not_executed(&options.pushdown),
                    None,
                ))
            }
        }
    }
}

fn execute_rows_on_object_surface(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveQlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    match &planned.resolved.root {
        ResolvedRoot::Object(_)
        | ResolvedRoot::Association(_)
        | ResolvedRoot::Edge(_)
        | ResolvedRoot::Evidence(_) => {
            execute_object_surface_root(surface, planned, options, started)
        }
        ResolvedRoot::Node(_) if planned.resolved.method_chain.traversals.is_empty()
            && planned.resolved.method_chain.graph_algorithms.is_empty() =>
        {
            execute_object_surface_root(surface, planned, options, started)
        }
        ResolvedRoot::Node(root) => {
            let (result, row_counts) =
                execute_graph_traverse_surface_root(surface, planned, options, started, root)?;
            Ok((
                result,
                row_counts,
                PushdownReport::not_applicable(
                    &options.pushdown,
                    "object surface execution reads a pre-composed COVE-O surface and cannot apply byte-level pushdown",
                ),
                None,
            ))
        }
        ResolvedRoot::Projection(_) | ResolvedRoot::Table(_) => Err(exec_error(
            "E_UNSUPPORTED_SURFACE_ROOT",
            "object surface execution does not yet support projection or table roots; use materialized snapshot execution",
            json!({}),
        )),
    }
}

fn execute_object_backed_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveQlExecutionResult,
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
        &source.object_rows,
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

fn execute_object_surface_root(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    (
        CoveQlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    let mut source = object_surface_row_source(
        surface,
        planned,
        options,
        started,
        PushdownReport::not_applicable(
            &options.pushdown,
            "object surface execution reads a pre-composed COVE-O surface and cannot apply byte-level pushdown",
        ),
    )?;
    let evidence_authority = source.evidence_authority;
    let (result, row_counts) = finish_materialized_rows(
        source.rows,
        &source.associations,
        &source.evidence_rows,
        &source.object_rows,
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
    object_rows: Vec<MaterializedObjectRow>,
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
        CoveQlExecutionResult,
        ExecutionRowCounts,
        PushdownReport,
        Option<EvidenceAuthority>,
    ),
    BuildExecutionError,
> {
    if matches!(
        planned.resolved.root,
        ResolvedRoot::Node(_)
            | ResolvedRoot::Edge(_)
            | ResolvedRoot::Table(_)
            | ResolvedRoot::Projection(_)
            | ResolvedRoot::Evidence(_)
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
    let mut object_rows = Vec::new();
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
            ResolvedRoot::Table(root) => {
                rows.extend(
                    projection_root_execution_rows(
                        member.bytes,
                        &member_plan,
                        options,
                        started,
                        &root.projection.projection_id,
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
            ResolvedRoot::Object(_)
            | ResolvedRoot::Association(_)
            | ResolvedRoot::Node(_)
            | ResolvedRoot::Edge(_)
            | ResolvedRoot::Evidence(_) => {
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
                object_rows.extend(source.object_rows.into_iter().map(|row| {
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
        &object_rows,
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
            "manifest execution read {file_count} validated member files and applied global materialized CoveQL residual semantics for {} root",
            match planned.resolved.root {
                ResolvedRoot::Object(_) => "object",
                ResolvedRoot::Association(_) => "association",
                ResolvedRoot::Node(_) => "node",
                ResolvedRoot::Edge(_) => "edge",
                ResolvedRoot::Table(_) => "table",
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
    object_surface_row_source(&surface, planned, options, started, pushdown_plan.report)
}

fn object_surface_row_source(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    pushdown_report: PushdownReport,
) -> Result<MaterializedRowSource, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let states = object_states_for_temporal_context(surface, planned)?;
    let object_rows = states
        .iter()
        .map(MaterializedObjectRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::LatestState))
        .collect::<Vec<_>>();
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
            temporal_object_rows(surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Association(root)
            if planned.resolved.method_chain.history.is_some()
                || planned.resolved.method_chain.changes.is_some() =>
        {
            temporal_association_rows(surface, planned, root.object_type_id)?
        }
        ResolvedRoot::Object(root) => object_rows
            .iter()
            .filter(|state| state.object_type_id == root.object_type_id)
            .cloned()
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
        ResolvedRoot::Node(root) => object_rows
            .iter()
            .filter(|state| state.object_type_id == root.object.object_type_id)
            .cloned()
            .map(ExecutionRow::Object)
            .collect::<Vec<_>>(),
        ResolvedRoot::Edge(root) => associations
            .iter()
            .filter(|association| association.object_type_id == root.association.object_type_id)
            .filter(|association| {
                association_row_matches_temporal(
                    association,
                    &root.association,
                    association_valid_at(planned),
                )
            })
            .cloned()
            .map(ExecutionRow::Association)
            .collect::<Vec<_>>(),
        ResolvedRoot::Evidence(_) => evidence_execution_rows,
        ResolvedRoot::Table(_) => {
            unreachable!("table roots are handled through projection readback")
        }
        ResolvedRoot::Projection(_) => unreachable!("projection handled separately"),
    };
    Ok(MaterializedRowSource {
        rows: all_rows,
        associations: context_associations,
        evidence_rows: context_evidence_rows,
        object_rows,
        pushdown_report,
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
            AstChangeMode::FinalRows => {
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
            AstChangeMode::FinalRows => {
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
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let pushed_filters = projection_filters_for_plan(planned);
    let output_columns = projection_output_columns_for_plan(planned);
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns,
        pushed_filters: pushed_filters.clone(),
        batch_size: options.batch_size,
        candidate_projection_rows: None,
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
    if let Some(batches) = projection_arrow_passthrough_batches(planned, &batches, &pushed_filters)
        .map_err(|err| {
            exec_error(
                "E_ARROW_OUTPUT",
                format!("projection Arrow passthrough failed: {err}"),
                json!({ "projection_id": projection_id }),
            )
        })?
    {
        let row_count = batches.iter().map(RecordBatch::num_rows).sum();
        return Ok((
            CoveQlExecutionResult::ArrowRecordBatches(batches),
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
    finish_materialized_rows(rows, &[], &[], &[], planned, options, started)
}

fn projection_arrow_passthrough_batches(
    planned: &PlannedQuery,
    batches: &[RecordBatch],
    pushed_filters: &[ProjectionFilter],
) -> Result<Option<Vec<RecordBatch>>, String> {
    if !projection_arrow_passthrough_shape(planned) {
        return Ok(None);
    }
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Ok(None);
    };
    if !projection_arrow_filters_are_exact(planned, schema.as_ref(), pushed_filters) {
        return Ok(None);
    }
    let Some(final_projection) = projection_arrow_final_projection(planned, schema.as_ref()) else {
        return Ok(None);
    };
    if let Some(indices) = final_projection {
        return batches
            .iter()
            .map(|batch| batch.project(&indices).map_err(|err| err.to_string()))
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    Ok(Some(batches.to_vec()))
}

fn projection_arrow_passthrough_shape(planned: &PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. }
    ) {
        return false;
    }
    let chain = &planned.resolved.method_chain;
    chain.lookups.is_empty()
        && chain.joins.is_empty()
        && chain.set_operations.is_empty()
        && chain.windows.is_empty()
        && chain.traversals.is_empty()
        && chain.graph_algorithms.is_empty()
        && chain.group_by.is_none()
        && chain.order_by.is_none()
        && chain.skip.is_none()
        && chain.take.is_none()
        && chain.history.is_none()
        && chain.changes.is_none()
}

fn projection_arrow_filters_are_exact(
    planned: &PlannedQuery,
    schema: &arrow_schema::Schema,
    pushed_filters: &[ProjectionFilter],
) -> bool {
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return true;
    };
    if pushed_filters.is_empty() || projection_filters_for_predicate(predicate).is_none() {
        return false;
    }
    let root = projection_contract_root(&planned.resolved.root);
    pushed_filters.iter().all(|filter| {
        let predicate = projection_predicate_for_schema(schema, filter);
        let form = classify_predicate(&predicate, &root, "projection_readback");
        form.representation.proof_state == PredicateProofState::ProvenExact
            && form.residual_reason.is_none()
    })
}

fn projection_contract_root(root: &ResolvedRoot) -> ResolvedRoot {
    match root {
        ResolvedRoot::Table(root) => ResolvedRoot::Projection(root.projection.clone()),
        ResolvedRoot::Projection(root) => ResolvedRoot::Projection(root.clone()),
        other => other.clone(),
    }
}

fn projection_arrow_final_projection(
    planned: &PlannedQuery,
    schema: &arrow_schema::Schema,
) -> Option<Option<Vec<usize>>> {
    let Some(select) = &planned.resolved.method_chain.select else {
        return Some(None);
    };
    let mut indices = Vec::with_capacity(select.len());
    for item in select {
        let ResolvedExpr::Path(path) = &item.expr else {
            return None;
        };
        path.projection_column.as_ref()?;
        let column = projection_column(path);
        if item.alias.as_ref().is_some_and(|alias| alias != &column) {
            return None;
        }
        let index = schema.index_of(&column).ok()?;
        indices.push(index);
    }
    Some(Some(indices))
}

fn projection_predicate_for_schema(
    schema: &arrow_schema::Schema,
    filter: &ProjectionFilter,
) -> ResolvedPredicate {
    match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => ResolvedPredicate::Compare {
            left: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            op: projection_ast_op(*op),
            right: ResolvedExpr::Literal(projection_filter_resolved_literal(literal)),
        },
        ProjectionFilter::InList { column, literals } => ResolvedPredicate::InList {
            expr: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            values: literals
                .iter()
                .map(projection_filter_resolved_literal)
                .collect(),
        },
        ProjectionFilter::IsNull { column, negated } => ResolvedPredicate::NullCheck {
            expr: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            negated: *negated,
        },
    }
}

fn projection_filter_resolved_path(schema: &arrow_schema::Schema, column: &str) -> ResolvedPath {
    let (logical_type, nullable) = schema
        .field_with_name(column)
        .map(|field| {
            (
                logical_type_for_arrow(field.data_type()).to_string(),
                field.is_nullable(),
            )
        })
        .unwrap_or_else(|_| ("utf8".into(), true));
    ResolvedPath {
        display_name: column.into(),
        root_kind: ResolvedPathRootKind::Projection,
        object_type_id: None,
        property_id: None,
        association_type_id: None,
        evidence_field_id: None,
        projection_id: None,
        projection_column: Some(column.into()),
        system_field: None,
        logical_type: logical_type.clone(),
        physical_kind: physical_kind_for_logical_name(&logical_type).into(),
        collation_id: None,
        nullable,
        null_policy: if nullable { "allow" } else { "reject" }.into(),
        temporal_role: None,
        code_domain_id: CodeDomainId::Placeholder {
            root: "projection".into(),
            object_type_id: None,
            property_id: None,
            projection_id: None,
            field: Some(column.into()),
        },
    }
}

fn projection_filter_resolved_literal(literal: &ProjectionFilterLiteral) -> ResolvedLiteral {
    match literal {
        ProjectionFilterLiteral::Null => ResolvedLiteral {
            literal: crate::AstLiteral::Null,
            logical_type: "null".into(),
            canonical: "null".into(),
            typed_value: ResolvedLiteralValue::Null,
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Boolean(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Boolean(*value),
            logical_type: "bool".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Boolean(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Int64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Integer(value.to_string()),
            logical_type: "int64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::SignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::UInt64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Integer(value.to_string()),
            logical_type: "uint64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::UnsignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Float64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Decimal(value.to_string()),
            logical_type: "float64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Decimal {
                canonical: value.to_string(),
                precision: 0,
                scale: 0,
            },
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Utf8(value) => ResolvedLiteral {
            literal: crate::AstLiteral::String(value.clone()),
            logical_type: "utf8".into(),
            canonical: value.clone(),
            typed_value: ResolvedLiteralValue::String(value.clone()),
            precision: None,
            scale: None,
        },
    }
}

fn projection_ast_op(op: ProjectionFilterOp) -> AstCompareOp {
    match op {
        ProjectionFilterOp::Eq => AstCompareOp::Eq,
        ProjectionFilterOp::Ne => AstCompareOp::Ne,
        ProjectionFilterOp::Lt => AstCompareOp::Lt,
        ProjectionFilterOp::LtEq => AstCompareOp::Le,
        ProjectionFilterOp::Gt => AstCompareOp::Gt,
        ProjectionFilterOp::GtEq => AstCompareOp::Ge,
    }
}

fn logical_type_for_arrow(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::Boolean => "bool",
        DataType::Int8 => "int8",
        DataType::Int16 => "int16",
        DataType::Int32 => "int32",
        DataType::Int64 => "int64",
        DataType::UInt8 => "uint8",
        DataType::UInt16 => "uint16",
        DataType::UInt32 => "uint32",
        DataType::UInt64 => "uint64",
        DataType::Float32 => "float32",
        DataType::Float64 => "float64",
        DataType::Date32 => "date_days",
        DataType::Timestamp(_, _) => "timestamp_micros",
        _ => "utf8",
    }
}

fn physical_kind_for_logical_name(logical: &str) -> &'static str {
    match logical {
        "bool" | "boolean" => "boolean",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "num_code",
        "uuid" | "decimal128" => "fixed_bytes",
        "list" => "list",
        "struct" => "struct",
        "map" => "map",
        _ => "var_bytes",
    }
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
        candidate_projection_rows: None,
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

fn projection_rows_all_columns(
    bytes: &[u8],
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns: None,
        pushed_filters: Vec::new(),
        batch_size: options.batch_size,
        candidate_projection_rows: None,
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
    record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })
}

fn table_relation_context_required(planned: &PlannedQuery) -> bool {
    !planned.resolved.method_chain.lookups.is_empty()
        || !planned.resolved.method_chain.joins.is_empty()
        || !planned.resolved.method_chain.set_operations.is_empty()
        || !planned.resolved.method_chain.windows.is_empty()
        || matches!(
            &planned.resolved.root,
            ResolvedRoot::Table(root) if table_root_uses_registered_rows(root)
        )
        || planned
            .resolved
            .method_chain
            .where_predicate
            .as_ref()
            .is_some_and(predicate_contains_table_exists)
}

fn table_root_uses_registered_rows(root: &crate::ResolvedTableRoot) -> bool {
    matches!(
        &root.execution_authority,
        TableExecutionAuthority::MaterializedRows { .. }
            | TableExecutionAuthority::RawRows { .. }
            | TableExecutionAuthority::ExternalRows { .. }
    )
}

fn predicate_contains_table_exists(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::TableExists(_)) => true,
        ResolvedPredicate::Compare { left, right, .. } => {
            expr_contains_table_exists(left) || expr_contains_table_exists(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => expr_contains_table_exists(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_table_exists(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_table_exists)
        }
    }
}

fn expr_contains_table_exists(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::TableExists(_) => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(expr_contains_table_exists),
        ResolvedExpr::AggregateCall { arg, .. } => {
            arg.as_deref().is_some_and(expr_contains_table_exists)
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_table_exists(predicate)
                || expr_contains_table_exists(then_expr)
                || expr_contains_table_exists(else_expr)
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => false,
    }
}

fn execute_table_relation_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    table: &crate::ResolvedTableRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let left_rows = table_rows_all_columns(bytes, options, started, table)?;
    let mut joined_rows = left_rows
        .into_iter()
        .map(|row| namespace_table_projection_row(row, table))
        .collect::<Vec<_>>();
    let context = EvalContext::for_plan(&[], &[], planned);
    for lookup in &planned.resolved.method_chain.lookups {
        let right_rows = table_rows_all_columns(bytes, options, started, &lookup.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &lookup.right))
            .collect::<Vec<_>>();
        let mut next_rows = Vec::new();
        for left in joined_rows {
            let mut matches = Vec::new();
            for right in &right_rows {
                let candidate = merge_lookup_projection_rows(&left, right);
                let row = ExecutionRow::Projection(candidate.clone());
                let predicate_matches =
                    eval_predicate(&lookup.on, &row, &context).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!("lookup predicate evaluation failed: {}", err.message),
                            json!({}),
                        )
                    })?;
                if predicate_matches
                    || (lookup.nulls_match && lookup_join_keys_are_both_null(&lookup.on, &row))
                {
                    matches.push(candidate);
                }
            }
            if matches.is_empty() {
                match lookup.unmatched_policy {
                    TableLookupUnmatchedPolicy::Nulls => next_rows.push(left),
                    TableLookupUnmatchedPolicy::Reject => {
                        return Err(exec_error(
                            "E_LOOKUP_UNMATCHED_ROW",
                            "lookup unmatched: reject encountered a visible left row without a visible matching right row",
                            json!({
                                "right_table": lookup.right.table_name,
                                "cardinality": format!("{:?}", lookup.cardinality),
                            }),
                        ));
                    }
                }
                continue;
            }
            if lookup.cardinality == TableLookupCardinality::One
                && matches.len() > 1
                && lookup.duplicate_policy == TableLookupDuplicatePolicy::Reject
            {
                return Err(exec_error(
                    "E_LOOKUP_DUPLICATE_MATCH",
                    "lookup cardinality: one found more than one visible right-side match",
                    json!({
                        "right_table": lookup.right.table_name,
                        "match_count": matches.len(),
                    }),
                ));
            }
            match lookup.cardinality {
                TableLookupCardinality::One => {
                    next_rows.push(matches.into_iter().next().expect("non-empty matches"));
                }
                TableLookupCardinality::Many => {
                    next_rows.extend(matches);
                }
            }
        }
        joined_rows = next_rows;
    }
    for join in &planned.resolved.method_chain.joins {
        let right_rows = table_rows_all_columns(bytes, options, started, &join.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &join.right))
            .collect::<Vec<_>>();
        joined_rows = apply_materialized_table_join(joined_rows, &right_rows, join, &context)?;
    }
    for operation in &planned.resolved.method_chain.set_operations {
        let right_rows = table_rows_all_columns(bytes, options, started, &operation.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &operation.right))
            .collect::<Vec<_>>();
        joined_rows = apply_materialized_set_operation(joined_rows, right_rows, operation);
    }
    if !planned.resolved.method_chain.windows.is_empty() {
        joined_rows = apply_materialized_windows(joined_rows, planned, &context)?;
    }
    let table_exists_storage = table_exists_row_storage(bytes, planned, options, started)?;
    let table_exists_refs = table_exists_storage
        .iter()
        .map(|(table_id, rows)| (table_id.clone(), rows.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let context =
        EvalContext::for_plan_with_table_exists_rows(&[], &[], planned, table_exists_refs);
    let rows = joined_rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

fn apply_materialized_table_join(
    left_rows: Vec<MaterializedProjectionRow>,
    right_rows: &[MaterializedProjectionRow],
    join: &crate::ResolvedTableJoin,
    context: &EvalContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let mut out = Vec::new();
    let mut matched_right = BTreeSet::new();
    for left in left_rows {
        let mut matches = Vec::new();
        for (right_index, right) in right_rows.iter().enumerate() {
            let candidate = merge_lookup_projection_rows(&left, right);
            let row = ExecutionRow::Projection(candidate.clone());
            let predicate_matches = eval_predicate(&join.on, &row, context).map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("join predicate evaluation failed: {}", err.message),
                    json!({}),
                )
            })?;
            if predicate_matches
                || (join.nulls_match && lookup_join_keys_are_both_null(&join.on, &row))
            {
                matched_right.insert(right_index);
                matches.push(candidate);
            }
        }
        if matches.is_empty() {
            match join.join_kind {
                TableJoinKind::Left | TableJoinKind::Full => out.push(left),
                TableJoinKind::Anti => out.push(left),
                TableJoinKind::Inner | TableJoinKind::Right | TableJoinKind::Semi => {}
            }
            if join.unmatched_policy == TableLookupUnmatchedPolicy::Reject
                && matches!(join.join_kind, TableJoinKind::Inner | TableJoinKind::Semi)
            {
                return Err(exec_error(
                    "E_JOIN_UNMATCHED_ROW",
                    "join unmatched: reject encountered a visible left row without a visible matching right row",
                    json!({ "right_table": join.right.table_name }),
                ));
            }
            continue;
        }
        if join.cardinality == TableLookupCardinality::One
            && matches.len() > 1
            && join.duplicate_policy == TableLookupDuplicatePolicy::Reject
        {
            return Err(exec_error(
                "E_JOIN_DUPLICATE_MATCH",
                "join cardinality: one found more than one visible right-side match",
                json!({ "right_table": join.right.table_name, "match_count": matches.len() }),
            ));
        }
        match join.join_kind {
            TableJoinKind::Semi => out.push(left),
            TableJoinKind::Anti => {}
            TableJoinKind::Inner
            | TableJoinKind::Left
            | TableJoinKind::Right
            | TableJoinKind::Full => {
                if join.cardinality == TableLookupCardinality::One {
                    out.push(matches.into_iter().next().expect("non-empty join matches"));
                } else {
                    out.extend(matches);
                }
            }
        }
    }
    if matches!(join.join_kind, TableJoinKind::Right | TableJoinKind::Full) {
        for (right_index, right) in right_rows.iter().enumerate() {
            if !matched_right.contains(&right_index) {
                out.push(right.clone());
            }
        }
    }
    Ok(out)
}

fn apply_materialized_set_operation(
    left_rows: Vec<MaterializedProjectionRow>,
    right_rows: Vec<MaterializedProjectionRow>,
    operation: &crate::ResolvedSetOperation,
) -> Vec<MaterializedProjectionRow> {
    match operation.kind {
        crate::SetOperationKind::Union if operation.all => {
            let mut out = left_rows;
            out.extend(right_rows);
            out
        }
        crate::SetOperationKind::Union => {
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .chain(right_rows)
                .filter(|row| seen.insert(materialized_projection_row_key(row)))
                .collect()
        }
        crate::SetOperationKind::Intersect => {
            let mut right_counts = multiset_counts(&right_rows);
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .filter(|row| {
                    let key = materialized_projection_row_key(row);
                    if operation.all {
                        let Some(count) = right_counts.get_mut(&key) else {
                            return false;
                        };
                        if *count == 0 {
                            return false;
                        }
                        *count -= 1;
                        true
                    } else if right_counts.contains_key(&key) {
                        seen.insert(key)
                    } else {
                        false
                    }
                })
                .collect()
        }
        crate::SetOperationKind::Except => {
            let mut right_counts = multiset_counts(&right_rows);
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .filter(|row| {
                    let key = materialized_projection_row_key(row);
                    if operation.all {
                        if let Some(count) = right_counts.get_mut(&key) {
                            if *count > 0 {
                                *count -= 1;
                                return false;
                            }
                        }
                        true
                    } else if right_counts.contains_key(&key) {
                        false
                    } else {
                        seen.insert(key)
                    }
                })
                .collect()
        }
    }
}

fn multiset_counts(rows: &[MaterializedProjectionRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts
            .entry(materialized_projection_row_key(row))
            .or_insert(0) += 1;
    }
    counts
}

fn materialized_projection_row_key(row: &MaterializedProjectionRow) -> String {
    stable_value_key(&Value::Object(
        row.values
            .iter()
            .filter(|(key, _)| {
                !key.contains('.') && !key.starts_with(INTERNAL_PROJECTION_FIELD_PREFIX)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}

fn apply_materialized_windows(
    mut rows: Vec<MaterializedProjectionRow>,
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let window_functions = window_functions_for_plan(planned);
    for window in &planned.resolved.method_chain.windows {
        let mut partitions = BTreeMap::<String, Vec<usize>>::new();
        for (index, row) in rows.iter().enumerate() {
            let exec_row = ExecutionRow::Projection(row.clone());
            let key_values = window
                .partition_by
                .iter()
                .map(|expr| eval_expr(expr, &exec_row, context))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!(
                            "window partition expression evaluation failed: {}",
                            err.message
                        ),
                        json!({}),
                    )
                })?;
            let key = serde_json::to_string(&key_values).unwrap_or_else(|_| "[]".into());
            partitions.entry(key).or_default().push(index);
        }
        for indices in partitions.into_values() {
            let mut ordered = indices;
            if let Some(order) = &window.order_by {
                ordered.sort_by(|left, right| {
                    let left_row = ExecutionRow::Projection(rows[*left].clone());
                    let right_row = ExecutionRow::Projection(rows[*right].clone());
                    let left_value =
                        eval_expr(&order.expr, &left_row, context).unwrap_or(Value::Null);
                    let right_value =
                        eval_expr(&order.expr, &right_row, context).unwrap_or(Value::Null);
                    compare_sort_values(
                        &left_value,
                        &right_value,
                        order.direction,
                        order.nulls,
                        expr_logical_type(&order.expr),
                        expr_collation_id(&order.expr),
                    )
                    .then_with(|| {
                        materialized_projection_row_key(&rows[*left])
                            .cmp(&materialized_projection_row_key(&rows[*right]))
                    })
                });
            }
            let mut previous_order_key: Option<String> = None;
            let mut rank = 1usize;
            let mut dense_rank = 1usize;
            for (position, row_index) in ordered.iter().enumerate() {
                let order_key = if let Some(order) = &window.order_by {
                    let exec_row = ExecutionRow::Projection(rows[*row_index].clone());
                    let value = eval_expr(&order.expr, &exec_row, context).unwrap_or(Value::Null);
                    stable_value_key(&value)
                } else {
                    position.to_string()
                };
                if let Some(previous) = &previous_order_key {
                    if previous != &order_key {
                        rank = position + 1;
                        dense_rank += 1;
                    }
                }
                previous_order_key = Some(order_key);
                let row_number = json!((position + 1) as u64);
                let rank_value = json!(rank as u64);
                let dense_rank_value = json!(dense_rank as u64);
                rows[*row_index]
                    .values
                    .insert("row_number".into(), row_number.clone());
                rows[*row_index]
                    .values
                    .insert("rank".into(), rank_value.clone());
                rows[*row_index]
                    .values
                    .insert("dense_rank".into(), dense_rank_value.clone());
                for function in &window_functions {
                    let value = match function.name.as_str() {
                        "row_number" => row_number.clone(),
                        "rank" => rank_value.clone(),
                        "dense_rank" => dense_rank_value.clone(),
                        _ => evaluate_window_function(
                            function, &ordered, position, window, &rows, context,
                        )?,
                    };
                    rows[*row_index].values.insert(function.key.clone(), value);
                }
            }
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowFunctionSpec {
    key: String,
    name: String,
    args: Vec<ResolvedExpr>,
}

fn window_functions_for_plan(planned: &PlannedQuery) -> Vec<WindowFunctionSpec> {
    let mut functions = BTreeMap::<String, WindowFunctionSpec>::new();
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_window_functions_from_expr(&item.expr, &mut functions);
        }
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_window_functions_from_predicate(predicate, &mut functions);
    }
    if let Some(order) = &planned.resolved.method_chain.order_by {
        collect_window_functions_from_expr(&order.expr, &mut functions);
    }
    functions.into_values().collect()
}

fn collect_window_functions_from_predicate(
    predicate: &ResolvedPredicate,
    out: &mut BTreeMap<String, WindowFunctionSpec>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_window_functions_from_expr(left, out);
            collect_window_functions_from_expr(right, out);
        }
        ResolvedPredicate::InList { expr, .. } | ResolvedPredicate::NullCheck { expr, .. } => {
            collect_window_functions_from_expr(expr, out);
        }
        ResolvedPredicate::Exists(expr) | ResolvedPredicate::BoolExpr(expr) => {
            collect_window_functions_from_expr(expr, out);
        }
        ResolvedPredicate::Not(inner) => collect_window_functions_from_predicate(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_window_functions_from_predicate(part, out);
            }
        }
    }
}

fn collect_window_functions_from_expr(
    expr: &ResolvedExpr,
    out: &mut BTreeMap<String, WindowFunctionSpec>,
) {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id, args, ..
        } => {
            for arg in args {
                collect_window_functions_from_expr(arg, out);
            }
            if is_materialized_window_function(function_id) {
                let key = window_function_key(function_id, args);
                out.entry(key.clone())
                    .or_insert_with(|| WindowFunctionSpec {
                        key,
                        name: function_id.clone(),
                        args: args.clone(),
                    });
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_window_functions_from_predicate(predicate, out);
            collect_window_functions_from_expr(then_expr, out);
            collect_window_functions_from_expr(else_expr, out);
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_window_functions_from_expr(arg, out);
            }
        }
        ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_)
        | ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::TableExists(_) => {}
    }
}

fn is_materialized_window_function(name: &str) -> bool {
    matches!(
        name,
        "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
            | "first_value"
            | "last_value"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "count"
    )
}

fn evaluate_window_function(
    function: &WindowFunctionSpec,
    ordered: &[usize],
    position: usize,
    window: &crate::ResolvedWindowSpec,
    rows: &[MaterializedProjectionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    let (start, end) = window_frame_bounds(window, position, ordered.len())?;
    match function.name.as_str() {
        "lag" | "lead" => {
            let offset = window_offset_arg(function)?;
            let target_position = if function.name == "lag" {
                position.checked_sub(offset)
            } else {
                position
                    .checked_add(offset)
                    .filter(|target| *target < ordered.len())
            };
            let Some(target_position) = target_position else {
                return Ok(Value::Null);
            };
            evaluate_window_arg(&function.args[0], ordered[target_position], rows, context)
        }
        "first_value" => evaluate_window_arg(&function.args[0], ordered[start], rows, context),
        "last_value" => evaluate_window_arg(&function.args[0], ordered[end], rows, context),
        "count" => {
            if function.args.is_empty() {
                return Ok(json!((end - start + 1) as u64));
            }
            let mut count = 0u64;
            for row_index in &ordered[start..=end] {
                if !evaluate_window_arg(&function.args[0], *row_index, rows, context)?.is_null() {
                    count += 1;
                }
            }
            Ok(json!(count))
        }
        "sum" | "avg" | "min" | "max" => {
            let values = ordered[start..=end]
                .iter()
                .map(|row_index| evaluate_window_arg(&function.args[0], *row_index, rows, context))
                .collect::<Result<Vec<_>, _>>()?;
            match function.name.as_str() {
                "sum" => numeric_aggregate(AstAggregateName::Sum, function.args.first(), &values),
                "avg" => numeric_aggregate(AstAggregateName::Avg, function.args.first(), &values),
                "min" => Ok(ordered_aggregate(
                    AstAggregateName::Min,
                    function.args.first(),
                    &values,
                )),
                "max" => Ok(ordered_aggregate(
                    AstAggregateName::Max,
                    function.args.first(),
                    &values,
                )),
                _ => unreachable!("guarded by caller"),
            }
        }
        "row_number" | "rank" | "dense_rank" => unreachable!("handled by caller"),
        _ => Ok(Value::Null),
    }
}

fn evaluate_window_arg(
    expr: &ResolvedExpr,
    row_index: usize,
    rows: &[MaterializedProjectionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    eval_expr(
        expr,
        &ExecutionRow::Projection(rows[row_index].clone()),
        context,
    )
    .map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!("window expression evaluation failed: {}", err.message),
            json!({}),
        )
    })
}

fn window_offset_arg(function: &WindowFunctionSpec) -> Result<usize, BuildExecutionError> {
    let Some(offset) = function.args.get(1) else {
        return Ok(1);
    };
    let ResolvedExpr::Literal(literal) = offset else {
        return Err(exec_error(
            "E_WINDOW",
            "lag/lead offset must be an integer literal",
            json!({ "function": function.name }),
        ));
    };
    let value = match &literal.typed_value {
        ResolvedLiteralValue::SignedInteger(value) => usize::try_from(*value).ok(),
        ResolvedLiteralValue::UnsignedInteger(value) => usize::try_from(*value).ok(),
        _ => None,
    }
    .ok_or_else(|| {
        exec_error(
            "E_WINDOW",
            "lag/lead offset must be a non-negative integer in range",
            json!({ "function": function.name }),
        )
    })?;
    Ok(value)
}

fn window_frame_bounds(
    window: &crate::ResolvedWindowSpec,
    position: usize,
    len: usize,
) -> Result<(usize, usize), BuildExecutionError> {
    if len == 0 {
        return Err(exec_error("E_WINDOW", "empty window partition", json!({})));
    }
    let start = match window.start.as_str() {
        "unbounded_preceding" => 0,
        "current_row" => position,
        value => {
            return Err(exec_error(
                "E_WINDOW",
                format!("unsupported window frame start {value}"),
                json!({ "frame": format!("{:?}", window.frame) }),
            ))
        }
    };
    let end = match window.end.as_str() {
        "current_row" => position,
        "unbounded_following" => len - 1,
        value => {
            return Err(exec_error(
                "E_WINDOW",
                format!("unsupported window frame end {value}"),
                json!({ "frame": format!("{:?}", window.frame) }),
            ))
        }
    };
    if start > end {
        return Err(exec_error(
            "E_WINDOW",
            "window frame start is after frame end",
            json!({ "start": window.start, "end": window.end }),
        ));
    }
    Ok((start, end))
}

fn table_rows_all_columns(
    bytes: &[u8],
    options: &ExecutionOptions,
    started: Instant,
    table: &crate::ResolvedTableRoot,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    match &table.execution_authority {
        TableExecutionAuthority::DeterministicProjection { projection_id } => {
            projection_rows_all_columns(bytes, options, started, projection_id)
        }
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows } => Ok(rows
            .iter()
            .map(|values| MaterializedProjectionRow {
                projection_id: table.table_id.clone(),
                values: values.clone(),
            })
            .collect()),
        TableExecutionAuthority::ExternalRows {
            provider_id: _,
            rows,
        } => Ok(rows
            .iter()
            .map(|values| MaterializedProjectionRow {
                projection_id: table.table_id.clone(),
                values: values.clone(),
            })
            .collect()),
    }
}

fn lookup_join_keys_are_both_null(predicate: &ResolvedPredicate, row: &ExecutionRow) -> bool {
    let ResolvedPredicate::Compare {
        left,
        op: AstCompareOp::Eq,
        right,
    } = predicate
    else {
        return false;
    };
    let (ResolvedExpr::Path(left), ResolvedExpr::Path(right)) = (left, right) else {
        return false;
    };
    row.value_for_path(left).is_null() && row.value_for_path(right).is_null()
}

fn table_exists_row_storage(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Vec<(String, Vec<MaterializedProjectionRow>)>, BuildExecutionError> {
    let mut roots = Vec::new();
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_table_exists_roots(predicate, &mut roots);
    }
    roots
        .into_iter()
        .map(|root| {
            let rows = table_rows_all_columns(bytes, options, started, &root)?
                .into_iter()
                .map(|row| namespace_table_projection_row(row, &root))
                .collect::<Vec<_>>();
            Ok((root.table_id, rows))
        })
        .collect()
}

fn collect_table_exists_roots(
    predicate: &ResolvedPredicate,
    out: &mut Vec<crate::ResolvedTableRoot>,
) {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::TableExists(exists)) => {
            if !out
                .iter()
                .any(|root| root.table_id == exists.right.table_id)
            {
                out.push(exists.right.clone());
            }
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_table_exists_expr_roots(left, out);
            collect_table_exists_expr_roots(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => collect_table_exists_expr_roots(expr, out),
        ResolvedPredicate::Not(inner) => collect_table_exists_roots(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_table_exists_roots(part, out);
            }
        }
    }
}

fn collect_table_exists_expr_roots(expr: &ResolvedExpr, out: &mut Vec<crate::ResolvedTableRoot>) {
    match expr {
        ResolvedExpr::TableExists(exists) => {
            if !out
                .iter()
                .any(|root| root.table_id == exists.right.table_id)
            {
                out.push(exists.right.clone());
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_table_exists_expr_roots(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg.as_deref() {
                collect_table_exists_expr_roots(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_table_exists_roots(predicate, out);
            collect_table_exists_expr_roots(then_expr, out);
            collect_table_exists_expr_roots(else_expr, out);
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => {}
    }
}

fn namespace_table_projection_row(
    mut row: MaterializedProjectionRow,
    table: &crate::ResolvedTableRoot,
) -> MaterializedProjectionRow {
    let base = row.values.clone();
    for label in table_binding_labels(table) {
        for (column, value) in &base {
            row.values
                .insert(format!("{label}.{column}"), value.clone());
        }
    }
    row.projection_id = table.table_id.clone();
    row
}

fn table_binding_labels(table: &crate::ResolvedTableRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &table.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &table.table_name) {
        labels.push(table.table_name.clone());
    }
    labels
}

fn merge_lookup_projection_rows(
    left: &MaterializedProjectionRow,
    right: &MaterializedProjectionRow,
) -> MaterializedProjectionRow {
    let mut values = left.values.clone();
    for (key, value) in &right.values {
        if key.contains('.') {
            values.insert(key.clone(), value.clone());
        } else {
            values.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("lookup:{}:{}", left.projection_id, right.projection_id),
        values,
    }
}

fn execute_graph_traverse_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    root: &crate::ResolvedGraphNodeRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let read_result = read_object_surface_from_bytes_with_pushdown_options(
        bytes,
        &CoveObjectReadWithPushdownOptions {
            read: object_read_options(planned),
            pushdown: CoveObjectReadPushdownOptions::default(),
        },
    )
    .map_err(|err| {
        exec_error(
            "E_READBACK",
            format!("COVE-O materialized readback failed: {err}"),
            json!({}),
        )
    })?;
    let surface = read_result.surface;
    execute_graph_traverse_surface_root(&surface, planned, options, started, root)
}

fn execute_graph_traverse_surface_root(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    root: &crate::ResolvedGraphNodeRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let states = object_states_for_temporal_context(surface, planned)?;
    let object_rows = states
        .iter()
        .map(MaterializedObjectRow::from_state)
        .collect::<Vec<_>>();
    let visible_object_rows = filter_object_context_rows(&object_rows, planned, options);
    let associations = states
        .iter()
        .filter_map(MaterializedAssociationRow::from_state)
        .collect::<Vec<_>>();
    let visible_associations = filter_association_context_rows(&associations, planned, options);
    let mut by_type_and_goid = BTreeMap::<(u32, String), MaterializedObjectRow>::new();
    let object_goids = object_rows
        .iter()
        .map(|row| row.goid.clone())
        .collect::<BTreeSet<_>>();
    let mut visible_object_goids = BTreeSet::<String>::new();
    for row in &visible_object_rows {
        visible_object_goids.insert(row.goid.clone());
        by_type_and_goid.insert((row.object_type_id, row.goid.clone()), row.clone());
    }
    let mut rows = visible_object_rows
        .iter()
        .filter(|row| row.object_type_id == root.object.object_type_id)
        .map(|row| {
            let state = GraphPathState::new(row.goid.clone());
            with_graph_path_state(namespace_graph_node_projection_row(row, root, true), &state)
        })
        .collect::<Vec<_>>();
    for traversal in &planned.resolved.method_chain.traversals {
        rows = expand_graph_traversal_rows(
            &rows,
            traversal,
            root,
            &visible_associations,
            &by_type_and_goid,
            &object_goids,
            &visible_object_goids,
            planned,
            options,
            started,
        )?;
    }
    if !planned.resolved.method_chain.graph_algorithms.is_empty() {
        rows = apply_graph_algorithms(
            rows,
            &planned.resolved.method_chain.graph_algorithms,
            root,
            &visible_associations,
            &by_type_and_goid,
            &object_goids,
            &visible_object_goids,
            planned,
            options,
            started,
        )?;
    }
    let rows = rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    let context = EvalContext::for_plan_with_objects(
        &visible_associations,
        &[],
        &visible_object_rows,
        planned,
    );
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

fn apply_graph_algorithms(
    rows: Vec<MaterializedProjectionRow>,
    algorithms: &[crate::ResolvedGraphAlgorithm],
    root: &crate::ResolvedGraphNodeRoot,
    visible_associations: &[MaterializedAssociationRow],
    by_type_and_goid: &BTreeMap<(u32, String), MaterializedObjectRow>,
    object_goids: &BTreeSet<String>,
    visible_object_goids: &BTreeSet<String>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let mut rows = rows;
    for algorithm in algorithms {
        check_time(&options.resource_budget, started)?;
        rows = apply_graph_algorithm(
            rows,
            algorithm,
            root,
            visible_associations,
            by_type_and_goid,
            object_goids,
            visible_object_goids,
            planned,
            options,
            started,
        )?;
    }
    Ok(rows)
}

fn apply_graph_algorithm(
    rows: Vec<MaterializedProjectionRow>,
    algorithm: &crate::ResolvedGraphAlgorithm,
    root: &crate::ResolvedGraphNodeRoot,
    visible_associations: &[MaterializedAssociationRow],
    by_type_and_goid: &BTreeMap<(u32, String), MaterializedObjectRow>,
    object_goids: &BTreeSet<String>,
    visible_object_goids: &BTreeSet<String>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    match algorithm.kind {
        GraphAlgorithmKind::AllPaths | GraphAlgorithmKind::KShortestPaths => {
            let Some(edge) = &algorithm.edge else {
                return Ok(rows
                    .into_iter()
                    .map(|mut row| {
                        row.values.insert("path_count".into(), json!(0u64));
                        row
                    })
                    .collect());
            };
            let traversal = crate::ResolvedGraphTraversal {
                direction: algorithm.direction,
                edge: edge.clone(),
                target: algorithm.target.clone(),
                min_depth: 1,
                max_depth: algorithm
                    .max_depth
                    .unwrap_or(algorithm.contract.max_depth)
                    .max(1),
                mode: GraphTraversalMode::SimplePath,
                distinct: GraphTraversalDistinctPolicy::Path,
                contract: Some(GraphTraversalContract {
                    contract_version: algorithm.contract.contract_version.clone(),
                    allow_variable_length: true,
                    supported_modes: vec![
                        GraphTraversalMode::Walk,
                        GraphTraversalMode::Trail,
                        GraphTraversalMode::SimplePath,
                    ],
                    supported_distinct_policies: vec![
                        GraphTraversalDistinctPolicy::None,
                        GraphTraversalDistinctPolicy::Path,
                        GraphTraversalDistinctPolicy::EndNode,
                    ],
                    max_depth: algorithm.contract.max_depth,
                    max_fanout_per_node: options.resource_budget.maximum_graph_traversal_fanout,
                    max_paths: algorithm.max_paths.unwrap_or(algorithm.contract.max_paths),
                    max_frontier: options.resource_budget.maximum_graph_traversal_frontier,
                    path_identity: vec![
                        "start_goid".into(),
                        "edge_goids".into(),
                        "node_goids".into(),
                    ],
                    hidden_endpoint_policy: algorithm.contract.disclosure_policy.clone(),
                    ordering_policy: algorithm.contract.ordering_policy.clone(),
                    execution_authority: "materialized_graph_algorithm_oracle".into(),
                }),
            };
            return expand_graph_traversal_rows(
                &rows,
                &traversal,
                root,
                visible_associations,
                by_type_and_goid,
                object_goids,
                visible_object_goids,
                planned,
                options,
                started,
            );
        }
        _ => {}
    }

    let adjacency = GraphAdjacency::from_visible_associations(
        visible_associations,
        algorithm.edge.as_ref(),
        algorithm.direction,
        object_goids,
        visible_object_goids,
        planned,
    );
    let start_nodes = rows
        .iter()
        .filter_map(|row| graph_row_current_goid(row, root))
        .collect::<BTreeSet<_>>();
    let graph_nodes = graph_algorithm_nodes(&adjacency, &start_nodes);
    let component_ids = if matches!(
        algorithm.kind,
        GraphAlgorithmKind::ConnectedComponents | GraphAlgorithmKind::Community
    ) {
        Some(graph_component_ids(&adjacency, algorithm.variant.as_str()))
    } else {
        None
    };
    let betweenness_scores =
        if algorithm.kind == GraphAlgorithmKind::Centrality && algorithm.variant == "betweenness" {
            Some(graph_betweenness_centrality_scores(
                &adjacency,
                &graph_nodes,
            ))
        } else {
            None
        };
    let community_ids = if algorithm.kind == GraphAlgorithmKind::Community
        && algorithm.variant == "label_propagation"
    {
        Some(graph_label_propagation_communities(
            &adjacency,
            &graph_nodes,
            graph_algorithm_iterations(algorithm),
        ))
    } else {
        None
    };
    let spanning_forest = if algorithm.kind == GraphAlgorithmKind::SpanningTree {
        Some(graph_spanning_forest(
            &adjacency,
            &graph_nodes,
            algorithm,
            visible_associations,
            planned,
            started,
            options,
        )?)
    } else {
        None
    };
    let pagerank_scores = if matches!(algorithm.kind, GraphAlgorithmKind::PageRank) {
        Some(graph_pagerank_scores(&adjacency, &graph_nodes, algorithm))
    } else {
        None
    };
    let hits_scores = if matches!(algorithm.kind, GraphAlgorithmKind::Hits) {
        Some(graph_hits_scores(&adjacency, &graph_nodes, algorithm))
    } else {
        None
    };
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        let Some(start) = graph_row_current_goid(&row, root) else {
            out.push(row);
            continue;
        };
        let reachable = graph_reachable_distances(
            &adjacency,
            &start,
            algorithm.max_depth.unwrap_or(algorithm.contract.max_depth),
            algorithm.max_paths.unwrap_or(algorithm.contract.max_paths),
            started,
            options,
        )?;
        match algorithm.kind {
            GraphAlgorithmKind::Reachable => {
                row.values
                    .insert("reachable".into(), json!(!reachable.is_empty()));
                row.values
                    .insert("reachable_count".into(), json!(reachable.len() as u64));
            }
            GraphAlgorithmKind::ShortestPath => {
                let distance = shortest_target_distance(
                    &reachable,
                    algorithm.target.as_ref(),
                    by_type_and_goid,
                );
                row.values.insert(
                    "shortest_distance".into(),
                    distance.map_or(Value::Null, |distance| json!(distance)),
                );
            }
            GraphAlgorithmKind::ConnectedComponents => {
                let component = component_ids
                    .as_ref()
                    .and_then(|components| components.get(&start))
                    .copied()
                    .unwrap_or(0);
                row.values
                    .insert("component_id".into(), json!(component as u64));
            }
            GraphAlgorithmKind::Degree => {
                let out_degree = adjacency.outgoing.get(&start).map(Vec::len).unwrap_or(0);
                let in_degree = adjacency.incoming.get(&start).map(Vec::len).unwrap_or(0);
                let degree = match algorithm.variant.as_str() {
                    "in" => in_degree,
                    "total" => out_degree + in_degree,
                    _ => out_degree,
                };
                row.values
                    .insert("out_degree".into(), json!(out_degree as u64));
                row.values
                    .insert("in_degree".into(), json!(in_degree as u64));
                row.values.insert("degree".into(), json!(degree as u64));
            }
            GraphAlgorithmKind::PageRank => {
                let score = pagerank_scores
                    .as_ref()
                    .and_then(|scores| scores.get(&start))
                    .copied()
                    .unwrap_or(0.0);
                row.values.insert("pagerank".into(), json!(score));
            }
            GraphAlgorithmKind::Hits => {
                let (authority, hub) = hits_scores
                    .as_ref()
                    .and_then(|scores| scores.get(&start))
                    .copied()
                    .unwrap_or((0.0, 0.0));
                row.values.insert("authority".into(), json!(authority));
                row.values.insert("hub".into(), json!(hub));
            }
            GraphAlgorithmKind::Centrality => {
                let score = match algorithm.variant.as_str() {
                    "degree" => graph_degree_centrality(&adjacency, &start, graph_nodes.len()),
                    "betweenness" => betweenness_scores
                        .as_ref()
                        .and_then(|scores| scores.get(&start))
                        .copied()
                        .unwrap_or(0.0),
                    _ => graph_closeness_centrality(
                        &adjacency,
                        &start,
                        graph_nodes.len(),
                        algorithm,
                        started,
                        options,
                    )?,
                };
                row.values.insert("centrality".into(), json!(score));
            }
            GraphAlgorithmKind::TriangleCount => {
                row.values.insert(
                    "triangle_count".into(),
                    json!(triangle_count_for(&adjacency, &start) as u64),
                );
            }
            GraphAlgorithmKind::ClusteringCoefficient => {
                let coefficient = clustering_coefficient_for(&adjacency, &start);
                row.values
                    .insert("clustering_coefficient".into(), json!(coefficient));
            }
            GraphAlgorithmKind::Community => {
                let community = community_ids
                    .as_ref()
                    .and_then(|communities| communities.get(&start))
                    .copied()
                    .or_else(|| {
                        component_ids
                            .as_ref()
                            .and_then(|components| components.get(&start))
                            .copied()
                    })
                    .unwrap_or_else(|| community_id_for(&start));
                row.values
                    .insert("community_id".into(), json!(community as u64));
            }
            GraphAlgorithmKind::SpanningTree => {
                let (parent, depth) = spanning_forest
                    .as_ref()
                    .and_then(|forest| forest.get(&start))
                    .cloned()
                    .unwrap_or((None, 0));
                row.values.insert(
                    "tree_parent".into(),
                    parent.map_or(Value::Null, Value::String),
                );
                row.values.insert("tree_depth".into(), json!(depth));
            }
            GraphAlgorithmKind::AllPaths | GraphAlgorithmKind::KShortestPaths => {
                unreachable!("handled above")
            }
        }
        out.push(row);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
struct GraphAdjacency {
    outgoing: BTreeMap<String, Vec<String>>,
    incoming: BTreeMap<String, Vec<String>>,
}

impl GraphAdjacency {
    fn from_visible_associations(
        associations: &[MaterializedAssociationRow],
        edge: Option<&crate::ResolvedGraphEdgeRoot>,
        direction: crate::AstAssociationDirection,
        object_goids: &BTreeSet<String>,
        visible_object_goids: &BTreeSet<String>,
        planned: &PlannedQuery,
    ) -> Self {
        let mut graph = Self::default();
        for association in associations {
            if edge.as_ref().is_some_and(|edge| {
                association.object_type_id != edge.association.object_type_id
                    || !association_row_matches_temporal(
                        association,
                        &edge.association,
                        association_valid_at(planned),
                    )
            }) {
                continue;
            }
            let (Some(source), Some(target)) = (&association.source_goid, &association.target_goid)
            else {
                continue;
            };
            if (object_goids.contains(source) && !visible_object_goids.contains(source))
                || (object_goids.contains(target) && !visible_object_goids.contains(target))
            {
                continue;
            }
            match direction {
                crate::AstAssociationDirection::Out => graph.add_edge(source, target),
                crate::AstAssociationDirection::In => graph.add_edge(target, source),
                crate::AstAssociationDirection::Either => {
                    graph.add_edge(source, target);
                    graph.add_edge(target, source);
                }
            }
        }
        graph.normalize();
        graph
    }

    fn add_edge(&mut self, source: &str, target: &str) {
        self.outgoing
            .entry(source.to_string())
            .or_default()
            .push(target.to_string());
        self.incoming
            .entry(target.to_string())
            .or_default()
            .push(source.to_string());
    }

    fn normalize(&mut self) {
        for targets in self.outgoing.values_mut() {
            targets.sort();
            targets.dedup();
        }
        for sources in self.incoming.values_mut() {
            sources.sort();
            sources.dedup();
        }
    }
}

fn graph_reachable_distances(
    adjacency: &GraphAdjacency,
    start: &str,
    max_depth: u32,
    max_paths: usize,
    started: Instant,
    options: &ExecutionOptions,
) -> Result<BTreeMap<String, u32>, BuildExecutionError> {
    let mut distances = BTreeMap::new();
    let mut frontier = vec![(start.to_string(), 0u32)];
    let mut seen = BTreeSet::from([start.to_string()]);
    while let Some((node, depth)) = frontier.pop() {
        check_time(&options.resource_budget, started)?;
        if depth >= max_depth {
            continue;
        }
        if let Some(targets) = adjacency.outgoing.get(&node) {
            for target in targets {
                if !seen.insert(target.clone()) {
                    continue;
                }
                let next_depth = depth + 1;
                distances.insert(target.clone(), next_depth);
                if distances.len() > max_paths {
                    return Err(resource_error(
                        "maximum_graph_traversal_paths",
                        distances.len(),
                    ));
                }
                frontier.push((target.clone(), next_depth));
            }
        }
    }
    Ok(distances)
}

fn graph_algorithm_nodes(
    adjacency: &GraphAdjacency,
    start_nodes: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut nodes = start_nodes.clone();
    for (source, targets) in &adjacency.outgoing {
        nodes.insert(source.clone());
        nodes.extend(targets.iter().cloned());
    }
    for (target, sources) in &adjacency.incoming {
        nodes.insert(target.clone());
        nodes.extend(sources.iter().cloned());
    }
    nodes
}

fn graph_algorithm_iterations(algorithm: &crate::ResolvedGraphAlgorithm) -> usize {
    algorithm
        .max_iterations
        .unwrap_or(20)
        .min(algorithm.contract.max_iterations)
        .max(1)
}

fn graph_algorithm_tolerance(algorithm: &crate::ResolvedGraphAlgorithm) -> f64 {
    algorithm
        .tolerance
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1e-9)
}

fn graph_pagerank_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
    algorithm: &crate::ResolvedGraphAlgorithm,
) -> BTreeMap<String, f64> {
    let count = nodes.len();
    if count == 0 {
        return BTreeMap::new();
    }
    let damping = 0.85f64;
    let base = (1.0 - damping) / count as f64;
    let mut scores = nodes
        .iter()
        .map(|node| (node.clone(), 1.0 / count as f64))
        .collect::<BTreeMap<_, _>>();
    let tolerance = graph_algorithm_tolerance(algorithm);
    for _ in 0..graph_algorithm_iterations(algorithm) {
        let mut next = nodes
            .iter()
            .map(|node| (node.clone(), base))
            .collect::<BTreeMap<_, _>>();
        let mut sink_score = 0.0f64;
        for node in nodes {
            let score = *scores.get(node).unwrap_or(&0.0);
            let outgoing = adjacency
                .outgoing
                .get(node)
                .map(|targets| {
                    targets
                        .iter()
                        .filter(|target| nodes.contains(*target))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if outgoing.is_empty() {
                sink_score += score;
                continue;
            }
            let share = damping * score / outgoing.len() as f64;
            for target in outgoing {
                *next.entry(target.clone()).or_insert(base) += share;
            }
        }
        if sink_score > 0.0 {
            let share = damping * sink_score / count as f64;
            for value in next.values_mut() {
                *value += share;
            }
        }
        let delta = nodes
            .iter()
            .map(|node| {
                (next.get(node).copied().unwrap_or(0.0) - scores.get(node).copied().unwrap_or(0.0))
                    .abs()
            })
            .sum::<f64>();
        scores = next;
        if delta <= tolerance {
            break;
        }
    }
    scores
}

fn graph_hits_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
    algorithm: &crate::ResolvedGraphAlgorithm,
) -> BTreeMap<String, (f64, f64)> {
    if nodes.is_empty() {
        return BTreeMap::new();
    }
    let mut authority = nodes
        .iter()
        .map(|node| (node.clone(), 1.0f64))
        .collect::<BTreeMap<_, _>>();
    let mut hub = authority.clone();
    let tolerance = graph_algorithm_tolerance(algorithm);
    for _ in 0..graph_algorithm_iterations(algorithm) {
        let mut next_authority = BTreeMap::new();
        for node in nodes {
            let value = adjacency
                .incoming
                .get(node)
                .into_iter()
                .flat_map(|sources| sources.iter())
                .filter(|source| nodes.contains(*source))
                .map(|source| hub.get(source).copied().unwrap_or(0.0))
                .sum::<f64>();
            next_authority.insert(node.clone(), value);
        }
        normalize_scores(&mut next_authority);
        let mut next_hub = BTreeMap::new();
        for node in nodes {
            let value = adjacency
                .outgoing
                .get(node)
                .into_iter()
                .flat_map(|targets| targets.iter())
                .filter(|target| nodes.contains(*target))
                .map(|target| next_authority.get(target).copied().unwrap_or(0.0))
                .sum::<f64>();
            next_hub.insert(node.clone(), value);
        }
        normalize_scores(&mut next_hub);
        let delta = nodes
            .iter()
            .map(|node| {
                (next_authority.get(node).copied().unwrap_or(0.0)
                    - authority.get(node).copied().unwrap_or(0.0))
                .abs()
                    + (next_hub.get(node).copied().unwrap_or(0.0)
                        - hub.get(node).copied().unwrap_or(0.0))
                    .abs()
            })
            .sum::<f64>();
        authority = next_authority;
        hub = next_hub;
        if delta <= tolerance {
            break;
        }
    }
    nodes
        .iter()
        .map(|node| {
            (
                node.clone(),
                (
                    authority.get(node).copied().unwrap_or(0.0),
                    hub.get(node).copied().unwrap_or(0.0),
                ),
            )
        })
        .collect()
}

fn normalize_scores(scores: &mut BTreeMap<String, f64>) {
    let norm = scores
        .values()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return;
    }
    for value in scores.values_mut() {
        *value /= norm;
    }
}

fn shortest_target_distance(
    reachable: &BTreeMap<String, u32>,
    target: Option<&crate::ResolvedGraphNodeRoot>,
    by_type_and_goid: &BTreeMap<(u32, String), MaterializedObjectRow>,
) -> Option<u32> {
    if let Some(target) = target {
        reachable
            .iter()
            .filter(|(goid, _)| {
                by_type_and_goid.contains_key(&(target.object.object_type_id, (*goid).clone()))
            })
            .map(|(_, distance)| *distance)
            .min()
    } else {
        reachable.values().copied().min()
    }
}

fn graph_closeness_centrality(
    adjacency: &GraphAdjacency,
    start: &str,
    node_count: usize,
    algorithm: &crate::ResolvedGraphAlgorithm,
    started: Instant,
    options: &ExecutionOptions,
) -> Result<f64, BuildExecutionError> {
    if node_count <= 1 {
        return Ok(0.0);
    }
    let reachable = graph_reachable_distances(
        adjacency,
        start,
        algorithm.max_depth.unwrap_or(algorithm.contract.max_depth),
        algorithm.max_paths.unwrap_or(algorithm.contract.max_paths),
        started,
        options,
    )?;
    let distance_sum = reachable
        .values()
        .map(|distance| *distance as f64)
        .sum::<f64>();
    if distance_sum <= f64::EPSILON {
        return Ok(0.0);
    }
    Ok(reachable.len() as f64 / distance_sum)
}

fn graph_component_ids(adjacency: &GraphAdjacency, kind: &str) -> BTreeMap<String, usize> {
    if kind == "strong" {
        return graph_strong_component_ids(adjacency);
    }
    graph_weak_component_ids(adjacency)
}

fn graph_weak_component_ids(adjacency: &GraphAdjacency) -> BTreeMap<String, usize> {
    let mut nodes = BTreeSet::new();
    for (source, targets) in &adjacency.outgoing {
        nodes.insert(source.clone());
        nodes.extend(targets.iter().cloned());
    }
    let mut components = BTreeMap::new();
    let mut component_id = 0usize;
    for node in nodes {
        if components.contains_key(&node) {
            continue;
        }
        component_id += 1;
        let mut stack = vec![node.clone()];
        while let Some(current) = stack.pop() {
            if components.insert(current.clone(), component_id).is_some() {
                continue;
            }
            if let Some(next) = adjacency.outgoing.get(&current) {
                stack.extend(next.iter().cloned());
            }
            if let Some(prev) = adjacency.incoming.get(&current) {
                stack.extend(prev.iter().cloned());
            }
        }
    }
    components
}

fn graph_strong_component_ids(adjacency: &GraphAdjacency) -> BTreeMap<String, usize> {
    let nodes = graph_algorithm_nodes(adjacency, &BTreeSet::new());
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::new();
    for node in &nodes {
        graph_scc_finish_order(node, adjacency, &mut visited, &mut finish_order);
    }
    let mut components = BTreeMap::new();
    let mut component_id = 0usize;
    for node in finish_order.into_iter().rev() {
        if components.contains_key(&node) {
            continue;
        }
        component_id += 1;
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if components.insert(current.clone(), component_id).is_some() {
                continue;
            }
            if let Some(sources) = adjacency.incoming.get(&current) {
                for source in sources.iter().rev() {
                    if !components.contains_key(source) {
                        stack.push(source.clone());
                    }
                }
            }
        }
    }
    components
}

fn graph_scc_finish_order(
    node: &str,
    adjacency: &GraphAdjacency,
    visited: &mut BTreeSet<String>,
    finish_order: &mut Vec<String>,
) {
    if !visited.insert(node.to_string()) {
        return;
    }
    if let Some(targets) = adjacency.outgoing.get(node) {
        for target in targets {
            graph_scc_finish_order(target, adjacency, visited, finish_order);
        }
    }
    finish_order.push(node.to_string());
}

fn graph_degree_centrality(adjacency: &GraphAdjacency, node: &str, node_count: usize) -> f64 {
    if node_count <= 1 {
        return 0.0;
    }
    let out_degree = adjacency.outgoing.get(node).map(Vec::len).unwrap_or(0);
    let in_degree = adjacency.incoming.get(node).map(Vec::len).unwrap_or(0);
    (out_degree + in_degree) as f64 / (node_count - 1) as f64
}

fn graph_betweenness_centrality_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
) -> BTreeMap<String, f64> {
    let mut scores = nodes
        .iter()
        .map(|node| (node.clone(), 0.0f64))
        .collect::<BTreeMap<_, _>>();
    for source in nodes {
        let mut stack = Vec::<String>::new();
        let mut predecessors = nodes
            .iter()
            .map(|node| (node.clone(), Vec::<String>::new()))
            .collect::<BTreeMap<_, _>>();
        let mut sigma = nodes
            .iter()
            .map(|node| (node.clone(), 0.0f64))
            .collect::<BTreeMap<_, _>>();
        let mut distance = nodes
            .iter()
            .map(|node| (node.clone(), -1i64))
            .collect::<BTreeMap<_, _>>();
        sigma.insert(source.clone(), 1.0);
        distance.insert(source.clone(), 0);
        let mut queue = std::collections::VecDeque::from([source.clone()]);
        while let Some(current) = queue.pop_front() {
            stack.push(current.clone());
            let current_distance = distance.get(&current).copied().unwrap_or(-1);
            for target in adjacency.outgoing.get(&current).into_iter().flatten() {
                if !nodes.contains(target) {
                    continue;
                }
                if distance.get(target).copied().unwrap_or(-1) < 0 {
                    distance.insert(target.clone(), current_distance + 1);
                    queue.push_back(target.clone());
                }
                if distance.get(target).copied().unwrap_or(-1) == current_distance + 1 {
                    let current_sigma = sigma.get(&current).copied().unwrap_or(0.0);
                    *sigma.entry(target.clone()).or_insert(0.0) += current_sigma;
                    predecessors
                        .entry(target.clone())
                        .or_default()
                        .push(current.clone());
                }
            }
        }
        let mut dependency = nodes
            .iter()
            .map(|node| (node.clone(), 0.0f64))
            .collect::<BTreeMap<_, _>>();
        while let Some(node) = stack.pop() {
            for predecessor in predecessors.get(&node).into_iter().flatten() {
                let numerator = sigma.get(predecessor).copied().unwrap_or(0.0);
                let denominator = sigma.get(&node).copied().unwrap_or(0.0);
                if denominator <= f64::EPSILON {
                    continue;
                }
                let contribution =
                    numerator / denominator * (1.0 + dependency.get(&node).copied().unwrap_or(0.0));
                *dependency.entry(predecessor.clone()).or_insert(0.0) += contribution;
            }
            if &node != source {
                *scores.entry(node.clone()).or_insert(0.0) +=
                    dependency.get(&node).copied().unwrap_or(0.0);
            }
        }
    }
    scores
}

fn graph_label_propagation_communities(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
    max_iterations: usize,
) -> BTreeMap<String, usize> {
    let mut labels = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.clone(), index + 1))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..max_iterations.max(1) {
        let mut changed = false;
        let mut next = labels.clone();
        for node in nodes {
            let mut counts = BTreeMap::<usize, usize>::new();
            for neighbor in adjacency
                .outgoing
                .get(node)
                .into_iter()
                .flatten()
                .chain(adjacency.incoming.get(node).into_iter().flatten())
            {
                if let Some(label) = labels.get(neighbor) {
                    *counts.entry(*label).or_insert(0) += 1;
                }
            }
            let Some((label, _)) = counts.into_iter().max_by(
                |(left_label, left_count), (right_label, right_count)| {
                    left_count
                        .cmp(right_count)
                        .then_with(|| right_label.cmp(left_label))
                },
            ) else {
                continue;
            };
            if next.get(node).copied() != Some(label) {
                next.insert(node.clone(), label);
                changed = true;
            }
        }
        labels = next;
        if !changed {
            break;
        }
    }
    labels
}

fn graph_spanning_forest(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
    algorithm: &crate::ResolvedGraphAlgorithm,
    associations: &[MaterializedAssociationRow],
    planned: &PlannedQuery,
    started: Instant,
    options: &ExecutionOptions,
) -> Result<BTreeMap<String, (Option<String>, u32)>, BuildExecutionError> {
    match algorithm.variant.as_str() {
        "dfs" => Ok(graph_search_spanning_forest(adjacency, &nodes, true)),
        "min_weight" => graph_min_weight_spanning_forest(
            nodes,
            algorithm,
            associations,
            planned,
            started,
            options,
        ),
        _ => Ok(graph_search_spanning_forest(adjacency, nodes, false)),
    }
}

fn graph_search_spanning_forest(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<String>,
    depth_first: bool,
) -> BTreeMap<String, (Option<String>, u32)> {
    let mut forest = BTreeMap::<String, (Option<String>, u32)>::new();
    for root in nodes {
        if forest.contains_key(root) {
            continue;
        }
        forest.insert(root.clone(), (None, 0));
        if depth_first {
            let mut stack = vec![root.clone()];
            while let Some(node) = stack.pop() {
                let depth = forest.get(&node).map(|(_, depth)| *depth).unwrap_or(0);
                let Some(targets) = adjacency.outgoing.get(&node) else {
                    continue;
                };
                for target in targets.iter().rev() {
                    if forest.contains_key(target) {
                        continue;
                    }
                    forest.insert(target.clone(), (Some(node.clone()), depth + 1));
                    stack.push(target.clone());
                }
            }
        } else {
            let mut queue = std::collections::VecDeque::from([root.clone()]);
            while let Some(node) = queue.pop_front() {
                let depth = forest.get(&node).map(|(_, depth)| *depth).unwrap_or(0);
                let Some(targets) = adjacency.outgoing.get(&node) else {
                    continue;
                };
                for target in targets {
                    if forest.contains_key(target) {
                        continue;
                    }
                    forest.insert(target.clone(), (Some(node.clone()), depth + 1));
                    queue.push_back(target.clone());
                }
            }
        }
    }
    forest
}

#[derive(Debug, Clone)]
struct GraphWeightedEdge {
    source: String,
    target: String,
    weight: f64,
}

fn graph_min_weight_spanning_forest(
    nodes: &BTreeSet<String>,
    algorithm: &crate::ResolvedGraphAlgorithm,
    associations: &[MaterializedAssociationRow],
    planned: &PlannedQuery,
    started: Instant,
    options: &ExecutionOptions,
) -> Result<BTreeMap<String, (Option<String>, u32)>, BuildExecutionError> {
    let mut disjoint = nodes
        .iter()
        .map(|node| (node.clone(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut edges = graph_visible_weighted_edges(nodes, algorithm, associations, planned)?;
    edges.sort_by(|left, right| {
        left.weight
            .partial_cmp(&right.weight)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
    });
    let mut tree = GraphAdjacency::default();
    for edge in edges {
        check_time(&options.resource_budget, started)?;
        let source_root = graph_find_root(&mut disjoint, &edge.source);
        let target_root = graph_find_root(&mut disjoint, &edge.target);
        if source_root == target_root {
            continue;
        }
        disjoint.insert(target_root, source_root);
        tree.add_edge(&edge.source, &edge.target);
        tree.add_edge(&edge.target, &edge.source);
    }
    tree.normalize();
    Ok(graph_search_spanning_forest(&tree, nodes, false))
}

fn graph_find_root(parents: &mut BTreeMap<String, String>, node: &str) -> String {
    let parent = parents
        .get(node)
        .cloned()
        .unwrap_or_else(|| node.to_string());
    if parent == node {
        return parent;
    }
    let root = graph_find_root(parents, &parent);
    parents.insert(node.to_string(), root.clone());
    root
}

fn graph_visible_weighted_edges(
    nodes: &BTreeSet<String>,
    algorithm: &crate::ResolvedGraphAlgorithm,
    associations: &[MaterializedAssociationRow],
    planned: &PlannedQuery,
) -> Result<Vec<GraphWeightedEdge>, BuildExecutionError> {
    let Some(weight_expr) = &algorithm.weight else {
        return Err(exec_error(
            "E_GRAPH_ALGORITHM",
            "min_weight spanning tree requires a weight expression",
            json!({}),
        ));
    };
    let context = EvalContext::for_plan(&[], &[], planned);
    let mut edges = Vec::new();
    for association in associations {
        if algorithm.edge.as_ref().is_some_and(|edge| {
            association.object_type_id != edge.association.object_type_id
                || !association_row_matches_temporal(
                    association,
                    &edge.association,
                    association_valid_at(planned),
                )
        }) {
            continue;
        }
        let weight_value = eval_expr(
            weight_expr,
            &ExecutionRow::Association(association.clone()),
            &context,
        )
        .map_err(|err| {
            exec_error(
                "E_GRAPH_ALGORITHM",
                format!("spanningTree weight evaluation failed: {}", err.message),
                json!({}),
            )
        })?;
        let weight = match weight_value {
            Value::Number(number) => number_to_f64(&number),
            _ => None,
        }
        .ok_or_else(|| {
            exec_error(
                "E_GRAPH_ALGORITHM",
                "spanningTree weight must evaluate to a numeric value",
                json!({}),
            )
        })?;
        if !weight.is_finite() || weight < 0.0 {
            return Err(exec_error(
                "E_GRAPH_ALGORITHM",
                "spanningTree weight must be finite and non-negative",
                json!({ "weight": weight }),
            ));
        }
        let (Some(source), Some(target)) = (&association.source_goid, &association.target_goid)
        else {
            continue;
        };
        for (source, target) in graph_oriented_edge_pairs(source, target, algorithm.direction) {
            if nodes.contains(&source) && nodes.contains(&target) {
                edges.push(GraphWeightedEdge {
                    source,
                    target,
                    weight,
                });
            }
        }
    }
    Ok(edges)
}

fn graph_oriented_edge_pairs(
    source: &str,
    target: &str,
    direction: crate::AstAssociationDirection,
) -> Vec<(String, String)> {
    match direction {
        crate::AstAssociationDirection::Out => vec![(source.to_string(), target.to_string())],
        crate::AstAssociationDirection::In => vec![(target.to_string(), source.to_string())],
        crate::AstAssociationDirection::Either => vec![
            (source.to_string(), target.to_string()),
            (target.to_string(), source.to_string()),
        ],
    }
}

fn triangle_count_for(adjacency: &GraphAdjacency, node: &str) -> usize {
    let Some(neighbors) = adjacency.outgoing.get(node) else {
        return 0;
    };
    let neighbor_set = neighbors.iter().collect::<BTreeSet<_>>();
    let mut count = 0usize;
    for neighbor in neighbors {
        if let Some(second_hop) = adjacency.outgoing.get(neighbor) {
            count += second_hop
                .iter()
                .filter(|candidate| neighbor_set.contains(candidate))
                .count();
        }
    }
    count / 2
}

fn clustering_coefficient_for(adjacency: &GraphAdjacency, node: &str) -> f64 {
    let degree = adjacency.outgoing.get(node).map(Vec::len).unwrap_or(0);
    if degree < 2 {
        return 0.0;
    }
    let triangles = triangle_count_for(adjacency, node) as f64;
    let possible = (degree * (degree - 1) / 2) as f64;
    triangles / possible
}

fn community_id_for(goid: &str) -> usize {
    goid.as_bytes().iter().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(*byte as usize)
    })
}

#[derive(Debug, Clone)]
struct GraphPathState {
    start_goid: String,
    current_goid: String,
    node_goids: Vec<String>,
    edge_goids: Vec<String>,
    depth: u32,
}

impl GraphPathState {
    fn new(start_goid: String) -> Self {
        Self {
            start_goid: start_goid.clone(),
            current_goid: start_goid.clone(),
            node_goids: vec![start_goid],
            edge_goids: Vec::new(),
            depth: 0,
        }
    }

    fn extend(&self, edge_goid: String, target_goid: String) -> Self {
        let mut node_goids = self.node_goids.clone();
        node_goids.push(target_goid.clone());
        let mut edge_goids = self.edge_goids.clone();
        edge_goids.push(edge_goid);
        Self {
            start_goid: self.start_goid.clone(),
            current_goid: target_goid,
            node_goids,
            edge_goids,
            depth: self.depth + 1,
        }
    }
}

fn expand_graph_traversal_rows(
    rows: &[MaterializedProjectionRow],
    traversal: &crate::ResolvedGraphTraversal,
    root: &crate::ResolvedGraphNodeRoot,
    visible_associations: &[MaterializedAssociationRow],
    by_type_and_goid: &BTreeMap<(u32, String), MaterializedObjectRow>,
    object_goids: &BTreeSet<String>,
    visible_object_goids: &BTreeSet<String>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let fanout_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_fanout_per_node)
        .unwrap_or(options.resource_budget.maximum_graph_traversal_fanout)
        .min(options.resource_budget.maximum_graph_traversal_fanout);
    let path_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_paths)
        .unwrap_or(options.resource_budget.maximum_graph_traversal_paths)
        .min(options.resource_budget.maximum_graph_traversal_paths);
    let frontier_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_frontier)
        .unwrap_or(options.resource_budget.maximum_graph_traversal_frontier)
        .min(options.resource_budget.maximum_graph_traversal_frontier);
    let mut emitted = Vec::new();
    let mut seen = BTreeSet::<String>::new();
    if traversal.min_depth == 0 {
        for row in rows {
            if let Some(state) = graph_row_path_state(row, root) {
                push_graph_emitted_row(
                    row.clone(),
                    &state,
                    traversal.distinct,
                    &mut seen,
                    &mut emitted,
                );
            }
        }
    }
    let mut frontier = rows.to_vec();
    for depth in 1..=traversal.max_depth {
        check_time(&options.resource_budget, started)?;
        let mut next = Vec::new();
        for row in &frontier {
            let Some(state) = graph_row_path_state(row, root) else {
                continue;
            };
            let mut fanout = 0usize;
            for association in visible_associations.iter().filter(|association| {
                association.object_type_id == traversal.edge.association.object_type_id
                    && association_row_matches_temporal(
                        association,
                        &traversal.edge.association,
                        association_valid_at(planned),
                    )
            }) {
                let Some((target_goid, source_matches)) =
                    traversal_target_goid(&state.current_goid, association, traversal.direction)
                else {
                    continue;
                };
                if !source_matches
                    || (object_goids.contains(&target_goid)
                        && !visible_object_goids.contains(&target_goid))
                {
                    continue;
                }
                let edge_goid = association.goid.clone();
                if !graph_traversal_mode_allows(&state, &edge_goid, &target_goid, traversal.mode) {
                    continue;
                }
                fanout += 1;
                if fanout > fanout_limit {
                    return Err(resource_error("maximum_graph_traversal_fanout", fanout));
                }
                let mut candidate = merge_lookup_projection_rows(
                    row,
                    &namespace_graph_edge_projection_row(association, &traversal.edge),
                );
                if let Some(target) = &traversal.target {
                    let Some(target_row) =
                        by_type_and_goid.get(&(target.object.object_type_id, target_goid.clone()))
                    else {
                        continue;
                    };
                    candidate = merge_lookup_projection_rows(
                        &candidate,
                        &namespace_graph_node_projection_row(target_row, target, false),
                    );
                }
                let next_state = state.extend(edge_goid, target_goid);
                candidate = with_graph_path_state(candidate, &next_state);
                if depth >= traversal.min_depth {
                    push_graph_emitted_row(
                        candidate.clone(),
                        &next_state,
                        traversal.distinct,
                        &mut seen,
                        &mut emitted,
                    );
                    if emitted.len() > path_limit {
                        return Err(resource_error(
                            "maximum_graph_traversal_paths",
                            emitted.len(),
                        ));
                    }
                }
                next.push(candidate);
                if next.len() > frontier_limit {
                    return Err(resource_error(
                        "maximum_graph_traversal_frontier",
                        next.len(),
                    ));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(emitted)
}

fn graph_traversal_mode_allows(
    state: &GraphPathState,
    edge_goid: &str,
    target_goid: &str,
    mode: GraphTraversalMode,
) -> bool {
    match mode {
        GraphTraversalMode::Walk => true,
        GraphTraversalMode::Trail => !state.edge_goids.iter().any(|edge| edge == edge_goid),
        GraphTraversalMode::SimplePath => {
            !state.edge_goids.iter().any(|edge| edge == edge_goid)
                && !state.node_goids.iter().any(|node| node == target_goid)
        }
    }
}

fn push_graph_emitted_row(
    row: MaterializedProjectionRow,
    state: &GraphPathState,
    distinct: GraphTraversalDistinctPolicy,
    seen: &mut BTreeSet<String>,
    emitted: &mut Vec<MaterializedProjectionRow>,
) {
    let key = match distinct {
        GraphTraversalDistinctPolicy::None => None,
        GraphTraversalDistinctPolicy::Path => Some(format!(
            "{}|{}",
            state.start_goid,
            state.edge_goids.join(">")
        )),
        GraphTraversalDistinctPolicy::EndNode => {
            Some(format!("{}|{}", state.start_goid, state.current_goid))
        }
    };
    if let Some(key) = key {
        if !seen.insert(key) {
            return;
        }
    }
    emitted.push(row);
}

fn graph_row_path_state(
    row: &MaterializedProjectionRow,
    root: &crate::ResolvedGraphNodeRoot,
) -> Option<GraphPathState> {
    let current_goid = graph_row_current_goid(row, root)?;
    let start_goid = row
        .values
        .get(GRAPH_PATH_START_GOID_FIELD)
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| current_goid.clone());
    let node_goids = graph_string_array_field(row, GRAPH_PATH_NODE_GOIDS_FIELD)
        .unwrap_or_else(|| vec![current_goid.clone()]);
    let edge_goids = graph_string_array_field(row, GRAPH_PATH_EDGE_GOIDS_FIELD).unwrap_or_default();
    let depth = row
        .values
        .get(GRAPH_PATH_DEPTH_FIELD)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(edge_goids.len() as u32);
    Some(GraphPathState {
        start_goid,
        current_goid,
        node_goids,
        edge_goids,
        depth,
    })
}

fn graph_string_array_field(row: &MaterializedProjectionRow, field: &str) -> Option<Vec<String>> {
    row.values
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
}

fn graph_row_current_goid(
    row: &MaterializedProjectionRow,
    root: &crate::ResolvedGraphNodeRoot,
) -> Option<String> {
    if let Some(goid) = row
        .values
        .get(GRAPH_CURRENT_GOID_FIELD)
        .and_then(Value::as_str)
    {
        return Some(goid.to_string());
    }
    table_binding_labels_for_graph_node(root)
        .into_iter()
        .find_map(|label| {
            row.values
                .get(&format!("{label}.goid"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            row.values
                .get("goid")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn with_graph_current_goid(
    mut row: MaterializedProjectionRow,
    goid: &str,
) -> MaterializedProjectionRow {
    debug_assert!(GRAPH_CURRENT_GOID_FIELD.starts_with(INTERNAL_PROJECTION_FIELD_PREFIX));
    row.values.insert(
        GRAPH_CURRENT_GOID_FIELD.into(),
        Value::String(goid.to_string()),
    );
    row
}

fn with_graph_path_state(
    mut row: MaterializedProjectionRow,
    state: &GraphPathState,
) -> MaterializedProjectionRow {
    row = with_graph_current_goid(row, &state.current_goid);
    row.values.insert(
        GRAPH_PATH_START_GOID_FIELD.into(),
        Value::String(state.start_goid.clone()),
    );
    row.values.insert(
        GRAPH_PATH_NODE_GOIDS_FIELD.into(),
        Value::Array(
            state
                .node_goids
                .iter()
                .map(|goid| Value::String(goid.clone()))
                .collect(),
        ),
    );
    row.values.insert(
        GRAPH_PATH_EDGE_GOIDS_FIELD.into(),
        Value::Array(
            state
                .edge_goids
                .iter()
                .map(|goid| Value::String(goid.clone()))
                .collect(),
        ),
    );
    row.values
        .insert(GRAPH_PATH_DEPTH_FIELD.into(), json!(state.depth));
    row
}

fn traversal_target_goid(
    current_goid: &str,
    association: &MaterializedAssociationRow,
    direction: crate::AstAssociationDirection,
) -> Option<(String, bool)> {
    let source = association.source_goid.as_deref();
    let target = association.target_goid.as_deref();
    match direction {
        crate::AstAssociationDirection::Out => Some((target?.to_string(), source? == current_goid)),
        crate::AstAssociationDirection::In => Some((source?.to_string(), target? == current_goid)),
        crate::AstAssociationDirection::Either => {
            if source? == current_goid {
                Some((target?.to_string(), true))
            } else if target? == current_goid {
                Some((source?.to_string(), true))
            } else {
                None
            }
        }
    }
}

fn namespace_graph_node_projection_row(
    row: &MaterializedObjectRow,
    node: &crate::ResolvedGraphNodeRoot,
    include_unqualified: bool,
) -> MaterializedProjectionRow {
    let base = object_projection_values(row);
    let mut values = if include_unqualified {
        base.clone()
    } else {
        BTreeMap::new()
    };
    for label in table_binding_labels_for_graph_node(node) {
        for (field, value) in &base {
            values.insert(format!("{label}.{field}"), value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("node:{}", node.label),
        values,
    }
}

fn namespace_graph_edge_projection_row(
    row: &MaterializedAssociationRow,
    edge: &crate::ResolvedGraphEdgeRoot,
) -> MaterializedProjectionRow {
    let base = association_projection_values(row);
    let mut values = BTreeMap::new();
    for label in table_binding_labels_for_graph_edge(edge) {
        for (field, value) in &base {
            values.insert(format!("{label}.{field}"), value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("edge:{}", edge.label),
        values,
    }
}

fn table_binding_labels_for_graph_node(root: &crate::ResolvedGraphNodeRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &root.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &root.label) {
        labels.push(root.label.clone());
    }
    labels
}

fn table_binding_labels_for_graph_edge(root: &crate::ResolvedGraphEdgeRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &root.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &root.label) {
        labels.push(root.label.clone());
    }
    labels
}

fn object_projection_values(row: &MaterializedObjectRow) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    values.insert("object_type_id".into(), json!(row.object_type_id));
    values.insert(
        "object_type_name".into(),
        Value::String(row.object_type_name.clone()),
    );
    values.insert("branch_key".into(), json!(row.branch_key));
    values.insert("goid".into(), Value::String(row.goid.clone()));
    values.insert("record_id".into(), Value::String(row.record_id.clone()));
    values.insert("timestamp_us".into(), json!(row.timestamp_us));
    values.insert("csn".into(), json!(row.csn));
    values.insert("record_kind".into(), Value::String(row.record_kind.clone()));
    for (property, value) in &row.properties {
        values.insert(property.clone(), value.clone());
    }
    values
}

fn association_projection_values(row: &MaterializedAssociationRow) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    values.insert("association_type_id".into(), json!(row.object_type_id));
    values.insert(
        "association_type".into(),
        row.association_type
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert("branch_key".into(), json!(row.branch_key));
    values.insert("goid".into(), Value::String(row.goid.clone()));
    values.insert("association_goid".into(), Value::String(row.goid.clone()));
    values.insert("record_id".into(), Value::String(row.record_id.clone()));
    values.insert(
        "source_goid".into(),
        row.source_goid
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert(
        "target_goid".into(),
        row.target_goid
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert("timestamp_us".into(), json!(row.timestamp_us));
    values.insert("csn".into(), json!(row.csn));
    for (property, value) in &row.properties {
        values.insert(property.clone(), value.clone());
    }
    values
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
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Projection(_) | ResolvedRoot::Table(_)
    ) {
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
    if !matches!(root, ResolvedRoot::Projection(_) | ResolvedRoot::Table(_)) {
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
        ResolvedExpr::TableExists(exists) => {
            collect_projection_predicate_columns(&exists.on, columns);
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
    let column = path
        .projection_column
        .clone()
        .unwrap_or_else(|| path.display_name.clone());
    if matches!(path.root_kind, crate::ResolvedPathRootKind::Table) {
        column
            .rsplit_once('.')
            .map(|(_, unqualified)| unqualified.to_string())
            .unwrap_or(column)
    } else {
        column
    }
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
    object_rows: &[MaterializedObjectRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let context_associations = filter_association_context_rows(associations, planned, options);
    let context_evidence_rows = filter_evidence_context_rows(evidence_rows, planned, options);
    let context_object_rows = filter_object_context_rows(object_rows, planned, options);
    let context = EvalContext::for_plan_with_objects(
        &context_associations,
        &context_evidence_rows,
        &context_object_rows,
        planned,
    );
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

fn finish_materialized_rows_with_context(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    context: &EvalContext<'_>,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
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
        CoveQlOutputMode::ObjectRows => Ok((
            CoveQlExecutionResult::ObjectRows(
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
        CoveQlOutputMode::AssociationRows => Ok((
            CoveQlExecutionResult::AssociationRows(
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
        CoveQlOutputMode::EvidenceRows => Ok((
            CoveQlExecutionResult::EvidenceRows(
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
        CoveQlOutputMode::ProjectionRows => Ok((
            CoveQlExecutionResult::ProjectionRows(
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
        CoveQlOutputMode::JsonRows => {
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
        CoveQlOutputMode::ArrowRecordBatch { .. } => {
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
                CoveQlExecutionResult::ArrowRecordBatches(vec![batch]),
                ExecutionRowCounts {
                    input_rows,
                    filtered_rows,
                    output_rows,
                },
            ))
        }
        CoveQlOutputMode::ExplainJson | CoveQlOutputMode::DataFusionTableProvider => {
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

fn filter_object_context_rows(
    rows: &[MaterializedObjectRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
) -> Vec<MaterializedObjectRow> {
    let Some(overlay) = active_visibility_overlay(planned, options) else {
        return rows.to_vec();
    };
    rows.iter()
        .filter(|row| object_row_visible_in_overlay(row, overlay))
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

fn object_row_visible_in_overlay(row: &MaterializedObjectRow, overlay: &VisibilityOverlay) -> bool {
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
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let output_rows = json_rows.len();
    if matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. }
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
            CoveQlExecutionResult::ArrowRecordBatches(vec![batch]),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        ))
    } else {
        Ok((
            CoveQlExecutionResult::JsonRows(json_rows),
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
            "zero-copy Arrow output was requested, but CoveQL execution emitted owned materialized buffers",
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
        CoveQlOutputMode::ArrowRecordBatch {
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
            CoveQlOutputMode::ExplainJson | CoveQlOutputMode::DataFusionTableProvider
        )
    {
        return Err(exec_error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            "multi-file dataset scopes require manifest-member execution; the single-input CoveQL executor refuses to run a manifest-scoped plan against one file",
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
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) | ResolvedRoot::Evidence(_)
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
        (_, CoveQlOutputMode::JsonRows)
        | (_, CoveQlOutputMode::ArrowRecordBatch { .. })
        | (_, CoveQlOutputMode::ExplainJson) => true,
        (ResolvedRoot::Object(_), CoveQlOutputMode::ObjectRows) => true,
        (ResolvedRoot::Association(_), CoveQlOutputMode::AssociationRows) => true,
        (ResolvedRoot::Evidence(_), CoveQlOutputMode::EvidenceRows) => true,
        (ResolvedRoot::Projection(_), CoveQlOutputMode::ProjectionRows) => true,
        (ResolvedRoot::Table(_), CoveQlOutputMode::ProjectionRows) => true,
        (_, CoveQlOutputMode::DataFusionTableProvider) => true,
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
        ResolvedRoot::Node(_) => "node",
        ResolvedRoot::Edge(_) => "edge",
        ResolvedRoot::Table(_) => "table",
        ResolvedRoot::Projection(_) => "projection",
        ResolvedRoot::Evidence(_) => "evidence",
    }
}

pub(crate) fn enforce_result_budgets(
    result: &CoveQlExecutionResult,
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
    let decode_bytes = result_decode_size(result)?;
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
    result: &CoveQlExecutionResult,
) -> Result<String, BuildExecutionError> {
    if let CoveQlExecutionResult::ArrowRecordBatches(batches) = result {
        return Ok(arrow_record_batches_fingerprint(batches));
    }
    let value = result_json(result)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn result_decode_size(result: &CoveQlExecutionResult) -> Result<usize, BuildExecutionError> {
    if let CoveQlExecutionResult::ArrowRecordBatches(batches) = result {
        return Ok(arrow_record_batches_memory_size(batches));
    }
    serde_json::to_vec(&result_json(result)?)
        .map(|bytes| bytes.len())
        .map_err(|err| exec_error("E_OUTPUT", err.to_string(), json!({})))
}

fn arrow_record_batches_memory_size(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::get_array_memory_size).sum()
}

fn arrow_record_batches_fingerprint(batches: &[RecordBatch]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"coveql-arrow-record-batches-v1");
    hash_usize(&mut hasher, batches.len());
    for batch in batches {
        hash_usize(&mut hasher, batch.num_rows());
        hash_usize(&mut hasher, batch.num_columns());
        hash_arrow_schema(&mut hasher, batch.schema().as_ref());
        for column in batch.columns() {
            hash_arrow_array_data(&mut hasher, &column.to_data());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn hash_arrow_schema(hasher: &mut Sha256, schema: &arrow_schema::Schema) {
    hash_usize(hasher, schema.fields().len());
    for field in schema.fields() {
        hash_str(hasher, field.name());
        hash_str(hasher, &format!("{:?}", field.data_type()));
        hasher.update([u8::from(field.is_nullable())]);
        let mut metadata = field.metadata().iter().collect::<Vec<_>>();
        metadata.sort_by(|left, right| left.0.cmp(right.0));
        hash_usize(hasher, metadata.len());
        for (key, value) in metadata {
            hash_str(hasher, key);
            hash_str(hasher, value);
        }
    }
}

fn hash_arrow_array_data(hasher: &mut Sha256, data: &ArrayData) {
    hash_str(hasher, &format!("{:?}", data.data_type()));
    hash_usize(hasher, data.len());
    hash_usize(hasher, data.offset());
    if let Some(nulls) = data.nulls() {
        hasher.update([1]);
        hash_usize(hasher, nulls.len());
        hash_usize(hasher, nulls.offset());
        hash_usize(hasher, nulls.null_count());
        hash_usize(hasher, nulls.validity().len());
        hasher.update(nulls.validity());
    } else {
        hasher.update([0]);
    }
    hash_usize(hasher, data.buffers().len());
    for buffer in data.buffers() {
        hash_usize(hasher, buffer.len());
        hasher.update(buffer.as_slice());
    }
    hash_usize(hasher, data.child_data().len());
    for child in data.child_data() {
        hash_arrow_array_data(hasher, child);
    }
}

fn hash_str(hasher: &mut Sha256, value: &str) {
    hash_usize(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_usize(hasher: &mut Sha256, value: usize) {
    hasher.update((value as u64).to_le_bytes());
}

pub(crate) fn result_json(result: &CoveQlExecutionResult) -> Result<Value, BuildExecutionError> {
    Ok(match result {
        CoveQlExecutionResult::ObjectRows(rows) => {
            Value::Array(rows.iter().map(MaterializedObjectRow::to_json).collect())
        }
        CoveQlExecutionResult::AssociationRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedAssociationRow::to_json)
                .collect(),
        ),
        CoveQlExecutionResult::EvidenceRows(rows) => {
            Value::Array(rows.iter().map(MaterializedEvidenceRow::to_json).collect())
        }
        CoveQlExecutionResult::ProjectionRows(rows) => Value::Array(
            rows.iter()
                .map(MaterializedProjectionRow::to_json)
                .collect(),
        ),
        CoveQlExecutionResult::ArrowRecordBatches(batches) => Value::Array(
            record_batches_to_json_rows(batches)
                .map_err(|err| exec_error("E_ARROW_OUTPUT", err, json!({})))?,
        ),
        CoveQlExecutionResult::JsonRows(rows) => Value::Array(rows.clone()),
        CoveQlExecutionResult::ExplainJson(value) => value.clone(),
    })
}

fn max_output_columns(result: &CoveQlExecutionResult) -> Result<usize, BuildExecutionError> {
    let rows = match result {
        CoveQlExecutionResult::ArrowRecordBatches(batches) => {
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
        ResolvedExpr::TableExists(_) => "exists".into(),
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
