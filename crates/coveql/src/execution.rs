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
use arrow_schema::{ArrowError, DataType};
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
    graph_execution::{
        self, GraphAlgorithmOutput, GraphAlgorithmSpec, GraphEdge, GraphEdgeId,
        GraphExecutionBudget, GraphExecutionError, GraphNodeId, GraphPath, GraphTraversalLimits,
        GraphTraversalRow, GraphTraversalSpec,
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

mod aggregates;
mod graph;
mod manifest_execution;
mod projection_passthrough;
mod rendering;
mod resource_limits;
mod row_sources;
mod streaming;
mod table_relations;
mod temporal;
mod visibility;
mod windows;

use aggregates::*;
use graph::*;
use projection_passthrough::*;
use rendering::*;
use resource_limits::*;
use row_sources::*;
use table_relations::*;
use temporal::*;
use visibility::*;
use windows::*;

pub(crate) use aggregates::{apply_skip_take, sort_rows};
pub(crate) use manifest_execution::{
    combined_evidence_authority, manifest_member_plan, validate_manifest_execution_members,
    ManifestExecutionMemberRef,
};
pub use manifest_execution::{
    execute_manifest_planned_query, execute_manifest_planned_query_retained,
};
pub(crate) use projection_passthrough::{
    execute_projection_root, projection_readback_pushdown_report, projection_root_execution_rows,
};
pub(crate) use rendering::{
    finish_json_rows, finish_materialized_rows, predicate_matches, select_json_rows,
};
pub(crate) use resource_limits::{
    check_time, enforce_result_budgets, evidence_object_rows_from_states, exec_error, exec_warning,
    object_read_options, reconstruction_options, resource_error, result_fingerprint, result_json,
    validate_execution_grain, validate_execution_output_mode, validate_security_scope,
    zero_copy_owned_fallback_warning,
};
pub use streaming::{execute_planned_query_stream, CoveQlResultStream};
pub(crate) use temporal::{role_bound_states_at, temporal_association_rows, temporal_object_rows};

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

#[derive(Debug)]
enum ProjectionArrowPassthroughError {
    Project(ArrowError),
}

impl fmt::Display for ProjectionArrowPassthroughError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Project(err) => write!(f, "{err}"),
        }
    }
}

impl Error for ProjectionArrowPassthroughError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Project(err) => Some(err),
        }
    }
}

impl From<ArrowError> for ProjectionArrowPassthroughError {
    fn from(err: ArrowError) -> Self {
        Self::Project(err)
    }
}

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

#[allow(clippy::too_many_arguments)]
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

#[derive(Debug, Clone)]
struct MaterializedRowSource {
    rows: Vec<ExecutionRow>,
    associations: Vec<MaterializedAssociationRow>,
    evidence_rows: Vec<MaterializedEvidenceRow>,
    object_rows: Vec<MaterializedObjectRow>,
    pushdown_report: PushdownReport,
    evidence_authority: Option<EvidenceAuthority>,
}

#[cfg(test)]
mod tests;
