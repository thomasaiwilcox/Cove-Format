//! CoveQL operation context, parser, resolver, planner, and materialized executor.
//!
//! Materialized COVE-O and COVE-MAP readback remains the correctness baseline.
//! The crate covers Phase 0 through Phase 8 surfaces: operation-context
//! validation, parsing, resolution, logical planning, materialized execution,
//! conservative pushdown, physical sidecar validation, proof-gated execution,
//! and association/evidence optimization reports with fallback paths.
//!
//! Runtime-authoritative acceleration is intentionally narrow. Validated
//! COVE-I/COVX index-only aggregate answers, exact empty COVI lookup
//! short-circuits, exact COVI/COVX row-range lookup pruning, and validated
//! COVE-COVERAGE row-range pruning can execute when operation-context policy,
//! visibility, redaction, temporal mode, and proof metadata permit it;
//! materialized fingerprint comparison is available as a guard. Coded object
//! kernels cover same-domain FileCode equality, typed numeric predicates, and
//! COVE-E FileCode-to-execution-code equality remaps with residual materialized
//! verification. COVE-L zero-copy object Arrow output can execute through the
//! retained-input APIs for direct no-null NumCode property projections when
//! validated COVE-O page authority, lifetime, layout compatibility, visibility,
//! redaction, and disclosure policy all permit it. Borrowed-input APIs remain
//! backward compatible and either fail closed or materialize according to the
//! fallback policy. Dictionary-lifted functions and non-row-range COVI/COVERAGE
//! candidates stay fail-closed or diagnostic-only unless a runtime branch proves
//! equivalence to the materialized executor.

#![allow(
    clippy::cloned_ref_to_slice_refs,
    clippy::collapsible_match,
    clippy::derivable_impls,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::items_after_test_module,
    clippy::large_enum_variant,
    clippy::len_zero,
    clippy::manual_inspect,
    clippy::manual_is_multiple_of,
    clippy::manual_map,
    clippy::match_like_matches_macro,
    clippy::needless_borrow,
    clippy::needless_lifetimes,
    clippy::redundant_closure,
    clippy::too_many_arguments,
    clippy::trim_split_whitespace,
    clippy::type_complexity,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_map_or
)]

mod acceleration;
mod arrow_output;
mod association_opt;
mod ast;
mod beginner;
mod builder;
mod conformance;
mod dependencies;
#[cfg(feature = "datafusion")]
#[path = "datafusion.rs"]
mod df_provider;
mod evidence_opt;
mod execution;
mod explain;
mod expr_eval;
mod kernel_arrow;
mod kernel_execution;
mod kernel_metrics;
mod kernel_plan;
mod kernel_predicate;
mod kernel_reconstruct;
mod lineage;
mod logical_plan;
mod materialized;
mod operation_context;
mod parser;
mod physical_plan;
mod physical_predicate;
mod physical_printer;
mod physical_proofs;
mod physical_sidecars;
mod plan_printer;
mod predicate;
mod pushdown;
mod resolver;
mod zero_copy_arrow;

use serde::{Deserialize, Serialize};

