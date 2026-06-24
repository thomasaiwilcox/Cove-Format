use cove_core::reader::ValidationOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{error::Error, fmt};

use crate::{
    kernel_plan::native_temporal_direct_projection_shape,
    logical_plan::{LogicalPlanNodeKind, LogicalRootKind},
    physical_predicate::{
        build_predicate_normal_forms, PhysicalCodeDomainDescriptor,
        PhysicalExecutionCodeDomainDescriptor, PhysicalPredicateForm, PhysicalPredicateNormalForms,
        PhysicalRepresentationClass,
    },
    physical_printer,
    physical_proofs::{
        validate_sidecars, IndexCapabilityReport, LayoutRangePlan, PhysicalFallbackReport,
        PhysicalMetadataReports, ProofValidationReport, SidecarPlanningFlags,
        ZeroCopyEligibilityReport,
    },
    physical_sidecars::{PhysicalSidecarInputs, PhysicalSidecarStatus, PhysicalSidecarValidation},
    AssociationDirectionPlan, AssociationOptimizationReport, AstAggregateName, AstChangeMode,
    AstHistoryMode, BuildLogicalPlanError, CoveQlOutputMode, DiagnosticSeverity, EvidenceGrainKind,
    EvidenceOptimizationReport, EvidenceTargetIndexKind, MetadataDisclosurePolicy, ParseOptions,
    PlanOptions, PlannedQuery, ResolveOptions,
};

pub type PhysicalPlanFingerprint = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPlanOptions {
    pub enable_coverage_candidates: bool,
    pub enable_covi_candidates: bool,
    pub enable_covx_candidates: bool,
    pub enable_layout_candidates: bool,
    pub enable_cache_candidates: bool,
    pub enable_execution_code_candidates: bool,
    pub enable_index_only_candidates: bool,
    pub allow_index_only_answers: bool,
    pub enable_zero_copy_candidates: bool,
    pub allow_zero_copy_output: bool,
    pub allow_file_code_literal_candidates: bool,
    pub optional_metadata_fail_open: bool,
    pub sidecars: PhysicalSidecarInputs,
}

