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

mod arrow_output;
mod association_opt;
mod ast;
mod builder;
#[cfg(feature = "datafusion")]
mod datafusion;
mod dependencies;
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

use std::{collections::BTreeSet, error::Error, fmt};

use cove_core::{
    artifact::covm::CovmFile,
    checksum, compression,
    constants::{DigestAlgorithm, PrimaryProfile, SectionKind, FEATURE_FILE_DICTIONARY},
    digest::compute_digest,
    feature_binding::OperationKindV2,
    feature_scope::{FeatureScopeTable, FeatureUseRequestV2},
    mount::{mount_cove_file, MountOptions, OutputRepresentation},
    profile::cove_map::{
        parse_embedded_section, EmbeddedMapSection, MapEvidenceIndex, MapProjectionCatalog,
    },
    profile::cove_o::ObjectTypeCatalog,
    reader::{
        feature_scope_table_for_feature_use, validate_bytes,
        validate_bytes_for_feature_use_with_optional_profile_validator, IgnoredOptionalSection,
        ValidatedCoveFile, ValidationOptions, ValidationReport, ValidationStage,
        ValidationStageReport, ValidationStageStatus,
    },
    CoveError,
};
use cove_profile_validation::EmbeddedOptionalProfileValidator;
use cove_runtime::RuntimeSession;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub use association_opt::{
    AssociationDirectionPlan, AssociationOptimizationDecision, AssociationOptimizationReport,
};
pub use ast::*;
pub use builder::*;
#[cfg(feature = "datafusion")]
pub use datafusion::{
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
pub use dependencies::*;
pub use evidence_opt::{
    EvidenceGrainIndexReport, EvidenceGrainKind, EvidenceOptimizationReport,
    EvidenceTargetIndexKind,
};
pub use execution::{
    execute_manifest_planned_query, execute_manifest_planned_query_retained, execute_planned_query,
    execute_planned_query_retained, execute_planned_query_stream,
    parse_resolve_plan_and_execute_query, BuildExecutionError, CoveQlExecutionResult,
    CoveQlResultStream, CoveQlRetainedInput, CoveQlRetainedManifestMember, EvidenceAuthority,
    ExecutedQuery, ExecutionAuthorityReport, ExecutionAuthoritySource, ExecutionDiagnostic,
    ExecutionOptions, ExecutionRowCounts, VisibilityOverlay,
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
}

impl CoveQlProfileId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Table => "table",
            Self::Graph => "graph",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "object" => Some(Self::Object),
            "table" => Some(Self::Table),
            "graph" => Some(Self::Graph),
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
    ],
    profile_methods: &["lookup"],
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
    ],
    profile_methods: &["traverse"],
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

pub const COVEQL_BUILTIN_PROFILE_CONTRACTS: &[CoveQlProfileContract] = &[
    COVEQL_OBJECT_PROFILE_CONTRACT,
    COVEQL_TABLE_PROFILE_CONTRACT,
    COVEQL_GRAPH_PROFILE_CONTRACT,
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
        source_profile: CoveQlProfileId::Object,
        target_profile: CoveQlProfileId::Graph,
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoveQlConformanceProfile {
    pub language_version: &'static str,
    pub core_version: &'static str,
    pub grammar_version: &'static str,
    pub resolved_ast_version: &'static str,
    pub logical_plan_version: &'static str,
    pub physical_plan_version: &'static str,
    pub explain_json_schema_version: &'static str,
    pub profile_contract_version: &'static str,
    pub bridge_contract_version: &'static str,
    pub object_profile_version: &'static str,
    pub table_profile_version: &'static str,
    pub graph_profile_version: &'static str,
    pub projection_dependency_contract_version: &'static str,
    pub predicate_normal_form_version: &'static str,
    pub coded_operator_contract_version: &'static str,
    pub predicate_representation_contract_version: &'static str,
    pub physical_operator_contract_version: &'static str,
    pub physical_sidecar_validation_version: &'static str,
    pub datafusion_coveql_report_version: &'static str,
    pub mandatory_history_modes: &'static [&'static str],
    pub mandatory_change_modes: &'static [&'static str],
    pub mandatory_functions: &'static [&'static str],
    pub projection_default_order: &'static [&'static str],
    pub evidence_shorthands: &'static [&'static str],
    pub required_fingerprint_fields: &'static [&'static str],
    pub required_coded_explain_fields: &'static [&'static str],
    pub required_coded_operator_contract_fields: &'static [&'static str],
    pub required_physical_sidecar_validation_fields: &'static [&'static str],
    pub required_physical_plan_sidecar_fields: &'static [&'static str],
    pub required_datafusion_scan_negotiation_fields: &'static [&'static str],
    pub required_diagnostic_fields: &'static [&'static str],
    pub required_diagnostic_codes: &'static [&'static str],
    pub conformance_tiers: &'static [CoveQlConformanceTier],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CoveQlConformanceTier {
    pub tier: u8,
    pub name: &'static str,
    pub authority: &'static str,
    pub required_surfaces: &'static [&'static str],
    pub required_invariant: &'static str,
}

pub const COVEQL_CONFORMANCE_PROFILE: CoveQlConformanceProfile = CoveQlConformanceProfile {
    language_version: COVEQL_LANGUAGE_VERSION,
    core_version: COVEQL_CORE_VERSION,
    grammar_version: COVEQL_GRAMMAR_VERSION,
    resolved_ast_version: RESOLVED_AST_VERSION,
    logical_plan_version: LOGICAL_PLAN_VERSION,
    physical_plan_version: PHYSICAL_PLAN_VERSION,
    explain_json_schema_version: EXPLAIN_JSON_SCHEMA_VERSION,
    profile_contract_version: COVEQL_PROFILE_CONTRACT_VERSION,
    bridge_contract_version: COVEQL_BRIDGE_CONTRACT_VERSION,
    object_profile_version: COVEQL_OBJECT_PROFILE_VERSION,
    table_profile_version: COVEQL_TABLE_PROFILE_VERSION,
    graph_profile_version: COVEQL_GRAPH_PROFILE_VERSION,
    projection_dependency_contract_version: PROJECTION_DEPENDENCY_CONTRACT_VERSION,
    predicate_normal_form_version: PREDICATE_NORMAL_FORM_VERSION,
    coded_operator_contract_version: CODED_OPERATOR_CONTRACT_VERSION,
    predicate_representation_contract_version: PREDICATE_REPRESENTATION_CONTRACT_VERSION,
    physical_operator_contract_version: PHYSICAL_OPERATOR_CONTRACT_VERSION,
    physical_sidecar_validation_version: PHYSICAL_SIDECAR_VALIDATION_VERSION,
    datafusion_coveql_report_version: DATAFUSION_COVEQL_REPORT_VERSION,
    mandatory_history_modes: &["records", "states", "records_and_states"],
    mandatory_change_modes: &[
        "records",
        "state_transitions",
        "property_diffs",
        "final_rows",
    ],
    mandatory_functions: &[
        "isNull",
        "isNotNull",
        "coalesce",
        "cast",
        "lower",
        "upper",
        "trim",
        "length",
        "startsWith",
        "identity",
    ],
    projection_default_order: &[
        "declared_projection_ordering",
        "canonical_row_identity",
        "manifest_or_file_ordinal_then_source_row_ordinal",
    ],
    evidence_shorthands: &[
        "evidence()",
        "evidence(self)",
        "evidence(path)",
        "evidence(root as binding)",
        "evidence(association(...))",
        "evidence(projection(...))",
    ],
    required_fingerprint_fields: &[
        "query_text",
        "parsed_ast",
        "resolved_query",
        "predicate_ast",
        "predicate_cnf",
        "projection_dependency",
        "logical_plan",
        "physical_plan",
    ],
    required_coded_explain_fields: &[
        "eligible",
        "coded_suitability",
        "stage",
        "fallback_reason",
        "fallback_reasons",
        "residual_verification",
        "residual_verification_required",
        "pushed_filters",
        "pushed_columns",
        "operator_contracts",
        "decode_boundaries",
        "bridge_decisions",
        "kernel_shape",
    ],
    required_coded_operator_contract_fields: &[
        "contract_version",
        "operator",
        "representation_class",
        "exact",
        "residual_required",
        "reason",
        "row_grain",
        "proof_obligation",
        "required_metadata",
        "residual_reason",
        "fallback_boundary",
    ],
    required_physical_sidecar_validation_fields: &[
        "report_version",
        "name",
        "status",
        "candidate_count",
        "safe_details",
        "fallback_reason",
        "redacted",
    ],
    required_physical_plan_sidecar_fields: &[
        "proof_validation_report",
        "index_capability_report",
        "layout_range_plan",
        "runtime_compatibility",
        "cache_compatibility",
        "codec_compatibility",
        "zero_copy_eligibility",
        "sidecar_validations",
    ],
    required_datafusion_scan_negotiation_fields: &[
        "report_version",
        "provider_kind",
        "root_kind",
        "dataset_file_count",
        "received_projection_columns",
        "projection_pushdown_supported",
        "projection_pushed_to_coveql",
        "pushed_projection_columns",
        "received_filters",
        "filter_outcomes",
        "pushed_filters",
        "trusted_filters",
        "residual_filters",
        "rejected_filters",
        "lowered_coveql_predicates",
        "proof_states",
        "filters_trusted_exact",
        "received_limit",
        "limit_pushed_to_coveql",
        "pushed_limit",
        "residual_filter_authority",
        "scan_execution_policy",
        "unhandled_residuals",
    ],
    required_diagnostic_fields: &[
        "code",
        "severity",
        "message",
        "span",
        "phase",
        "safe_details",
        "redacted",
    ],
    required_diagnostic_codes: &[
        "E_PARSE",
        "E_UNSUPPORTED_CONSTRUCT",
        "E_DUPLICATE_METHOD",
        "E_METHOD_CONFLICT",
        "E_AMBIGUOUS_PATH",
        "E_UNKNOWN_OBJECT_TYPE",
        "E_UNKNOWN_PROPERTY",
        "E_UNKNOWN_PROJECTION",
        "E_UNKNOWN_EVIDENCE_GRAIN",
        "E_AMBIGUOUS_BRANCH",
        "E_UNSUPPORTED_TEMPORAL_ROLE",
        "E_UNSUPPORTED_HISTORY_MODE",
        "E_UNSUPPORTED_CHANGE_MODE",
        "E_UNSAFE_CODE_DOMAIN",
        "E_STALE_SIDECAR",
        "E_CORRUPT_PROOF",
        "E_RESOURCE_BUDGET_EXCEEDED",
        "E_SECURITY_DISCLOSURE_FORBIDDEN",
        "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
        "E_INDEX_ONLY_FORBIDDEN",
        "E_ZERO_COPY_FORBIDDEN",
        "E_DATAFUSION_PUSH_FILTER_UNSAFE",
        "E_UNSUPPORTED_PROFILE",
        "E_UNSUPPORTED_PROFILE_METHOD",
        "E_UNKNOWN_TABLE_SURFACE",
        "E_UNKNOWN_GRAPH_LABEL",
        "E_UNKNOWN_BRIDGE",
        "E_AMBIGUOUS_PROFILE",
        "E_UNKNOWN_BINDING",
        "E_BINDING_OUT_OF_SCOPE",
    ],
    conformance_tiers: &[
        CoveQlConformanceTier {
            tier: 0,
            name: "semantic_correctness",
            authority: "materialized_coveql",
            required_surfaces: &[
                "object",
                "association",
                "evidence",
                "projection",
                "temporal",
                "json",
                "arrow",
                "datafusion",
                "explain",
            ],
            required_invariant:
                "valid CoveQL semantics match materialized COVE-O/COVE-MAP readback without relying on optional accelerators",
        },
        CoveQlConformanceTier {
            tier: 1,
            name: "fallback_invariance",
            authority: "materialized_coveql_with_explicit_fallbacks",
            required_surfaces: &[
                "absent_optional_metadata",
                "valid_optional_metadata",
                "stale_optional_metadata",
                "corrupt_optional_metadata",
                "unsupported_optional_metadata",
                "security_blocked_optional_metadata",
            ],
            required_invariant:
                "optional metadata may change plans and diagnostics but not visible rows unless policy requires a structured rejection",
        },
        CoveQlConformanceTier {
            tier: 2,
            name: "acceleration_proof",
            authority: "optimized_output_compared_or_proven_equivalent",
            required_surfaces: &[
                "coded_kernel",
                "index_only",
                "coverage_pruning",
                "execution_code_remap",
                "zero_copy",
                "datafusion_pushdown",
                "manifest_bridges",
            ],
            required_invariant:
                "every optimized, coded, indexed, zero-copy, or pushed-down path proves exact equivalence or compares against materialized output",
        },
    ],
};

pub fn conformance_profile() -> &'static CoveQlConformanceProfile {
    &COVEQL_CONFORMANCE_PROFILE
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplainJsonSchema {
    pub version: &'static str,
    pub modes: &'static [&'static str],
    pub required_top_level_fields: &'static [&'static str],
    pub required_operation_context_fields: &'static [&'static str],
    pub required_coded_execution_fields: &'static [&'static str],
    pub required_coded_operator_contract_fields: &'static [&'static str],
    pub required_physical_sidecar_validation_fields: &'static [&'static str],
    pub required_physical_plan_sidecar_fields: &'static [&'static str],
}