pub use acceleration::{
    acceleration_report_json, apply_acceleration_bundle, discover_acceleration_bundle,
    generate_acceleration_sidecars, plan_acceleration, AccelerationBundleOptions,
    CoveAccelerationBundle, CoveAccelerationDiagnostic, CoveAccelerationSidecar,
    CoveAccelerationSidecarStatus, CoveGeneratedSidecar, CoveOptimizationAction,
    CoveOptimizationOptions, CoveOptimizationPlan, CoveOptimizationStep, CoveOptimizeReport,
    CoveSkippedSidecar,
};
pub use association_opt::{
    AssociationDirectionPlan, AssociationOptimizationDecision, AssociationOptimizationReport,
};
pub use ast::*;
pub use beginner::*;
pub use builder::*;
pub use conformance::*;
pub use dependencies::*;
#[cfg(feature = "datafusion")]
pub use df_provider::{
    datafusion_coveql_provider_for_plan, datafusion_dataset_provider_for_plan,
    datafusion_manifest_coveql_provider_for_plan, datafusion_object_pushdown_report_for_plan,
    datafusion_projection_pushdown_report_for_plan, datafusion_row_pushdown_report_for_plan,
    register_datafusion_coveql_memtable_for_plan, register_datafusion_coveql_provider_for_plan,
    register_datafusion_dataset_for_plan, register_datafusion_manifest_coveql_provider_for_plan,
    register_datafusion_projection_for_plan, CoveQlTableProvider, DataFusionCoveQlFilterOutcome,
    DataFusionCoveQlFilterOutcomeKind, DataFusionCoveQlProviderReport,
    DataFusionCoveQlPushdownReport, DataFusionCoveQlScanNegotiationReport,
    ManifestCoveQlTableProvider,
};
pub use evidence_opt::{
    EvidenceGrainIndexReport, EvidenceGrainKind, EvidenceOptimizationReport,
    EvidenceTargetIndexKind,
};
pub use execution::{
    execute_manifest_planned_query, execute_manifest_planned_query_retained, execute_planned_query,
    execute_planned_query_on_object_surface, execute_planned_query_retained,
    execute_planned_query_stream, parse_resolve_plan_and_execute_query,
    parse_resolve_plan_and_execute_query_on_object_surface, BuildExecutionError,
    CoveQlExecutionResult, CoveQlResultStream, CoveQlRetainedInput, CoveQlRetainedManifestMember,
    EvidenceAuthority, ExecutedQuery, ExecutionAuthorityReport, ExecutionAuthoritySource,
    ExecutionDiagnostic, ExecutionOptions, ExecutionRowCounts, VisibilityOverlay,
};
pub use explain::render_explain_text;
pub use kernel_execution::{
    execute_manifest_physical_planned_query, execute_physical_planned_query,
    execute_physical_planned_query_retained, parse_resolve_plan_build_physical_and_execute_query,
    parse_resolve_plan_build_physical_and_execute_query_retained, KernelExecutedQuery,
};
pub use kernel_metrics::{
    KernelCounters, KernelDecision, KernelDecisionKind, KernelExecutionMode,
    KernelExecutionOptions, KernelExecutionReport, KernelFallbackReason, KernelMetricSnapshot,
    OptimizationAuthorityReport, OptimizationAuthorityState,
};
pub use kernel_plan::{
    CodedOperatorContract, CodedRepresentationClass, KernelRootKind, KernelShape,
};
pub use lineage::LineageReuseReport;
pub use logical_plan::{
    build_logical_plan, parse_resolve_and_plan_query, BuildLogicalPlanError, CoveOLogicalPlan,
    ExprContext, LogicalNodeId, LogicalPlanDiagnostic, LogicalPlanFingerprint, LogicalPlanNode,
    LogicalPlanNodeKind, LogicalRootKind, PlanContext, PlanOptions, PlannedQuery, ScanGrain,
};
pub use materialized::{
    MaterializedAssociationRow, MaterializedChangeDetail, MaterializedChangeDiffKind,
    MaterializedEvidenceRow, MaterializedObjectRow, MaterializedProjectionRow, OutputGrain,
};
pub(crate) use operation_context::hex_lower;
pub use operation_context::*;
pub use parser::parse_query;
pub use physical_plan::{
    build_physical_plan, parse_resolve_plan_and_build_physical_plan, BuildPhysicalPlanError,
    CoveOPhysicalPlan, PhysicalNodeId, PhysicalOperatorContract, PhysicalPlanDiagnostic,
    PhysicalPlanFingerprint, PhysicalPlanNode, PhysicalPlanNodeKind, PhysicalPlanOptions,
    PhysicalPlannedQuery,
};
pub use physical_predicate::{
    PhysicalCodeDomainDescriptor, PhysicalExecutionCodeDomainDescriptor, PhysicalPredicateForm,
    PhysicalPredicateFormKind, PhysicalPredicateNormalForms, PhysicalRepresentationClass,
    SecurityScopeDescriptor,
};
pub use physical_proofs::{
    IndexCapabilityReport, LayoutRangePlan, PhysicalFallbackReport, ProofValidationReport,
    ZeroCopyEligibilityReport,
};
pub use physical_sidecars::{
    PhysicalSidecarInputs, PhysicalSidecarStatus, PhysicalSidecarValidation,
};
pub use predicate::{
    FilterClassification, LogicalPredicateForm, LogicalPredicateKind, PredicatePlacement,
    PredicateProofState, RepresentationClass,
};
pub use pushdown::{
    PushdownCounters, PushdownDecision, PushdownDecisionKind, PushdownOptions, PushdownOutcome,
    PushdownReport,
};
pub use resolver::{parse_and_resolve_query, resolve_query, BuildResolvedQueryError};