impl Default for PhysicalPlanOptions {
    fn default() -> Self {
        Self {
            enable_coverage_candidates: true,
            enable_covi_candidates: true,
            enable_covx_candidates: true,
            enable_layout_candidates: true,
            enable_cache_candidates: true,
            enable_execution_code_candidates: true,
            enable_index_only_candidates: true,
            allow_index_only_answers: false,
            enable_zero_copy_candidates: true,
            allow_zero_copy_output: false,
            allow_file_code_literal_candidates: false,
            optional_metadata_fail_open: true,
            sidecars: PhysicalSidecarInputs::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhysicalPlannedQuery {
    pub planned: PlannedQuery,
    pub physical_plan: CoveOPhysicalPlan,
    pub diagnostics: Vec<PhysicalPlanDiagnostic>,
    pub sidecars: PhysicalSidecarInputs,
    pub allow_index_only_answers: bool,
    pub allow_zero_copy_output: bool,
    pub allow_file_code_literal_candidates: bool,
    pub proof_validation_report: ProofValidationReport,
    pub index_capability_report: IndexCapabilityReport,
    pub layout_range_plan: LayoutRangePlan,
    pub runtime_compatibility_report: Vec<PhysicalSidecarValidation>,
    pub cache_compatibility_report: Vec<PhysicalSidecarValidation>,
    pub codec_compatibility_report: Vec<PhysicalSidecarValidation>,
    pub zero_copy_eligibility: ZeroCopyEligibilityReport,
    pub sidecar_validations: Vec<PhysicalSidecarValidation>,
    pub physical_plan_fingerprint: PhysicalPlanFingerprint,
}

impl PhysicalPlannedQuery {
    pub fn explain_json(&self) -> Value {
        crate::explain::physical_planned_query_explain_json(self)
    }

    pub fn explain_text(&self) -> String {
        crate::render_explain_text(&self.explain_json())
    }

    pub fn physical_plan_text(&self) -> String {
        self.physical_plan
            .to_text(self.planned.resolved.diagnostic_policy.metadata_disclosure)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOPhysicalPlan {
    pub root_kind: LogicalRootKind,
    pub nodes: Vec<PhysicalPlanNode>,
    pub predicate_normal_forms: PhysicalPredicateNormalForms,
    pub proof_validation_report: ProofValidationReport,
    pub index_capability_report: IndexCapabilityReport,
    pub layout_range_plan: LayoutRangePlan,
    pub runtime_compatibility: Vec<PhysicalSidecarValidation>,
    pub cache_compatibility: Vec<PhysicalSidecarValidation>,
    pub codec_compatibility: Vec<PhysicalSidecarValidation>,
    pub zero_copy_eligibility: ZeroCopyEligibilityReport,
    pub execution_code_domains: Vec<PhysicalExecutionCodeDomainDescriptor>,
    pub code_domains: Vec<PhysicalCodeDomainDescriptor>,
    pub fallbacks: Vec<PhysicalFallbackReport>,
    pub sidecar_validations: Vec<PhysicalSidecarValidation>,
    pub physical_plan_fingerprint: PhysicalPlanFingerprint,
}

impl CoveOPhysicalPlan {
    pub fn to_json(&self, disclosure: MetadataDisclosurePolicy) -> Value {
        physical_printer::physical_plan_json(self, disclosure)
    }

    pub fn to_text(&self, disclosure: MetadataDisclosurePolicy) -> String {
        physical_printer::physical_plan_text(self, disclosure)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PhysicalNodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPlanNode {
    pub id: PhysicalNodeId,
    pub kind: PhysicalPlanNodeKind,
    pub contract: PhysicalOperatorContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalOperatorContract {
    pub contract_version: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    pub cardinality: String,
    pub ordering: String,
    pub protected_metadata: Vec<String>,
    pub pre_redaction_safe: bool,
    pub index_only_eligible: bool,
    pub fallback: String,
    pub explain_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PhysicalPlanNodeKind {
    ValidateFeatureScopes {
        selected_feature_use_count: usize,
        optional_metadata_candidates: Vec<String>,
    },
    BuildPredicateNormalForms {
        ast_forms: usize,
        cnf_forms: usize,
        interval_forms: usize,
        encoded_forms: usize,
        coverage_forms: usize,
        residual_forms: usize,
    },
    ReadObjectCatalog {
        root_kind: LogicalRootKind,
        object_type_count: usize,
        property_count: usize,
    },
    SelectTemporalSegments {
        temporal_mode: String,
        branch_policy: String,
        tombstone_policy: String,
    },
    TemporalBloomProbe {
        enabled: bool,
        outcome: String,
    },
    ValidateCoverageProofs {
        accepted_proofs: usize,
        ignored_proofs: usize,
    },
    CoveragePrune {
        candidate_count: usize,
        proof_safe: bool,
    },
    ValidateCoviOrCovx {
        lookup_candidates: usize,
        index_only_candidates: usize,
        exact_candidates: usize,
    },
    CoviLookup {
        candidate_count: usize,
        require_exact: bool,
    },
    PlanLayoutRanges {
        range_count: usize,
        page_cluster_count: usize,
    },
    RangeReadCoalesce {
        coalesced_range_count: usize,
    },
    ReadSystemColumns {
        columns: Vec<String>,
    },
    ReadPropertyColumns {
        property_count: usize,
        property_ids: Vec<u32>,
    },
    MorselBitmapEval {
        candidate_form_count: usize,
    },
    FileCodePredicate {
        forms: Vec<PhysicalPredicateForm>,
    },
    ExecutionCodePredicate {
        domains: Vec<PhysicalExecutionCodeDomainDescriptor>,
        forms: Vec<PhysicalPredicateForm>,
    },
    NumericPredicate {
        forms: Vec<PhysicalPredicateForm>,
    },
    DictionaryLiftedPredicate {
        forms: Vec<PhysicalPredicateForm>,
    },
    ReconstructObjectState {
        required: bool,
    },
    TemporalGrainReconstruction {
        mode: String,
        row_grain: String,
        native_exact: bool,
    },
    AssociationLinkScan {
        association_type_count: usize,
        endpoint_plans: Vec<AssociationDirectionPlan>,
    },
    AssociationSemiJoin {
        predicate_count: usize,
        anti_join_candidate: bool,
        endpoint_fast_path_candidates: usize,
        endpoint_fast_path_exact: bool,
        direction_plans: Vec<AssociationDirectionPlan>,
    },
    AssociationAntiJoin {
        predicate_count: usize,
        endpoint_fast_path_candidates: usize,
        endpoint_fast_path_exact: bool,
        direction_plans: Vec<AssociationDirectionPlan>,
    },
    AssociationAggregate {
        aggregate_count: usize,
        count_fast_path_candidates: usize,
        distinct_target_fast_path_candidates: usize,
        validity_interval_fast_path_candidates: usize,
        aggregate_fast_path_exact: bool,
    },
    EvidenceRead {
        evidence_field_count: usize,
        grains: Vec<EvidenceGrainKind>,
        target_index_kinds: Vec<EvidenceTargetIndexKind>,
        target_filtered: bool,
        index_candidate: bool,
        existence_fast_path_candidates: usize,
        existence_fast_path_exact: bool,
        count_fast_path_candidates: usize,
        count_fast_path_exact: bool,
        hidden_entry_filtering_required: bool,
    },
    ApplyVisibilityAndRedaction {
        visibility_policy: String,
        redaction_policy: String,
    },
    ZeroCopyArrowProjection {
        requested: bool,
        candidate: bool,
        compatible: bool,
    },
    ArrowProjection {
        owned_buffers: bool,
    },
    JsonProjection {
        canonical: bool,
    },
    MaterializedFilter {
        residual_count: usize,
    },
    MaterializedSort {
        default_ordering_applied: bool,
    },
    FallbackBoundary {
        fallbacks: Vec<PhysicalFallbackReport>,
    },
    OutputBoundary {
        output_mode: CoveQlOutputMode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPlanDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub phase: String,
    pub safe_details: Value,
    pub redacted: bool,
}

#[derive(Debug, Clone)]
pub struct BuildPhysicalPlanError {
    pub diagnostics: Vec<PhysicalPlanDiagnostic>,
    pub source: Option<String>,
}

impl BuildPhysicalPlanError {
    fn from_logical(error: BuildLogicalPlanError) -> Self {
        Self {
            diagnostics: error
                .diagnostics
                .into_iter()
                .map(|diagnostic| PhysicalPlanDiagnostic {
                    code: diagnostic.code,
                    severity: diagnostic.severity,
                    message: diagnostic.message,
                    phase: diagnostic.phase,
                    safe_details: diagnostic.safe_details,
                    redacted: diagnostic.redacted,
                })
                .collect(),
            source: error.source,
        }
    }

    fn single(diagnostic: PhysicalPlanDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            source: None,
        }
    }

    pub fn explain_json(&self) -> Value {
        crate::explain::error_explain_json(
            "physical_plan",
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

impl fmt::Display for BuildPhysicalPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "CoveQL physical plan build failed")
        }
    }
}

impl Error for BuildPhysicalPlanError {}

pub fn parse_resolve_plan_and_build_physical_plan(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    physical_options: PhysicalPlanOptions,
    validation_options: ValidationOptions,
) -> Result<PhysicalPlannedQuery, BuildPhysicalPlanError> {
    let planned = crate::logical_plan::parse_resolve_and_plan_query(
        bytes,
        text,
        parse_options,
        resolve_options,
        plan_options,
        validation_options.clone(),
    )
    .map_err(BuildPhysicalPlanError::from_logical)?;
    build_physical_plan(bytes, planned, physical_options, validation_options)
}

pub fn build_physical_plan(
    bytes: &[u8],
    planned: PlannedQuery,
    options: PhysicalPlanOptions,
    validation_options: ValidationOptions,
) -> Result<PhysicalPlannedQuery, BuildPhysicalPlanError> {
    let normal_forms = build_predicate_normal_forms(
        &planned,
        options.enable_coverage_candidates,
        options.enable_execution_code_candidates,
    );
    let reports = validate_sidecars(
        bytes,
        &planned,
        &options.sidecars,
        SidecarPlanningFlags {
            enable_coverage: options.enable_coverage_candidates,
            enable_covi: options.enable_covi_candidates,
            enable_covx: options.enable_covx_candidates,
            enable_layout: options.enable_layout_candidates,
            enable_cache: options.enable_cache_candidates,
            enable_execution_code: options.enable_execution_code_candidates,
            enable_zero_copy_candidates: options.enable_zero_copy_candidates,
            allow_file_code_literal_candidates: options.allow_file_code_literal_candidates,
        },
        &validation_options,
    );
    if !options.optional_metadata_fail_open
        && reports
            .sidecar_validations
            .iter()
            .any(|validation| validation.status == PhysicalSidecarStatus::Ignored)
    {
        return Err(BuildPhysicalPlanError::single(physical_error(
            "E_PHYSICAL_METADATA_VALIDATION",
            "optional physical metadata failed validation and fail-open is disabled",
            &planned,
        )));
    }

    let fallbacks = collect_fallbacks(&reports);
    let diagnostics = physical_diagnostics(&planned, &normal_forms, &reports);
    let mut nodes = PhysicalNodeBuilder::default();
    build_nodes(&planned, &normal_forms, &reports, &fallbacks, &mut nodes);

    let mut physical_plan = CoveOPhysicalPlan {
        root_kind: planned.logical_plan.context.root_kind,
        nodes: nodes.nodes,
        predicate_normal_forms: normal_forms.clone(),
        proof_validation_report: reports.proofs.clone(),
        index_capability_report: reports.index.clone(),
        layout_range_plan: reports.layout.clone(),
        runtime_compatibility: reports.runtime.clone(),
        cache_compatibility: reports.cache.clone(),
        codec_compatibility: reports.codec.clone(),
        zero_copy_eligibility: reports.zero_copy.clone(),
        execution_code_domains: reports.execution_code_domains.clone(),
        code_domains: normal_forms.code_domains.clone(),
        fallbacks,
        sidecar_validations: reports.sidecar_validations.clone(),
        physical_plan_fingerprint: String::new(),
    };
    let physical_plan_fingerprint = fingerprint(&physical_plan, &reports.sidecar_validations);
    physical_plan.physical_plan_fingerprint = physical_plan_fingerprint.clone();

    Ok(PhysicalPlannedQuery {
        planned,
        physical_plan,
        diagnostics,
        sidecars: options.sidecars,
        allow_index_only_answers: options.allow_index_only_answers,
        allow_zero_copy_output: options.allow_zero_copy_output,
        allow_file_code_literal_candidates: options.allow_file_code_literal_candidates,
        proof_validation_report: reports.proofs,
        index_capability_report: reports.index,
        layout_range_plan: reports.layout,
        runtime_compatibility_report: reports.runtime,
        cache_compatibility_report: reports.cache,
        codec_compatibility_report: reports.codec,
        zero_copy_eligibility: reports.zero_copy,
        sidecar_validations: reports.sidecar_validations,
        physical_plan_fingerprint,
    })
}

#[derive(Default)]
struct PhysicalNodeBuilder {
    nodes: Vec<PhysicalPlanNode>,
}

impl PhysicalNodeBuilder {
    fn push(&mut self, kind: PhysicalPlanNodeKind, contract: PhysicalOperatorContract) {
        let id = PhysicalNodeId(self.nodes.len() as u32);
        self.nodes.push(PhysicalPlanNode { id, kind, contract });
    }
}

fn build_nodes(
    planned: &PlannedQuery,
    forms: &PhysicalPredicateNormalForms,
    reports: &PhysicalMetadataReports,
    fallbacks: &[PhysicalFallbackReport],
    nodes: &mut PhysicalNodeBuilder,
) {
    let association_report = AssociationOptimizationReport::for_plan(planned, &[]);
    let evidence_report = EvidenceOptimizationReport::for_plan(planned, None);
    nodes.push(
        PhysicalPlanNodeKind::ValidateFeatureScopes {
            selected_feature_use_count: planned
                .resolved
                .operation_context
                .selected_feature_uses
                .len(),
            optional_metadata_candidates: planned
                .resolved
                .operation_context
                .optional_metadata
                .iter()
                .map(|metadata| format!("{:?}:{:?}", metadata.kind, metadata.status))
                .collect(),
        },
        contract(
            "validate feature scopes",
            &["operation_context"],
            &["validated_feature_scope"],
            "reject on required feature mismatch",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::BuildPredicateNormalForms {
            ast_forms: forms.ast_forms.len(),
            cnf_forms: forms.cnf_forms.len(),
            interval_forms: forms.interval_forms.len(),
            encoded_forms: forms.encoded_forms.len(),
            coverage_forms: forms.coverage_forms.len(),
            residual_forms: forms.residual_forms.len(),
        },
        contract(
            "build predicate normal forms",
            &["resolved_predicates"],
            &["ast", "cnf", "interval", "encoded", "coverage", "residual"],
            "unsupported forms remain residual",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ReadObjectCatalog {
            root_kind: planned.logical_plan.context.root_kind,
            object_type_count: planned.dependencies.object_type_ids.len(),
            property_count: planned.dependencies.property_ids.len(),
        },
        contract(
            "read object catalog",
            &["validated_file"],
            &["object_catalog"],
            "materialized readback remains available",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::SelectTemporalSegments {
            temporal_mode: format!("{:?}", planned.resolved.temporal.mode),
            branch_policy: format!("{:?}", planned.resolved.branch.selector),
            tombstone_policy: format!("{:?}", planned.resolved.tombstone),
        },
        contract(
            "select temporal segments",
            &["object_catalog", "temporal_context"],
            &["segment_candidates"],
            "scan wider if temporal proof is unavailable",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::TemporalBloomProbe {
            enabled: true,
            outcome: "candidate_only".into(),
        },
        contract(
            "temporal bloom probe",
            &["segment_candidates"],
            &["segment_candidates"],
            "ignore absent or unsupported bloom metadata",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ValidateCoverageProofs {
            accepted_proofs: reports.proofs.accepted_proof_count,
            ignored_proofs: reports.proofs.ignored_proof_count,
        },
        contract(
            "validate coverage proofs",
            &["coverage_sidecars", "predicate_forms"],
            &["coverage_candidates"],
            "ignore any proof without no-false-negative semantics",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::CoveragePrune {
            candidate_count: reports.proofs.accepted_proof_count,
            proof_safe: reports.proofs.accepted_proof_count > 0,
        },
        contract(
            "coverage prune",
            &["coverage_candidates"],
            &["wider_or_equal_candidates"],
            "fallback to full scan candidate set",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ValidateCoviOrCovx {
            lookup_candidates: reports.index.lookup_candidates,
            index_only_candidates: reports.index.index_only_candidates,
            exact_candidates: reports.index.exact_candidates,
        },
        contract(
            "validate COVE-I/COVX",
            &["index_sidecars", "predicate_forms"],
            &["index_candidates"],
            "ignore stale or inexact index metadata",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::CoviLookup {
            candidate_count: reports.index.lookup_candidates,
            require_exact: true,
        },
        contract(
            "plan COVI lookup",
            &["index_candidates"],
            &["row_or_range_candidates"],
            "residual materialized predicates remain final truth",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::PlanLayoutRanges {
            range_count: reports.layout.range_count,
            page_cluster_count: reports.layout.page_cluster_count,
        },
        contract(
            "plan layout ranges",
            &["layout_sidecars", "row_or_range_candidates"],
            &["range_plan"],
            "use ordinary read ranges when layout metadata is absent",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::RangeReadCoalesce {
            coalesced_range_count: reports.layout.coalesced_range_count,
        },
        contract(
            "coalesce range reads",
            &["range_plan"],
            &["coalesced_ranges"],
            "uncoalesced ranges remain valid",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ReadSystemColumns {
            columns: planned
                .dependencies
                .system_fields
                .iter()
                .map(|field| format!("{field:?}"))
                .collect(),
        },
        contract(
            "read system columns",
            &["coalesced_ranges"],
            &["system_column_values"],
            "materialized readback can request all system columns",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ReadPropertyColumns {
            property_count: planned.dependencies.property_ids.len(),
            property_ids: planned.dependencies.property_ids.iter().copied().collect(),
        },
        contract(
            "read property columns",
            &["coalesced_ranges"],
            &["property_column_values"],
            "materialized readback can decode required properties",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::MorselBitmapEval {
            candidate_form_count: forms.encoded_forms.len()
                + forms.interval_forms.len()
                + forms.coverage_forms.len(),
        },
        contract(
            "evaluate morsel bitmaps",
            &["encoded_predicates", "property_column_values"],
            &["candidate_bitmaps"],
            "exact predicate proofs emit authoritative bitmaps; candidate forms require residual verification",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::FileCodePredicate {
            forms: forms_by_representation(forms, PhysicalRepresentationClass::FileCodeLiteral),
        },
        contract(
            "file-code predicate",
            &["file_codes", "code_domains"],
            &["candidate_bitmaps"],
            "same-domain equality and IN proofs are authoritative; otherwise decode or residualize",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ExecutionCodePredicate {
            domains: reports.execution_code_domains.clone(),
            forms: forms_by_representation(
                forms,
                PhysicalRepresentationClass::ExecutionCodeRemapped,
            ),
        },
        contract(
            "execution-code predicate",
            &["execution_code_domains"],
            &["candidate_bitmaps"],
            "validated complete COVE-E remaps are authoritative; stale or incomplete maps fall back",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::NumericPredicate {
            forms: forms_by_representation(forms, PhysicalRepresentationClass::NumericCoded),
        },
        contract(
            "numeric predicate",
            &["typed_numeric_lanes"],
            &["candidate_bitmaps"],
            "typed NumCode lanes are authoritative for compatible equality, range, and IN predicates",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::DictionaryLiftedPredicate {
            forms: forms_by_representation(forms, PhysicalRepresentationClass::DictionaryLifted),
        },
        contract(
            "dictionary-lifted predicate",
            &["dictionary_domain"],
            &["candidate_bitmaps"],
            "deterministic dictionary-lift contracts are authoritative; unsupported collation or redaction residualizes",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ReconstructObjectState {
            required: reconstruction_required(planned),
        },
        contract(
            "reconstruct object state",
            &["candidate_record_chains"],
            &["materialized_object_states"],
            "retain complete chains for candidate objects",
        ),
    );
    if let Some((mode, row_grain, native_exact)) = temporal_grain_reconstruction(planned) {
        nodes.push(
            PhysicalPlanNodeKind::TemporalGrainReconstruction {
                mode,
                row_grain,
                native_exact,
            },
            contract(
                "temporal grain reconstruction",
                &["candidate_record_chains"],
                &["history_or_change_rows"],
                if native_exact {
                    "history/change direct projection uses exact temporal row-grain reconstruction before final output materialization"
                } else {
                    "history/change modes use materialized temporal reconstruction as the semantic authority unless an exact temporal row-grain proof is available"
                },
            ),
        );
    }
    if planned.dependencies.association_type_ids.len() > 0
        || matches!(
            planned.logical_plan.context.root_kind,
            LogicalRootKind::Association
        )
    {
        nodes.push(
            PhysicalPlanNodeKind::AssociationLinkScan {
                association_type_count: planned.dependencies.association_type_ids.len(),
                endpoint_plans: association_report.endpoint_plans.clone(),
            },
            contract(
                "association link scan",
                &["materialized_object_states"],
                &["association_links"],
                "materialize association links when coded proof is absent",
            ),
        );
    }
    if planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::AssociationSemiJoin { .. }))
    {
        let endpoint_fast_path_exact = association_endpoint_fast_path_exact(
            planned,
            &association_report,
            association_report.semi_join_candidates,
            false,
        );
        nodes.push(
            PhysicalPlanNodeKind::AssociationSemiJoin {
                predicate_count: planned.dependencies.association_type_ids.len(),
                anti_join_candidate: association_report.anti_join_candidates > 0,
                endpoint_fast_path_candidates: association_report.semi_join_candidates,
                endpoint_fast_path_exact,
                direction_plans: association_report.endpoint_plans.clone(),
            },
            contract(
                "association semi-join",
                &["association_links"],
                &["candidate_object_keys"],
                if endpoint_fast_path_exact {
                    "association endpoint edge-table semi-join is an exact prefilter candidate before final visibility/redaction checks"
                } else {
                    "semi-join is materialized unless safe link proof is available"
                },
            ),
        );
    }
    let anti_join_predicate_count = planned
        .logical_plan
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            LogicalPlanNodeKind::AssociationAntiJoin { predicates } => Some(predicates.len()),
            _ => None,
        })
        .sum::<usize>();
    if anti_join_predicate_count > 0 {
        let endpoint_fast_path_exact = association_endpoint_fast_path_exact(
            planned,
            &association_report,
            anti_join_predicate_count,
            true,
        );
        nodes.push(
            PhysicalPlanNodeKind::AssociationAntiJoin {
                predicate_count: anti_join_predicate_count,
                endpoint_fast_path_candidates: association_report.anti_join_candidates,
                endpoint_fast_path_exact,
                direction_plans: association_report.endpoint_plans.clone(),
            },
            contract(
                "association anti-join",
                &["association_links"],
                &["candidate_object_keys"],
                if endpoint_fast_path_exact {
                    "association endpoint edge-table anti-join is an exact absence prefilter candidate under protected disclosure policy"
                } else {
                    "anti-join is materialized unless safe link absence proof and disclosure policy are available"
                },
            ),
        );
    }
    let association_aggregate_candidates = association_report.count_fast_path_candidates
        + association_report.distinct_target_fast_path_candidates;
    if (aggregate_count(planned) > 0
        && matches!(
            planned.logical_plan.context.root_kind,
            LogicalRootKind::Association
        ))
        || association_aggregate_candidates > 0
    {
        let aggregate_fast_path_exact = association_aggregate_fast_path_exact(
            planned,
            &association_report,
            association_aggregate_candidates,
        );
        nodes.push(
            PhysicalPlanNodeKind::AssociationAggregate {
                aggregate_count: aggregate_count(planned),
                count_fast_path_candidates: association_report.count_fast_path_candidates,
                distinct_target_fast_path_candidates: association_report
                    .distinct_target_fast_path_candidates,
                validity_interval_fast_path_candidates: association_report
                    .validity_interval_fast_path_candidates,
                aggregate_fast_path_exact,
            },
            contract(
                "association aggregate",
                &["association_links"],
                &["aggregate_states"],
                if aggregate_fast_path_exact {
                    "association helper aggregates have exact endpoint edge-table fast-path authority under protected metadata and exact aggregate disclosure"
                } else {
                    "materialized aggregate remains final truth unless endpoint, visibility, and disclosure proofs are complete"
                },
            ),
        );
    }
    if planned.dependencies.evidence_fields.len() > 0
        || matches!(
            planned.logical_plan.context.root_kind,
            LogicalRootKind::Evidence
        )
    {
        nodes.push(
            PhysicalPlanNodeKind::EvidenceRead {
                evidence_field_count: planned.dependencies.evidence_fields.len(),
                grains: evidence_report
                    .index_reports
                    .iter()
                    .map(|report| report.grain)
                    .collect(),
                target_index_kinds: evidence_report.target_index_kinds.clone(),
                target_filtered: evidence_report.filtered_by_target,
                index_candidate: evidence_report.enabled,
                existence_fast_path_candidates: evidence_report.existence_fast_path_candidates,
                existence_fast_path_exact: evidence_report.existence_fast_path_exact,
                count_fast_path_candidates: evidence_report.count_fast_path_candidates,
                count_fast_path_exact: evidence_report.count_fast_path_exact,
                hidden_entry_filtering_required: evidence_report.hidden_entry_filtering_applied,
            },
            contract(
                "evidence read",
                &["materialized_object_states"],
                &["evidence_rows"],
                "evidence reads happen after visibility and redaction unless proven safe",
            ),
        );
    }
    nodes.push(
        PhysicalPlanNodeKind::ApplyVisibilityAndRedaction {
            visibility_policy: format!(
                "{:?}",
                planned
                    .resolved
                    .operation_context
                    .security
                    .visibility_policy
            ),
            redaction_policy: format!(
                "{:?}",
                planned.resolved.operation_context.security.redaction_policy
            ),
        },
        contract(
            "apply visibility and redaction",
            &["materialized_rows"],
            &["visible_rows"],
            "reject or materialize if metadata-only disclosure is unsafe",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::ZeroCopyArrowProjection {
            requested: reports.zero_copy.requested,
            candidate: reports.zero_copy.candidate,
            compatible: reports.zero_copy.compatible,
        },
        contract(
            "zero-copy Arrow projection",
            &["visible_rows", "zero_copy_map"],
            &["arrow_projection_candidate"],
            "validated COVE-L compatibility can retain buffers; otherwise owned Arrow fallback or rejection applies",
        ),
    );
    if matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. } | CoveQlOutputMode::DataFusionTableProvider
    ) {
        nodes.push(
            PhysicalPlanNodeKind::ArrowProjection {
                owned_buffers: true,
            },
            contract(
                "Arrow projection",
                &["visible_rows"],
                &["owned_arrow_batches"],
                "materialize Arrow buffers",
            ),
        );
    }
    nodes.push(
        PhysicalPlanNodeKind::JsonProjection { canonical: true },
        contract(
            "JSON projection",
            &["visible_rows"],
            &["canonical_json_rows"],
            "JSON remains golden-test output",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::MaterializedFilter {
            residual_count: forms.residual_forms.len(),
        },
        contract(
            "materialized filter",
            &["visible_rows"],
            &["filtered_rows"],
            "all where predicates are rechecked after materialization",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::MaterializedSort {
            default_ordering_applied: planned.logical_plan.default_ordering_applied,
        },
        contract(
            "materialized sort",
            &["filtered_rows"],
            &["sorted_rows"],
            "raw FileCode integer order is never logical order",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::FallbackBoundary {
            fallbacks: fallbacks.to_vec(),
        },
        contract(
            "fallback boundary",
            &["physical_candidates"],
            &["materialized_baseline"],
            "fallback to materialized execution preserves visible rows",
        ),
    );
    nodes.push(
        PhysicalPlanNodeKind::OutputBoundary {
            output_mode: planned.resolved.output_mode.clone(),
        },
        contract(
            "output boundary",
            &["sorted_rows"],
            &["requested_output"],
            "output authority is reported as materialized, exact kernel, exact index-only, or zero-copy",
        ),
    );
}

fn contract(
    name: &str,
    inputs: &[&str],
    outputs: &[&str],
    fallback: &str,
) -> PhysicalOperatorContract {
    PhysicalOperatorContract {
        contract_version: crate::PHYSICAL_OPERATOR_CONTRACT_VERSION.into(),
        inputs: inputs.iter().map(|item| (*item).into()).collect(),
        outputs: outputs.iter().map(|item| (*item).into()).collect(),
        preconditions: vec![format!("{name} preconditions must be validated before use")],
        postconditions: vec![format!(
            "{name} must preserve logical truth under its validated authority contract"
        )],
        cardinality:
            "exact-authoritative when proofs pass; otherwise no false negatives with residual checks"
                .into(),
        ordering: "does not establish logical ordering unless materialized sort follows".into(),
        protected_metadata: vec![
            "paths".into(),
            "literals".into(),
            "sidecar identifiers".into(),
        ],
        pre_redaction_safe: false,
        index_only_eligible: false,
        fallback: fallback.into(),
        explain_fields: vec![
            "operator".into(),
            "candidate_count".into(),
            "fallback".into(),
            "redacted_metadata".into(),
        ],
    }
}

fn forms_by_representation(
    forms: &PhysicalPredicateNormalForms,
    representation: PhysicalRepresentationClass,
) -> Vec<PhysicalPredicateForm> {
    forms
        .encoded_forms
        .iter()
        .chain(forms.interval_forms.iter())
        .chain(forms.residual_forms.iter())
        .filter(|form| form.representation == representation)
        .cloned()
        .collect()
}

fn reconstruction_required(planned: &PlannedQuery) -> bool {
    matches!(
        planned.logical_plan.context.root_kind,
        LogicalRootKind::Object | LogicalRootKind::Association | LogicalRootKind::Evidence
    )
}

fn association_endpoint_fast_path_exact(
    planned: &PlannedQuery,
    report: &AssociationOptimizationReport,
    candidate_count: usize,
    absence_proof: bool,
) -> bool {
    if candidate_count == 0
        || report.endpoint_plans.is_empty()
        || !report.fallback_reasons.is_empty()
    {
        return false;
    }
    if absence_proof
        && planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
    {
        return false;
    }
    report
        .endpoint_plans
        .iter()
        .all(|plan| plan.endpoint_flags_complete)
}

fn temporal_grain_reconstruction(planned: &PlannedQuery) -> Option<(String, String, bool)> {
    let native_exact = native_temporal_direct_projection_shape(planned);
    if let Some(history) = planned.resolved.method_chain.history {
        return Some((
            format!("history_{}", physical_history_mode_name(history)),
            physical_history_row_grain(history).into(),
            native_exact,
        ));
    }
    planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .map(|changes| {
            (
                format!("changes_{}", physical_change_mode_name(changes.mode)),
                physical_change_row_grain(changes.mode).into(),
                native_exact,
            )
        })
}

fn physical_history_mode_name(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "records",
        AstHistoryMode::States => "states",
        AstHistoryMode::RecordsAndStates => "records_and_states",
    }
}

fn physical_history_row_grain(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "history_record",
        AstHistoryMode::States => "history_state",
        AstHistoryMode::RecordsAndStates => "history_records_and_states",
    }
}

fn physical_change_mode_name(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "records",
        AstChangeMode::StateTransitions => "state_transitions",
        AstChangeMode::PropertyDiffs => "property_diffs",
        AstChangeMode::FinalRows => "final_rows",
    }
}

fn physical_change_row_grain(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "change_record",
        AstChangeMode::StateTransitions => "change_state_transition",
        AstChangeMode::PropertyDiffs => "change_property_diff",
        AstChangeMode::FinalRows => "change_final_row",
    }
}

fn association_aggregate_fast_path_exact(
    planned: &PlannedQuery,
    report: &AssociationOptimizationReport,
    candidate_count: usize,
) -> bool {
    candidate_count > 0
        && report.fallback_reasons.is_empty()
        && planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected
        && planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            == crate::AggregateDisclosurePolicy::AllowExact
        && report
            .endpoint_plans
            .iter()
            .all(|plan| plan.endpoint_flags_complete)
}

fn aggregate_count(planned: &PlannedQuery) -> usize {
    planned
        .dependencies
        .aggregate_kinds
        .iter()
        .filter(|kind| {
            matches!(
                kind.as_str(),
                "count" | "min" | "max" | "sum" | "avg" | "exists" | "distinct_count"
            )
        })
        .count()
        + planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .map(|select| {
                select
                    .iter()
                    .filter(|item| matches_aggregate(&item.expr))
                    .count()
            })
            .unwrap_or_default()
}

fn matches_aggregate(expr: &crate::ResolvedExpr) -> bool {
    matches!(
        expr,
        crate::ResolvedExpr::AggregateCall {
            name: AstAggregateName::Count
                | AstAggregateName::Min
                | AstAggregateName::Max
                | AstAggregateName::Sum
                | AstAggregateName::Avg
                | AstAggregateName::Exists
                | AstAggregateName::DistinctCount,
            ..
        }
    )
}

fn collect_fallbacks(reports: &PhysicalMetadataReports) -> Vec<PhysicalFallbackReport> {
    let mut out = Vec::new();
    out.extend(reports.proofs.fallbacks.clone());
    out.extend(reports.index.fallbacks.clone());
    out.extend(reports.layout.fallbacks.clone());
    out.extend(reports.zero_copy.fallbacks.clone());
    for validation in &reports.sidecar_validations {
        if let Some(reason) = &validation.fallback_reason {
            out.push(PhysicalFallbackReport::new(
                validation.name.clone(),
                reason.clone(),
            ));
        }
    }
    out.sort_by(|left, right| (&left.source, &left.reason).cmp(&(&right.source, &right.reason)));
    out.dedup_by(|left, right| left.source == right.source && left.reason == right.reason);
    out
}

fn physical_diagnostics(
    planned: &PlannedQuery,
    forms: &PhysicalPredicateNormalForms,
    reports: &PhysicalMetadataReports,
) -> Vec<PhysicalPlanDiagnostic> {
    let mut diagnostics = planned
        .diagnostics
        .iter()
        .map(|diagnostic| PhysicalPlanDiagnostic {
            code: diagnostic.code.clone(),
            severity: diagnostic.severity,
            message: diagnostic.message.clone(),
            phase: diagnostic.phase.clone(),
            safe_details: diagnostic.safe_details.clone(),
            redacted: diagnostic.redacted,
        })
        .collect::<Vec<_>>();
    if !forms.residual_forms.is_empty() {
        diagnostics.push(physical_warning(
            "W_PHYSICAL_RESIDUAL_PREDICATES",
            "one or more predicates remain materialized residual checks",
            json!({ "residual_count": forms.residual_forms.len() }),
            planned,
        ));
    }
    if reports.index.index_only_candidates > 0 {
        diagnostics.push(physical_warning(
            "W_INDEX_ONLY_CANDIDATE_REQUIRES_EXECUTION_PROOF",
            "index-only metadata is available only when execution can validate exactness, visibility, and disclosure policy",
            json!({ "index_only_candidates": reports.index.index_only_candidates }),
            planned,
        ));
    }
    diagnostics.push(physical_warning(
        "W_CODED_KERNELS_NOT_EXECUTED",
        "physical planning built coded candidates; runtime execution may apply proof-gated branches with materialized execution as the semantic authority",
        json!({}),
        planned,
    ));
    diagnostics
}

fn physical_error(
    code: impl Into<String>,
    message: impl Into<String>,
    planned: &PlannedQuery,
) -> PhysicalPlanDiagnostic {
    PhysicalPlanDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: "physical_plan".into(),
        safe_details: json!({}),
        redacted: planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected,
    }
}

fn physical_warning(
    code: impl Into<String>,
    message: impl Into<String>,
    safe_details: Value,
    planned: &PlannedQuery,
) -> PhysicalPlanDiagnostic {
    PhysicalPlanDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Warning,
        message: message.into(),
        phase: "physical_plan".into(),
        safe_details,
        redacted: planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected,
    }
}

fn fingerprint(
    plan: &CoveOPhysicalPlan,
    sidecar_validations: &[PhysicalSidecarValidation],
) -> String {
    let value = json!({
        "root_kind": plan.root_kind,
        "nodes": plan.nodes,
        "predicate_normal_forms": plan.predicate_normal_forms,
        "proof_validation_report": plan.proof_validation_report,
        "index_capability_report": plan.index_capability_report,
        "layout_range_plan": plan.layout_range_plan,
        "runtime_compatibility": plan.runtime_compatibility,
        "cache_compatibility": plan.cache_compatibility,
        "codec_compatibility": plan.codec_compatibility,
        "zero_copy_eligibility": plan.zero_copy_eligibility,
        "execution_code_domains": plan.execution_code_domains,
        "code_domains": plan.code_domains,
        "fallbacks": plan.fallbacks,
        "sidecar_validations": sidecar_validations,
    });
    sha256_hex(
        serde_json::to_string(&value)
            .expect("physical plan fingerprint JSON serializes")
            .as_bytes(),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}