pub const EXPLAIN_JSON_SCHEMA: ExplainJsonSchema = ExplainJsonSchema {
    version: EXPLAIN_JSON_SCHEMA_VERSION,
    modes: &["public", "developer", "proof", "coded", "forensic"],
    required_top_level_fields: &[
        "schema_version",
        "mode",
        "coveql_version",
        "core_version",
        "primary_profile",
        "profiles",
        "profile_contracts",
        "root",
        "grain",
        "operation",
        "temporal_mode",
        "canonical_order",
        "visibility_applied",
        "redaction_applied",
        "fingerprints",
        "operation_context",
        "logical_plan",
        "physical_plan",
        "resolved_dependencies",
        "predicate_forms",
        "trusted_metadata",
        "ignored_metadata",
        "fallbacks",
        "rejections",
        "decode_boundaries",
        "residual_predicates",
        "visibility",
        "redactions",
        "warnings",
        "diagnostics",
        "execution",
    ],
    required_operation_context_fields: &[
        "language_version",
        "core_version",
        "grammar_version",
        "resolved_ast_version",
        "logical_plan_version",
        "physical_plan_version",
        "projection_dependency_contract_version",
        "predicate_normal_form_version",
        "explain_json_schema_version",
        "profile_contract_version",
        "bridge_contract_version",
        "operation",
        "file_len",
        "file_id",
        "footer_crc32c",
        "primary_profile",
        "dataset_id",
        "snapshot_id",
        "selected_snapshot_ref",
        "schema_fingerprint",
        "semantic_map_fingerprint",
        "file_digest",
        "authority",
        "dataset",
        "temporal_mode",
        "temporal",
        "branch",
        "tombstone",
        "visibility_applied",
        "redaction_applied",
        "security",
    ],
    required_coded_execution_fields: COVEQL_CONFORMANCE_PROFILE.required_coded_explain_fields,
    required_coded_operator_contract_fields: COVEQL_CONFORMANCE_PROFILE
        .required_coded_operator_contract_fields,
    required_physical_sidecar_validation_fields: COVEQL_CONFORMANCE_PROFILE
        .required_physical_sidecar_validation_fields,
    required_physical_plan_sidecar_fields: COVEQL_CONFORMANCE_PROFILE
        .required_physical_plan_sidecar_fields,
};

pub fn explain_json_schema() -> &'static ExplainJsonSchema {
    &EXPLAIN_JSON_SCHEMA
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveQlOperationRequest {
    pub selected_operation: CoveQlSelectedOperation,
    pub output_mode: CoveQlOutputMode,
    pub temporal: TemporalContext,
    pub branch: BranchContext,
    pub tombstone: TombstoneContext,
    pub security: SecurityContext,
    pub fallback_policy: FallbackPolicy,
    pub resource_budget: ResourceBudgetPolicy,
    pub resource_use: ResourceUseEstimate,
    pub query_text_fingerprint: Option<String>,
    pub parsed_ast_fingerprint: Option<String>,
    pub evidence_metadata_requested: bool,
    pub execution_code_mapping_requested: bool,
    pub cache_hook: Option<CacheHookRef>,
}