pub const COVEQL_LANGUAGE_VERSION: &str = "0.1";
pub const COVEQL_CORE_VERSION: &str = "0.1";
pub const COVEQL_GRAMMAR_VERSION: &str = "0.1";
pub const RESOLVED_AST_VERSION: &str = "0.1";
pub const LOGICAL_PLAN_VERSION: &str = "0.1";
pub const PHYSICAL_PLAN_VERSION: &str = "0.1";
pub const EXPLAIN_JSON_SCHEMA_VERSION: &str = "0.1";
pub const COVEQL_PROFILE_CONTRACT_VERSION: &str = "0.1";
pub const COVEQL_BRIDGE_CONTRACT_VERSION: &str = "0.1";
pub const COVEQL_OBJECT_PROFILE_VERSION: &str = "0.1";
pub const COVEQL_TABLE_PROFILE_VERSION: &str = "0.1";
pub const COVEQL_GRAPH_PROFILE_VERSION: &str = "0.1";
pub const COVEQL_AI_PROFILE_VERSION: &str = "0.1";
pub const PROJECTION_DEPENDENCY_CONTRACT_VERSION: &str = "0.3";
pub const PREDICATE_NORMAL_FORM_VERSION: &str = "0.1";
pub const CODED_OPERATOR_CONTRACT_VERSION: &str = "0.1";
pub const PREDICATE_REPRESENTATION_CONTRACT_VERSION: &str = "0.1";
pub const PHYSICAL_OPERATOR_CONTRACT_VERSION: &str = "0.1";
pub const PHYSICAL_SIDECAR_VALIDATION_VERSION: &str = "0.1";
pub const DATAFUSION_COVEQL_REPORT_VERSION: &str = "0.1";
pub const DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE: &str = "E_DATAFUSION_PUSH_FILTER_UNSAFE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlProfileId {
    Object,
    Table,
    Graph,
    Ai,
}

impl CoveQlProfileId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Table => "table",
            Self::Graph => "graph",
            Self::Ai => "ai",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "object" => Some(Self::Object),
            "table" => Some(Self::Table),
            "graph" => Some(Self::Graph),
            "ai" | "coveql-ai" | "coveql_ai" => Some(Self::Ai),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlRootKind {
    Object,
    Association,
    Evidence,
    Projection,
    Table,
    Node,
    Edge,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlInputGrain {
    ObjectState,
    AssociationState,
    EvidenceRow,
    ProjectionRow,
    TableRow,
    NodeState,
    EdgeState,
    PathBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoveQlCoreContract {
    pub language_version: &'static str,
    pub core_version: &'static str,
    pub grammar_version: &'static str,
    pub resolved_ast_version: &'static str,
    pub logical_plan_version: &'static str,
    pub physical_plan_version: &'static str,
    pub explain_json_schema_version: &'static str,
    pub profile_contract_version: &'static str,
    pub bridge_contract_version: &'static str,
    pub primary_profiles: &'static [CoveQlProfileId],
    pub common_roots: &'static [CoveQlRootKind],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoveQlProfileContract {
    pub profile_id: CoveQlProfileId,
    pub profile_version: &'static str,
    pub implemented: bool,
    pub supported_roots: &'static [CoveQlRootKind],
    pub input_grains: &'static [CoveQlInputGrain],
    pub root_authority: &'static [&'static str],
    pub identity_model: &'static [&'static str],
    pub canonical_order: &'static [&'static str],
    pub temporal_capabilities: &'static [&'static str],
    pub evidence_targets: &'static [&'static str],
    pub relationship_capabilities: &'static [&'static str],
    pub profile_methods: &'static [&'static str],
    pub bridge_requirements: &'static [&'static str],
    pub aggregate_rules: &'static [&'static str],
    pub null_missing_nan_rules: &'static [&'static str],
    pub security_barriers: &'static [&'static str],
    pub materialization_boundaries: &'static [&'static str],
    pub output_modes: &'static [&'static str],
    pub fingerprint_fields: &'static [&'static str],
    pub explain_fields: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoveQlBridgeContract {
    pub bridge_version: &'static str,
    pub source_profile: CoveQlProfileId,
    pub target_profile: CoveQlProfileId,
    pub requires_validated_bridge_or_materialized_fallback: bool,
}

pub const COVEQL_CORE_CONTRACT: CoveQlCoreContract = CoveQlCoreContract {
    language_version: COVEQL_LANGUAGE_VERSION,
    core_version: COVEQL_CORE_VERSION,
    grammar_version: COVEQL_GRAMMAR_VERSION,
    resolved_ast_version: RESOLVED_AST_VERSION,
    logical_plan_version: LOGICAL_PLAN_VERSION,
    physical_plan_version: PHYSICAL_PLAN_VERSION,
    explain_json_schema_version: EXPLAIN_JSON_SCHEMA_VERSION,
    profile_contract_version: COVEQL_PROFILE_CONTRACT_VERSION,
    bridge_contract_version: COVEQL_BRIDGE_CONTRACT_VERSION,
    primary_profiles: &[
        CoveQlProfileId::Object,
        CoveQlProfileId::Table,
        CoveQlProfileId::Graph,
        CoveQlProfileId::Ai,
    ],
    common_roots: &[CoveQlRootKind::Projection, CoveQlRootKind::Evidence],
};

pub const COVEQL_OBJECT_PROFILE_CONTRACT: CoveQlProfileContract = CoveQlProfileContract {
    profile_id: CoveQlProfileId::Object,
    profile_version: COVEQL_OBJECT_PROFILE_VERSION,
    implemented: true,
    supported_roots: &[
        CoveQlRootKind::Object,
        CoveQlRootKind::Association,
        CoveQlRootKind::Projection,
        CoveQlRootKind::Evidence,
    ],
    input_grains: &[
        CoveQlInputGrain::ObjectState,
        CoveQlInputGrain::AssociationState,
        CoveQlInputGrain::ProjectionRow,
        CoveQlInputGrain::EvidenceRow,
    ],
    root_authority: &[
        "cove_o_object_catalog",
        "cove_map_projection_catalog",
        "cove_map_evidence_index",
    ],
    identity_model: &[
        "object_goid",
        "association_endpoint_identity",
        "projection_row_identity",
    ],
    canonical_order: &[
        "object_type_id",
        "branch_key",
        "goid",
        "projection_declared_ordering",
    ],
    temporal_capabilities: &["latest", "as_of", "history", "changes"],
    evidence_targets: &["object", "property", "association", "projection", "source"],
    relationship_capabilities: &[
        "association_exists",
        "association_count",
        "object_relative_association",
    ],
    profile_methods: &[],
    bridge_requirements: &[
        "object_graph_identity_bridge",
        "object_table_optional_identity_bridge",
    ],
    aggregate_rules: &[
        "visible_grain_after_visibility_and_redaction",
        "aggregate_disclosure_policy",
    ],
    null_missing_nan_rules: &[
        "missing_property_unknown",
        "null_three_valued_logic",
        "nan_ordering_requires_policy",
    ],
    security_barriers: &[
        "visibility",
        "redaction",
        "metadata_disclosure",
        "aggregate_disclosure",
    ],
    materialization_boundaries: &[
        "final_output",
        "unsupported_udf",
        "unsafe_collation",
        "unproven_code_domain",
    ],
    output_modes: &[
        "object_rows",
        "association_rows",
        "projection_rows",
        "evidence_rows",
        "json_rows",
        "arrow_record_batch",
        "datafusion_table_provider",
        "explain_json",
    ],
    fingerprint_fields: &["profile_id", "profile_version", "root", "grain"],
    explain_fields: &[
        "primary_profile",
        "profiles",
        "root",
        "grain",
        "operation",
        "temporal_mode",
    ],
};