impl Default for CoveQlOperationRequest {
    fn default() -> Self {
        Self {
            selected_operation: CoveQlSelectedOperation::Object,
            output_mode: CoveQlOutputMode::ObjectRows,
            temporal: TemporalContext::default(),
            branch: BranchContext::default(),
            tombstone: TombstoneContext::default(),
            security: SecurityContext::default(),
            fallback_policy: FallbackPolicy::default(),
            resource_budget: ResourceBudgetPolicy::default(),
            resource_use: ResourceUseEstimate::default(),
            query_text_fingerprint: None,
            parsed_ast_fingerprint: None,
            evidence_metadata_requested: false,
            execution_code_mapping_requested: false,
            cache_hook: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlSelectedOperation {
    Object,
    Association,
    GraphNode,
    GraphEdge,
    Table,
    Projection,
    Evidence,
    IndexOnlyAnswer,
    ArrowExport {
        zero_copy_requested: bool,
    },
    Explain {
        target: CoveQlExplainTarget,
        mode: ExplainMode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlExplainTarget {
    Object,
    Association,
    GraphNode,
    GraphEdge,
    Table,
    Projection,
    Evidence,
    IndexOnlyAnswer,
    ArrowExport { zero_copy_requested: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoveQlOutputMode {
    ObjectRows,
    AssociationRows,
    EvidenceRows,
    ProjectionRows,
    ArrowRecordBatch { zero_copy_requested: bool },
    JsonRows,
    DataFusionTableProvider,
    ExplainJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplainMode {
    Public,
    Developer,
    Proof,
    Coded,
    Forensic,
}

impl Default for ExplainMode {
    fn default() -> Self {
        Self::Public
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalMode {
    Latest,
    AsOfCsn(u64),
    AsOfTimestampMicros(i64),
    HistoryRecords,
    HistoryStates,
    HistoryRecordsAndStates,
    ChangesRecords,
    ChangesStateTransitions,
    ChangesPropertyDiffs,
    #[serde(rename = "changes_final_rows", alias = "changes_final_objects")]
    ChangesFinalObjects,
}

impl TemporalMode {
    pub fn is_point_in_time(&self) -> bool {
        matches!(
            self,
            Self::Latest | Self::AsOfCsn(_) | Self::AsOfTimestampMicros(_)
        )
    }

    pub fn is_history_or_changes(&self) -> bool {
        matches!(
            self,
            Self::HistoryRecords
                | Self::HistoryStates
                | Self::HistoryRecordsAndStates
                | Self::ChangesRecords
                | Self::ChangesStateTransitions
                | Self::ChangesPropertyDiffs
                | Self::ChangesFinalObjects
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalContext {
    pub mode: TemporalMode,
    pub role: TemporalRole,
    pub role_binding: Option<String>,
}

impl Default for TemporalContext {
    fn default() -> Self {
        Self {
            mode: TemporalMode::Latest,
            role: TemporalRole::CommitTime,
            role_binding: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRole {
    CommitTime,
    ValidTime,
    ObservedTime,
    SourceEventTime,
    AssociationValidTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchContext {
    pub selector: BranchSelector,
}

impl Default for BranchContext {
    fn default() -> Self {
        Self {
            selector: BranchSelector::Default,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchSelector {
    Default,
    BranchKey(u64),
    RejectAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneContext {
    pub include_tombstones: bool,
}

impl Default for TombstoneContext {
    fn default() -> Self {
        Self {
            include_tombstones: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityContext {
    pub principal_or_session: Option<String>,
    pub visibility_policy: VisibilityPolicy,
    pub redaction_policy: RedactionPolicy,
    pub explain_policy: ExplainDisclosurePolicy,
    pub aggregate_disclosure_policy: AggregateDisclosurePolicy,
    pub aggregate_disclosure_threshold: Option<u64>,
    pub metadata_disclosure_policy: MetadataDisclosurePolicy,
    pub zero_copy_permission: bool,
    pub index_only_answer_permission: bool,
}

impl Default for SecurityContext {
    fn default() -> Self {
        Self {
            principal_or_session: None,
            visibility_policy: VisibilityPolicy::AllRows,
            redaction_policy: RedactionPolicy::ProtectedValuesRedacted,
            explain_policy: ExplainDisclosurePolicy::PublicOnly,
            aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowMaterializedOnly,
            aggregate_disclosure_threshold: None,
            metadata_disclosure_policy: MetadataDisclosurePolicy::DenyProtected,
            zero_copy_permission: false,
            index_only_answer_permission: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityPolicy {
    AllRows,
    ExternalOverlay(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionPolicy {
    ProtectedValuesRedacted,
    RefuseProtectedValues,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplainDisclosurePolicy {
    PublicOnly,
    Developer,
    Proof,
    Forensic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateDisclosurePolicy {
    AllowExact,
    AllowMaterializedOnly,
    AllowThresholded,
    AllowRedacted,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataDisclosurePolicy {
    AllowProtected,
    DenyProtected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    AllowMaterializedFallback,
    RejectOnFallback,
}

impl Default for FallbackPolicy {
    fn default() -> Self {
        Self::AllowMaterializedFallback
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudgetPolicy {
    pub maximum_query_bytes: usize,
    pub maximum_ast_depth: usize,
    pub maximum_method_count: usize,
    pub maximum_in_list_size: usize,
    pub maximum_disjunction_count: usize,
    pub maximum_output_columns: usize,
    pub maximum_groups: usize,
    pub maximum_rows_without_explicit_take: usize,
    pub maximum_decode_bytes: usize,
    pub maximum_range_requests: usize,
    pub maximum_graph_traversal_depth: u32,
    pub maximum_graph_traversal_fanout: usize,
    pub maximum_graph_traversal_paths: usize,
    pub maximum_graph_traversal_frontier: usize,
    pub maximum_planning_time_ms: u64,
    pub maximum_execution_time_ms: u64,
}

impl Default for ResourceBudgetPolicy {
    fn default() -> Self {
        Self {
            maximum_query_bytes: 64 * 1024,
            maximum_ast_depth: 64,
            maximum_method_count: 128,
            maximum_in_list_size: 10_000,
            maximum_disjunction_count: 1_024,
            maximum_output_columns: 1_024,
            maximum_groups: 1_000_000,
            maximum_rows_without_explicit_take: 10_000,
            maximum_decode_bytes: 512 * 1024 * 1024,
            maximum_range_requests: 10_000,
            maximum_graph_traversal_depth: 8,
            maximum_graph_traversal_fanout: 10_000,
            maximum_graph_traversal_paths: 100_000,
            maximum_graph_traversal_frontier: 100_000,
            maximum_planning_time_ms: 5_000,
            maximum_execution_time_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUseEstimate {
    pub query_bytes: Option<usize>,
    pub ast_depth: Option<usize>,
    pub method_count: Option<usize>,
    pub in_list_size: Option<usize>,
    pub disjunction_count: Option<usize>,
    pub output_columns: Option<usize>,
    pub groups: Option<usize>,
    pub rows_without_explicit_take: Option<usize>,
    pub decode_bytes: Option<usize>,
    pub range_requests: Option<usize>,
    pub graph_traversal_depth: Option<u32>,
    pub graph_traversal_fanout: Option<usize>,
    pub graph_traversal_paths: Option<usize>,
    pub graph_traversal_frontier: Option<usize>,
    pub planning_time_ms: Option<u64>,
    pub execution_time_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheHookRef {
    pub hook_id: String,
}

#[derive(Debug, Clone)]
pub struct OperationContext {
    pub language_version: &'static str,
    pub core_version: &'static str,
    pub grammar_version: &'static str,
    pub resolved_ast_version: &'static str,
    pub logical_plan_version: &'static str,
    pub physical_plan_version: &'static str,
    pub explain_json_schema_version: &'static str,
    pub profile_contract_version: &'static str,
    pub bridge_contract_version: &'static str,
    pub projection_dependency_contract_version: &'static str,
    pub predicate_normal_form_version: &'static str,
    pub request: CoveQlOperationRequest,
    pub selected_feature_uses: Vec<FeatureUseRequestV2>,
    pub file: ValidatedFileIdentity,
    pub dataset: DatasetScopeContext,
    pub feature_scope_table: FeatureScopeTable,
    pub validation_reports: Vec<ValidationReportSummary>,
    pub snapshot: SnapshotContext,
    pub semantic_map: SemanticMapContext,
    pub temporal: TemporalContext,
    pub branch: BranchContext,
    pub tombstone: TombstoneContext,
    pub security: SecurityContext,
    pub resource_budget: ResourceBudgetPolicy,
    pub optional_metadata: Vec<OptionalMetadataOutcome>,
    pub fallbacks: Vec<FallbackReport>,
    pub rejections: Vec<RejectionReport>,
    pub diagnostics: Vec<CoveQlDiagnostic>,
}

impl OperationContext {
    pub fn explain_json(&self) -> Value {
        crate::explain::operation_context_explain_json(self)
    }
}

#[cfg(feature = "datafusion")]
#[derive(Debug, Clone)]
pub struct DatasetOperationContext {
    pub language_version: &'static str,
    pub grammar_version: &'static str,
    pub resolved_ast_version: &'static str,
    pub logical_plan_version: &'static str,
    pub physical_plan_version: &'static str,
    pub explain_json_schema_version: &'static str,
    pub projection_dependency_contract_version: &'static str,
    pub predicate_normal_form_version: &'static str,
    pub request: CoveQlOperationRequest,
    pub dataset: DatasetScopeContext,
    pub temporal: TemporalContext,
    pub branch: BranchContext,
    pub tombstone: TombstoneContext,
    pub security: SecurityContext,
    pub resource_budget: ResourceBudgetPolicy,
    pub fallbacks: Vec<FallbackReport>,
    pub rejections: Vec<RejectionReport>,
    pub diagnostics: Vec<CoveQlDiagnostic>,
}

#[cfg(feature = "datafusion")]
impl DatasetOperationContext {
    pub fn explain_json(&self) -> Value {
        json!({
            "schema_version": self.explain_json_schema_version,
            "language_version": self.language_version,
            "grammar_version": self.grammar_version,
            "resolved_ast_version": self.resolved_ast_version,
            "logical_plan_version": self.logical_plan_version,
            "physical_plan_version": self.physical_plan_version,
            "projection_dependency_contract_version": self.projection_dependency_contract_version,
            "predicate_normal_form_version": self.predicate_normal_form_version,
            "operation_context": {
                "operation": selected_operation_name(&self.request.selected_operation),
                "dataset": self.dataset,
                "temporal": self.temporal,
                "branch": self.branch,
                "tombstone": self.tombstone,
                "security": self.security,
            },
            "fallbacks": self.fallbacks.iter().map(FallbackReport::to_json).collect::<Vec<_>>(),
            "rejections": self.rejections.iter().map(RejectionReport::to_json).collect::<Vec<_>>(),
            "diagnostics": self.diagnostics.iter().map(CoveQlDiagnostic::to_json).collect::<Vec<_>>(),
        })
    }
}

#[cfg(feature = "datafusion")]
pub fn build_dataset_operation_context(
    dataset: &cove_datafusion::dataset_state::DatasetState,
    request: CoveQlOperationRequest,
) -> Result<DatasetOperationContext, BuildOperationContextError> {
    check_resource_budget(&request)?;

    let mut diagnostics = Vec::new();
    let mut fallbacks = Vec::new();
    enforce_security_gates(&request, &mut diagnostics, &mut fallbacks)?;
    let dataset = dataset_scope_context_from_state(dataset);

    Ok(DatasetOperationContext {
        language_version: COVEQL_LANGUAGE_VERSION,
        grammar_version: COVEQL_GRAMMAR_VERSION,
        resolved_ast_version: RESOLVED_AST_VERSION,
        logical_plan_version: LOGICAL_PLAN_VERSION,
        physical_plan_version: PHYSICAL_PLAN_VERSION,
        explain_json_schema_version: EXPLAIN_JSON_SCHEMA_VERSION,
        projection_dependency_contract_version: PROJECTION_DEPENDENCY_CONTRACT_VERSION,
        predicate_normal_form_version: PREDICATE_NORMAL_FORM_VERSION,
        temporal: request.temporal.clone(),
        branch: request.branch.clone(),
        tombstone: request.tombstone.clone(),
        security: request.security.clone(),
        resource_budget: request.resource_budget.clone(),
        dataset,
        fallbacks,
        rejections: Vec::new(),
        diagnostics,
        request,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedFileIdentity {
    pub file_id: [u8; 16],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub primary_profile: u8,
    pub version_major: u16,
    pub version_minor: u16,
    pub section_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetScopeContext {
    pub scope_version: u16,
    pub dataset_id: Option<String>,
    pub manifest_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub file_membership_fingerprint: String,
    pub object_schema_fingerprint: Option<String>,
    pub semantic_map_fingerprint: Option<String>,
    pub projection_catalog_fingerprint: Option<String>,
    pub files: Vec<DatasetFileIdentity>,
    pub cross_file_ordering: CrossFileOrderingPolicy,
    pub object_identity: CrossFileObjectIdentityPolicy,
    pub association_identity: CrossFileAssociationIdentityPolicy,
    pub dictionary_epochs: Vec<DictionaryEpochContext>,
    pub security_scope: DatasetSecurityScopeContext,
    pub code_domain_bridges: Vec<CodeDomainBridgeContext>,
    pub execution_code_domains: Vec<ExecutionCodeDomainContext>,
}

impl DatasetScopeContext {
    fn single_file(
        file: &ValidatedFileIdentity,
        snapshot: &SnapshotContext,
        security: &SecurityContext,
    ) -> Self {
        Self::single_file_with_source(file, snapshot, security, "in_memory".into())
    }

    pub(crate) fn single_file_with_source(
        file: &ValidatedFileIdentity,
        snapshot: &SnapshotContext,
        security: &SecurityContext,
        source: String,
    ) -> Self {
        let member = DatasetFileIdentity {
            ordinal: 0,
            source,
            file_id: file.file_id,
            file_len: file.file_len,
            footer_crc32c: file.footer_crc32c,
            primary_profile: file.primary_profile,
        };
        let file_membership_fingerprint = dataset_membership_fingerprint(&[member.clone()]);
        Self {
            scope_version: 1,
            dataset_id: snapshot.dataset_id.clone(),
            manifest_id: None,
            snapshot_id: snapshot.snapshot_id.clone(),
            file_membership_fingerprint,
            object_schema_fingerprint: snapshot.schema_fingerprint.clone(),
            semantic_map_fingerprint: snapshot.semantic_map_fingerprint.clone(),
            projection_catalog_fingerprint: None,
            files: vec![member],
            cross_file_ordering: CrossFileOrderingPolicy::SingleFile,
            object_identity: CrossFileObjectIdentityPolicy::SingleFileGoid,
            association_identity: CrossFileAssociationIdentityPolicy::SingleFileEndpoints,
            dictionary_epochs: Vec::new(),
            security_scope: DatasetSecurityScopeContext::from_security(security, None),
            code_domain_bridges: Vec::new(),
            execution_code_domains: Vec::new(),
        }
    }
}

impl Default for DatasetScopeContext {
    fn default() -> Self {
        Self {
            scope_version: 1,
            dataset_id: None,
            manifest_id: None,
            snapshot_id: None,
            file_membership_fingerprint:
                "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            object_schema_fingerprint: None,
            semantic_map_fingerprint: None,
            projection_catalog_fingerprint: None,
            files: Vec::new(),
            cross_file_ordering: CrossFileOrderingPolicy::SingleFile,
            object_identity: CrossFileObjectIdentityPolicy::SingleFileGoid,
            association_identity: CrossFileAssociationIdentityPolicy::SingleFileEndpoints,
            dictionary_epochs: Vec::new(),
            security_scope: DatasetSecurityScopeContext::default(),
            code_domain_bridges: Vec::new(),
            execution_code_domains: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetFileIdentity {
    pub ordinal: usize,
    pub source: String,
    pub file_id: [u8; 16],
    pub file_len: u64,
    pub footer_crc32c: u32,
    pub primary_profile: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossFileOrderingPolicy {
    SingleFile,
    CanonicalDatasetOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossFileObjectIdentityPolicy {
    SingleFileGoid,
    DatasetFileIdAndGoid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossFileAssociationIdentityPolicy {
    SingleFileEndpoints,
    DatasetFileQualifiedEndpoints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDomainBridgeContext {
    pub domain_id: String,
    pub bridge_kind: String,
    pub epoch: Option<u64>,
    pub security_scope_id: Option<String>,
    pub exact: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionCodeDomainContext {
    pub engine_profile_id: String,
    pub code_space_id: String,
    pub comparison_scope: String,
    pub lifetime: String,
    pub epoch: Option<u64>,
    pub null_code_policy: String,
    pub semantic_domain_id: Option<String>,
    pub security_scope_id: Option<String>,
    pub exact: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEpochContext {
    pub source: String,
    pub domain_id: String,
    pub epoch: Option<u64>,
    pub exact: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetSecurityScopeContext {
    pub tenant_id: Option<String>,
    pub principal_or_session: Option<String>,
    pub visibility_policy: VisibilityPolicy,
    pub redaction_policy: RedactionPolicy,
    pub metadata_disclosure_policy: MetadataDisclosurePolicy,
}

impl DatasetSecurityScopeContext {
    fn from_security(security: &SecurityContext, tenant_id: Option<String>) -> Self {
        Self {
            tenant_id,
            principal_or_session: security.principal_or_session.clone(),
            visibility_policy: security.visibility_policy.clone(),
            redaction_policy: security.redaction_policy.clone(),
            metadata_disclosure_policy: security.metadata_disclosure_policy,
        }
    }
}

impl Default for DatasetSecurityScopeContext {
    fn default() -> Self {
        Self::from_security(&SecurityContext::default(), None)
    }
}

#[derive(Debug, Clone)]
pub struct ManifestDatasetMember<'a> {
    pub source: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Default)]
pub struct ManifestDatasetScopeOptions {
    pub snapshot_id: Option<String>,
    pub tenant_id: Option<String>,
    pub security: SecurityContext,
    pub code_domain_bridge_proofs: Vec<ManifestCodeDomainBridgeProof>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestCodeDomainBridgeProof {
    pub domain_id: String,
    pub bridge_kind: String,
    pub exact: bool,
    pub epoch: Option<u64>,
    pub reason: String,
}

pub fn build_manifest_dataset_scope_context(
    manifest_bytes: &[u8],
    members: &[ManifestDatasetMember<'_>],
    options: ManifestDatasetScopeOptions,
) -> Result<DatasetScopeContext, BuildOperationContextError> {
    validate_manifest_security_scope(&options)?;
    validate_manifest_code_domain_bridge_proofs(&options)?;
    let manifest = CovmFile::parse(manifest_bytes)
        .map_err(|error| validation_error(error, "dataset_manifest"))?;
    if manifest.files.len() != members.len() {
        return Err(manifest_scope_error(format!(
            "COVM member count mismatch: manifest lists {} files, caller provided {}",
            manifest.files.len(),
            members.len()
        )));
    }

    let mut used_members = vec![false; members.len()];
    let mut seen_file_ids = BTreeSet::new();
    let mut files = Vec::with_capacity(manifest.files.len());
    let mut dictionary_epochs = Vec::new();
    let mut execution_code_domains = Vec::new();
    let mut object_schema_fingerprint = None::<(String, String)>;
    let mut semantic_map_fingerprint = None::<(String, String)>;
    let mut semantic_map_member_count = 0usize;
    let mut projection_catalog_fingerprint = None::<(String, String)>;
    let mut projection_catalog_member_count = 0usize;

    for (ordinal, entry) in manifest.files.iter().enumerate() {
        if !seen_file_ids.insert(entry.file_id) {
            return Err(manifest_scope_error(format!(
                "COVM manifest repeats file_id {}",
                hex_lower(&entry.file_id)
            )));
        }
        let Some((member_index, member)) = members
            .iter()
            .enumerate()
            .find(|(_, member)| member.source == entry.uri)
        else {
            return Err(manifest_scope_error(format!(
                "COVM member {} is missing from supplied member files",
                entry.uri
            )));
        };
        if used_members[member_index] {
            return Err(manifest_scope_error(format!(
                "COVM member {} was supplied more than once",
                entry.uri
            )));
        }
        used_members[member_index] = true;

        let validated = validate_bytes(member.bytes)
            .map_err(|error| validation_error(error, "dataset_member"))?;
        let file = ValidatedFileIdentity::from(&validated);
        if let Some(schema_fingerprint) =
            object_catalog_schema_fingerprint(member.bytes, &validated)?
        {
            if let Some((expected_source, expected_fingerprint)) = &object_schema_fingerprint {
                if expected_fingerprint != &schema_fingerprint {
                    return Err(manifest_scope_error(format!(
                        "COVM member {} has object schema fingerprint {}, which is incompatible with {} ({})",
                        member.source, schema_fingerprint, expected_source, expected_fingerprint
                    )));
                }
            } else {
                object_schema_fingerprint = Some((member.source.to_string(), schema_fingerprint));
            }
        }
        if let Some(map_fingerprint) = semantic_map_identity_fingerprint(member.bytes, &validated)?
        {
            semantic_map_member_count += 1;
            if let Some((expected_source, expected_fingerprint)) = &semantic_map_fingerprint {
                if expected_fingerprint != &map_fingerprint {
                    return Err(manifest_scope_error(format!(
                        "COVM member {} has semantic-map identity fingerprint {}, which is incompatible with {} ({})",
                        member.source, map_fingerprint, expected_source, expected_fingerprint
                    )));
                }
            } else {
                semantic_map_fingerprint = Some((member.source.to_string(), map_fingerprint));
            }
        }
        if let Some(catalog_fingerprint) =
            projection_catalog_schema_fingerprint(member.bytes, &validated)?
        {
            projection_catalog_member_count += 1;
            if let Some((expected_source, expected_fingerprint)) = &projection_catalog_fingerprint {
                if expected_fingerprint != &catalog_fingerprint {
                    return Err(manifest_scope_error(format!(
                        "COVM member {} has projection catalog fingerprint {}, which is incompatible with {} ({})",
                        member.source, catalog_fingerprint, expected_source, expected_fingerprint
                    )));
                }
            } else {
                projection_catalog_fingerprint =
                    Some((member.source.to_string(), catalog_fingerprint));
            }
        }
        let digest_algorithm =
            DigestAlgorithm::from_u16(entry.digest_algorithm).ok_or_else(|| {
                manifest_scope_error(format!(
                    "COVM member {} declares unsupported digest algorithm {}",
                    entry.uri, entry.digest_algorithm
                ))
            })?;
        let digest = compute_digest(digest_algorithm, member.bytes)
            .map_err(|error| validation_error(error, "dataset_member_digest"))?;
        entry
            .verify_against(&file.file_id, file.file_len, file.footer_crc32c, &digest)
            .map_err(|error| validation_error(error, "dataset_manifest"))?;

        if file_has_dictionary(&validated) {
            dictionary_epochs.push(DictionaryEpochContext {
                source: member.source.into(),
                domain_id: format!("file:{}:dictionary", hex_lower(&file.file_id)),
                epoch: None,
                exact: false,
                reason: "file-local dictionary has no manifest-level canonical epoch or remap proof; raw codes remain file scoped".into(),
            });
        }
        execution_code_domains.extend(manifest_member_execution_code_domains(
            member.bytes,
            member.source,
            &options.security,
            options.tenant_id.as_deref(),
        ));

        files.push(DatasetFileIdentity {
            ordinal,
            source: member.source.into(),
            file_id: file.file_id,
            file_len: file.file_len,
            footer_crc32c: file.footer_crc32c,
            primary_profile: file.primary_profile,
        });
    }

    if used_members.iter().any(|used| !*used) {
        return Err(manifest_scope_error(
            "caller supplied member files not referenced by the COVM manifest".into(),
        ));
    }
    if semantic_map_member_count > 0 && semantic_map_member_count != members.len() {
        return Err(manifest_scope_error(format!(
            "COVM semantic-map compatibility requires every member to declare the same COVE-MAP identity; {} of {} members declared COVE-MAP metadata",
            semantic_map_member_count,
            members.len()
        )));
    }
    if projection_catalog_member_count > 0 && projection_catalog_member_count != members.len() {
        return Err(manifest_scope_error(format!(
            "COVM projection catalog compatibility requires every member to declare the same projection catalog; {} of {} members declared one",
            projection_catalog_member_count,
            members.len()
        )));
    }
    if manifest_bridge_security_block_reason(&options.security, options.tenant_id.as_deref())
        .is_none()
    {
        validate_manifest_code_domain_bridge_proofs_against_members(
            &options.code_domain_bridge_proofs,
            &execution_code_domains,
            members.len(),
        )?;
    }

    let file_membership_fingerprint = dataset_membership_fingerprint(&files);
    let manifest_id = format!("covm:{}", hex_lower(&manifest.header.dataset_id));
    let snapshot_id = options.snapshot_id.or_else(|| {
        manifest_snapshot_id(manifest_bytes, &file_membership_fingerprint)
            .map(|snapshot| format!("{manifest_id}:{snapshot}"))
    });
    let object_schema_fingerprint = object_schema_fingerprint.map(|(_, fingerprint)| fingerprint);
    let semantic_map_fingerprint = semantic_map_fingerprint.map(|(_, fingerprint)| fingerprint);
    let projection_catalog_fingerprint =
        projection_catalog_fingerprint.map(|(_, fingerprint)| fingerprint);
    let multi_file = files.len() > 1;
    Ok(DatasetScopeContext {
        scope_version: 1,
        dataset_id: Some(manifest_id.clone()),
        manifest_id: Some(manifest_id.clone()),
        snapshot_id,
        file_membership_fingerprint,
        object_schema_fingerprint,
        semantic_map_fingerprint,
        projection_catalog_fingerprint,
        files,
        cross_file_ordering: if multi_file {
            CrossFileOrderingPolicy::CanonicalDatasetOrder
        } else {
            CrossFileOrderingPolicy::SingleFile
        },
        object_identity: if multi_file {
            CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid
        } else {
            CrossFileObjectIdentityPolicy::SingleFileGoid
        },
        association_identity: if multi_file {
            CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints
        } else {
            CrossFileAssociationIdentityPolicy::SingleFileEndpoints
        },
        dictionary_epochs,
        security_scope: DatasetSecurityScopeContext::from_security(
            &options.security,
            options.tenant_id.clone(),
        ),
        code_domain_bridges: manifest_code_domain_bridges(
            &manifest_id,
            multi_file,
            &options.security,
            options.tenant_id.as_deref(),
            &options.code_domain_bridge_proofs,
        ),
        execution_code_domains,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotContext {
    pub dataset_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub selected_snapshot_ref: Option<u32>,
    pub schema_fingerprint: Option<String>,
    pub semantic_map_fingerprint: Option<String>,
    pub file_digest: Option<String>,
    pub authority: Option<String>,
}

impl Default for SnapshotContext {
    fn default() -> Self {
        Self {
            dataset_id: None,
            snapshot_id: None,
            selected_snapshot_ref: None,
            schema_fingerprint: None,
            semantic_map_fingerprint: None,
            file_digest: None,
            authority: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticMapContext {
    pub projection_version: Option<String>,
    pub cache_state: CacheState,
}

impl Default for SemanticMapContext {
    fn default() -> Self {
        Self {
            projection_version: None,
            cache_state: CacheState::Disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheState {
    Disabled,
    HookRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReportSummary {
    pub semantic_checked: bool,
    pub dict_entry_count: Option<u32>,
    pub stages: Vec<ValidationStageSummary>,
    pub ignored_optional_sections: Vec<IgnoredOptionalSectionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationStageSummary {
    pub stage: String,
    pub status: String,
    pub sections_checked: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredOptionalSectionSummary {
    pub section_id: u32,
    pub section_kind: u16,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionalMetadataKind {
    CoveCoverage,
    CoveIOrCovx,
    CoveL,
    CoveE,
    CoveR,
    CoveCache,
    CoveCx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionalMetadataStatus {
    Trusted,
    Ignored,
    Disabled,
    NotRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalMetadataOutcome {
    pub kind: OptionalMetadataKind,
    pub status: OptionalMetadataStatus,
    pub reason: String,
}

impl OptionalMetadataOutcome {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "kind": optional_metadata_kind_name(self.kind),
            "status": optional_metadata_status_name(self.status),
            "reason": self.reason,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    MaterializedArrowBuffers,
    MetadataOnlyDenied,
    OptionalMetadataIgnored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackReport {
    pub kind: FallbackKind,
    pub reason: String,
}

impl FallbackReport {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "kind": fallback_kind_name(self.kind),
            "reason": self.reason,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionKind {
    FeatureValidation,
    ResourceBudget,
    SecurityPolicy,
    UnsupportedDatasetScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectionReport {
    pub kind: RejectionKind,
    pub reason: String,
}

impl RejectionReport {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "kind": rejection_kind_name(self.kind),
            "reason": self.reason,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveQlDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub phase: String,
    pub safe_details: Value,
    pub redacted: bool,
}

impl CoveQlDiagnostic {
    fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        phase: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            phase: phase.into(),
            safe_details: json!({}),
            redacted: true,
        }
    }

    fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        phase: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            phase: phase.into(),
            safe_details: json!({}),
            redacted: true,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "code": self.code,
            "severity": diagnostic_severity_name(self.severity),
            "message": self.message,
            "span": Value::Null,
            "phase": self.phase,
            "safe_details": self.safe_details,
            "redacted": self.redacted,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BuildOperationContextError {
    pub diagnostics: Vec<CoveQlDiagnostic>,
    pub rejections: Vec<RejectionReport>,
    pub source: Option<String>,
}

impl BuildOperationContextError {
    fn single(diagnostic: CoveQlDiagnostic, rejection: RejectionReport) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            rejections: vec![rejection],
            source: None,
        }
    }

    pub fn explain_json(&self) -> Value {
        crate::explain::error_explain_json(
            "operation_context",
            self.diagnostics
                .iter()
                .map(CoveQlDiagnostic::to_json)
                .collect(),
            self.rejections
                .iter()
                .map(RejectionReport::to_json)
                .collect(),
        )
    }
}

impl fmt::Display for BuildOperationContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "CoveQL operation context build failed")
        }
    }
}

impl Error for BuildOperationContextError {}

pub fn build_operation_context(
    bytes: &[u8],
    request: CoveQlOperationRequest,
    validation_options: ValidationOptions,
) -> Result<OperationContext, BuildOperationContextError> {
    check_resource_budget(&request)?;

    let mut diagnostics = Vec::new();
    let mut fallbacks = Vec::new();
    enforce_security_gates(&request, &mut diagnostics, &mut fallbacks)?;

    let feature_uses = selected_feature_uses(&request);
    let validator = EmbeddedOptionalProfileValidator::new(RuntimeSession::default_builtins());
    let mut reports = Vec::with_capacity(feature_uses.len());
    let mut first_report: Option<ValidationReport> = None;

    for feature_use in &feature_uses {
        let report = validate_bytes_for_feature_use_with_optional_profile_validator(
            bytes,
            validation_options.clone(),
            feature_use.clone(),
            &validator,
        )
        .map_err(|error| validation_error(error, "validation"))?;

        if first_report.is_none() {
            first_report = Some(report.clone());
        }
        reports.push(ValidationReportSummary::from(&report));
    }

    let first_report = first_report.ok_or_else(|| {
        BuildOperationContextError::single(
            CoveQlDiagnostic::error(
                "E_UNSUPPORTED_CONSTRUCT",
                "selected operation produced no feature-use request",
                "operation_context",
            ),
            RejectionReport {
                kind: RejectionKind::FeatureValidation,
                reason: "selected operation produced no feature-use request".into(),
            },
        )
    })?;
    let feature_scope_table = feature_scope_table_for_feature_use(bytes, &first_report.validated)
        .map_err(|error| validation_error(error, "validation"))?;

    for ignored in &first_report.ignored_optional_sections {
        fallbacks.push(FallbackReport {
            kind: FallbackKind::OptionalMetadataIgnored,
            reason: format!(
                "ignored optional section {}: {}",
                ignored.section_id, ignored.reason
            ),
        });
    }

    let optional_metadata = optional_metadata_outcomes(
        &first_report,
        &feature_uses,
        request.execution_code_mapping_requested,
        request.cache_hook.is_some(),
    );
    let snapshot = snapshot_context(bytes, &first_report);
    let file = ValidatedFileIdentity::from(&first_report);
    let mut dataset = DatasetScopeContext::single_file(&file, &snapshot, &request.security);
    dataset.execution_code_domains =
        execution_code_domain_contexts(bytes, &request, &validation_options, &dataset);
    let semantic_map = semantic_map_context(
        bytes,
        &first_report,
        if request.cache_hook.is_some() {
            CacheState::HookRegistered
        } else {
            CacheState::Disabled
        },
    );

    Ok(OperationContext {
        language_version: COVEQL_LANGUAGE_VERSION,
        core_version: COVEQL_CORE_VERSION,
        grammar_version: COVEQL_GRAMMAR_VERSION,
        resolved_ast_version: RESOLVED_AST_VERSION,
        logical_plan_version: LOGICAL_PLAN_VERSION,
        physical_plan_version: PHYSICAL_PLAN_VERSION,
        explain_json_schema_version: EXPLAIN_JSON_SCHEMA_VERSION,
        profile_contract_version: COVEQL_PROFILE_CONTRACT_VERSION,
        bridge_contract_version: COVEQL_BRIDGE_CONTRACT_VERSION,
        projection_dependency_contract_version: PROJECTION_DEPENDENCY_CONTRACT_VERSION,
        predicate_normal_form_version: PREDICATE_NORMAL_FORM_VERSION,
        selected_feature_uses: feature_uses,
        file,
        dataset,
        feature_scope_table,
        validation_reports: reports,
        snapshot,
        semantic_map,
        temporal: request.temporal.clone(),
        branch: request.branch.clone(),
        tombstone: request.tombstone.clone(),
        security: request.security.clone(),
        resource_budget: request.resource_budget.clone(),
        optional_metadata,
        fallbacks,
        rejections: Vec::new(),
        diagnostics,
        request,
    })
}

fn execution_code_domain_contexts(
    bytes: &[u8],
    request: &CoveQlOperationRequest,
    validation_options: &ValidationOptions,
    dataset: &DatasetScopeContext,
) -> Vec<ExecutionCodeDomainContext> {
    if !request.execution_code_mapping_requested {
        return Vec::new();
    }

    let security_scope_id = dataset_security_scope_id(&dataset.security_scope);
    let security_blocks_exact_use = request.security.metadata_disclosure_policy
        != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            request.security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        );

    match mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: validation_options.verify_digests,
            allow_unknown_optional_extensions: validation_options.allow_unknown_optional_extensions,
            covx: None,
            covm: None,
        },
        None,
    ) {
        Ok(mounted)
            if !mounted.execution_descriptors.is_empty()
                || !mounted.code_spaces.is_empty()
                || !mounted.engine_mount_policies.is_empty() =>
        {
            let descriptor = mounted.execution_descriptors.first();
            let code_space = mounted.code_spaces.first();
            let reason = if security_blocks_exact_use {
                "COVE-E execution-code metadata is present, but active security policy blocks coded bridge exposure"
            } else {
                "COVE-E execution-code metadata is validated; exact coded comparison still requires a runtime remap proof gate"
            };
            vec![ExecutionCodeDomainContext {
                engine_profile_id: mounted
                    .engine_profile_registries
                    .first()
                    .map(|_| "embedded_engine_profile".into())
                    .unwrap_or_else(|| "embedded_engine_profile:absent".into()),
                code_space_id: code_space
                    .map(|_| "embedded_code_space".into())
                    .unwrap_or_else(|| "embedded_code_space:absent".into()),
                comparison_scope: descriptor
                    .map(|descriptor| format!("{:?}", descriptor.comparison_scope))
                    .unwrap_or_else(|| "unknown".into()),
                lifetime: descriptor
                    .map(|descriptor| format!("{:?}", descriptor.lifetime))
                    .unwrap_or_else(|| "unknown".into()),
                epoch: code_space.map(|code_space| code_space.epoch),
                null_code_policy: descriptor
                    .map(|descriptor| format!("{:?}", descriptor.null_code_policy))
                    .unwrap_or_else(|| "unknown".into()),
                semantic_domain_id: None,
                security_scope_id,
                exact: false,
                reason: reason.into(),
            }]
        }
        Ok(_) => vec![ExecutionCodeDomainContext {
            engine_profile_id: "unavailable".into(),
            code_space_id: "unavailable".into(),
            comparison_scope: "unavailable".into(),
            lifetime: "unavailable".into(),
            epoch: None,
            null_code_policy: "unavailable".into(),
            semantic_domain_id: None,
            security_scope_id,
            exact: false,
            reason:
                "execution-code mapping was requested, but no embedded COVE-E metadata was found"
                    .into(),
        }],
        Err(error) => vec![ExecutionCodeDomainContext {
            engine_profile_id: "invalid".into(),
            code_space_id: "invalid".into(),
            comparison_scope: "invalid".into(),
            lifetime: "invalid".into(),
            epoch: None,
            null_code_policy: "invalid".into(),
            semantic_domain_id: None,
            security_scope_id,
            exact: false,
            reason: format!("COVE-E metadata validation failed: {error}"),
        }],
    }
}

fn manifest_member_execution_code_domains(
    bytes: &[u8],
    source: &str,
    security: &SecurityContext,
    tenant_id: Option<&str>,
) -> Vec<ExecutionCodeDomainContext> {
    let security_scope =
        DatasetSecurityScopeContext::from_security(security, tenant_id.map(str::to_string));
    let security_scope_id = dataset_security_scope_id(&security_scope);
    let security_blocks_exact_use = security.metadata_disclosure_policy
        != MetadataDisclosurePolicy::AllowProtected
        || matches!(
            security.visibility_policy,
            VisibilityPolicy::ExternalOverlay(_)
        );

    let Ok(mounted) = mount_cove_file(
        bytes,
        MountOptions {
            representation: OutputRepresentation::DecodeToValue,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            covx: None,
            covm: None,
        },
        None,
    ) else {
        return Vec::new();
    };
    if mounted.execution_descriptors.is_empty()
        && mounted.code_spaces.is_empty()
        && mounted.engine_mount_policies.is_empty()
    {
        return Vec::new();
    }
    let descriptor = mounted.execution_descriptors.first();
    let code_space = mounted.code_spaces.first();
    let semantic_domain_id = code_space.map(|code_space| {
        format!(
            "cove_e:{}:{}",
            code_space.namespace,
            String::from_utf8_lossy(&code_space.stable_id)
        )
    });
    let reason = if security_blocks_exact_use {
        "COVE-E execution-code metadata is present in a manifest member, but active security policy blocks coded bridge exposure"
    } else {
        "COVE-E execution-code metadata is present in a manifest member; exact cross-file use requires an epoch-bound manifest bridge proof for every member"
    };
    vec![ExecutionCodeDomainContext {
        engine_profile_id: mounted
            .engine_profile_registries
            .first()
            .map(|_| format!("{source}:embedded_engine_profile"))
            .unwrap_or_else(|| format!("{source}:embedded_engine_profile:absent")),
        code_space_id: code_space
            .map(|_| format!("{source}:embedded_code_space"))
            .unwrap_or_else(|| format!("{source}:embedded_code_space:absent")),
        comparison_scope: descriptor
            .map(|descriptor| format!("{:?}", descriptor.comparison_scope))
            .unwrap_or_else(|| "unknown".into()),
        lifetime: descriptor
            .map(|descriptor| format!("{:?}", descriptor.lifetime))
            .unwrap_or_else(|| "unknown".into()),
        epoch: code_space.map(|code_space| code_space.epoch),
        null_code_policy: descriptor
            .map(|descriptor| format!("{:?}", descriptor.null_code_policy))
            .unwrap_or_else(|| "unknown".into()),
        semantic_domain_id,
        security_scope_id,
        exact: false,
        reason: reason.into(),
    }]
}

fn dataset_security_scope_id(scope: &DatasetSecurityScopeContext) -> Option<String> {
    scope
        .tenant_id
        .as_ref()
        .map(|tenant| format!("tenant:{tenant}"))
        .or_else(|| {
            scope
                .principal_or_session
                .as_ref()
                .map(|principal| format!("principal:{principal}"))
        })
}

fn snapshot_context(bytes: &[u8], report: &ValidationReport) -> SnapshotContext {
    let file = ValidatedFileIdentity::from(report);
    let file_id = hex_lower(&file.file_id);
    let footer_crc = format!("{:08x}", file.footer_crc32c);
    let file_digest = format!("crc32c:{:08x}", checksum::crc32c(bytes));
    let snapshot_id = coverage_cache_snapshot_id(bytes, &file);
    let semantic_map_fingerprint = semantic_map_fingerprint(bytes, report);
    SnapshotContext {
        dataset_id: Some(format!("file:{file_id}")),
        snapshot_id,
        selected_snapshot_ref: None,
        schema_fingerprint: schema_fingerprint(bytes, report).or_else(|| {
            Some(format!(
                "footer:{footer_crc}:sections:{}",
                file.section_count
            ))
        }),
        semantic_map_fingerprint,
        file_digest: Some(file_digest),
        authority: Some(format!(
            "validated_file:file_id={file_id}:footer_crc32c={footer_crc}:len={}",
            file.file_len
        )),
    }
}

fn coverage_cache_snapshot_id(bytes: &[u8], file: &ValidatedFileIdentity) -> Option<String> {
    let file_digest = compute_digest(DigestAlgorithm::Sha256, bytes).ok()?;
    let mut seed = Vec::with_capacity(28 + file_digest.len());
    seed.extend_from_slice(&file.file_id);
    seed.extend_from_slice(&file.file_len.to_le_bytes());
    seed.extend_from_slice(&file.footer_crc32c.to_le_bytes());
    seed.extend_from_slice(&file_digest);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed).ok()?;
    let mut snapshot_id = [0u8; 16];
    snapshot_id.copy_from_slice(digest.get(..16)?);
    Some(hex_lower(&snapshot_id))
}

fn semantic_map_context(
    bytes: &[u8],
    report: &ValidationReport,
    cache_state: CacheState,
) -> SemanticMapContext {
    SemanticMapContext {
        projection_version: map_projection_version(bytes, report),
        cache_state,
    }
}

fn schema_fingerprint(bytes: &[u8], report: &ValidationReport) -> Option<String> {
    map_evidence_index(bytes, report).and_then(|index| {
        index
            .entries
            .iter()
            .find_map(|entry| entry.observed_schema_fingerprint.clone())
    })
}

fn semantic_map_fingerprint(bytes: &[u8], report: &ValidationReport) -> Option<String> {
    map_projection_catalog(bytes, report)
        .map(|catalog| {
            format!(
                "mapping:{}:version:{}:projections:{}",
                catalog.mapping_id,
                catalog.mapping_version,
                catalog.projections.len()
            )
        })
        .or_else(|| {
            map_evidence_index(bytes, report).map(|index| {
                format!(
                    "mapping:{}:version:{}:evidence:{}",
                    index.mapping_id,
                    index.mapping_version,
                    index.entries.len()
                )
            })
        })
}

fn map_projection_version(bytes: &[u8], report: &ValidationReport) -> Option<String> {
    map_projection_catalog(bytes, report)
        .map(|catalog| catalog.mapping_version)
        .or_else(|| map_evidence_index(bytes, report).map(|index| index.mapping_version))
}

fn map_projection_catalog(bytes: &[u8], report: &ValidationReport) -> Option<MapProjectionCatalog> {
    parse_first_map_section(
        bytes,
        report,
        SectionKind::MapProjectionCatalog,
        MapProjectionCatalog::parse,
    )
}

fn map_evidence_index(bytes: &[u8], report: &ValidationReport) -> Option<MapEvidenceIndex> {
    parse_first_map_section(
        bytes,
        report,
        SectionKind::MapEvidenceIndex,
        MapEvidenceIndex::parse,
    )
}

fn parse_first_map_section<T>(
    bytes: &[u8],
    report: &ValidationReport,
    kind: SectionKind,
    parse: fn(&[u8]) -> Result<T, CoveError>,
) -> Option<T> {
    report
        .validated
        .footer
        .sections
        .iter()
        .filter(|section| section.section_kind == kind as u16)
        .find_map(|section| {
            compression::section_payload(bytes, section)
                .ok()
                .and_then(|payload| parse(payload.as_ref()).ok())
        })
}

pub fn selected_feature_uses(request: &CoveQlOperationRequest) -> Vec<FeatureUseRequestV2> {
    let mut uses = Vec::new();
    append_operation_feature_uses(&request.selected_operation, &request.output_mode, &mut uses);
    if request.execution_code_mapping_requested {
        push_profile_and_operation(
            out_profile(PrimaryProfile::EngineExecution),
            OperationKindV2::EngineExecutionMapping,
            &mut uses,
        );
    }
    if request.evidence_metadata_requested {
        push_evidence_metadata(&mut uses);
    }
    dedup_feature_uses(uses)
}

fn append_operation_feature_uses(
    selected_operation: &CoveQlSelectedOperation,
    output_mode: &CoveQlOutputMode,
    out: &mut Vec<FeatureUseRequestV2>,
) {
    match selected_operation {
        CoveQlSelectedOperation::Object => push_object(out),
        CoveQlSelectedOperation::Association => push_object(out),
        CoveQlSelectedOperation::GraphNode => push_object(out),
        CoveQlSelectedOperation::GraphEdge => push_object(out),
        CoveQlSelectedOperation::Table => push_projection(out),
        CoveQlSelectedOperation::Projection => push_projection(out),
        CoveQlSelectedOperation::Evidence => push_evidence(out),
        CoveQlSelectedOperation::IndexOnlyAnswer => push_index_only(out),
        CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested,
        } => {
            push_object(out);
            if *zero_copy_requested {
                push_zero_copy(out);
            }
        }
        CoveQlSelectedOperation::Explain { target, .. } => {
            append_explain_target_feature_uses(target, out);
            append_explain_mapping_feature_uses(target, out);
            push_profile_and_operation(
                out_profile(PrimaryProfile::CoverageMetadata),
                OperationKindV2::CoveragePlanning,
                out,
            );
        }
    }

    if let CoveQlOutputMode::ArrowRecordBatch {
        zero_copy_requested: true,
    } = output_mode
    {
        push_zero_copy(out);
    }
}

fn append_explain_mapping_feature_uses(
    target: &CoveQlExplainTarget,
    out: &mut Vec<FeatureUseRequestV2>,
) {
    match target {
        CoveQlExplainTarget::Table
        | CoveQlExplainTarget::Projection
        | CoveQlExplainTarget::Evidence => push_mapping_explanation(out),
        CoveQlExplainTarget::Object
        | CoveQlExplainTarget::Association
        | CoveQlExplainTarget::GraphNode
        | CoveQlExplainTarget::GraphEdge
        | CoveQlExplainTarget::IndexOnlyAnswer
        | CoveQlExplainTarget::ArrowExport { .. } => {}
    }
}

fn append_explain_target_feature_uses(
    target: &CoveQlExplainTarget,
    out: &mut Vec<FeatureUseRequestV2>,
) {
    match target {
        CoveQlExplainTarget::Object | CoveQlExplainTarget::Association => push_object(out),
        CoveQlExplainTarget::GraphNode | CoveQlExplainTarget::GraphEdge => push_object(out),
        CoveQlExplainTarget::Table => push_projection(out),
        CoveQlExplainTarget::Projection => push_projection(out),
        CoveQlExplainTarget::Evidence => push_evidence(out),
        CoveQlExplainTarget::IndexOnlyAnswer => push_index_only(out),
        CoveQlExplainTarget::ArrowExport {
            zero_copy_requested,
        } => {
            push_object(out);
            if *zero_copy_requested {
                push_zero_copy(out);
            }
        }
    }
}

fn push_object(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::ObjectTemporal),
        OperationKindV2::ObjectReconstruction,
        out,
    );
}

fn push_projection(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::SemanticMapping),
        OperationKindV2::ProjectionReadback,
        out,
    );
}

fn push_evidence(out: &mut Vec<FeatureUseRequestV2>) {
    push_object(out);
    push_evidence_metadata(out);
}

fn push_evidence_metadata(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::SemanticMapping),
        OperationKindV2::EvidenceReadback,
        out,
    );
}

fn push_mapping_explanation(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::SemanticMapping),
        OperationKindV2::MappingExplanation,
        out,
    );
}

fn push_index_only(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::SecondaryIndex),
        OperationKindV2::IndexOnlyAnswer,
        out,
    );
}

fn push_zero_copy(out: &mut Vec<FeatureUseRequestV2>) {
    push_profile_and_operation(
        out_profile(PrimaryProfile::LayoutPlanning),
        OperationKindV2::ZeroCopyExport,
        out,
    );
}

fn push_profile_and_operation(
    profile: u8,
    operation: OperationKindV2,
    out: &mut Vec<FeatureUseRequestV2>,
) {
    out.push(FeatureUseRequestV2::new().with_profile(profile));
    out.push(FeatureUseRequestV2::new().with_operation(operation));
}

fn out_profile(profile: PrimaryProfile) -> u8 {
    profile as u8
}

fn dedup_feature_uses(uses: Vec<FeatureUseRequestV2>) -> Vec<FeatureUseRequestV2> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for feature_use in uses {
        let key = feature_use_key(&feature_use);
        if seen.insert(key) {
            out.push(feature_use);
        }
    }
    out
}

fn feature_use_key(
    feature_use: &FeatureUseRequestV2,
) -> (Option<u8>, Option<u16>, Vec<u32>, Vec<(u32, u64)>) {
    (
        feature_use.requested_profile,
        feature_use
            .requested_operation
            .map(|operation| operation as u16),
        feature_use.needed_section_ids.iter().copied().collect(),
        feature_use
            .needed_page_refs
            .iter()
            .map(|target| (target.section_id, target.target_local_ref))
            .collect(),
    )
}

fn enforce_security_gates(
    request: &CoveQlOperationRequest,
    diagnostics: &mut Vec<CoveQlDiagnostic>,
    fallbacks: &mut Vec<FallbackReport>,
) -> Result<(), BuildOperationContextError> {
    if selected_operation_requires_index_only(&request.selected_operation)
        && !request.security.index_only_answer_permission
    {
        return Err(BuildOperationContextError::single(
            CoveQlDiagnostic::error(
                "E_INDEX_ONLY_FORBIDDEN",
                "index-only answers are not permitted by the active security context",
                "security",
            ),
            RejectionReport {
                kind: RejectionKind::SecurityPolicy,
                reason: "index-only answers are not permitted".into(),
            },
        ));
    }

    if zero_copy_requested(request) && !request.security.zero_copy_permission {
        match request.fallback_policy {
            FallbackPolicy::AllowMaterializedFallback => {
                diagnostics.push(CoveQlDiagnostic::warning(
                    "E_ZERO_COPY_FORBIDDEN",
                    "zero-copy output is not permitted; materialized owned buffers are required",
                    "security",
                ));
                fallbacks.push(FallbackReport {
                    kind: FallbackKind::MaterializedArrowBuffers,
                    reason: "zero-copy output is not permitted by the active security context"
                        .into(),
                });
            }
            FallbackPolicy::RejectOnFallback => {
                return Err(BuildOperationContextError::single(
                    CoveQlDiagnostic::error(
                        "E_ZERO_COPY_FORBIDDEN",
                        "zero-copy output is not permitted by the active security context",
                        "security",
                    ),
                    RejectionReport {
                        kind: RejectionKind::SecurityPolicy,
                        reason: "zero-copy output is not permitted".into(),
                    },
                ));
            }
        }
    }

    if let Some(mode) = requested_explain_mode(&request.selected_operation) {
        let protected_allowed =
            request.security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected;
        if matches!(
            mode,
            ExplainMode::Developer
                | ExplainMode::Proof
                | ExplainMode::Coded
                | ExplainMode::Forensic
        ) && !protected_allowed
        {
            diagnostics.push(CoveQlDiagnostic::warning(
                "E_SECURITY_DISCLOSURE_FORBIDDEN",
                "protected explain metadata will be redacted",
                "security",
            ));
        }
    }

    Ok(())
}

fn check_resource_budget(
    request: &CoveQlOperationRequest,
) -> Result<(), BuildOperationContextError> {
    let budget = &request.resource_budget;
    let usage = &request.resource_use;
    check_usize(
        usage.query_bytes,
        budget.maximum_query_bytes,
        "maximum_query_bytes",
    )?;
    check_usize(
        usage.ast_depth,
        budget.maximum_ast_depth,
        "maximum_ast_depth",
    )?;
    check_usize(
        usage.method_count,
        budget.maximum_method_count,
        "maximum_method_count",
    )?;
    check_usize(
        usage.in_list_size,
        budget.maximum_in_list_size,
        "maximum_in_list_size",
    )?;
    check_usize(
        usage.disjunction_count,
        budget.maximum_disjunction_count,
        "maximum_disjunction_count",
    )?;
    check_usize(
        usage.output_columns,
        budget.maximum_output_columns,
        "maximum_output_columns",
    )?;
    check_usize(usage.groups, budget.maximum_groups, "maximum_groups")?;
    check_usize(
        usage.rows_without_explicit_take,
        budget.maximum_rows_without_explicit_take,
        "maximum_rows_without_explicit_take",
    )?;
    check_usize(
        usage.decode_bytes,
        budget.maximum_decode_bytes,
        "maximum_decode_bytes",
    )?;
    check_usize(
        usage.range_requests,
        budget.maximum_range_requests,
        "maximum_range_requests",
    )?;
    check_u32(
        usage.graph_traversal_depth,
        budget.maximum_graph_traversal_depth,
        "maximum_graph_traversal_depth",
    )?;
    check_usize(
        usage.graph_traversal_fanout,
        budget.maximum_graph_traversal_fanout,
        "maximum_graph_traversal_fanout",
    )?;
    check_usize(
        usage.graph_traversal_paths,
        budget.maximum_graph_traversal_paths,
        "maximum_graph_traversal_paths",
    )?;
    check_usize(
        usage.graph_traversal_frontier,
        budget.maximum_graph_traversal_frontier,
        "maximum_graph_traversal_frontier",
    )?;
    check_u64(
        usage.planning_time_ms,
        budget.maximum_planning_time_ms,
        "maximum_planning_time_ms",
    )?;
    check_u64(
        usage.execution_time_ms,
        budget.maximum_execution_time_ms,
        "maximum_execution_time_ms",
    )?;
    Ok(())
}

fn check_usize(
    value: Option<usize>,
    limit: usize,
    field: &str,
) -> Result<(), BuildOperationContextError> {
    if let Some(value) = value {
        if value > limit {
            return Err(resource_budget_error(
                field,
                value.to_string(),
                limit.to_string(),
            ));
        }
    }
    Ok(())
}

fn check_u32(
    value: Option<u32>,
    limit: u32,
    field: &str,
) -> Result<(), BuildOperationContextError> {
    if let Some(value) = value {
        if value > limit {
            return Err(resource_budget_error(
                field,
                value.to_string(),
                limit.to_string(),
            ));
        }
    }
    Ok(())
}

fn check_u64(
    value: Option<u64>,
    limit: u64,
    field: &str,
) -> Result<(), BuildOperationContextError> {
    if let Some(value) = value {
        if value > limit {
            return Err(resource_budget_error(
                field,
                value.to_string(),
                limit.to_string(),
            ));
        }
    }
    Ok(())
}

fn resource_budget_error(field: &str, value: String, limit: String) -> BuildOperationContextError {
    BuildOperationContextError::single(
        CoveQlDiagnostic::error(
            "E_RESOURCE_BUDGET_EXCEEDED",
            format!("{field} budget exceeded: {value} > {limit}"),
            "planning",
        ),
        RejectionReport {
            kind: RejectionKind::ResourceBudget,
            reason: format!("{field} budget exceeded"),
        },
    )
}

fn validation_error(error: CoveError, phase: &'static str) -> BuildOperationContextError {
    let code = diagnostic_code_for_error(&error);
    let message = error.to_string();
    BuildOperationContextError::single(
        CoveQlDiagnostic::error(code, message.clone(), phase),
        RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: message,
        },
    )
}

fn optional_metadata_outcomes(
    report: &ValidationReport,
    feature_uses: &[FeatureUseRequestV2],
    execution_code_mapping_requested: bool,
    cache_hook_present: bool,
) -> Vec<OptionalMetadataOutcome> {
    let ignored_kinds = ignored_optional_metadata_kinds(&report.ignored_optional_sections);
    let present_kinds = present_optional_metadata_kinds(report);
    let requested =
        requested_optional_metadata_kinds(feature_uses, execution_code_mapping_requested);
    let mut outcomes = Vec::new();

    for kind in [
        OptionalMetadataKind::CoveCoverage,
        OptionalMetadataKind::CoveIOrCovx,
        OptionalMetadataKind::CoveL,
        OptionalMetadataKind::CoveE,
        OptionalMetadataKind::CoveR,
        OptionalMetadataKind::CoveCache,
        OptionalMetadataKind::CoveCx,
    ] {
        let status = if kind == OptionalMetadataKind::CoveCache && !cache_hook_present {
            OptionalMetadataStatus::Disabled
        } else if ignored_kinds.contains(&kind) {
            OptionalMetadataStatus::Ignored
        } else if requested.contains(&kind) && present_kinds.contains(&kind) {
            OptionalMetadataStatus::Trusted
        } else if requested.contains(&kind) {
            OptionalMetadataStatus::NotRequested
        } else {
            OptionalMetadataStatus::Disabled
        };
        let reason = match status {
            OptionalMetadataStatus::Trusted => "validated for selected operation",
            OptionalMetadataStatus::Ignored => "ignored under optional metadata fallback policy",
            OptionalMetadataStatus::Disabled if kind == OptionalMetadataKind::CoveCache => {
                "COVE-CACHE disabled without an explicit cache hook"
            }
            OptionalMetadataStatus::Disabled => "not selected for this Phase 0 operation",
            OptionalMetadataStatus::NotRequested => "selected operation found no matching metadata",
        };
        outcomes.push(OptionalMetadataOutcome {
            kind,
            status,
            reason: reason.into(),
        });
    }
    outcomes
}

fn requested_optional_metadata_kinds(
    feature_uses: &[FeatureUseRequestV2],
    execution_code_mapping_requested: bool,
) -> BTreeSet<OptionalMetadataKind> {
    let mut out = BTreeSet::new();
    for feature_use in feature_uses {
        match feature_use.requested_operation {
            Some(OperationKindV2::CoveragePlanning) => {
                out.insert(OptionalMetadataKind::CoveCoverage);
            }
            Some(OperationKindV2::IndexOnlyAnswer) => {
                out.insert(OptionalMetadataKind::CoveIOrCovx);
            }
            Some(OperationKindV2::ZeroCopyExport) => {
                out.insert(OptionalMetadataKind::CoveL);
            }
            Some(OperationKindV2::RuntimeAdapterSelection) => {
                out.insert(OptionalMetadataKind::CoveR);
            }
            Some(OperationKindV2::EngineExecutionMapping) => {
                out.insert(OptionalMetadataKind::CoveE);
            }
            _ => {}
        }
    }
    if execution_code_mapping_requested {
        out.insert(OptionalMetadataKind::CoveE);
    }
    out
}

fn present_optional_metadata_kinds(report: &ValidationReport) -> BTreeSet<OptionalMetadataKind> {
    report
        .validated
        .footer
        .sections
        .iter()
        .filter_map(|section| SectionKind::from_u16(section.section_kind))
        .filter_map(optional_kind_for_section)
        .collect()
}

fn ignored_optional_metadata_kinds(
    ignored_sections: &[IgnoredOptionalSection],
) -> BTreeSet<OptionalMetadataKind> {
    ignored_sections
        .iter()
        .filter_map(|section| SectionKind::from_u16(section.section_kind))
        .filter_map(optional_kind_for_section)
        .collect()
}

fn optional_kind_for_section(kind: SectionKind) -> Option<OptionalMetadataKind> {
    match kind {
        SectionKind::CoverageProviderRegistry
        | SectionKind::CoverageSet
        | SectionKind::CoveragePlanCandidate
        | SectionKind::PredicateNormalForm
        | SectionKind::CoverageProofRecord => Some(OptionalMetadataKind::CoveCoverage),
        SectionKind::IndexOnlyCapability => Some(OptionalMetadataKind::CoveIOrCovx),
        SectionKind::LayoutPlan
        | SectionKind::ScanSplitIndex
        | SectionKind::PageClusterDirectory
        | SectionKind::ZeroCopyBufferMap
        | SectionKind::FastMetadataIndex => Some(OptionalMetadataKind::CoveL),
        SectionKind::RuntimeCompatibilityHints => Some(OptionalMetadataKind::CoveR),
        SectionKind::CodecExtensionRegistry => Some(OptionalMetadataKind::CoveCx),
        SectionKind::EngineProfileRegistry
        | SectionKind::ExecutionCodeDescriptor
        | SectionKind::ExecutionScopeDescriptor
        | SectionKind::CodeSpaceDescriptor
        | SectionKind::EngineMountPolicy => Some(OptionalMetadataKind::CoveE),
        _ => None,
    }
}

fn selected_operation_requires_index_only(operation: &CoveQlSelectedOperation) -> bool {
    matches!(operation, CoveQlSelectedOperation::IndexOnlyAnswer)
        || matches!(
            operation,
            CoveQlSelectedOperation::Explain {
                target: CoveQlExplainTarget::IndexOnlyAnswer,
                ..
            }
        )
}

fn zero_copy_requested(request: &CoveQlOperationRequest) -> bool {
    match &request.selected_operation {
        CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: true,
        } => true,
        CoveQlSelectedOperation::Explain {
            target:
                CoveQlExplainTarget::ArrowExport {
                    zero_copy_requested: true,
                },
            ..
        } => true,
        _ => matches!(
            request.output_mode,
            CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: true
            }
        ),
    }
}

fn requested_explain_mode(operation: &CoveQlSelectedOperation) -> Option<ExplainMode> {
    match operation {
        CoveQlSelectedOperation::Explain { mode, .. } => Some(*mode),
        _ => None,
    }
}

impl From<&ValidationReport> for ValidationReportSummary {
    fn from(report: &ValidationReport) -> Self {
        Self {
            semantic_checked: report.semantic_checked,
            dict_entry_count: report.dict_entry_count,
            stages: report
                .stages
                .iter()
                .map(ValidationStageSummary::from)
                .collect(),
            ignored_optional_sections: report
                .ignored_optional_sections
                .iter()
                .map(IgnoredOptionalSectionSummary::from)
                .collect(),
        }
    }
}

impl From<&ValidationStageReport> for ValidationStageSummary {
    fn from(stage: &ValidationStageReport) -> Self {
        Self {
            stage: validation_stage_name(stage.stage).into(),
            status: validation_stage_status_name(stage.status).into(),
            sections_checked: stage.sections_checked,
        }
    }
}

impl From<&IgnoredOptionalSection> for IgnoredOptionalSectionSummary {
    fn from(ignored: &IgnoredOptionalSection) -> Self {
        Self {
            section_id: ignored.section_id,
            section_kind: ignored.section_kind,
            reason: ignored.reason.clone(),
        }
    }
}

impl From<&ValidationReport> for ValidatedFileIdentity {
    fn from(report: &ValidationReport) -> Self {
        Self {
            file_id: report.validated.header.file_id,
            file_len: report.validated.postscript.file_len,
            footer_crc32c: report.validated.postscript.footer.crc32c,
            primary_profile: report.validated.header.primary_profile,
            version_major: report.validated.header.version_major,
            version_minor: report.validated.header.version_minor,
            section_count: report.validated.footer.header.section_count,
        }
    }
}

impl From<&ValidatedCoveFile> for ValidatedFileIdentity {
    fn from(validated: &ValidatedCoveFile) -> Self {
        Self {
            file_id: validated.header.file_id,
            file_len: validated.postscript.file_len,
            footer_crc32c: validated.postscript.footer.crc32c,
            primary_profile: validated.header.primary_profile,
            version_major: validated.header.version_major,
            version_minor: validated.header.version_minor,
            section_count: validated.footer.header.section_count,
        }
    }
}

fn manifest_scope_error(message: String) -> BuildOperationContextError {
    BuildOperationContextError::single(
        CoveQlDiagnostic::error(
            "E_UNSUPPORTED_DATASET_SCOPE",
            message.clone(),
            "dataset_manifest",
        ),
        RejectionReport {
            kind: RejectionKind::UnsupportedDatasetScope,
            reason: message,
        },
    )
}

fn file_has_dictionary(validated: &ValidatedCoveFile) -> bool {
    validated.header.required_features & FEATURE_FILE_DICTIONARY != 0
        || validated
            .footer
            .sections
            .iter()
            .any(|section| section.section_kind == SectionKind::FileDictionaryIndex as u16)
}

fn manifest_snapshot_id(
    manifest_bytes: &[u8],
    file_membership_fingerprint: &str,
) -> Option<String> {
    let manifest_digest = compute_digest(DigestAlgorithm::Sha256, manifest_bytes).ok()?;
    let mut seed = Vec::with_capacity(file_membership_fingerprint.len() + manifest_digest.len());
    seed.extend_from_slice(file_membership_fingerprint.as_bytes());
    seed.extend_from_slice(&manifest_digest);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed).ok()?;
    Some(format!("sha256:{}", hex_lower(&digest)))
}

fn object_catalog_schema_fingerprint(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<Option<String>, BuildOperationContextError> {
    let Some(section) = validated
        .footer
        .sections
        .iter()
        .find(|section| section.section_kind == SectionKind::ObjectTypeCatalog as u16)
    else {
        return Ok(None);
    };
    let payload = compression::section_payload(bytes, section)
        .map_err(|error| validation_error(error, "dataset_member_schema"))?;
    let catalog = ObjectTypeCatalog::parse(payload.as_ref())
        .map_err(|error| validation_error(error, "dataset_member_schema"))?;
    let mut hasher = Sha256::new();
    hasher.update(catalog.flags.to_le_bytes());
    let mut types = catalog.types.iter().collect::<Vec<_>>();
    types.sort_by(|left, right| {
        left.object_type_id
            .cmp(&right.object_type_id)
            .then_with(|| left.type_name.cmp(&right.type_name))
    });
    for object_type in types {
        hasher.update(object_type.object_type_id.to_le_bytes());
        hash_string(&mut hasher, &object_type.type_name);
        hasher.update(object_type.flags.to_le_bytes());
        let mut properties = object_type.properties.iter().collect::<Vec<_>>();
        properties.sort_by(|left, right| {
            left.property_id
                .cmp(&right.property_id)
                .then_with(|| left.property_name.cmp(&right.property_name))
        });
        for property in properties {
            hasher.update(property.property_id.to_le_bytes());
            hash_string(&mut hasher, &property.property_name);
            hasher.update((property.logical_type as u16).to_le_bytes());
            hasher.update((property.physical_kind as u16).to_le_bytes());
            hasher.update([u8::from(property.nullable)]);
            hasher.update(property.collation_id.to_le_bytes());
            hasher.update(property.flags.to_le_bytes());
        }
    }
    Ok(Some(format!("sha256:{}", hex_lower(&hasher.finalize()))))
}

fn semantic_map_identity_fingerprint(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<Option<String>, BuildOperationContextError> {
    let mut identities = Vec::<(u16, String, String)>::new();
    for section in &validated.footer.sections {
        let Some(kind) = SectionKind::from_u16(section.section_kind) else {
            continue;
        };
        if !is_embedded_map_section(kind) {
            continue;
        }
        let payload = compression::section_payload(bytes, section)
            .map_err(|error| validation_error(error, "dataset_member_semantic_map"))?;
        let embedded = parse_embedded_section(kind, payload.as_ref())
            .map_err(|error| validation_error(error, "dataset_member_semantic_map"))?;
        let (mapping_id, mapping_version) = embedded_map_identity(&embedded);
        identities.push((
            kind as u16,
            mapping_id.to_string(),
            mapping_version.to_string(),
        ));
    }
    if identities.is_empty() {
        return Ok(None);
    }
    identities.sort();
    let mut hasher = Sha256::new();
    for (section_kind, mapping_id, mapping_version) in identities {
        hasher.update(section_kind.to_le_bytes());
        hash_string(&mut hasher, &mapping_id);
        hash_string(&mut hasher, &mapping_version);
    }
    Ok(Some(format!("sha256:{}", hex_lower(&hasher.finalize()))))
}

fn embedded_map_identity(section: &EmbeddedMapSection) -> (&str, &str) {
    match section {
        EmbeddedMapSection::SourceCatalog(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::FunctionRegistry(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::IdentityRuleCatalog(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::RowSemanticsCatalog(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::AssertionLog(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::IdentityEquivalenceIndex(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::EvidenceIndex(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::ConversionReport(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        EmbeddedMapSection::ProjectionCatalog(section) => {
            (&section.mapping_id, &section.mapping_version)
        }
        _ => ("unknown_non_exhaustive_map_section", "unknown"),
    }
}

fn is_embedded_map_section(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapSourceCatalog
            | SectionKind::MapFunctionRegistry
            | SectionKind::MapIdentityRuleCatalog
            | SectionKind::MapRowSemanticsCatalog
            | SectionKind::MapAssertionLog
            | SectionKind::MapIdentityEquivalenceIndex
            | SectionKind::MapEvidenceIndex
            | SectionKind::MapConversionReport
            | SectionKind::MapProjectionCatalog
    )
}

fn projection_catalog_schema_fingerprint(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<Option<String>, BuildOperationContextError> {
    let Some(section) = validated
        .footer
        .sections
        .iter()
        .find(|section| section.section_kind == SectionKind::MapProjectionCatalog as u16)
    else {
        return Ok(None);
    };
    let payload = compression::section_payload(bytes, section)
        .map_err(|error| validation_error(error, "dataset_member_projection_catalog"))?;
    let catalog = MapProjectionCatalog::parse(payload.as_ref())
        .map_err(|error| validation_error(error, "dataset_member_projection_catalog"))?;
    let mut hasher = Sha256::new();
    hash_string(&mut hasher, &catalog.mapping_id);
    hash_string(&mut hasher, &catalog.mapping_version);
    let mut projections = catalog.projections.iter().collect::<Vec<_>>();
    projections.sort_by(|left, right| left.projection_id.cmp(&right.projection_id));
    for projection in projections {
        hash_string(&mut hasher, &projection.projection_id);
        hash_string_vec(&mut hasher, &projection.assertion_ids);
        hash_option_string(&mut hasher, projection.output_table.as_deref());
        hash_option_string(&mut hasher, projection.row_grain.as_deref());
        if let Some(anchor) = &projection.anchor {
            hasher.update([1]);
            hash_option_string(&mut hasher, anchor.object_type.as_deref());
            hash_option_string(&mut hasher, anchor.association_type.as_deref());
        } else {
            hasher.update([0]);
        }
        hash_option_string(&mut hasher, projection.temporal_mode.as_deref());
        hash_option_string(&mut hasher, projection.multi_value_policy.as_deref());
        hash_string(&mut hasher, &projection.missing_policy);
        hash_string_vec(&mut hasher, &projection.ordering);
        hash_string(&mut hasher, &projection.evidence_policy);
        let mut output_modes = projection.output_modes.clone();
        output_modes.sort();
        hash_string_vec(&mut hasher, &output_modes);
        for column in &projection.columns {
            hash_string(&mut hasher, &column.name);
            hash_string(&mut hasher, &column.value);
            hash_option_string(&mut hasher, column.logical_type.as_deref());
            hash_option_string(&mut hasher, column.nested_shape.as_deref());
            hash_string(&mut hasher, &column.conflict_policy);
            hash_string(&mut hasher, &column.missing_policy);
        }
    }
    Ok(Some(format!("sha256:{}", hex_lower(&hasher.finalize()))))
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn hash_option_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_string(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn hash_string_vec(hasher: &mut Sha256, values: &[String]) {
    hasher.update((values.len() as u64).to_le_bytes());
    for value in values {
        hash_string(hasher, value);
    }
}

fn validate_manifest_security_scope(
    options: &ManifestDatasetScopeOptions,
) -> Result<(), BuildOperationContextError> {
    if let (Some(tenant_id), VisibilityPolicy::ExternalOverlay(overlay_id)) =
        (&options.tenant_id, &options.security.visibility_policy)
    {
        if tenant_id != overlay_id {
            return Err(manifest_scope_error(format!(
                "COVM tenant/security scope mismatch: tenant_id {tenant_id} does not match visibility overlay {overlay_id}"
            )));
        }
    }
    Ok(())
}

fn validate_manifest_code_domain_bridge_proofs(
    options: &ManifestDatasetScopeOptions,
) -> Result<(), BuildOperationContextError> {
    let mut seen_domains = BTreeSet::new();
    for proof in &options.code_domain_bridge_proofs {
        let domain_id = proof.domain_id.trim();
        if domain_id.is_empty() {
            return Err(manifest_scope_error(
                "COVM code-domain bridge proof must declare a non-empty domain id".into(),
            ));
        }
        if !seen_domains.insert(domain_id.to_string()) {
            return Err(manifest_scope_error(format!(
                "COVM code-domain bridge proof for {domain_id} is ambiguous because the manifest supplied more than one proof for the same domain; raw local codes must not be compared across files without one epoch-bound bridge decision"
            )));
        }
        if proof.bridge_kind.trim().is_empty() {
            return Err(manifest_scope_error(format!(
                "COVM code-domain bridge proof for {domain_id} must declare a bridge kind"
            )));
        }
        if proof.exact && proof.epoch.is_none() {
            return Err(manifest_scope_error(format!(
                "exact COVM code-domain bridge proof for {domain_id} is missing a dictionary/domain epoch; raw local codes must not be compared across files without an epoch-bound remap proof"
            )));
        }
        if proof.exact && !declares_canonical_bridge_kind(&proof.bridge_kind) {
            return Err(manifest_scope_error(format!(
                "exact COVM code-domain bridge proof for {domain_id} must declare a canonical remap or materialized canonical value bridge; raw local-code equality is not a valid cross-file proof"
            )));
        }
    }
    Ok(())
}

fn validate_manifest_code_domain_bridge_proofs_against_members(
    bridge_proofs: &[ManifestCodeDomainBridgeProof],
    execution_code_domains: &[ExecutionCodeDomainContext],
    member_count: usize,
) -> Result<(), BuildOperationContextError> {
    for proof in bridge_proofs.iter().filter(|proof| proof.exact) {
        let Some(epoch) = proof.epoch else {
            continue;
        };
        let matching_member_count = execution_code_domains
            .iter()
            .filter(|domain| {
                domain.epoch == Some(epoch)
                    && domain
                        .semantic_domain_id
                        .as_deref()
                        .is_some_and(|domain_id| domain_id == proof.domain_id)
            })
            .count();
        if matching_member_count < member_count {
            return Err(manifest_scope_error(format!(
                "exact COVM code-domain bridge proof for {} epoch {} was observed on {} of {} member files; exact cross-file coded comparison requires the same COVE-E code-domain epoch to be validated on every member",
                proof.domain_id, epoch, matching_member_count, member_count
            )));
        }
    }
    Ok(())
}

fn declares_canonical_bridge_kind(bridge_kind: &str) -> bool {
    let bridge_kind = bridge_kind.trim().to_ascii_lowercase();
    (bridge_kind.contains("canonical")
        && (bridge_kind.contains("remap")
            || bridge_kind.contains("bridge")
            || bridge_kind.contains("value")))
        || bridge_kind.contains("execution_code_remap")
        || bridge_kind.contains("execution-code-remap")
}

pub(crate) fn code_domain_bridge_is_exact_coded_remap(bridge: &CodeDomainBridgeContext) -> bool {
    bridge.exact && declares_coded_remap_bridge_kind(&bridge.bridge_kind)
}

fn declares_coded_remap_bridge_kind(bridge_kind: &str) -> bool {
    let bridge_kind = bridge_kind.trim().to_ascii_lowercase();
    (bridge_kind.contains("canonical") && bridge_kind.contains("remap"))
        || bridge_kind.contains("execution_code_remap")
        || bridge_kind.contains("execution-code-remap")
}

fn manifest_code_domain_bridges(
    manifest_id: &str,
    multi_file: bool,
    security: &SecurityContext,
    tenant_id: Option<&str>,
    bridge_proofs: &[ManifestCodeDomainBridgeProof],
) -> Vec<CodeDomainBridgeContext> {
    if !multi_file {
        return Vec::new();
    }
    let security_scope_id = manifest_bridge_security_scope_id(security, tenant_id);
    if let Some(reason) = manifest_bridge_security_block_reason(security, tenant_id) {
        return vec![CodeDomainBridgeContext {
            domain_id: "redacted:manifest_code_domains".into(),
            bridge_kind: "security_blocked".into(),
            epoch: None,
            security_scope_id,
            exact: false,
            reason: reason.into(),
        }];
    }
    if !bridge_proofs.is_empty() {
        return bridge_proofs
            .iter()
            .map(|proof| CodeDomainBridgeContext {
                domain_id: proof.domain_id.clone(),
                bridge_kind: proof.bridge_kind.clone(),
                epoch: proof.epoch,
                security_scope_id: security_scope_id.clone(),
                exact: proof.exact,
                reason: if proof.exact {
                    format!(
                        "{}; epoch={}",
                        proof.reason,
                        proof
                            .epoch
                            .map(|epoch| epoch.to_string())
                            .unwrap_or_else(|| "unspecified".into())
                    )
                } else {
                    proof.reason.clone()
                },
            })
            .collect();
    }
    vec![CodeDomainBridgeContext {
        domain_id: format!("{manifest_id}:file_local_code_domains"),
        bridge_kind: "manifest_candidate_requires_canonical_remap".into(),
        epoch: None,
        security_scope_id,
        exact: false,
        reason: "COVM validates file membership and canonical ordering, but does not prove a cross-file canonical code remap; raw local codes must not be compared across files".into(),
    }]
}

fn manifest_bridge_security_scope_id(
    security: &SecurityContext,
    tenant_id: Option<&str>,
) -> Option<String> {
    let scope = DatasetSecurityScopeContext::from_security(security, tenant_id.map(str::to_string));
    dataset_security_scope_id(&scope)
}

fn manifest_bridge_security_block_reason(
    security: &SecurityContext,
    tenant_id: Option<&str>,
) -> Option<&'static str> {
    if security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected {
        Some("active security policy blocks manifest code-domain bridge exposure; raw local codes must not be compared across files and execution must materialize or reject coded pushdown")
    } else if matches!(
        security.visibility_policy,
        VisibilityPolicy::ExternalOverlay(_)
    ) {
        Some("external visibility overlays block manifest code-domain bridge exposure; raw local codes must not be compared across files and execution must materialize or reject coded pushdown")
    } else if tenant_id.is_none() {
        Some("tenant-scoped security context is required before exposing manifest code-domain bridge details; raw local codes must not be compared across files and execution must materialize or reject coded pushdown")
    } else if security.principal_or_session.is_none() {
        Some("principal or session scope is required before exposing manifest code-domain bridge details; raw local codes must not be compared across files and execution must materialize or reject coded pushdown")
    } else {
        None
    }
}

fn diagnostic_code_for_error(error: &CoveError) -> &'static str {
    match error {
        CoveError::UnknownRequiredFeature(_) => "E_UNSUPPORTED_CONSTRUCT",
        CoveError::SidecarStale => "E_STALE_SIDECAR",
        CoveError::BadCoverage => "E_CORRUPT_PROOF",
        CoveError::CoverageStale => "E_STALE_SIDECAR",
        CoveError::BadCovi => "E_STALE_SIDECAR",
        CoveError::IndexOnlyUnsafe => "E_INDEX_ONLY_FORBIDDEN",
        CoveError::RuntimeHintUnsupported => "E_UNSUPPORTED_CONSTRUCT",
        CoveError::RedactionPolicy => "E_SECURITY_DISCLOSURE_FORBIDDEN",
        CoveError::CodecUnsupported => "E_UNSUPPORTED_CONSTRUCT",
        _ => "E_UNSUPPORTED_CONSTRUCT",
    }
}

fn optional_metadata_kind_name(kind: OptionalMetadataKind) -> &'static str {
    match kind {
        OptionalMetadataKind::CoveCoverage => "cove_coverage",
        OptionalMetadataKind::CoveIOrCovx => "cove_i_or_covx",
        OptionalMetadataKind::CoveL => "cove_l",
        OptionalMetadataKind::CoveE => "cove_e",
        OptionalMetadataKind::CoveR => "cove_r",
        OptionalMetadataKind::CoveCache => "cove_cache",
        OptionalMetadataKind::CoveCx => "cove_cx",
    }
}

fn optional_metadata_status_name(status: OptionalMetadataStatus) -> &'static str {
    match status {
        OptionalMetadataStatus::Trusted => "trusted",
        OptionalMetadataStatus::Ignored => "ignored",
        OptionalMetadataStatus::Disabled => "disabled",
        OptionalMetadataStatus::NotRequested => "not_requested",
    }
}

fn fallback_kind_name(kind: FallbackKind) -> &'static str {
    match kind {
        FallbackKind::MaterializedArrowBuffers => "materialized_arrow_buffers",
        FallbackKind::MetadataOnlyDenied => "metadata_only_denied",
        FallbackKind::OptionalMetadataIgnored => "optional_metadata_ignored",
    }
}

fn rejection_kind_name(kind: RejectionKind) -> &'static str {
    match kind {
        RejectionKind::FeatureValidation => "feature_validation",
        RejectionKind::ResourceBudget => "resource_budget",
        RejectionKind::SecurityPolicy => "security_policy",
        RejectionKind::UnsupportedDatasetScope => "unsupported_dataset_scope",
    }
}

pub(crate) fn diagnostic_severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Error => "error",
    }
}

fn validation_stage_name(stage: ValidationStage) -> &'static str {
    match stage {
        ValidationStage::Bootstrap => "bootstrap",
        ValidationStage::Structural => "structural",
        ValidationStage::SharedSemantic => "shared_semantic",
        ValidationStage::DigestVerification => "digest_verification",
        ValidationStage::CoveTable => "cove_table",
        ValidationStage::CoveObject => "cove_object",
        ValidationStage::CoveEngine => "cove_engine",
        ValidationStage::CoveHarbor => "cove_harbor",
        ValidationStage::CoveMap => "cove_map",
        _ => "unknown",
    }
}

fn validation_stage_status_name(status: ValidationStageStatus) -> &'static str {
    match status {
        ValidationStageStatus::Checked => "checked",
        ValidationStageStatus::Skipped => "skipped",
        _ => "unknown",
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

fn dataset_membership_fingerprint(files: &[DatasetFileIdentity]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.ordinal.to_le_bytes());
        hasher.update(file.source.as_bytes());
        hasher.update([0]);
        hasher.update(file.file_id);
        hasher.update(file.file_len.to_le_bytes());
        hasher.update(file.footer_crc32c.to_le_bytes());
        hasher.update([file.primary_profile]);
    }
    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

#[cfg(feature = "datafusion")]
fn dataset_scope_context_from_state(
    dataset: &cove_datafusion::dataset_state::DatasetState,
) -> DatasetScopeContext {
    let files = dataset
        .files()
        .iter()
        .enumerate()
        .map(|(ordinal, file)| DatasetFileIdentity {
            ordinal,
            source: file.source().into(),
            file_id: file.identity().file_id,
            file_len: file.identity().file_len,
            footer_crc32c: file.identity().footer_crc32c,
            primary_profile: file.mounted().header.primary_profile,
        })
        .collect::<Vec<_>>();
    let file_membership_fingerprint = dataset_membership_fingerprint(&files);
    let dataset_id = Some(format!("dataset:{}", hex_lower(dataset.file_id())));
    let snapshot_id = Some(format!(
        "dataset:{}:{}",
        file_membership_fingerprint,
        dataset.footer_crc32c()
    ));
    let multi_file = files.len() > 1;
    let dictionary_epochs = dataset
        .files()
        .iter()
        .filter(|file| file.mounted().header.required_features & FEATURE_FILE_DICTIONARY != 0)
        .map(|file| DictionaryEpochContext {
            source: file.source().into(),
            domain_id: format!("file:{}:dictionary", hex_lower(&file.identity().file_id)),
            epoch: None,
            exact: false,
            reason: if multi_file {
                "file-local dictionary participates in multi-file scope without a manifest-level canonical epoch proof".into()
            } else {
                "single-file dictionary stays within the file-local code domain".into()
            },
        })
        .collect::<Vec<_>>();
    DatasetScopeContext {
        scope_version: 1,
        dataset_id,
        manifest_id: if multi_file {
            Some("dataset_state:implicit_manifest".into())
        } else {
            None
        },
        snapshot_id,
        file_membership_fingerprint,
        object_schema_fingerprint: None,
        semantic_map_fingerprint: None,
        projection_catalog_fingerprint: None,
        files,
        cross_file_ordering: if multi_file {
            CrossFileOrderingPolicy::CanonicalDatasetOrder
        } else {
            CrossFileOrderingPolicy::SingleFile
        },
        object_identity: if multi_file {
            CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid
        } else {
            CrossFileObjectIdentityPolicy::SingleFileGoid
        },
        association_identity: if multi_file {
            CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints
        } else {
            CrossFileAssociationIdentityPolicy::SingleFileEndpoints
        },
        dictionary_epochs,
        security_scope: DatasetSecurityScopeContext::default(),
        code_domain_bridges: code_domain_bridges_from_state(dataset, multi_file),
        execution_code_domains: Vec::new(),
    }
}

#[cfg(feature = "datafusion")]
fn code_domain_bridges_from_state(
    dataset: &cove_datafusion::dataset_state::DatasetState,
    multi_file: bool,
) -> Vec<CodeDomainBridgeContext> {
    let mut bridge_keys = BTreeSet::new();
    let mut bridges = Vec::new();
    for file in dataset.files() {
        for domain in &file.mounted().column_domains {
            let header = &domain.header;
            let domain_id = format!(
                "object_or_table:{}:property_or_column:{}:logical_type:{}:collation:{}",
                header.table_or_object_id,
                header.column_or_property_id,
                header.logical_type,
                header.collation_id
            );
            if !bridge_keys.insert(domain_id.clone()) {
                continue;
            }
            bridges.push(CodeDomainBridgeContext {
                domain_id,
                bridge_kind: if multi_file {
                    "manifest_candidate_requires_canonical_remap".into()
                } else {
                    "single_file_domain".into()
                },
                epoch: None,
                security_scope_id: None,
                exact: !multi_file,
                reason: if multi_file {
                    "multi-file CoveQL scope exposes matching domain metadata, but raw FileCode equality is not trusted without an explicit canonical remap under the query snapshot".into()
                } else {
                    "single-file scope keeps FileCode comparisons within one file-local domain".into()
                },
            });
        }
    }
    bridges
}

#[cfg(feature = "datafusion")]
fn selected_operation_name(operation: &CoveQlSelectedOperation) -> &'static str {
    match operation {
        CoveQlSelectedOperation::Object => "object",
        CoveQlSelectedOperation::Association => "association",
        CoveQlSelectedOperation::GraphNode => "graph_node",
        CoveQlSelectedOperation::GraphEdge => "graph_edge",
        CoveQlSelectedOperation::Table => "table",
        CoveQlSelectedOperation::Projection => "projection",
        CoveQlSelectedOperation::Evidence => "evidence",
        CoveQlSelectedOperation::IndexOnlyAnswer => "index_only_answer",
        CoveQlSelectedOperation::ArrowExport { .. } => "arrow_export",
        CoveQlSelectedOperation::Explain { .. } => "explain",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(selected_operation: CoveQlSelectedOperation) -> CoveQlOperationRequest {
        CoveQlOperationRequest {
            selected_operation,
            ..CoveQlOperationRequest::default()
        }
    }

    #[test]
    fn object_maps_to_object_reconstruction() {
        let uses = selected_feature_uses(&request(CoveQlSelectedOperation::Object));
        assert_eq!(uses.len(), 2);
        assert!(uses.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::ObjectReconstruction)
        }));
        assert!(uses.iter().any(|feature_use| {
            feature_use.requested_profile == Some(PrimaryProfile::ObjectTemporal as u8)
        }));
    }

    #[test]
    fn projection_uses_map_and_evidence_uses_object_and_map_metadata() {
        let projection = selected_feature_uses(&request(CoveQlSelectedOperation::Projection));
        assert!(projection.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::ProjectionReadback)
        }));

        let evidence = selected_feature_uses(&request(CoveQlSelectedOperation::Evidence));
        assert!(evidence.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::ObjectReconstruction)
        }));
        assert!(evidence.iter().any(|feature_use| {
            feature_use.requested_profile == Some(PrimaryProfile::ObjectTemporal as u8)
        }));
        assert!(evidence.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::EvidenceReadback)
        }));
        assert!(evidence.iter().any(|feature_use| {
            feature_use.requested_profile == Some(PrimaryProfile::SemanticMapping as u8)
        }));
    }

    #[test]
    fn evidence_helper_request_adds_map_metadata() {
        let mut req = request(CoveQlSelectedOperation::Object);
        req.evidence_metadata_requested = true;
        let uses = selected_feature_uses(&req);
        assert!(uses.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::ObjectReconstruction)
        }));
        assert!(uses.iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::EvidenceReadback)
        }));
    }

    #[test]
    fn arrow_zero_copy_adds_layout_feature_use() {
        let uses = selected_feature_uses(&request(CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: true,
        }));
        assert_eq!(uses.len(), 4);
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::ObjectReconstruction)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::ZeroCopyExport)));
    }

    #[test]
    fn explain_adds_target_and_coverage_feature_use() {
        let uses = selected_feature_uses(&request(CoveQlSelectedOperation::Explain {
            target: CoveQlExplainTarget::Projection,
            mode: ExplainMode::Proof,
        }));
        assert_eq!(uses.len(), 5);
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::ProjectionReadback)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::MappingExplanation)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::CoveragePlanning)));
    }

    #[test]
    fn object_explain_does_not_request_map_metadata_without_map_target_or_helper() {
        let uses = selected_feature_uses(&request(CoveQlSelectedOperation::Explain {
            target: CoveQlExplainTarget::Object,
            mode: ExplainMode::Proof,
        }));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::ObjectReconstruction)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::CoveragePlanning)));
        assert!(!uses.iter().any(|feature_use| {
            matches!(
                feature_use.requested_operation,
                Some(
                    OperationKindV2::MappingExplanation
                        | OperationKindV2::ProjectionReadback
                        | OperationKindV2::EvidenceReadback
                )
            )
        }));
    }

    #[test]
    fn evidence_explain_requests_evidence_readback_and_mapping_explanation() {
        let uses = selected_feature_uses(&request(CoveQlSelectedOperation::Explain {
            target: CoveQlExplainTarget::Evidence,
            mode: ExplainMode::Coded,
        }));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::EvidenceReadback)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::MappingExplanation)));
        assert!(uses
            .iter()
            .any(|feature_use| feature_use.requested_operation
                == Some(OperationKindV2::CoveragePlanning)));
    }

    #[test]
    fn execution_code_mapping_is_explicit_only() {
        let mut req = request(CoveQlSelectedOperation::Object);
        assert!(!selected_feature_uses(&req).iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::EngineExecutionMapping)
        }));
        req.execution_code_mapping_requested = true;
        assert!(selected_feature_uses(&req).iter().any(|feature_use| {
            feature_use.requested_operation == Some(OperationKindV2::EngineExecutionMapping)
        }));
    }

    #[test]
    fn denied_zero_copy_falls_back_when_allowed() {
        let mut req = request(CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: true,
        });
        let mut diagnostics = Vec::new();
        let mut fallbacks = Vec::new();
        enforce_security_gates(&req, &mut diagnostics, &mut fallbacks).unwrap();
        assert_eq!(diagnostics[0].code, "E_ZERO_COPY_FORBIDDEN");
        assert_eq!(fallbacks[0].kind, FallbackKind::MaterializedArrowBuffers);

        req.fallback_policy = FallbackPolicy::RejectOnFallback;
        let err = enforce_security_gates(&req, &mut Vec::new(), &mut Vec::new()).unwrap_err();
        assert_eq!(err.diagnostics[0].code, "E_ZERO_COPY_FORBIDDEN");
    }

    #[test]
    fn denied_index_only_rejects() {
        let req = request(CoveQlSelectedOperation::IndexOnlyAnswer);
        let err = enforce_security_gates(&req, &mut Vec::new(), &mut Vec::new()).unwrap_err();
        assert_eq!(err.diagnostics[0].code, "E_INDEX_ONLY_FORBIDDEN");
    }

    #[test]
    fn proof_explain_redacts_without_metadata_permission() {
        let req = request(CoveQlSelectedOperation::Explain {
            target: CoveQlExplainTarget::Object,
            mode: ExplainMode::Proof,
        });
        let mut diagnostics = Vec::new();
        enforce_security_gates(&req, &mut diagnostics, &mut Vec::new()).unwrap();
        assert_eq!(diagnostics[0].code, "E_SECURITY_DISCLOSURE_FORBIDDEN");
        assert!(diagnostics[0].redacted);
    }

    #[test]
    fn resource_budget_checks_all_phase_zero_hints() {
        let cases = [
            (
                "maximum_query_bytes",
                ResourceUseEstimate {
                    query_bytes: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_ast_depth",
                ResourceUseEstimate {
                    ast_depth: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_method_count",
                ResourceUseEstimate {
                    method_count: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_in_list_size",
                ResourceUseEstimate {
                    in_list_size: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_disjunction_count",
                ResourceUseEstimate {
                    disjunction_count: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_output_columns",
                ResourceUseEstimate {
                    output_columns: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_groups",
                ResourceUseEstimate {
                    groups: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_rows_without_explicit_take",
                ResourceUseEstimate {
                    rows_without_explicit_take: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_decode_bytes",
                ResourceUseEstimate {
                    decode_bytes: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_range_requests",
                ResourceUseEstimate {
                    range_requests: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_graph_traversal_depth",
                ResourceUseEstimate {
                    graph_traversal_depth: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_graph_traversal_fanout",
                ResourceUseEstimate {
                    graph_traversal_fanout: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_graph_traversal_paths",
                ResourceUseEstimate {
                    graph_traversal_paths: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_graph_traversal_frontier",
                ResourceUseEstimate {
                    graph_traversal_frontier: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_planning_time_ms",
                ResourceUseEstimate {
                    planning_time_ms: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
            (
                "maximum_execution_time_ms",
                ResourceUseEstimate {
                    execution_time_ms: Some(2),
                    ..ResourceUseEstimate::default()
                },
            ),
        ];

        for (field, usage) in cases {
            let mut req = CoveQlOperationRequest::default();
            req.resource_budget = ResourceBudgetPolicy {
                maximum_query_bytes: 1,
                maximum_ast_depth: 1,
                maximum_method_count: 1,
                maximum_in_list_size: 1,
                maximum_disjunction_count: 1,
                maximum_output_columns: 1,
                maximum_groups: 1,
                maximum_rows_without_explicit_take: 1,
                maximum_decode_bytes: 1,
                maximum_range_requests: 1,
                maximum_graph_traversal_depth: 1,
                maximum_graph_traversal_fanout: 1,
                maximum_graph_traversal_paths: 1,
                maximum_graph_traversal_frontier: 1,
                maximum_planning_time_ms: 1,
                maximum_execution_time_ms: 1,
            };
            req.resource_use = usage;
            let err = check_resource_budget(&req).unwrap_err();
            assert!(err.diagnostics[0].message.contains(field));
        }
    }
}