pub const COVEQL_TABLE_PROFILE_CONTRACT: CoveQlProfileContract = CoveQlProfileContract {
    profile_id: CoveQlProfileId::Table,
    profile_version: COVEQL_TABLE_PROFILE_VERSION,
    implemented: true,
    supported_roots: &[CoveQlRootKind::Table],
    input_grains: &[CoveQlInputGrain::TableRow],
    root_authority: &[
        "deterministic_cove_map_projection",
        "validated_table_surface_contract",
    ],
    identity_model: &[
        "declared_projection_row_identity",
        "canonical_physical_row_identity_fallback",
    ],
    canonical_order: &[
        "declared_projection_ordering",
        "canonical_row_identity",
        "manifest_or_file_ordinal_then_source_row_ordinal",
    ],
    temporal_capabilities: &["latest", "as_of_when_projection_is_recomputable"],
    evidence_targets: &["row", "column", "projection", "source", "root_binding"],
    relationship_capabilities: &[
        "lookup_left_preserving",
        "lookup_exists_semijoin",
        "lookup_cardinality_contract",
        "materialized_inner_left_right_full_join",
        "materialized_semijoin_antijoin",
        "bag_and_distinct_set_operations",
        "materialized_window_rows",
    ],
    profile_methods: &[
        "lookup",
        "join",
        "semiJoin",
        "antiJoin",
        "union",
        "intersect",
        "except",
        "window",
        "with",
        "withRecursive",
    ],
    bridge_requirements: &[
        "validated_bridge_or_materialized_canonical_values_for_cross_profile_or_cross_file_codes",
    ],
    aggregate_rules: &[
        "visible_table_row_grain",
        "logical_value_grouping",
        "aggregate_disclosure_policy",
    ],
    null_missing_nan_rules: &[
        "sql_style_three_valued_predicates",
        "missing_base_column_rejects",
        "nan_ordering_requires_policy",
    ],
    security_barriers: &[
        "visibility",
        "redaction",
        "metadata_disclosure",
        "aggregate_disclosure",
    ],
    materialization_boundaries: &[
        "final_output",
        "unsupported_general_join_without_lookup_contract",
        "unsafe_code_domain",
        "unsafe_collation",
        "raw_table_surface_requires_table_contract",
    ],
    output_modes: &[
        "json_rows",
        "arrow_record_batch",
        "datafusion_table_provider",
        "explain_json",
    ],
    fingerprint_fields: &["profile_id", "profile_version", "root", "grain"],
    explain_fields: &["profiles", "root", "grain", "diagnostics"],
};

pub const COVEQL_GRAPH_PROFILE_CONTRACT: CoveQlProfileContract = CoveQlProfileContract {
    profile_id: CoveQlProfileId::Graph,
    profile_version: COVEQL_GRAPH_PROFILE_VERSION,
    implemented: true,
    supported_roots: &[
        CoveQlRootKind::Node,
        CoveQlRootKind::Edge,
        CoveQlRootKind::Path,
    ],
    input_grains: &[
        CoveQlInputGrain::NodeState,
        CoveQlInputGrain::EdgeState,
        CoveQlInputGrain::PathBinding,
    ],
    root_authority: &["cove_o_object_catalog", "cove_o_association_catalog"],
    identity_model: &[
        "node_object_goid",
        "edge_association_identity",
        "path_binding_identity",
    ],
    canonical_order: &["node_identity", "edge_identity", "path_identity"],
    temporal_capabilities: &["latest", "as_of"],
    evidence_targets: &["node", "edge", "path", "source"],
    relationship_capabilities: &[
        "in_edge",
        "out_edge",
        "either_edge",
        "one_hop_traverse",
        "chained_traverse",
        "multi_hop_path_binding",
        "finite_variable_length_traverse_with_graph_traversal_contract",
        "relationship_exists",
        "relationship_count_exists_distinct_count",
        "relationship_target_node_filter",
        "materialized_graph_algorithm_oracle",
    ],
    profile_methods: &[
        "traverse",
        "reachable",
        "shortestPath",
        "allPaths",
        "kShortestPaths",
        "connectedComponents",
        "degree",
        "pageRank",
        "hits",
        "centrality",
        "triangleCount",
        "clusteringCoefficient",
        "community",
        "spanningTree",
    ],
    bridge_requirements: &[
        "object_graph_identity_bridge",
        "association_graph_edge_identity_bridge",
    ],
    aggregate_rules: &[
        "visible_node_edge_path_grain",
        "aggregate_disclosure_policy",
    ],
    null_missing_nan_rules: &[
        "missing_property_unknown",
        "null_three_valued_logic",
        "nan_ordering_requires_policy",
    ],
    security_barriers: &[
        "visibility",
        "redaction",
        "metadata_disclosure",
        "hidden_endpoint_suppression",
    ],
    materialization_boundaries: &[
        "hidden_endpoint_policy",
        "unsafe_code_domain",
        "variable_length_traversal_requires_explicit_contract",
    ],
    output_modes: &["json_rows", "datafusion_table_provider", "explain_json"],
    fingerprint_fields: &["profile_id", "profile_version", "root", "grain"],
    explain_fields: &["profiles", "root", "grain", "diagnostics"],
};

pub const COVEQL_AI_PROFILE_CONTRACT: CoveQlProfileContract = CoveQlProfileContract {
    profile_id: CoveQlProfileId::Ai,
    profile_version: COVEQL_AI_PROFILE_VERSION,
    implemented: true,
    supported_roots: &[
        CoveQlRootKind::Object,
        CoveQlRootKind::Association,
        CoveQlRootKind::Projection,
        CoveQlRootKind::Evidence,
        CoveQlRootKind::Table,
        CoveQlRootKind::Node,
        CoveQlRootKind::Edge,
        CoveQlRootKind::Path,
    ],
    input_grains: &[
        CoveQlInputGrain::ObjectState,
        CoveQlInputGrain::AssociationState,
        CoveQlInputGrain::ProjectionRow,
        CoveQlInputGrain::EvidenceRow,
        CoveQlInputGrain::TableRow,
        CoveQlInputGrain::NodeState,
        CoveQlInputGrain::EdgeState,
        CoveQlInputGrain::PathBinding,
    ],
    root_authority: &[
        "validated_host_coveql_profile",
        "validated_cove_ai_sidecar_or_embedded_ai_sections",
    ],
    identity_model: &[
        "host_grain_identity",
        "ai_reference_table_ids",
        "digest_bound_source_binding",
    ],
    canonical_order: &[
        "host_profile_canonical_order",
        "ai_result_score_then_evidence_identity_when_authoritative",
    ],
    temporal_capabilities: &["inherits_host_temporal_context"],
    evidence_targets: &[
        "chunk",
        "token",
        "vector",
        "training_sample",
        "multimodal_sequence",
        "generator_provenance",
    ],
    relationship_capabilities: &[
        "sidecar_filecode_binding",
        "chunk_to_source_binding",
        "token_to_source_binding",
        "sample_to_source_binding",
    ],
    profile_methods: &[
        "similar",
        "embedding",
        "chunks",
        "tokens",
        "context",
        "hybrid",
        "trainingSamples",
        "split",
        "pack",
        "multimodal",
        "asPromptContext",
        "rerank",
        "generatorAudit",
    ],
    bridge_requirements: &[
        "validated_ai_reference_spaces",
        "digest_bound_source_binding",
        "privacy_and_redaction_policy_match",
    ],
    aggregate_rules: &[
        "ai_operations_preserve_host_visible_grain",
        "runtime_scores_are_advisory_unless_persisted_or_fixed_point_deterministic",
    ],
    null_missing_nan_rules: &[
        "missing_ai_sidecar_is_structured_rejection_for_selected_ai_operation",
        "withheld_policy_results_are_diagnostic_rows_not_silent_skips",
    ],
    security_barriers: &[
        "visibility",
        "redaction",
        "privacy_summary",
        "payload_integrity",
        "source_binding_freshness",
    ],
    materialization_boundaries: &[
        "direct_payload_bytes",
        "external_model_api",
        "runtime_float_composition",
        "prompt_context_export",
    ],
    output_modes: &["json_rows", "arrow_record_batch", "explain_json"],
    fingerprint_fields: &[
        "profile_id",
        "profile_version",
        "host_root",
        "ai_operation",
        "sidecar_binding",
    ],
    explain_fields: &[
        "ai_operation",
        "sidecar_required",
        "policy_scope",
        "authority",
        "redaction",
        "fallback",
    ],
};

pub const COVEQL_BUILTIN_PROFILE_CONTRACTS: &[CoveQlProfileContract] = &[
    COVEQL_OBJECT_PROFILE_CONTRACT,
    COVEQL_TABLE_PROFILE_CONTRACT,
    COVEQL_GRAPH_PROFILE_CONTRACT,
    COVEQL_AI_PROFILE_CONTRACT,
];

pub const COVEQL_COMMON_BRIDGE_CONTRACTS: &[CoveQlBridgeContract] = &[
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Object,
        target_profile: CoveQlProfileId::Table,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Table,
        target_profile: CoveQlProfileId::Object,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Object,
        target_profile: CoveQlProfileId::Graph,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Graph,
        target_profile: CoveQlProfileId::Object,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Table,
        target_profile: CoveQlProfileId::Graph,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Graph,
        target_profile: CoveQlProfileId::Table,
        requires_validated_bridge_or_materialized_fallback: true,
    },
    CoveQlBridgeContract {
        bridge_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        source_profile: CoveQlProfileId::Table,
        target_profile: CoveQlProfileId::Table,
        requires_validated_bridge_or_materialized_fallback: true,
    },
];

pub fn coveql_core_contract() -> &'static CoveQlCoreContract {
    &COVEQL_CORE_CONTRACT
}

pub fn builtin_coveql_profile_contracts() -> &'static [CoveQlProfileContract] {
    COVEQL_BUILTIN_PROFILE_CONTRACTS
}

pub fn builtin_coveql_bridge_contracts() -> &'static [CoveQlBridgeContract] {
    COVEQL_COMMON_BRIDGE_CONTRACTS
}

pub fn coveql_profile_contract(profile: CoveQlProfileId) -> &'static CoveQlProfileContract {
    match profile {
        CoveQlProfileId::Object => &COVEQL_OBJECT_PROFILE_CONTRACT,
        CoveQlProfileId::Table => &COVEQL_TABLE_PROFILE_CONTRACT,
        CoveQlProfileId::Graph => &COVEQL_GRAPH_PROFILE_CONTRACT,
        CoveQlProfileId::Ai => &COVEQL_AI_PROFILE_CONTRACT,
    }
}
