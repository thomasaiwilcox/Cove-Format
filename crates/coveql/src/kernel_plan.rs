use cove_core::collation::CollationKind;
use serde::{Deserialize, Serialize};

use crate::{
    kernel_metrics::KernelFallbackReason, kernel_predicate::compile_kernel_predicates,
    AggregateDisclosurePolicy, AstAggregateName, AstChangeMode, AstHistoryMode, CoveQlOutputMode,
    MetadataDisclosurePolicy, PhysicalPlannedQuery, ResolvedExpr, ResolvedPath, ResolvedPredicate,
    ResolvedRoot, TemporalMode, TemporalRole,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelShape {
    pub root_kind: KernelRootKind,
    pub root_object_type_id: u32,
    pub root_type_name: String,
    pub output_mode: CoveQlOutputMode,
    pub direct_projection: bool,
    pub predicate_count: usize,
    pub operator_contracts: Vec<CodedOperatorContract>,
    pub decode_boundaries: Vec<String>,
    pub bridge_decisions: Vec<String>,
    pub residual_verification_required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodedOperatorContract {
    pub contract_version: String,
    pub operator: String,
    pub representation_class: CodedRepresentationClass,
    pub exact: bool,
    pub residual_required: bool,
    pub reason: String,
    pub row_grain: String,
    pub proof_obligation: String,
    pub required_metadata: Vec<String>,
    pub residual_reason: Option<String>,
    pub fallback_boundary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodedOperatorName(String);

impl CodedOperatorName {
    fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

impl From<&str> for CodedOperatorName {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for CodedOperatorName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodedContractText(String);

impl CodedContractText {
    fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

impl From<&str> for CodedContractText {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for CodedContractText {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&&str> for CodedContractText {
    fn from(value: &&str) -> Self {
        Self((*value).to_string())
    }
}

impl CodedOperatorContract {
    fn new(
        operator: impl Into<CodedOperatorName>,
        representation_class: CodedRepresentationClass,
        exact: bool,
        residual_required: bool,
        reason: impl Into<CodedContractText>,
    ) -> Self {
        let operator = operator.into();
        let reason = reason.into();
        Self {
            contract_version: crate::CODED_OPERATOR_CONTRACT_VERSION.into(),
            operator: operator.clone().into_string(),
            representation_class,
            exact,
            residual_required,
            row_grain: "visible_rows_after_reconstruction".into(),
            proof_obligation: if exact {
                format!(
                    "{} has an explicit CoveQL-equivalence proof for this shape",
                    operator.as_str()
                )
            } else {
                format!(
                    "{} must prove row-grain, null, type, collation, and security semantics before becoming authoritative",
                    operator.as_str()
                )
            },
            required_metadata: Vec::new(),
            residual_reason: residual_required.then(|| reason.as_str().to_string()),
            fallback_boundary: residual_required
                .then(|| "materialized_residual_verification".into()),
            reason: reason.into_string(),
        }
    }

    fn with_row_grain(mut self, row_grain: impl Into<CodedContractText>) -> Self {
        self.row_grain = row_grain.into().into_string();
        self
    }

    fn with_proof_obligation(mut self, proof_obligation: impl Into<CodedContractText>) -> Self {
        self.proof_obligation = proof_obligation.into().into_string();
        self
    }

    fn with_required_metadata(mut self, required_metadata: &[&str]) -> Self {
        self.required_metadata = required_metadata
            .iter()
            .map(CodedContractText::from)
            .map(CodedContractText::into_string)
            .collect();
        self
    }

    fn with_fallback_boundary(mut self, fallback_boundary: impl Into<CodedContractText>) -> Self {
        self.fallback_boundary = Some(fallback_boundary.into().into_string());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodedRepresentationClass {
    CodePure,
    LocalShadowAccelerated,
    DictionaryLifted,
    OrdinalMapAssisted,
    TypedNumericCoded,
    CrossSourceCodeBridge,
    DecodeBoundary,
    MaterializedResidual,
    NonBeneficialCodedPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelRootKind {
    Object,
    Association,
    Table,
    Evidence,
    Projection,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledKernelShape {
    pub public: KernelShape,
    pub predicates: Vec<crate::kernel_predicate::KernelPredicate>,
}

pub(crate) fn compile_kernel_shape(
    physical: &PhysicalPlannedQuery,
) -> Result<CompiledKernelShape, KernelFallbackReason> {
    let planned = &physical.planned;
    let (root_kind, root_object_type_id, root_type_name) = match &planned.resolved.root {
        ResolvedRoot::Object(root) => (
            KernelRootKind::Object,
            root.object_type_id,
            root.type_name.clone(),
        ),
        ResolvedRoot::Association(root) => (
            KernelRootKind::Association,
            root.object_type_id,
            root.type_name.clone(),
        ),
        ResolvedRoot::Node(root) => (
            KernelRootKind::Object,
            root.object.object_type_id,
            format!("node:{}", root.label),
        ),
        ResolvedRoot::Edge(root) => (
            KernelRootKind::Association,
            root.association.object_type_id,
            format!("edge:{}", root.label),
        ),
        ResolvedRoot::Evidence(_) => (KernelRootKind::Evidence, 0, "evidence".into()),
        ResolvedRoot::Table(root) => (
            KernelRootKind::Table,
            0,
            format!("{}:{}", root.table_name, root.projection.projection_id),
        ),
        ResolvedRoot::Projection(root) => {
            (KernelRootKind::Projection, 0, root.projection_id.clone())
        }
    };
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ObjectRows
            | CoveQlOutputMode::AssociationRows
            | CoveQlOutputMode::EvidenceRows
            | CoveQlOutputMode::ProjectionRows
            | CoveQlOutputMode::JsonRows
            | CoveQlOutputMode::ArrowRecordBatch { .. }
    ) {
        return Err(KernelFallbackReason::UnsupportedOutputMode);
    }
    if root_kind == KernelRootKind::Object
        && matches!(
            planned.resolved.output_mode,
            CoveQlOutputMode::AssociationRows
        )
    {
        return Err(KernelFallbackReason::UnsupportedOutputMode);
    }
    if root_kind == KernelRootKind::Association
        && matches!(planned.resolved.output_mode, CoveQlOutputMode::ObjectRows)
    {
        return Err(KernelFallbackReason::UnsupportedOutputMode);
    }
    if root_kind == KernelRootKind::Evidence
        && matches!(
            planned.resolved.output_mode,
            CoveQlOutputMode::ObjectRows | CoveQlOutputMode::AssociationRows
        )
    {
        return Err(KernelFallbackReason::UnsupportedOutputMode);
    }
    let native_temporal_direct_projection = native_temporal_direct_projection_shape(planned);
    if !planned.resolved.temporal.mode.is_point_in_time() && !native_temporal_direct_projection {
        return Err(KernelFallbackReason::UnsupportedTemporalMode);
    }
    if planned.resolved.temporal.role_binding.is_some()
        && !native_role_bound_direct_projection_shape(planned)
    {
        return Err(KernelFallbackReason::UnsupportedTemporalMode);
    }
    if !planned.resolved.method_chain.lookups.is_empty()
        || !planned.resolved.method_chain.traversals.is_empty()
        || planned
            .resolved
            .method_chain
            .where_predicate
            .as_ref()
            .is_some_and(predicate_contains_table_exists)
        || planned_contains_target_node_association(planned)
    {
        return Err(KernelFallbackReason::UnsupportedProjection);
    }
    if predicate_contains_unsafe_coded(
        planned,
        planned.resolved.method_chain.where_predicate.as_ref(),
    ) {
        return Err(KernelFallbackReason::UnsafeCodedPredicate);
    }

    let predicates =
        match compile_kernel_predicates(planned.resolved.method_chain.where_predicate.as_ref()) {
            Ok(predicates) => predicates,
            Err(_)
                if planned
                    .resolved
                    .method_chain
                    .where_predicate
                    .as_ref()
                    .is_some_and(|predicate| {
                        row_root_predicate_is_direct_safe(planned, predicate)
                    }) =>
            {
                Vec::new()
            }
            Err(_)
                if predicate_contains_association_or_evidence(
                    planned.resolved.method_chain.where_predicate.as_ref(),
                ) =>
            {
                Vec::new()
            }
            Err(_) => return Err(KernelFallbackReason::UnsupportedPredicate),
        };
    let operator_contracts = coded_operator_contracts(planned);
    let residual_verification_required =
        kernel_shape_residual_verification_required(planned, &operator_contracts);
    Ok(CompiledKernelShape {
        public: KernelShape {
            root_kind,
            root_object_type_id,
            root_type_name,
            output_mode: planned.resolved.output_mode.clone(),
            direct_projection: planned
                .resolved
                .method_chain
                .select
                .as_ref()
                .map_or(true, |select| {
                    select
                        .iter()
                        .all(|item| native_projection_expr_is_exact(&item.expr))
                }),
            predicate_count: predicates.len(),
            operator_contracts,
            decode_boundaries: coded_decode_boundaries(planned),
            bridge_decisions: coded_bridge_decisions(planned),
            residual_verification_required,
            reason: match root_kind {
                KernelRootKind::Object => {
                    if residual_verification_required {
                        "eligible object-root latest/as-of shape with residual verification"
                    } else {
                        "eligible object-root latest/as-of shape with exact native kernel authority"
                    }
                }
                KernelRootKind::Association => {
                    if residual_verification_required {
                        "eligible association-root scan shape with residual verification"
                    } else {
                        "eligible association-root direct scan shape with exact native kernel authority"
                    }
                }
                KernelRootKind::Evidence => {
                    if residual_verification_required {
                        "eligible evidence-root lookup shape with residual verification"
                    } else {
                        "eligible evidence-root direct scan shape with exact native kernel authority"
                    }
                }
                KernelRootKind::Table => {
                    if residual_verification_required {
                        "table root uses a deterministic projection readback boundary with residual verification"
                    } else {
                        "eligible projection-backed table scan with exact COVE-MAP provider authority"
                    }
                }
                KernelRootKind::Projection => {
                    if residual_verification_required {
                        "projection root uses an explicit COVE-MAP materialized readback boundary inside the kernel wrapper"
                    } else {
                        "eligible projection-root direct scan shape with exact COVE-MAP provider authority"
                    }
                }
            }
            .into(),
        },
        predicates,
    })
}

pub(crate) fn diagnostic_kernel_shape_for_plan(
    planned: &crate::PlannedQuery,
    fallback_reason: KernelFallbackReason,
) -> KernelShape {
    let (root_kind, root_object_type_id, root_type_name) = match &planned.resolved.root {
        ResolvedRoot::Object(root) => (
            KernelRootKind::Object,
            root.object_type_id,
            root.type_name.clone(),
        ),
        ResolvedRoot::Association(root) => (
            KernelRootKind::Association,
            root.object_type_id,
            root.type_name.clone(),
        ),
        ResolvedRoot::Node(root) => (
            KernelRootKind::Object,
            root.object.object_type_id,
            format!("node:{}", root.label),
        ),
        ResolvedRoot::Edge(root) => (
            KernelRootKind::Association,
            root.association.object_type_id,
            format!("edge:{}", root.label),
        ),
        ResolvedRoot::Evidence(_) => (KernelRootKind::Evidence, 0, "evidence".into()),
        ResolvedRoot::Table(root) => (
            KernelRootKind::Table,
            0,
            format!("{}:{}", root.table_name, root.projection.projection_id),
        ),
        ResolvedRoot::Projection(root) => {
            (KernelRootKind::Projection, 0, root.projection_id.clone())
        }
    };
    let mut decode_boundaries = coded_decode_boundaries(planned);
    decode_boundaries.push(format!(
        "kernel_fallback: {fallback_reason:?}; materialized execution remains the semantic authority"
    ));
    KernelShape {
        root_kind,
        root_object_type_id,
        root_type_name,
        output_mode: planned.resolved.output_mode.clone(),
        direct_projection: planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .map_or(true, |select| {
                select
                    .iter()
                    .all(|item| native_projection_expr_is_exact(&item.expr))
            }),
        predicate_count: resolved_predicate_count(
            planned.resolved.method_chain.where_predicate.as_ref(),
        ),
        operator_contracts: coded_operator_contracts(planned),
        decode_boundaries,
        bridge_decisions: coded_bridge_decisions(planned),
        residual_verification_required: true,
        reason: format!(
            "ineligible coded kernel shape ({fallback_reason:?}); materialized fallback is explicit"
        ),
    }
}

fn coded_operator_contracts(planned: &crate::PlannedQuery) -> Vec<CodedOperatorContract> {
    let mut contracts = Vec::new();
    let native_bool_group_count = native_bool_group_count_shape(planned);
    let native_grouped_helper_aggregate = native_grouped_helper_aggregate_shape(planned);
    let native_typed_order = native_typed_order_shape(planned);
    let native_helper_aggregate = native_helper_aggregate_shape(planned);
    let native_direct_aggregate = native_direct_aggregate_shape(planned);
    let native_direct_projection = native_direct_projection_shape(planned);
    let native_temporal_direct_projection = native_temporal_direct_projection_shape(planned);
    let native_role_bound_direct_projection = native_role_bound_direct_projection_shape(planned);
    let native_projection_root_scan = native_projection_root_scan_shape(planned);
    let native_direct_projection_order = native_direct_projection_order_is_exact(planned);
    let native_association_root_scan = native_association_root_scan_shape(planned);
    let native_evidence_root_scan = native_evidence_root_scan_shape(planned);
    let multi_file = planned.resolved.operation_context.dataset.files.len() > 1;
    let exact_cross_file_bridge =
        dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset);
    let root_scan_exact = match planned.resolved.root {
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) => native_projection_root_scan,
        ResolvedRoot::Association(_) => {
            (!multi_file || exact_cross_file_bridge)
                && (native_association_root_scan
                    || native_direct_projection
                    || native_temporal_direct_projection)
        }
        ResolvedRoot::Evidence(_) => {
            (!multi_file || exact_cross_file_bridge)
                && (native_evidence_root_scan || native_direct_projection)
        }
        _ => !multi_file || exact_cross_file_bridge,
    };
    contracts.push(
        CodedOperatorContract::new(
            "root_scan",
            if matches!(
                planned.resolved.root,
                ResolvedRoot::Table(_) | ResolvedRoot::Projection(_)
            )
                && native_projection_root_scan
            {
                CodedRepresentationClass::DecodeBoundary
            } else if multi_file {
                CodedRepresentationClass::CrossSourceCodeBridge
            } else {
                CodedRepresentationClass::CodePure
            },
            root_scan_exact,
            !root_scan_exact,
            if !native_association_root_scan
                && !native_evidence_root_scan
                && matches!(
                    planned.resolved.root,
                    ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
                )
            {
                "association/evidence root scans are exact only for direct point-in-time row-output scans or direct selected projections; other root forms keep materialized semantics"
            } else if matches!(
                planned.resolved.root,
                ResolvedRoot::Table(_) | ResolvedRoot::Projection(_)
            )
                && !native_projection_root_scan
            {
                "projection-backed root scans are exact only when COVE-MAP can satisfy direct selected columns and primitive pushed filters without residual CoveQL operators"
            } else if native_projection_root_scan {
                "projection-backed root scan uses exact COVE-MAP projection batches with direct selected columns and primitive pushed filters; manifest scans merge logical provider rows without comparing raw local codes"
            } else if multi_file && exact_cross_file_bridge {
                "multi-file scope has an exact manifest code-domain bridge; raw code comparison remains scoped to that validated bridge"
            } else if multi_file {
                "multi-file scope requires an exact manifest bridge before raw code comparison"
            } else if native_temporal_direct_projection {
                "temporal history/changes direct projection uses exact CoveQL temporal row-grain reconstruction before final selected-output materialization"
            } else if native_association_root_scan {
                "association root scan returns reconstructed visible association states without predicate, aggregate, sort, or pagination residuals"
            } else if native_evidence_root_scan {
                "evidence root scan returns disclosure-filtered COVE-MAP evidence rows without predicate, aggregate, sort, or pagination residuals"
            } else {
                "single-file object identity and file-local codes remain within one domain"
            },
        )
        .with_row_grain("source_record_rows")
        .with_required_metadata(&["dataset_scope_context", "code_domain_bridge_context"])
        .with_proof_obligation(
            "root scans are code-pure only inside one validated file/domain scope; multi-file coded row scans require an exact bridge or materialized comparison, while projection-provider roots merge already-materialized logical projection rows",
        ),
    );
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        append_predicate_contracts(planned, predicate, &mut contracts);
    }
    append_temporal_grain_contract(planned, &mut contracts);
    if planned.resolved.temporal.role_binding.is_some() {
        if native_role_bound_direct_projection {
            contracts.push(
                CodedOperatorContract::new(
                    "temporal_role_bound_as_of",
                    CodedRepresentationClass::TypedNumericCoded,
                    true,
                    false,
                    "role-bound asOf uses the resolved timestamp binding to choose one visible state per object before direct projection",
                )
                .with_row_grain("reconstructed_visible_object_states")
                .with_required_metadata(&[
                    "temporal_role_binding",
                    "timestamp_value_lane",
                    "state_grain_contract",
                ])
                .with_proof_obligation(
                    "role-bound temporal reconstruction is exact because the binding is resolved to a timestamp property, records above the bound are excluded, and the selected record is reconstructed through the same state-grain routine as materialized execution",
                ),
            );
        } else {
            contracts.push(
                CodedOperatorContract::new(
                    "temporal_role_bound_as_of",
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "role-bound asOf needs a direct timestamp binding and simple direct projection shape before native execution is authoritative",
                )
                .with_row_grain("reconstructed_visible_object_states")
                .with_required_metadata(&[
                    "temporal_role_binding",
                    "timestamp_value_lane",
                    "state_grain_contract",
                ])
                .with_fallback_boundary("materialized_role_bound_temporal_reconstruction"),
            );
        }
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            if native_bool_group_count && matches!(item.expr, ResolvedExpr::AggregateCall { .. }) {
                let ResolvedExpr::AggregateCall { name, .. } = &item.expr else {
                    unreachable!("guarded by matches")
                };
                contracts.push(
                    CodedOperatorContract::new(
                        format!("aggregate:{}", aggregate_operator_name(*name)),
                        native_grouped_aggregate_representation_class(planned, &item.expr),
                        true,
                        false,
                        "native direct grouped aggregate kernel evaluates reconstructed visible rows without materialized aggregate residuals",
                    )
                    .with_row_grain("groups_over_reconstructed_visible_object_states")
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "aggregate_null_policy",
                        "aggregate_disclosure_policy",
                        "typed_value_lane",
                    ])
                    .with_proof_obligation(
                        "grouped direct aggregates are exact because they evaluate selected reconstructed rows in each direct value-domain group after CoveQL visibility and temporal reconstruction",
                    ),
                );
            } else if native_grouped_helper_aggregate {
                let ResolvedExpr::AggregateCall { name, arg, .. } = &item.expr else {
                    append_expr_contract(planned, "select", &item.expr, &mut contracts);
                    continue;
                };
                let helper_kind = match arg.as_deref() {
                    Some(ResolvedExpr::Association(_)) => "association",
                    Some(ResolvedExpr::Evidence(_)) => "evidence",
                    _ => "helper",
                };
                contracts.push(
                    CodedOperatorContract::new(
                        format!("aggregate:{}", aggregate_operator_name(*name)),
                        CodedRepresentationClass::OrdinalMapAssisted,
                        true,
                        false,
                        "native grouped helper aggregate kernel uses scoped association/evidence indexes inside each direct value-domain group",
                    )
                    .with_row_grain("groups_over_reconstructed_visible_object_states")
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "typed_value_lane",
                        "null_policy",
                        "association_endpoint_flags",
                        "cove_map_evidence_index",
                        "disclosure_policy",
                        "aggregate_disclosure_policy",
                    ])
                    .with_proof_obligation(format!(
                        "grouped {helper_kind} helper count/exists/distinct_count is exact because each helper lookup is scoped to reconstructed visible object states inside a proven direct group key, under protected metadata disclosure"
                    )),
                );
            } else if native_helper_aggregate {
                let ResolvedExpr::AggregateCall { name, arg, .. } = &item.expr else {
                    append_expr_contract(planned, "select", &item.expr, &mut contracts);
                    continue;
                };
                let helper_kind = match arg.as_deref() {
                    Some(ResolvedExpr::Association(_)) => "association",
                    Some(ResolvedExpr::Evidence(_)) => "evidence",
                    _ => "helper",
                };
                contracts.push(
                    CodedOperatorContract::new(
                        format!("aggregate:{}", aggregate_operator_name(*name)),
                        CodedRepresentationClass::OrdinalMapAssisted,
                        true,
                        false,
                        "native helper aggregate kernel uses scoped association/evidence indexes after visible object-state reconstruction",
                    )
                    .with_row_grain("reconstructed_visible_object_states")
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "association_endpoint_flags",
                        "cove_map_evidence_index",
                        "disclosure_policy",
                        "aggregate_disclosure_policy",
                    ])
                    .with_proof_obligation(format!(
                        "{helper_kind} helper count/exists/distinct_count is exact because it evaluates scoped edge/grain-index matches only after CoveQL visibility and temporal reconstruction, under protected metadata disclosure"
                    )),
                );
            } else if native_direct_aggregate {
                let ResolvedExpr::AggregateCall { name, .. } = &item.expr else {
                    append_expr_contract(planned, "select", &item.expr, &mut contracts);
                    continue;
                };
                contracts.push(
                    CodedOperatorContract::new(
                        format!("aggregate:{}", aggregate_operator_name(*name)),
                        native_direct_aggregate_representation_class(planned, *name),
                        true,
                        false,
                        "native direct aggregate kernel uses reconstructed visible rows plus direct path validity/value lanes without materialized aggregate residuals",
                    )
                    .with_row_grain("reconstructed_visible_object_states")
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "validity_lane",
                        "typed_value_lane",
                        "aggregate_disclosure_policy",
                    ])
                    .with_proof_obligation(
                        "direct count/exists/distinct_count/min/max/sum/avg is exact because it evaluates row counts, non-null direct-path values, logical distinct keys, typed min/max order, and exact numeric accumulators after CoveQL visibility and temporal reconstruction",
                    ),
                );
            } else if (native_direct_projection
                || native_role_bound_direct_projection
                || native_temporal_direct_projection)
                && native_projection_expr_is_exact(&item.expr)
            {
                if matches!(item.expr, ResolvedExpr::FunctionCall { .. }) {
                    append_expr_contract(planned, "select", &item.expr, &mut contracts);
                }
                contracts.push(
                    CodedOperatorContract::new(
                        "select",
                        native_direct_projection_expr_representation_class(
                            &item.expr,
                            &planned.resolved.operation_context.dataset,
                        ),
                        true,
                        false,
                        "native direct projection evaluates selected paths and coded-safe scalar expressions at the final projection boundary without materialized residual predicates",
                    )
                    .with_row_grain(if native_temporal_direct_projection {
                        temporal_direct_projection_row_grain(planned)
                    } else {
                        match planned.resolved.root {
                            ResolvedRoot::Association(_) => {
                                "reconstructed_visible_association_states"
                            }
                            ResolvedRoot::Evidence(_) => "disclosure_filtered_evidence_rows",
                            _ => "reconstructed_visible_object_states",
                        }
                    })
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "direct_value_lane",
                        "null_policy",
                        "final_materialization_boundary",
                    ])
                    .with_proof_obligation(
                        "direct projection is exact because CoveQL visibility, temporal selection, redaction policy, and row ordering have already been applied; variable-width values may decode only at the final output boundary and do not create a residual truth check",
                    ),
                );
            } else {
                append_expr_contract(planned, "select", &item.expr, &mut contracts);
            }
        }
    }
    if let Some(group_by) = &planned.resolved.method_chain.group_by {
        for expr in group_by {
            if (native_bool_group_count || native_grouped_helper_aggregate)
                && matches!(
                    expr,
                    ResolvedExpr::Path(path) if native_bool_group_count_path(planned).is_some_and(|group_path| paths_same_value_domain(path, group_path))
                )
            {
                contracts.push(
                    CodedOperatorContract::new(
                        "group_by",
                        native_bool_group_count_path(planned)
                            .map(|path| {
                                native_direct_group_count_representation_class(
                                    path,
                                    &planned.resolved.operation_context.dataset,
                                )
                            })
                            .unwrap_or(CodedRepresentationClass::CodePure),
                        true,
                        false,
                        "direct path grouping uses exact value-domain equality at reconstructed visible object-state grain",
                    )
                    .with_row_grain("groups_over_reconstructed_visible_object_states")
                    .with_required_metadata(&[
                        "state_grain_contract",
                        "typed_or_code_value_lane",
                        "semantic_domain",
                        "null_policy",
                    ])
                    .with_proof_obligation(
                            "GROUP BY is exact because the direct group key has stable CoveQL equality semantics; FileCode keys are grouped only inside a single validated file/domain scope or through an exact manifest code-domain bridge, never by comparing raw local codes across files",
                    ),
                );
            } else {
                append_expr_contract(planned, "group_by", expr, &mut contracts);
            }
        }
    }
    if let Some(order) = &planned.resolved.method_chain.order_by {
        if native_typed_order || native_direct_projection_order {
            let order_path = native_typed_order_path(planned);
            let representation_class = order_path
                .map(native_typed_order_representation_class)
                .unwrap_or(CodedRepresentationClass::TypedNumericCoded);
            let uses_file_code_sort_key =
                order_path.is_some_and(|path| path.physical_kind == "file_code");
            contracts.push(
                CodedOperatorContract::new(
                    "order_by_expr",
                    representation_class,
                    true,
                    false,
                    if uses_file_code_sort_key {
                        "direct FileCode order key materializes a decoded value sort key under the effective string collation and CoveQL null-order policy"
                    } else {
                        "direct typed order key uses a bool/numeric/date/time/string value lane and CoveQL null-order policy"
                    },
                )
                .with_required_metadata(if uses_file_code_sort_key {
                    &["effective_ordering_collation", "materialized_sort_key", "null_policy"]
                } else {
                    &["typed_value_lane", "null_policy"]
                }),
            );
            contracts.push(
                CodedOperatorContract::new(
                    "order_by",
                    representation_class,
                    true,
                    false,
                    if uses_file_code_sort_key {
                        "FileCode ORDER BY is exact only after decoded sort-key construction under a supported effective string collation; row identity provides deterministic tie-breaking"
                    } else {
                        "typed scalar ORDER BY has a total value order; row identity provides deterministic tie-breaking"
                    },
                )
                .with_required_metadata(if uses_file_code_sort_key {
                    &[
                        "effective_ordering_collation",
                        "materialized_sort_key",
                        "null_policy",
                        "stable_row_identity",
                    ]
                } else {
                    &["typed_value_lane", "null_policy", "stable_row_identity"]
                })
                .with_proof_obligation(if uses_file_code_sort_key {
                    "ORDER BY over FileCode strings is exact because raw dictionary code order is ignored, values are decoded into sort keys under the declared supported collation, null placement is explicit/defaulted by CoveQL, and ties use stable row identity"
                } else {
                    "ORDER BY over direct bool/numeric/date/time/string lanes is exact because the typed comparator defines value order, null placement is explicit/defaulted by CoveQL, and ties use stable row identity"
                }),
            );
        } else {
            append_expr_contract(planned, "order_by_expr", &order.expr, &mut contracts);
            contracts.push(
                CodedOperatorContract::new(
                    "order_by",
                    CodedRepresentationClass::DecodeBoundary,
                    false,
                    true,
                    "explicit ordering needs typed order-preserving values or collation sidecars; raw dictionary code order is not trusted",
                )
                .with_required_metadata(&["typed_order_lane", "collation_sidecar"])
                .with_proof_obligation(
                    "ORDER BY can become coded only with a valid typed order lane or collation sidecar for the requested null and collation policy",
                )
                .with_fallback_boundary("materialized_sort_key_construction"),
            );
        }
    }
    if planned.resolved.method_chain.take.is_some() || planned.resolved.method_chain.skip.is_some()
    {
        if native_direct_projection {
            let explicit_order = native_direct_projection_order_is_exact(planned);
            contracts.push(
                CodedOperatorContract::new(
                    "limit_offset",
                    CodedRepresentationClass::CodePure,
                    true,
                    false,
                    if explicit_order {
                        "skip/take is applied after the proven explicit ordering in the native direct projection path"
                    } else {
                        "skip/take is applied after the canonical default ordering in the native direct projection path"
                    },
                )
                .with_required_metadata(if explicit_order {
                    &["stable_order_contract"]
                } else {
                    &["stable_default_order_contract"]
                })
                .with_proof_obligation(
                    if explicit_order {
                        "limit/offset is exact because native direct projection applies proven CoveQL ordering before pagination and before final projection"
                    } else {
                        "limit/offset is exact because native direct projection applies CoveQL default ordering before pagination and before final projection"
                    },
                ),
            );
        } else if native_projection_root_scan {
            contracts.push(
                CodedOperatorContract::new(
                    "limit_offset",
                    CodedRepresentationClass::CodePure,
                    true,
                    false,
                    "skip/take is applied after declared projection ordering in the exact COVE-MAP projection provider path",
                )
                .with_required_metadata(&["declared_projection_ordering"])
                .with_proof_obligation(
                    "projection-root limit/offset is exact because COVE-MAP declares the ordering key, the key is retained in the provider output columns, and pagination runs after CoveQL default ordering before final projection",
                ),
            );
        } else if native_typed_order {
            contracts.push(
                CodedOperatorContract::new(
                    "limit_offset",
                    native_typed_order_path(planned)
                        .map(native_typed_order_representation_class)
                        .unwrap_or(CodedRepresentationClass::TypedNumericCoded),
                    true,
                    false,
                    "skip/take is applied after the proven native typed ordering contract",
                )
                .with_required_metadata(&["stable_order_contract"])
                .with_proof_obligation(
                    "limit/offset is exact because it runs after exact typed ordering and before final projection",
                ),
            );
        } else {
            contracts.push(
                CodedOperatorContract::new(
                    "limit_offset",
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "skip/take is applied after stable CoveQL ordering and residual filtering",
                )
                .with_required_metadata(&["stable_order_contract"])
                .with_proof_obligation(
                    "limit/offset is authoritative only after the CoveQL ordering contract and residual predicates are complete",
                )
                .with_fallback_boundary("materialized_ordered_pagination"),
            );
        }
    }
    contracts
}

fn append_temporal_grain_contract(
    planned: &crate::PlannedQuery,
    contracts: &mut Vec<CodedOperatorContract>,
) {
    let native_temporal_direct_projection = native_temporal_direct_projection_shape(planned);
    if let Some(history_mode) = planned.resolved.method_chain.history {
        let contract = CodedOperatorContract::new(
            "temporal_history",
            if native_temporal_direct_projection {
                CodedRepresentationClass::DecodeBoundary
            } else {
                CodedRepresentationClass::MaterializedResidual
            },
            native_temporal_direct_projection,
            !native_temporal_direct_projection,
            if native_temporal_direct_projection {
                format!(
                    "history(mode: {}) uses exact temporal row-grain reconstruction before native direct projection",
                    history_mode_name(history_mode)
                )
            } else {
                format!(
                    "history(mode: {}) uses materialized temporal row reconstruction as the authority without an exact temporal row-grain proof",
                    history_mode_name(history_mode)
                )
            },
        )
        .with_row_grain(history_row_grain(history_mode))
        .with_required_metadata(&[
            "temporal_record_order",
            "state_reconstruction_contract",
            "history_output_grain",
            "stable_default_order_contract",
        ])
        .with_proof_obligation(if native_temporal_direct_projection {
            "history direct projection is exact because it uses the same temporal record/state grain reconstruction, tombstone policy, branch policy, default ordering, and final projection boundary as materialized CoveQL execution"
        } else {
            "native history execution must prove record/state output grain, tombstone policy, branch policy, default ordering, and stable row identity before it can bypass materialized reconstruction"
        });
        contracts.push(if native_temporal_direct_projection {
            contract
        } else {
            contract.with_fallback_boundary("materialized_history_reconstruction")
        });
    }
    if let Some(changes) = &planned.resolved.method_chain.changes {
        let contract = CodedOperatorContract::new(
            "temporal_changes",
            if native_temporal_direct_projection {
                CodedRepresentationClass::DecodeBoundary
            } else {
                CodedRepresentationClass::MaterializedResidual
            },
            native_temporal_direct_projection,
            !native_temporal_direct_projection,
            if native_temporal_direct_projection {
                format!(
                    "changes(mode: {}) uses exact temporal change row-grain reconstruction before native direct projection",
                    change_mode_name(changes.mode)
                )
            } else {
                format!(
                    "changes(mode: {}) uses materialized temporal diff/reconstruction as the authority without an exact temporal row-grain proof",
                    change_mode_name(changes.mode)
                )
            },
        )
        .with_row_grain(change_row_grain(changes.mode))
        .with_required_metadata(&[
            "temporal_change_bounds",
            "state_transition_contract",
            "property_diff_contract",
            "stable_default_order_contract",
        ])
        .with_proof_obligation(if native_temporal_direct_projection {
            "changes direct projection is exact because it uses the same interval bounds, state transition, property diff, final-object reconstruction, branch policy, output ordering, and final projection boundary as materialized CoveQL execution"
        } else {
            "native changes execution must prove interval bounds, state transitions, property-level diffs, final-object reconstruction, branch policy, and output ordering before it can bypass materialized reconstruction"
        });
        contracts.push(if native_temporal_direct_projection {
            contract
        } else {
            contract.with_fallback_boundary("materialized_changes_reconstruction")
        });
    }
}

fn history_mode_name(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "records",
        AstHistoryMode::States => "states",
        AstHistoryMode::RecordsAndStates => "records_and_states",
    }
}

fn history_row_grain(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "history_record",
        AstHistoryMode::States => "history_state",
        AstHistoryMode::RecordsAndStates => "history_records_and_states",
    }
}

fn change_mode_name(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "records",
        AstChangeMode::StateTransitions => "state_transitions",
        AstChangeMode::PropertyDiffs => "property_diffs",
        AstChangeMode::FinalRows => "final_rows",
    }
}

fn change_row_grain(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "change_record",
        AstChangeMode::StateTransitions => "change_state_transition",
        AstChangeMode::PropertyDiffs => "change_property_diff",
        AstChangeMode::FinalRows => "change_final_row",
    }
}

fn temporal_direct_projection_row_grain(planned: &crate::PlannedQuery) -> &'static str {
    if let Some(history_mode) = planned.resolved.method_chain.history {
        return history_row_grain(history_mode);
    }
    planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .map(|changes| change_row_grain(changes.mode))
        .unwrap_or("reconstructed_visible_rows")
}

fn kernel_shape_residual_verification_required(
    planned: &crate::PlannedQuery,
    contracts: &[CodedOperatorContract],
) -> bool {
    if native_bool_group_count_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_grouped_helper_aggregate_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_direct_aggregate_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_helper_aggregate_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_typed_order_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_direct_projection_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_temporal_direct_projection_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_role_bound_direct_projection_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_helper_exists_direct_projection_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_projection_root_scan_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_association_root_scan_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    if native_evidence_root_scan_shape(planned) {
        return contracts.iter().any(|contract| contract.residual_required);
    }
    true
}

pub(crate) fn native_direct_projection_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::JsonRows | CoveQlOutputMode::ArrowRecordBatch { .. }
    ) || !native_direct_root_scan_common(planned, true, true, true)
    {
        return false;
    }
    if matches!(
        planned.resolved.root,
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_)
    ) {
        return false;
    }
    if matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
        && planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
    {
        return false;
    }
    true
}

pub(crate) fn native_temporal_direct_projection_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::JsonRows | CoveQlOutputMode::ArrowRecordBatch { .. }
    ) || !(planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some())
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
        || !dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset)
        || !matches!(
            planned.resolved.root,
            ResolvedRoot::Object(_) | ResolvedRoot::Association(_)
        )
    {
        return false;
    }
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| native_projection_expr_is_exact(&item.expr))
        })
}

pub(crate) fn native_role_bound_direct_projection_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::JsonRows | CoveQlOutputMode::ArrowRecordBatch { .. }
    ) || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || !matches!(
            planned.resolved.temporal.mode,
            TemporalMode::AsOfTimestampMicros(_)
        )
        || !matches!(
            planned.resolved.temporal.role,
            TemporalRole::ValidTime | TemporalRole::ObservedTime | TemporalRole::SourceEventTime
        )
        || planned.resolved.temporal.role_binding.is_none()
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
        || !dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset)
        || !matches!(planned.resolved.root, ResolvedRoot::Object(_))
    {
        return false;
    }
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| native_projection_expr_is_exact(&item.expr))
        })
}

pub(crate) fn native_helper_exists_direct_projection_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
        || !dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset)
        || !matches!(planned.resolved.root, ResolvedRoot::Object(_))
    {
        return false;
    }
    let Some(ResolvedPredicate::Exists(ResolvedExpr::Association(_))) =
        planned.resolved.method_chain.where_predicate.as_ref()
    else {
        return false;
    };
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| native_projection_expr_is_exact(&item.expr))
        })
}

pub(crate) fn native_evidence_exists_direct_projection_candidate_shape(
    planned: &crate::PlannedQuery,
) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
        || planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            != AggregateDisclosurePolicy::AllowExact
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
        || !dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset)
        || !matches!(planned.resolved.root, ResolvedRoot::Object(_))
    {
        return false;
    }
    let Some(ResolvedPredicate::Exists(ResolvedExpr::Evidence(_))) =
        planned.resolved.method_chain.where_predicate.as_ref()
    else {
        return false;
    };
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| native_projection_expr_is_exact(&item.expr))
        })
}

pub(crate) fn native_projection_root_scan_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_)
    ) || !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ProjectionRows
            | CoveQlOutputMode::JsonRows
            | CoveQlOutputMode::ArrowRecordBatch { .. }
    ) || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || ((planned.resolved.method_chain.take.is_some()
            || planned.resolved.method_chain.skip.is_some())
            && !projection_root_pagination_has_exact_declared_ordering(planned))
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || !planned.resolved.method_chain.lookups.is_empty()
        || !planned.resolved.method_chain.traversals.is_empty()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    if planned
        .resolved
        .method_chain
        .where_predicate
        .as_ref()
        .is_some_and(|predicate| !projection_root_predicate_is_exact(predicate))
    {
        return false;
    }
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| projection_root_expr_is_provider_exact(&item.expr))
        })
}

fn projection_root_pagination_has_exact_declared_ordering(planned: &crate::PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_)
    ) || planned.resolved.method_chain.order_by.is_some()
    {
        return false;
    }
    let Some(contract) = planned.dependencies.projection_contracts.first() else {
        return false;
    };
    if contract.ordering.is_empty() {
        return false;
    }
    contract.ordering.iter().all(|ordering| {
        let column = ordering
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or(ordering.as_str())
            .rsplit('.')
            .next()
            .unwrap_or(ordering.as_str());
        contract.pushed_columns.contains(column) || contract.selected_columns.contains(column)
    })
}

pub(crate) fn native_association_root_scan_shape(planned: &crate::PlannedQuery) -> bool {
    if !native_direct_root_scan_common(planned, false, false, false)
        || !matches!(
            planned.resolved.output_mode,
            CoveQlOutputMode::AssociationRows
        )
    {
        return false;
    }
    matches!(planned.resolved.root, ResolvedRoot::Association(_))
}

pub(crate) fn native_evidence_root_scan_shape(planned: &crate::PlannedQuery) -> bool {
    if !native_direct_root_scan_common(planned, false, false, false)
        || planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
        || !matches!(planned.resolved.output_mode, CoveQlOutputMode::EvidenceRows)
    {
        return false;
    }
    matches!(planned.resolved.root, ResolvedRoot::Evidence(_))
}

fn native_direct_root_scan_common(
    planned: &crate::PlannedQuery,
    allow_object_where: bool,
    allow_pagination: bool,
    allow_order: bool,
) -> bool {
    let where_predicate = planned.resolved.method_chain.where_predicate.as_ref();
    if (!allow_object_where && where_predicate.is_some())
        || (allow_object_where
            && where_predicate.is_some()
            && !native_direct_projection_predicate_is_exact(planned, where_predicate))
        || !planned.resolved.method_chain.lookups.is_empty()
        || !planned.resolved.method_chain.traversals.is_empty()
        || planned.resolved.method_chain.group_by.is_some()
        || (!allow_order && planned.resolved.method_chain.order_by.is_some())
        || (allow_order
            && planned.resolved.method_chain.order_by.is_some()
            && !native_direct_projection_order_is_exact(planned))
        || (!allow_pagination
            && (planned.resolved.method_chain.take.is_some()
                || planned.resolved.method_chain.skip.is_some()))
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
        || !dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset)
    {
        return false;
    }
    planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(true, |select| {
            select
                .iter()
                .all(|item| native_projection_expr_is_exact(&item.expr))
        })
}

fn native_direct_projection_order_is_exact(planned: &crate::PlannedQuery) -> bool {
    let Some(order) = &planned.resolved.method_chain.order_by else {
        return false;
    };
    let ResolvedExpr::Path(path) = &order.expr else {
        return false;
    };
    let root_matches = match &planned.resolved.root {
        ResolvedRoot::Association(_) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Association)
        }
        ResolvedRoot::Node(_) => matches!(path.root_kind, crate::ResolvedPathRootKind::Node),
        ResolvedRoot::Edge(_) => matches!(path.root_kind, crate::ResolvedPathRootKind::Edge),
        ResolvedRoot::Evidence(_) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Evidence)
        }
        ResolvedRoot::Object(_) | ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) => false,
    };
    root_matches && native_typed_order_path_is_exact(path)
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

fn planned_contains_target_node_association(planned: &crate::PlannedQuery) -> bool {
    planned
        .resolved
        .method_chain
        .where_predicate
        .as_ref()
        .is_some_and(predicate_contains_target_node_association)
        || planned
            .resolved
            .method_chain
            .select
            .as_ref()
            .is_some_and(|select| {
                select
                    .iter()
                    .any(|item| expr_contains_target_node_association(&item.expr))
            })
        || planned
            .resolved
            .method_chain
            .order_by
            .as_ref()
            .is_some_and(|order| expr_contains_target_node_association(&order.expr))
        || planned
            .resolved
            .method_chain
            .group_by
            .as_ref()
            .is_some_and(|exprs| exprs.iter().any(expr_contains_target_node_association))
}

fn predicate_contains_target_node_association(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            expr_contains_target_node_association(left)
                || expr_contains_target_node_association(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => expr_contains_target_node_association(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_target_node_association(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_target_node_association)
        }
    }
}

fn expr_contains_target_node_association(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Association(association) => association.target_node_object_type_id.is_some(),
        ResolvedExpr::FunctionCall { args, .. } => {
            args.iter().any(expr_contains_target_node_association)
        }
        ResolvedExpr::AggregateCall { arg, .. } => arg
            .as_deref()
            .is_some_and(expr_contains_target_node_association),
        ResolvedExpr::TableExists(exists) => predicate_contains_target_node_association(&exists.on),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_target_node_association(predicate)
                || expr_contains_target_node_association(then_expr)
                || expr_contains_target_node_association(else_expr)
        }
        ResolvedExpr::Path(_) | ResolvedExpr::Literal(_) | ResolvedExpr::Evidence(_) => false,
    }
}

pub(crate) fn native_projection_expr_is_exact(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Path(_) | ResolvedExpr::Literal(_) => true,
        ResolvedExpr::FunctionCall {
            deterministic,
            args,
            contract,
            ..
        } => {
            *deterministic
                && function_contract_is_coded_safe(contract)
                && args.iter().all(native_projection_expr_is_exact)
        }
        ResolvedExpr::AggregateCall { .. }
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_)
        | ResolvedExpr::TableExists(_)
        | ResolvedExpr::Conditional { .. } => false,
    }
}

fn native_direct_projection_predicate_is_exact(
    planned: &crate::PlannedQuery,
    predicate: Option<&ResolvedPredicate>,
) -> bool {
    match &planned.resolved.root {
        ResolvedRoot::Object(_) => compile_kernel_predicates(predicate).is_ok(),
        ResolvedRoot::Node(_) => compile_kernel_predicates(predicate).is_ok(),
        ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_) => {
            predicate.is_some_and(|predicate| row_root_predicate_is_direct_safe(planned, predicate))
        }
        ResolvedRoot::Edge(_) => {
            predicate.is_some_and(|predicate| row_root_predicate_is_direct_safe(planned, predicate))
        }
        ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) => {
            predicate.is_some_and(projection_root_predicate_is_exact)
        }
    }
}

fn projection_root_expr_is_provider_exact(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Path(path) => {
            path.root_kind == crate::ResolvedPathRootKind::Projection
                && path.projection_column.is_some()
        }
        ResolvedExpr::Literal(_) => true,
        ResolvedExpr::FunctionCall { .. }
        | ResolvedExpr::AggregateCall { .. }
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_)
        | ResolvedExpr::TableExists(_)
        | ResolvedExpr::Conditional { .. } => false,
    }
}

fn projection_root_predicate_is_exact(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            projection_root_compare_is_exact(left, *op, right)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return false;
            };
            projection_root_path_is_exact_for_compare(path, crate::AstCompareOp::Eq)
                && values
                    .iter()
                    .all(|literal| projection_root_literal_is_filterable(literal))
        }
        ResolvedPredicate::NullCheck { expr, .. } => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if path.root_kind == crate::ResolvedPathRootKind::Projection
                        && path.projection_column.is_some()
            )
        }
        ResolvedPredicate::BoolExpr(expr) => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if path.root_kind == crate::ResolvedPathRootKind::Projection
                        && path.projection_column.is_some()
                        && matches!(path.logical_type.as_str(), "bool" | "boolean")
            )
        }
        ResolvedPredicate::Not(inner) => projection_root_negated_predicate_is_exact(inner),
        ResolvedPredicate::And(parts) => parts.iter().all(projection_root_predicate_is_exact),
        ResolvedPredicate::Or(_) | ResolvedPredicate::Exists(_) => false,
    }
}

fn projection_root_negated_predicate_is_exact(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            projection_root_compare_is_exact(left, negated_compare_op(*op), right)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return false;
            };
            projection_root_path_is_exact_for_compare(path, crate::AstCompareOp::Ne)
                && values.iter().all(|literal| {
                    projection_root_literal_is_filterable(literal)
                        && !matches!(literal.typed_value, crate::ResolvedLiteralValue::Null)
                })
        }
        ResolvedPredicate::NullCheck { expr, .. } => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if path.root_kind == crate::ResolvedPathRootKind::Projection
                        && path.projection_column.is_some()
            )
        }
        ResolvedPredicate::BoolExpr(expr) => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if path.root_kind == crate::ResolvedPathRootKind::Projection
                        && path.projection_column.is_some()
                        && matches!(path.logical_type.as_str(), "bool" | "boolean")
            )
        }
        ResolvedPredicate::Not(inner) => projection_root_predicate_is_exact(inner),
        ResolvedPredicate::And(_) | ResolvedPredicate::Or(_) | ResolvedPredicate::Exists(_) => {
            false
        }
    }
}

fn projection_root_compare_is_exact(
    left: &ResolvedExpr,
    op: crate::AstCompareOp,
    right: &ResolvedExpr,
) -> bool {
    if let (ResolvedExpr::Path(path), ResolvedExpr::Literal(literal)) = (left, right) {
        return projection_root_literal_is_filterable(literal)
            && projection_root_path_is_exact_for_compare(path, op);
    }
    if let (ResolvedExpr::Literal(literal), ResolvedExpr::Path(path)) = (left, right) {
        return projection_root_literal_is_filterable(literal)
            && projection_root_path_is_exact_for_compare(path, invert_compare_op(op));
    }
    false
}

fn projection_root_path_is_exact_for_compare(path: &ResolvedPath, op: crate::AstCompareOp) -> bool {
    if path.root_kind != crate::ResolvedPathRootKind::Projection || path.projection_column.is_none()
    {
        return false;
    }
    match path.physical_kind.as_str() {
        "boolean" => matches!(op, crate::AstCompareOp::Eq | crate::AstCompareOp::Ne),
        "num_code" => true,
        "fixed_bytes" => matches!(op, crate::AstCompareOp::Eq | crate::AstCompareOp::Ne),
        _ => false,
    }
}

fn projection_root_literal_is_filterable(literal: &crate::ResolvedLiteral) -> bool {
    !matches!(
        literal.typed_value,
        crate::ResolvedLiteralValue::BigInteger(_)
    )
}

fn invert_compare_op(op: crate::AstCompareOp) -> crate::AstCompareOp {
    match op {
        crate::AstCompareOp::Eq => crate::AstCompareOp::Eq,
        crate::AstCompareOp::Ne => crate::AstCompareOp::Ne,
        crate::AstCompareOp::Lt => crate::AstCompareOp::Gt,
        crate::AstCompareOp::Le => crate::AstCompareOp::Ge,
        crate::AstCompareOp::Gt => crate::AstCompareOp::Lt,
        crate::AstCompareOp::Ge => crate::AstCompareOp::Le,
    }
}

fn negated_compare_op(op: crate::AstCompareOp) -> crate::AstCompareOp {
    match op {
        crate::AstCompareOp::Eq => crate::AstCompareOp::Ne,
        crate::AstCompareOp::Ne => crate::AstCompareOp::Eq,
        crate::AstCompareOp::Lt => crate::AstCompareOp::Ge,
        crate::AstCompareOp::Le => crate::AstCompareOp::Gt,
        crate::AstCompareOp::Gt => crate::AstCompareOp::Le,
        crate::AstCompareOp::Ge => crate::AstCompareOp::Lt,
    }
}

pub(crate) fn row_root_predicate_is_direct_safe(
    planned: &crate::PlannedQuery,
    predicate: &ResolvedPredicate,
) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            row_root_path_literal_parts(planned, left, right).is_some()
                || row_root_path_literal_parts(planned, right, left).is_some()
        }
        ResolvedPredicate::InList { expr, values } => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if row_root_path_matches_root(planned, path)
                        && values.iter().all(|literal| !matches!(literal.typed_value, crate::ResolvedLiteralValue::BigInteger(_)))
            )
        }
        ResolvedPredicate::NullCheck { expr, .. } => {
            matches!(expr, ResolvedExpr::Path(path) if row_root_path_matches_root(planned, path))
        }
        ResolvedPredicate::BoolExpr(expr) => {
            matches!(
                expr,
                ResolvedExpr::Path(path)
                    if row_root_path_matches_root(planned, path)
                        && matches!(path.logical_type.as_str(), "bool" | "boolean")
            )
        }
        ResolvedPredicate::Not(_) => false,
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => parts
            .iter()
            .all(|part| row_root_predicate_is_direct_safe(planned, part)),
        ResolvedPredicate::Exists(_) => false,
    }
}

fn row_root_path_literal_parts<'a>(
    planned: &crate::PlannedQuery,
    path_expr: &'a ResolvedExpr,
    literal_expr: &'a ResolvedExpr,
) -> Option<(&'a ResolvedPath, &'a crate::ResolvedLiteral)> {
    let ResolvedExpr::Path(path) = path_expr else {
        return None;
    };
    let ResolvedExpr::Literal(literal) = literal_expr else {
        return None;
    };
    if !row_root_path_matches_root(planned, path)
        || matches!(
            literal.typed_value,
            crate::ResolvedLiteralValue::BigInteger(_)
        )
    {
        return None;
    }
    Some((path, literal))
}

fn row_root_path_matches_root(planned: &crate::PlannedQuery, path: &ResolvedPath) -> bool {
    match &planned.resolved.root {
        ResolvedRoot::Association(_) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Association)
        }
        ResolvedRoot::Node(_) => matches!(path.root_kind, crate::ResolvedPathRootKind::Node),
        ResolvedRoot::Edge(_) => matches!(path.root_kind, crate::ResolvedPathRootKind::Edge),
        ResolvedRoot::Evidence(_) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Evidence)
        }
        ResolvedRoot::Object(_) | ResolvedRoot::Table(_) | ResolvedRoot::Projection(_) => false,
    }
}

pub(crate) fn native_bool_group_count_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            != AggregateDisclosurePolicy::AllowExact
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return false;
    };
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return false;
    };
    if group_path.object_type_id != Some(root.object_type_id)
        || group_path.property_id.is_none()
        || group_path.system_field.is_some()
        || !native_direct_group_count_path_is_exact(
            group_path,
            &planned.resolved.operation_context.dataset,
        )
    {
        return false;
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return false;
    };
    if select.len() != 2 {
        return false;
    }
    let mut saw_group = false;
    let mut aggregate_count = 0usize;
    for item in select {
        match &item.expr {
            ResolvedExpr::Path(path) if paths_same_value_domain(path, group_path) => {
                saw_group = true;
            }
            ResolvedExpr::AggregateCall {
                name, star, arg, ..
            } if native_grouped_direct_aggregate_is_exact(root, *name, *star, arg.as_deref()) => {
                aggregate_count += 1;
            }
            _ => return false,
        }
    }
    saw_group && aggregate_count == 1
}

pub(crate) fn native_bool_group_count_path(planned: &crate::PlannedQuery) -> Option<&ResolvedPath> {
    let [ResolvedExpr::Path(path)] = planned.resolved.method_chain.group_by.as_deref()? else {
        return None;
    };
    Some(path)
}

pub(crate) fn native_direct_aggregate_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            != AggregateDisclosurePolicy::AllowExact
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return false;
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return false;
    };
    let [item] = select.as_slice() else {
        return false;
    };
    let ResolvedExpr::AggregateCall {
        name, arg, star, ..
    } = &item.expr
    else {
        return false;
    };
    match (name, *star, arg.as_deref()) {
        (AstAggregateName::Count | AstAggregateName::Exists, true, None) => true,
        (
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount
            | AstAggregateName::Min
            | AstAggregateName::Max
            | AstAggregateName::Sum
            | AstAggregateName::Avg,
            false,
            Some(ResolvedExpr::Path(path)),
        ) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
                && path.object_type_id == Some(root.object_type_id)
                && path.system_field.is_none()
                && path.property_id.is_some()
                && match name {
                    AstAggregateName::Min | AstAggregateName::Max => {
                        native_typed_order_path_is_exact(path)
                    }
                    AstAggregateName::Sum | AstAggregateName::Avg => {
                        native_direct_numeric_aggregate_path_is_exact(path)
                    }
                    _ => true,
                }
        }
        _ => false,
    }
}

pub(crate) fn native_direct_aggregate_path(planned: &crate::PlannedQuery) -> Option<&ResolvedPath> {
    let [item] = planned.resolved.method_chain.select.as_deref()? else {
        return None;
    };
    let ResolvedExpr::AggregateCall {
        arg, star: false, ..
    } = &item.expr
    else {
        return None;
    };
    let ResolvedExpr::Path(path) = arg.as_deref()? else {
        return None;
    };
    Some(path)
}

pub(crate) fn native_helper_aggregate_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            != AggregateDisclosurePolicy::AllowExact
        || planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    if !matches!(planned.resolved.root, ResolvedRoot::Object(_)) {
        return false;
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return false;
    };
    let [item] = select.as_slice() else {
        return false;
    };
    let ResolvedExpr::AggregateCall {
        name,
        arg: Some(arg),
        star: false,
        ..
    } = &item.expr
    else {
        return false;
    };
    matches!(
        name,
        AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
    ) && matches!(
        arg.as_ref(),
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_)
    )
}

pub(crate) fn native_grouped_helper_aggregate_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.order_by.is_some()
        || planned.resolved.method_chain.take.is_some()
        || planned.resolved.method_chain.skip.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || planned
            .resolved
            .operation_context
            .security
            .aggregate_disclosure_policy
            != AggregateDisclosurePolicy::AllowExact
        || planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy
            != MetadataDisclosurePolicy::AllowProtected
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return false;
    };
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return false;
    };
    if group_path.object_type_id != Some(root.object_type_id)
        || group_path.property_id.is_none()
        || group_path.system_field.is_some()
        || !native_direct_group_count_path_is_exact(
            group_path,
            &planned.resolved.operation_context.dataset,
        )
    {
        return false;
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return false;
    };
    if select.len() != 2 {
        return false;
    }
    let mut saw_group = false;
    let mut aggregate_count = 0usize;
    for item in select {
        match &item.expr {
            ResolvedExpr::Path(path) if paths_same_value_domain(path, group_path) => {
                saw_group = true;
            }
            ResolvedExpr::AggregateCall {
                name,
                arg: Some(arg),
                star: false,
                ..
            } if matches!(
                name,
                AstAggregateName::Count
                    | AstAggregateName::Exists
                    | AstAggregateName::DistinctCount
            ) && matches!(
                arg.as_ref(),
                ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_)
            ) =>
            {
                aggregate_count += 1;
            }
            _ => return false,
        }
    }
    saw_group && aggregate_count == 1
}

pub(crate) fn native_direct_aggregate_name(
    planned: &crate::PlannedQuery,
) -> Option<AstAggregateName> {
    let [item] = planned.resolved.method_chain.select.as_deref()? else {
        return None;
    };
    let ResolvedExpr::AggregateCall { name, .. } = &item.expr else {
        return None;
    };
    Some(*name)
}

fn native_grouped_direct_aggregate_is_exact(
    root: &crate::ResolvedObjectRoot,
    name: AstAggregateName,
    star: bool,
    arg: Option<&ResolvedExpr>,
) -> bool {
    match (name, star, arg) {
        (AstAggregateName::Count | AstAggregateName::Exists, true, None) => true,
        (
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount
            | AstAggregateName::Min
            | AstAggregateName::Max
            | AstAggregateName::Sum
            | AstAggregateName::Avg,
            false,
            Some(ResolvedExpr::Path(path)),
        ) => {
            matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
                && path.object_type_id == Some(root.object_type_id)
                && path.system_field.is_none()
                && path.property_id.is_some()
                && match name {
                    AstAggregateName::Min | AstAggregateName::Max => {
                        native_typed_order_path_is_exact(path)
                    }
                    AstAggregateName::Sum | AstAggregateName::Avg => {
                        native_direct_numeric_aggregate_path_is_exact(path)
                    }
                    _ => true,
                }
        }
        _ => false,
    }
}

fn native_grouped_aggregate_representation_class(
    planned: &crate::PlannedQuery,
    expr: &ResolvedExpr,
) -> CodedRepresentationClass {
    let ResolvedExpr::AggregateCall {
        name, star, arg, ..
    } = expr
    else {
        return CodedRepresentationClass::CodePure;
    };
    if matches!(
        (*name, *star),
        (AstAggregateName::Count | AstAggregateName::Exists, true)
    ) {
        return CodedRepresentationClass::CodePure;
    }
    match (name, arg.as_deref()) {
        (AstAggregateName::Count | AstAggregateName::Exists, _) => {
            CodedRepresentationClass::CodePure
        }
        (AstAggregateName::DistinctCount, Some(ResolvedExpr::Path(path)))
            if path.physical_kind == "file_code" =>
        {
            file_code_path_contract(&planned.resolved.operation_context.dataset).0
        }
        (
            AstAggregateName::DistinctCount
            | AstAggregateName::Min
            | AstAggregateName::Max
            | AstAggregateName::Sum
            | AstAggregateName::Avg,
            Some(ResolvedExpr::Path(path)),
        ) => native_typed_order_representation_class(path),
        _ => native_bool_group_count_path(planned)
            .map(|path| {
                native_direct_group_count_representation_class(
                    path,
                    &planned.resolved.operation_context.dataset,
                )
            })
            .unwrap_or(CodedRepresentationClass::CodePure),
    }
}

pub(crate) fn aggregate_operator_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
    }
}

fn native_direct_aggregate_representation_class(
    planned: &crate::PlannedQuery,
    name: AstAggregateName,
) -> CodedRepresentationClass {
    if matches!(
        name,
        AstAggregateName::Min
            | AstAggregateName::Max
            | AstAggregateName::Sum
            | AstAggregateName::Avg
    ) {
        native_direct_aggregate_path(planned)
            .map(native_typed_order_representation_class)
            .unwrap_or(CodedRepresentationClass::TypedNumericCoded)
    } else {
        CodedRepresentationClass::CodePure
    }
}

pub(crate) fn paths_same_value_domain(left: &ResolvedPath, right: &ResolvedPath) -> bool {
    left.root_kind == right.root_kind
        && left.object_type_id == right.object_type_id
        && left.property_id == right.property_id
        && left.association_type_id == right.association_type_id
        && left.evidence_field_id == right.evidence_field_id
        && left.projection_id == right.projection_id
        && left.projection_column == right.projection_column
        && left.system_field == right.system_field
        && left.logical_type == right.logical_type
        && left.null_policy == right.null_policy
}

pub(crate) fn native_typed_order_shape(planned: &crate::PlannedQuery) -> bool {
    if !matches!(planned.resolved.output_mode, CoveQlOutputMode::JsonRows)
        || planned.resolved.method_chain.where_predicate.is_some()
        || planned.resolved.method_chain.group_by.is_some()
        || planned.resolved.method_chain.history.is_some()
        || planned.resolved.method_chain.changes.is_some()
        || planned.resolved.temporal.role_binding.is_some()
        || !planned.resolved.temporal.mode.is_point_in_time()
        || !matches!(
            planned
                .resolved
                .operation_context
                .security
                .visibility_policy,
            crate::VisibilityPolicy::AllRows
        )
        || matches!(
            planned.resolved.branch.selector,
            crate::BranchSelector::RejectAmbiguous
        )
    {
        return false;
    }
    let ResolvedRoot::Object(root) = &planned.resolved.root else {
        return false;
    };
    let Some(order_path) = native_typed_order_path(planned) else {
        return false;
    };
    if order_path.object_type_id != Some(root.object_type_id)
        || order_path.property_id.is_none()
        || order_path.system_field.is_some()
        || !native_typed_order_path_is_exact(order_path)
    {
        return false;
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return false;
    };
    !select.is_empty()
        && select.iter().all(|item| {
            matches!(
                item.expr,
                ResolvedExpr::Path(ref path)
                    if matches!(path.root_kind, crate::ResolvedPathRootKind::Object)
                        && path.object_type_id == Some(root.object_type_id)
                        && path.system_field.is_none()
            )
        })
}

pub(crate) fn native_typed_order_path(planned: &crate::PlannedQuery) -> Option<&ResolvedPath> {
    let ResolvedExpr::Path(path) = &planned.resolved.method_chain.order_by.as_ref()?.expr else {
        return None;
    };
    Some(path)
}

fn native_typed_order_path_is_exact(path: &ResolvedPath) -> bool {
    if path.physical_kind == "execution_code" {
        return false;
    }
    if path.physical_kind == "file_code" {
        return matches!(path.logical_type.as_str(), "utf8" | "string")
            && path_has_ordering_collation_contract(path);
    }
    matches!(
        path.logical_type.as_str(),
        "bool"
            | "boolean"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "date_days"
            | "timestamp_micros"
            | "timestamp_nanos"
            | "utf8"
            | "string"
    )
}

fn native_direct_numeric_aggregate_path_is_exact(path: &ResolvedPath) -> bool {
    if matches!(path.physical_kind.as_str(), "file_code" | "execution_code") {
        return false;
    }
    matches!(
        path.logical_type.as_str(),
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "decimal128"
    )
}

fn native_direct_group_count_path_is_exact(
    path: &ResolvedPath,
    dataset: &crate::DatasetScopeContext,
) -> bool {
    if matches!(path.physical_kind.as_str(), "execution_code") {
        return false;
    }
    if path.physical_kind == "file_code" {
        return dataset_has_exact_code_domain_bridge(dataset);
    }
    matches!(
        path.logical_type.as_str(),
        "bool"
            | "boolean"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "decimal128"
            | "date_days"
            | "timestamp_micros"
            | "timestamp_nanos"
    )
}

fn native_direct_group_count_representation_class(
    path: &ResolvedPath,
    dataset: &crate::DatasetScopeContext,
) -> CodedRepresentationClass {
    if path.physical_kind == "file_code" {
        file_code_path_contract(dataset).0
    } else if matches!(path.logical_type.as_str(), "bool" | "boolean") {
        CodedRepresentationClass::CodePure
    } else {
        CodedRepresentationClass::TypedNumericCoded
    }
}

fn native_direct_projection_expr_representation_class(
    expr: &ResolvedExpr,
    dataset: &crate::DatasetScopeContext,
) -> CodedRepresentationClass {
    match expr {
        ResolvedExpr::Path(path) => match path.physical_kind.as_str() {
            "file_code" => file_code_path_contract(dataset).0,
            "boolean" | "fixed_bytes" => CodedRepresentationClass::CodePure,
            "num_code" | "int" | "uint" | "float" => CodedRepresentationClass::TypedNumericCoded,
            _ => CodedRepresentationClass::DecodeBoundary,
        },
        ResolvedExpr::FunctionCall { .. } => CodedRepresentationClass::DictionaryLifted,
        ResolvedExpr::Literal(_) => CodedRepresentationClass::CodePure,
        _ => CodedRepresentationClass::DecodeBoundary,
    }
}

fn native_typed_order_representation_class(path: &ResolvedPath) -> CodedRepresentationClass {
    if path.physical_kind == "file_code" {
        CodedRepresentationClass::DecodeBoundary
    } else if matches!(path.logical_type.as_str(), "bool" | "boolean") {
        CodedRepresentationClass::CodePure
    } else if matches!(path.logical_type.as_str(), "utf8" | "string") {
        CodedRepresentationClass::OrdinalMapAssisted
    } else {
        CodedRepresentationClass::TypedNumericCoded
    }
}

fn resolved_predicate_count(predicate: Option<&ResolvedPredicate>) -> usize {
    match predicate {
        None => 0,
        Some(ResolvedPredicate::Compare { .. })
        | Some(ResolvedPredicate::InList { .. })
        | Some(ResolvedPredicate::NullCheck { .. })
        | Some(ResolvedPredicate::Exists(_))
        | Some(ResolvedPredicate::BoolExpr(_)) => 1,
        Some(ResolvedPredicate::Not(inner)) => resolved_predicate_count(Some(inner)),
        Some(ResolvedPredicate::And(parts)) | Some(ResolvedPredicate::Or(parts)) => parts
            .iter()
            .map(|part| resolved_predicate_count(Some(part)))
            .sum(),
    }
}

fn append_predicate_contracts(
    planned: &crate::PlannedQuery,
    predicate: &ResolvedPredicate,
    contracts: &mut Vec<CodedOperatorContract>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            if identity_cast_literal_compare_is_kernel_safe(planned, left, *op, right) {
                contracts.push(
                    CodedOperatorContract::new(
                        "predicate_function:cast",
                        CodedRepresentationClass::CodePure,
                        true,
                        false,
                        "identity safe-cast over a direct path is equivalent to comparing the original path and preserves null/type semantics",
                    )
                    .with_required_metadata(&["safe_cast_contract", "null_policy", "logical_type"]),
                );
                return;
            }
            if string_function_literal_compare_is_kernel_safe(left, *op, right) {
                let function_id =
                    string_compare_function_id(left).or_else(|| string_compare_function_id(right));
                contracts.push(CodedOperatorContract::new(
                    format!(
                        "predicate_function:{}",
                        function_id.unwrap_or("string_scalar")
                    ),
                    CodedRepresentationClass::DictionaryLifted,
                    true,
                    false,
                    "registered string scalar comparison uses the same deterministic materialized function body and preserves null-as-unknown semantics",
                )
                .with_required_metadata(&["function_contract", "null_policy"]));
                return;
            }
            if length_expr_literal_compare_is_kernel_safe(left, *op, right) {
                contracts.push(
                    CodedOperatorContract::new(
                        "predicate_function:length",
                        CodedRepresentationClass::DictionaryLifted,
                        true,
                        false,
                        "length over a direct string path uses the materialized Unicode scalar-count contract and preserves null-as-unknown comparison semantics",
                    )
                    .with_required_metadata(&["function_contract", "null_policy"]),
                );
                return;
            }
            if bool_expr_literal_compare_is_kernel_safe(left, *op, right) {
                contracts.push(
                    CodedOperatorContract::new(
                        "predicate_bool_compare",
                        CodedRepresentationClass::CodePure,
                        true,
                        false,
                        "boolean function/path comparisons to boolean literals reuse the kernel three-valued predicate tree",
                    )
                    .with_required_metadata(&["null_policy", "three_valued_logic_contract"]),
                );
                return;
            }
            append_expr_contract(planned, "predicate_compare_left", left, contracts);
            append_expr_contract(planned, "predicate_compare_right", right, contracts);
            let ordered = matches!(
                op,
                crate::AstCompareOp::Lt
                    | crate::AstCompareOp::Le
                    | crate::AstCompareOp::Gt
                    | crate::AstCompareOp::Ge
            );
            let exact = if ordered {
                compare_operands_have_order_contract(left, right)
            } else {
                compare_operands_have_equality_contract(planned, left, right)
            };
            let file_code_order_boundary =
                ordered && compare_operands_include_file_code_path(left, right);
            let file_code_cross_source_bridge = !ordered
                && compare_operands_include_file_code_path(left, right)
                && planned.resolved.operation_context.dataset.files.len() > 1;
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_compare",
                    if file_code_order_boundary {
                        CodedRepresentationClass::DecodeBoundary
                    } else if file_code_cross_source_bridge {
                        CodedRepresentationClass::CrossSourceCodeBridge
                    } else if ordered {
                        CodedRepresentationClass::TypedNumericCoded
                    } else {
                        CodedRepresentationClass::CodePure
                    },
                    exact,
                    !exact,
                    if file_code_order_boundary {
                        "ordered FileCode comparison is exact only through decoded value comparison under the effective string collation; raw dictionary code order is not trusted"
                    } else if ordered {
                        "ordered comparison is coded only for typed numeric/date lanes or ordinal sidecars"
                    } else {
                        "equality comparison can stay coded only within a shared semantic domain, compatible literal type, and null contract"
                    },
                )
                .with_required_metadata(if ordered {
                    if file_code_order_boundary {
                        &["effective_ordering_collation", "materialized_compare_key", "null_policy"]
                    } else {
                        &["typed_order_lane", "ordinal_sidecar", "null_policy"]
                    }
                } else {
                    &["semantic_domain", "null_policy"]
                }),
            );
        }
        ResolvedPredicate::InList { expr, values } => {
            append_expr_contract(planned, "predicate_in", expr, contracts);
            let exact = in_list_has_equality_contract(planned, expr, values);
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_in",
                    CodedRepresentationClass::CodePure,
                    exact,
                    !exact,
                    if exact {
                        "IN uses the kernel three-valued membership truth table when the path/literal domain proof passes"
                    } else {
                        "IN requires every literal to belong to the path value domain before coded membership evaluation"
                    },
                )
                .with_required_metadata(&["semantic_domain", "null_policy"]),
            );
        }
        ResolvedPredicate::NullCheck { expr, .. } => {
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            if !matches!(expr, ResolvedExpr::Path(_)) {
                append_expr_contract(planned, "predicate_null_or_bool", expr, contracts);
            }
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_null_check",
                    if kernel_safe {
                    CodedRepresentationClass::CodePure
                } else {
                    CodedRepresentationClass::MaterializedResidual
                },
                    kernel_safe,
                    !kernel_safe,
                if kernel_safe {
                    "null checks use validity/null-lane semantics and do not require value materialization"
                } else {
                    "null checks require a direct path before coded validity-lane execution"
                },
                )
                .with_required_metadata(&["validity_lane", "null_policy"]),
            );
        }
        ResolvedPredicate::BoolExpr(ResolvedExpr::FunctionCall {
            function_id, args, ..
        }) if matches!(function_id.as_str(), "isNull" | "isNotNull")
            && args
                .first()
                .is_some_and(|arg| matches!(arg, ResolvedExpr::Path(_))) =>
        {
            append_expr_contract(planned, "predicate_null_or_bool", &args[0], contracts);
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            contracts.push(
                CodedOperatorContract::new(
                    format!("predicate_function:{function_id}"),
                    if kernel_safe {
                    CodedRepresentationClass::CodePure
                } else {
                    CodedRepresentationClass::MaterializedResidual
                },
                    kernel_safe,
                    !kernel_safe,
                if kernel_safe {
                    "function-form null checks use the same kernel validity/null-lane proof as path null predicates"
                } else {
                    "function-form null checks require a direct path argument before coded execution"
                },
                )
                .with_required_metadata(&["validity_lane", "function_contract", "null_policy"]),
            );
        }
        ResolvedPredicate::BoolExpr(ResolvedExpr::FunctionCall {
            function_id, args, ..
        }) if function_id == "identity"
            && args.first().is_some_and(|arg| {
                matches!(
                    arg,
                    ResolvedExpr::Path(path)
                        if matches!(path.logical_type.as_str(), "bool" | "boolean")
                )
            }) =>
        {
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_function:identity",
                    if kernel_safe {
                    CodedRepresentationClass::CodePure
                } else {
                    CodedRepresentationClass::MaterializedResidual
                },
                    kernel_safe,
                    !kernel_safe,
                if kernel_safe {
                    "identity over a direct boolean path is equivalent to the bool-path kernel predicate and preserves null semantics"
                } else {
                    "identity predicates require a direct boolean path before coded execution"
                },
                )
                .with_required_metadata(&["function_contract", "null_policy"]),
            );
        }
        ResolvedPredicate::BoolExpr(ResolvedExpr::FunctionCall {
            function_id, args, ..
        }) if function_id == "cast" && identity_cast_bool_args_are_kernel_safe(args) => {
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_function:cast",
                    CodedRepresentationClass::CodePure,
                    kernel_safe,
                    !kernel_safe,
                    if kernel_safe {
                        "identity safe-cast over a direct boolean path is equivalent to the bool-path kernel predicate"
                    } else {
                        "identity safe-cast predicates require a direct boolean path and matching target type"
                    },
                )
                .with_required_metadata(&["safe_cast_contract", "null_policy", "logical_type"]),
            );
        }
        ResolvedPredicate::BoolExpr(ResolvedExpr::FunctionCall {
            function_id, args, ..
        }) if function_id == "coalesce" && coalesce_bool_args_are_kernel_safe(args) => {
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_function:coalesce",
                    if kernel_safe {
                    CodedRepresentationClass::CodePure
                } else {
                    CodedRepresentationClass::MaterializedResidual
                },
                    kernel_safe,
                    !kernel_safe,
                if kernel_safe {
                    "coalesce over boolean paths/literals can stay in the kernel using null-lane semantics"
                } else {
                    "coalesce predicates require only boolean paths plus boolean/null literals before coded execution"
                },
                )
                .with_required_metadata(&["function_contract", "null_policy", "three_valued_logic_contract"]),
            );
        }
        ResolvedPredicate::BoolExpr(expr) => {
            append_expr_contract(planned, "predicate_null_or_bool", expr, contracts);
        }
        ResolvedPredicate::Exists(expr) => {
            append_expr_contract(planned, "predicate_exists", expr, contracts)
        }
        ResolvedPredicate::Not(inner) => {
            append_predicate_contracts(planned, inner, contracts);
            let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_not",
                    if kernel_safe {
                    CodedRepresentationClass::CodePure
                } else {
                    CodedRepresentationClass::MaterializedResidual
                },
                    kernel_safe,
                    !kernel_safe,
                if kernel_safe {
                    "NOT preserves the CoveQL three-valued truth table inside the kernel predicate tree"
                } else {
                    "complement semantics require three-valued-logic proof before coded pruning"
                },
                )
                .with_required_metadata(&["three_valued_logic_contract", "null_policy"]),
            );
        }
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                append_predicate_contracts(planned, part, contracts);
            }
            if matches!(predicate, ResolvedPredicate::Or(_)) {
                let kernel_safe = compile_kernel_predicates(Some(predicate)).is_ok();
                contracts.push(
                    CodedOperatorContract::new(
                        "predicate_or",
                        if kernel_safe {
                        CodedRepresentationClass::CodePure
                    } else {
                        CodedRepresentationClass::MaterializedResidual
                    },
                        kernel_safe,
                        !kernel_safe,
                    if kernel_safe {
                        "OR preserves the CoveQL three-valued truth table inside the kernel predicate tree"
                    } else {
                        "OR proof composition remains residual unless no-false-negative proof metadata is available"
                    },
                    )
                    .with_required_metadata(&["three_valued_logic_contract", "null_policy"]),
                );
            }
        }
    }
}

fn append_expr_contract(
    planned: &crate::PlannedQuery,
    operator: &str,
    expr: &ResolvedExpr,
    contracts: &mut Vec<CodedOperatorContract>,
) {
    match expr {
        ResolvedExpr::Path(path) => {
            let (representation_class, exact, reason) = match path.physical_kind.as_str() {
                "file_code" => file_code_path_contract(
                    &planned.resolved.operation_context.dataset,
                ),
                "boolean" | "fixed_bytes"
                    if !matches!(operator, "group_by" | "order_by_expr") =>
                {
                    (
                        CodedRepresentationClass::CodePure,
                        true,
                        "boolean and fixed-width identity lanes preserve equality/null semantics until the final output materialization boundary",
                    )
                }
                "boolean" | "fixed_bytes" => (
                    CodedRepresentationClass::DecodeBoundary,
                    false,
                    "grouping or ordering over boolean/fixed-width lanes still requires a row-grain/order proof before becoming a native coded operator",
                ),
                "execution_code" => (
                    CodedRepresentationClass::CrossSourceCodeBridge,
                    false,
                    "execution-code comparisons require an explicit COVE-E remap and snapshot/epoch proof",
                ),
                "num_code" | "int" | "uint" | "float" => (
                    CodedRepresentationClass::TypedNumericCoded,
                    true,
                    "typed numeric lanes preserve comparison semantics with explicit null handling",
                ),
                _ => (
                    CodedRepresentationClass::DecodeBoundary,
                    false,
                    "value materialization is required unless a typed/coded sidecar proves semantics",
                ),
            };
            contracts.push(
                CodedOperatorContract::new(operator, representation_class, exact, !exact, reason)
                    .with_required_metadata(match representation_class {
                        CodedRepresentationClass::CodePure => &["semantic_domain", "null_policy"],
                        CodedRepresentationClass::CrossSourceCodeBridge => {
                            &["execution_code_domain", "code_domain_bridge_context"]
                        }
                        CodedRepresentationClass::TypedNumericCoded => {
                            &["typed_lane", "null_policy"]
                        }
                        CodedRepresentationClass::DecodeBoundary => {
                            &["materialized_value", "null_policy"]
                        }
                        _ => &["null_policy"],
                    })
                    .with_proof_obligation(if operator == "group_by" {
                        "GROUP BY over a path can become coded only after code equality is proven to match CoveQL grouping equality at the reconstructed row grain"
                    } else {
                        "path expression uses its physical-kind contract plus CoveQL null/type semantics"
                    }),
            );
        }
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic,
            args,
            contract,
            ..
        } => {
            let coded = *deterministic
                && function_contract_is_coded_safe(contract)
                && args.iter().all(native_projection_expr_is_exact);
            if coded {
                for arg in args {
                    append_coded_function_arg_contract(planned, function_id, arg, contracts);
                }
            } else {
                for arg in args {
                    append_expr_contract(planned, operator, arg, contracts);
                }
            }
            contracts.push(
                CodedOperatorContract::new(
                    format!("function:{function_id}"),
                    if coded {
                        CodedRepresentationClass::DictionaryLifted
                    } else {
                        CodedRepresentationClass::DecodeBoundary
                    },
                    coded,
                    !coded,
                    if coded {
                        "deterministic function declares a coded-value contract"
                    } else {
                        "function requires materialized values or a missing coded contract"
                    },
                )
                .with_required_metadata(&["function_contract", "null_policy", "collation_policy"])
                .with_fallback_boundary("materialized_function_evaluation"),
            );
        }
        ResolvedExpr::AggregateCall { name, arg, .. } => {
            if let Some(arg) = arg.as_deref() {
                append_expr_contract(planned, operator, arg, contracts);
            }
            contracts.push(
                CodedOperatorContract::new(
                    format!("aggregate:{name:?}"),
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "aggregate output is checked against materialized semantics until merge/null/overflow contracts are proven",
                )
                .with_row_grain("groups_over_reconstructed_visible_rows")
                .with_required_metadata(&[
                    "state_grain_contract",
                    "aggregate_null_policy",
                    "aggregate_overflow_policy",
                ])
                .with_proof_obligation(
                    "aggregates cannot run over raw temporal records; they require reconstructed visible row grain plus exact merge/null/overflow semantics",
                )
                .with_fallback_boundary("materialized_aggregate_evaluation"),
            );
        }
        ResolvedExpr::Association(_) if operator == "predicate_exists" => {
            let native_helper_exists_direct_projection =
                native_helper_exists_direct_projection_shape(planned);
            let mut contract = CodedOperatorContract::new(
                "predicate_exists:association",
                CodedRepresentationClass::OrdinalMapAssisted,
                native_helper_exists_direct_projection,
                !native_helper_exists_direct_projection,
                if native_helper_exists_direct_projection {
                    "association existence semi-join is the full predicate and can be evaluated exactly through a scoped endpoint edge table"
                } else {
                    "association existence can use an endpoint edge-table prefilter, with materialized residual verification for visibility and disclosure semantics"
                },
            )
            .with_row_grain("reconstructed_visible_object_states")
            .with_required_metadata(&[
                "association_endpoint_flags",
                "association_visibility_policy",
                "disclosure_policy",
            ])
            .with_proof_obligation(if native_helper_exists_direct_projection {
                "association semi-join is exact because protected metadata disclosure is enabled, the helper predicate is positive and owns the whole WHERE clause, endpoint role/direction are resolved, and rows are already reconstructed visible object states"
            } else {
                "association semi/anti joins must prove endpoint role, direction, validity interval, visibility, and no-hidden-edge leakage before becoming authoritative"
            });
            if !native_helper_exists_direct_projection {
                contract =
                    contract.with_fallback_boundary("materialized_helper_residual_verification");
            }
            contracts.push(contract);
        }
        ResolvedExpr::Evidence(_) if operator == "predicate_exists" => {
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_exists:evidence",
                    CodedRepresentationClass::OrdinalMapAssisted,
                    false,
                    true,
                    "evidence existence can use a grain/target index prefilter, with materialized residual verification for disclosure semantics",
                )
                .with_row_grain("reconstructed_visible_object_states")
                .with_required_metadata(&[
                    "cove_map_evidence_index",
                    "evidence_disclosure_policy",
                    "target_grain_index",
                ])
                .with_proof_obligation(
                    "evidence existence must prove target grain, hidden-entry filtering, tenant visibility, and disclosure policy before becoming authoritative",
                )
                .with_fallback_boundary("materialized_helper_residual_verification"),
            );
        }
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => {
            contracts.push(
                CodedOperatorContract::new(
                    operator,
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "association/evidence helper aggregates or projections require metadata readback and residual authority",
                )
                .with_required_metadata(&["cove_map", "disclosure_policy"])
                .with_fallback_boundary("materialized_metadata_readback"),
            );
        }
        ResolvedExpr::TableExists(exists) => {
            append_predicate_contracts(planned, &exists.on, contracts);
            contracts.push(
                CodedOperatorContract::new(
                    "predicate_exists:table",
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "table exists semi/anti join uses projection-backed materialized rows unless an exact coded lookup-join proof is available",
                )
                .with_row_grain("visible_table_rows")
                .with_required_metadata(&["projection_catalog", "table_lookup_contract"])
                .with_proof_obligation(
                    "table semi/anti joins need exact key-domain, duplicate-row, visibility, and null-semantics proof before native coded execution",
                )
                .with_fallback_boundary("materialized_table_exists_evaluation"),
            );
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            append_predicate_contracts(planned, predicate, contracts);
            append_expr_contract(planned, operator, then_expr, contracts);
            append_expr_contract(planned, operator, else_expr, contracts);
            contracts.push(
                CodedOperatorContract::new(
                    "conditional",
                    CodedRepresentationClass::MaterializedResidual,
                    false,
                    true,
                    "conditional expression needs three-valued predicate and branch type proof before coded execution",
                )
                .with_required_metadata(&[
                    "three_valued_logic_contract",
                    "branch_type_contract",
                    "null_policy",
                ])
                .with_fallback_boundary("materialized_conditional_evaluation"),
            );
        }
        ResolvedExpr::Literal(_) => {}
    }
}

fn append_coded_function_arg_contract(
    planned: &crate::PlannedQuery,
    function_id: &str,
    arg: &ResolvedExpr,
    contracts: &mut Vec<CodedOperatorContract>,
) {
    match arg {
        ResolvedExpr::Path(path) => {
            let representation_class = match path.physical_kind.as_str() {
                "num_code" | "int" | "uint" | "float" => {
                    CodedRepresentationClass::TypedNumericCoded
                }
                "boolean" | "fixed_bytes" => CodedRepresentationClass::CodePure,
                "file_code" => file_code_path_contract(&planned.resolved.operation_context.dataset)
                    .0,
                "execution_code" => CodedRepresentationClass::CrossSourceCodeBridge,
                _ => CodedRepresentationClass::DictionaryLifted,
            };
            contracts.push(
                CodedOperatorContract::new(
                    format!("function_arg:{function_id}"),
                    representation_class,
                    true,
                    false,
                    "argument is consumed inside a deterministic coded-safe function contract without per-row materialized residual evaluation",
                )
                .with_required_metadata(&[
                    "function_contract",
                    "semantic_domain",
                    "null_policy",
                    "collation_policy",
                ])
                .with_proof_obligation(
                    "coded-safe function arguments are exact because the function contract proves null, type, collation, and materialization behavior for the argument domain",
                ),
            );
        }
        ResolvedExpr::Literal(_) => contracts.push(
            CodedOperatorContract::new(
                format!("function_literal:{function_id}"),
                CodedRepresentationClass::CodePure,
                true,
                false,
                "literal argument is folded into the deterministic coded-safe function contract",
            )
            .with_required_metadata(&["function_contract", "literal_type", "null_policy"])
            .with_proof_obligation(
                "literal function arguments are exact because their CoveQL literal type and null semantics are fixed during resolution",
            ),
        ),
        ResolvedExpr::FunctionCall {
            function_id: nested_function_id,
            deterministic,
            args,
            contract,
            ..
        } if *deterministic
            && function_contract_is_coded_safe(contract)
            && args.iter().all(native_projection_expr_is_exact) =>
        {
            for nested_arg in args {
                append_coded_function_arg_contract(
                    planned,
                    nested_function_id,
                    nested_arg,
                    contracts,
                );
            }
            contracts.push(
                CodedOperatorContract::new(
                    format!("function:{nested_function_id}"),
                    CodedRepresentationClass::DictionaryLifted,
                    true,
                    false,
                    "nested deterministic function declares a coded-value contract",
                )
                .with_required_metadata(&["function_contract", "null_policy", "collation_policy"])
                .with_proof_obligation(
                    "nested coded-safe function remains exact because its output feeds another coded-safe function without an intervening materialization-sensitive operation",
                ),
            );
        }
        _ => append_expr_contract(planned, "function_arg", arg, contracts),
    }
}

fn dataset_has_exact_code_domain_bridge(dataset: &crate::DatasetScopeContext) -> bool {
    dataset.files.len() <= 1
        || (!dataset.code_domain_bridges.is_empty()
            && dataset
                .code_domain_bridges
                .iter()
                .all(crate::code_domain_bridge_is_exact_coded_remap))
}

fn file_code_path_contract(
    dataset: &crate::DatasetScopeContext,
) -> (CodedRepresentationClass, bool, &'static str) {
    if dataset.files.len() <= 1 {
        return (
            CodedRepresentationClass::CodePure,
            true,
            "FileCode identity is code-pure inside a single validated file/domain scope",
        );
    }
    if dataset_has_exact_code_domain_bridge(dataset) {
        return (
            CodedRepresentationClass::CrossSourceCodeBridge,
            true,
            "FileCode identity is exact only through the manifest validated code-domain bridge",
        );
    }
    (
        CodedRepresentationClass::CrossSourceCodeBridge,
        false,
        "multi-file FileCode identity requires an exact canonical remap bridge; raw local codes remain file scoped",
    )
}

fn path_has_ordering_collation_contract(path: &ResolvedPath) -> bool {
    matches!(path.logical_type.as_str(), "utf8" | "string")
        && match path.collation_id {
            None => true,
            Some(id) if id == CollationKind::None.id() => true,
            Some(id) => {
                CollationKind::from_id(id).is_some_and(|kind| kind == CollationKind::Utf8Bytewise)
            }
        }
}

fn function_contract_is_coded_safe(contract: &crate::ResolvedFunctionContract) -> bool {
    matches!(
        contract.execution_class,
        crate::FunctionExecutionClass::CodedSafe
    )
}

fn compare_operands_have_order_contract(left: &ResolvedExpr, right: &ResolvedExpr) -> bool {
    [left, right].iter().all(|expr| match expr {
        ResolvedExpr::Path(path) if path.physical_kind == "file_code" => {
            path_has_ordering_collation_contract(path)
        }
        ResolvedExpr::Path(path) => path.physical_kind != "execution_code",
        ResolvedExpr::Literal(_) => true,
        _ => false,
    })
}

fn compare_operands_include_file_code_path(left: &ResolvedExpr, right: &ResolvedExpr) -> bool {
    [left, right].iter().any(|expr| {
        matches!(
            expr,
            ResolvedExpr::Path(path) if path.physical_kind == "file_code"
        )
    })
}

fn compare_operands_have_equality_contract(
    planned: &crate::PlannedQuery,
    left: &ResolvedExpr,
    right: &ResolvedExpr,
) -> bool {
    if let Some((path, literal)) = path_literal_compare_parts(left, right) {
        return path_literal_has_compatible_contract_in_scope(planned, path, literal);
    }
    true
}

fn path_literal_compare_parts<'a>(
    left: &'a ResolvedExpr,
    right: &'a ResolvedExpr,
) -> Option<(&'a crate::ResolvedPath, &'a crate::ResolvedLiteral)> {
    if let (Some(path), ResolvedExpr::Literal(literal)) = (path_or_identity_cast_path(left), right)
    {
        return Some((path, literal));
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) = (left, path_or_identity_cast_path(right))
    {
        return Some((path, literal));
    }
    None
}

fn in_list_has_equality_contract(
    planned: &crate::PlannedQuery,
    expr: &ResolvedExpr,
    values: &[crate::ResolvedLiteral],
) -> bool {
    let ResolvedExpr::Path(path) = expr else {
        return false;
    };
    values
        .iter()
        .all(|literal| path_literal_has_compatible_contract_in_scope(planned, path, literal))
}

fn coded_decode_boundaries(planned: &crate::PlannedQuery) -> Vec<String> {
    coded_operator_contracts(planned)
        .into_iter()
        .filter(|contract| {
            contract.residual_required
                || matches!(
                    contract.representation_class,
                    CodedRepresentationClass::DecodeBoundary
                )
        })
        .map(|contract| {
            if contract.residual_required {
                format!("{}: {}", contract.operator, contract.reason)
            } else {
                format!(
                    "{}: exact decode boundary; {}",
                    contract.operator, contract.reason
                )
            }
        })
        .collect()
}

fn coded_bridge_decisions(planned: &crate::PlannedQuery) -> Vec<String> {
    let mut decisions = planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges
        .iter()
        .map(|bridge| {
            format!(
                "{} exact={} kind={} reason={}",
                bridge.domain_id, bridge.exact, bridge.bridge_kind, bridge.reason
            )
        })
        .collect::<Vec<_>>();
    decisions.extend(
        planned
            .resolved
            .operation_context
            .dataset
            .execution_code_domains
            .iter()
            .map(|domain| {
                format!(
                    "execution_code_domain engine_profile={} code_space={} exact={} scope={} lifetime={} epoch={:?} null_policy={} security_scope={:?} reason={}",
                    domain.engine_profile_id,
                    domain.code_space_id,
                    domain.exact,
                    domain.comparison_scope,
                    domain.lifetime,
                    domain.epoch,
                    domain.null_code_policy,
                    domain.security_scope_id,
                    domain.reason
                )
            }),
    );
    decisions
}

fn predicate_contains_unsafe_coded(
    planned: &crate::PlannedQuery,
    predicate: Option<&ResolvedPredicate>,
) -> bool {
    let Some(predicate) = predicate else {
        return false;
    };
    if row_root_predicate_is_direct_safe(planned, predicate) {
        return false;
    }
    match predicate {
        ResolvedPredicate::Compare { left, right, op } => {
            expression_is_ordered_file_code(left, *op)
                || expression_is_ordered_file_code(right, *op)
                || path_literal_compare_is_kernel_incompatible(planned, left, right)
                || expression_contains_unsupported_coded(left)
                || expression_contains_unsupported_coded(right)
        }
        ResolvedPredicate::InList { expr, values } => {
            expression_contains_unsupported_coded(expr)
                || in_list_is_kernel_incompatible(planned, expr, values)
        }
        ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => expression_contains_unsupported_coded(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_unsafe_coded(planned, Some(inner)),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => parts
            .iter()
            .any(|part| predicate_contains_unsafe_coded(planned, Some(part))),
    }
}

fn predicate_contains_association_or_evidence(predicate: Option<&ResolvedPredicate>) -> bool {
    let Some(predicate) = predicate else {
        return false;
    };
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            expression_contains_association_or_evidence(left)
                || expression_contains_association_or_evidence(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => expression_contains_association_or_evidence(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_association_or_evidence(Some(inner)),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => parts
            .iter()
            .any(|part| predicate_contains_association_or_evidence(Some(part))),
    }
}

fn expression_is_ordered_file_code(expr: &ResolvedExpr, op: crate::AstCompareOp) -> bool {
    matches!(
        op,
        crate::AstCompareOp::Lt
            | crate::AstCompareOp::Le
            | crate::AstCompareOp::Gt
            | crate::AstCompareOp::Ge
    ) && matches!(
        expr,
        ResolvedExpr::Path(path)
            if path.physical_kind == "file_code" && !path_has_ordering_collation_contract(path)
    )
}

fn expression_contains_unsupported_coded(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Path(path) => path.physical_kind == "execution_code",
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic,
            args,
            contract,
            ..
        } => {
            !(*deterministic
                && match function_id.as_str() {
                    "startsWith" => {
                        function_contract_is_coded_safe(contract)
                            && starts_with_args_are_kernel_safe(args)
                    }
                    "length" => {
                        function_contract_is_coded_safe(contract)
                            && length_args_are_kernel_safe(args)
                    }
                    "lower" | "lowercase" | "upper" | "uppercase" | "trim" => {
                        function_contract_is_coded_safe(contract)
                            && string_scalar_args_are_kernel_safe(args)
                    }
                    "isNull" | "isNotNull" => null_check_args_are_kernel_safe(args),
                    "identity" => identity_bool_args_are_kernel_safe(args),
                    "cast" => identity_cast_args_are_kernel_safe(args),
                    "coalesce" => coalesce_bool_args_are_kernel_safe(args),
                    _ => false,
                })
        }
        ResolvedExpr::Conditional { .. } => true,
        ResolvedExpr::TableExists(_) => true,
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => false,
        ResolvedExpr::AggregateCall { .. } | ResolvedExpr::Literal(_) => false,
    }
}

fn path_literal_compare_is_kernel_incompatible(
    planned: &crate::PlannedQuery,
    left: &ResolvedExpr,
    right: &ResolvedExpr,
) -> bool {
    path_literal_compare_parts(left, right).is_some_and(|(path, literal)| {
        !path_literal_has_compatible_contract_in_scope(planned, path, literal)
    })
}

fn in_list_is_kernel_incompatible(
    planned: &crate::PlannedQuery,
    expr: &ResolvedExpr,
    values: &[crate::ResolvedLiteral],
) -> bool {
    let ResolvedExpr::Path(path) = expr else {
        return true;
    };
    values
        .iter()
        .any(|literal| !path_literal_has_compatible_contract_in_scope(planned, path, literal))
}

fn starts_with_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(literal)] = args else {
        return false;
    };
    matches!(path.logical_type.as_str(), "utf8" | "string" | "json")
        && path.physical_kind != "execution_code"
        && matches!(literal.typed_value, crate::ResolvedLiteralValue::String(_))
}

fn length_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    matches!(
        args,
        [ResolvedExpr::Path(path)]
            if matches!(path.logical_type.as_str(), "utf8" | "string")
                && path.physical_kind != "execution_code"
    )
}

fn string_scalar_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    matches!(
        args,
        [ResolvedExpr::Path(path)]
            if matches!(path.logical_type.as_str(), "utf8" | "string")
                && path.physical_kind != "execution_code"
    )
}

fn null_check_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    matches!(args, [ResolvedExpr::Path(path)] if path.physical_kind != "execution_code")
}

fn identity_bool_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    matches!(
        args,
        [ResolvedExpr::Path(path)]
            if matches!(path.logical_type.as_str(), "bool" | "boolean")
                && path.physical_kind != "execution_code"
    )
}

fn coalesce_bool_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    !args.is_empty()
        && args.iter().all(|arg| match arg {
            ResolvedExpr::Path(path) => {
                matches!(path.logical_type.as_str(), "bool" | "boolean")
                    && path.physical_kind != "execution_code"
            }
            ResolvedExpr::Literal(literal) => matches!(
                literal.typed_value,
                crate::ResolvedLiteralValue::Boolean(_) | crate::ResolvedLiteralValue::Null
            ),
            _ => false,
        })
}

fn bool_expr_literal_compare_is_kernel_safe(
    left: &ResolvedExpr,
    op: crate::AstCompareOp,
    right: &ResolvedExpr,
) -> bool {
    if !matches!(op, crate::AstCompareOp::Eq | crate::AstCompareOp::Ne) {
        return false;
    }
    match (left, right) {
        (expr, ResolvedExpr::Literal(literal)) | (ResolvedExpr::Literal(literal), expr)
            if matches!(literal.typed_value, crate::ResolvedLiteralValue::Boolean(_)) =>
        {
            bool_expr_is_kernel_safe(expr)
        }
        _ => false,
    }
}

fn length_expr_literal_compare_is_kernel_safe(
    left: &ResolvedExpr,
    _op: crate::AstCompareOp,
    right: &ResolvedExpr,
) -> bool {
    match (left, right) {
        (expr, ResolvedExpr::Literal(literal)) | (ResolvedExpr::Literal(literal), expr)
            if matches!(
                literal.typed_value,
                crate::ResolvedLiteralValue::SignedInteger(_)
                    | crate::ResolvedLiteralValue::UnsignedInteger(_)
                    | crate::ResolvedLiteralValue::Decimal { .. }
            ) =>
        {
            matches!(
                expr,
                ResolvedExpr::FunctionCall {
                    function_id,
                    deterministic: true,
                    args,
                    contract,
                    ..
                } if function_id == "length"
                    && function_contract_is_coded_safe(contract)
                    && length_args_are_kernel_safe(args)
            )
        }
        _ => false,
    }
}

fn string_function_literal_compare_is_kernel_safe(
    left: &ResolvedExpr,
    op: crate::AstCompareOp,
    right: &ResolvedExpr,
) -> bool {
    if !matches!(op, crate::AstCompareOp::Eq | crate::AstCompareOp::Ne) {
        return false;
    }
    match (left, right) {
        (expr, ResolvedExpr::Literal(literal)) | (ResolvedExpr::Literal(literal), expr)
            if matches!(literal.typed_value, crate::ResolvedLiteralValue::String(_)) =>
        {
            matches!(
                expr,
                ResolvedExpr::FunctionCall {
                    function_id,
                    deterministic: true,
                    args,
                    contract,
                    ..
                } if matches!(
                    function_id.as_str(),
                    "lower" | "lowercase" | "upper" | "uppercase" | "trim"
                ) && function_contract_is_coded_safe(contract)
                    && string_scalar_args_are_kernel_safe(args)
            )
        }
        _ => false,
    }
}

fn identity_cast_literal_compare_is_kernel_safe(
    planned: &crate::PlannedQuery,
    left: &ResolvedExpr,
    _op: crate::AstCompareOp,
    right: &ResolvedExpr,
) -> bool {
    if let (Some(path), ResolvedExpr::Literal(literal)) = (identity_cast_path(left), right) {
        return path_literal_has_compatible_contract_in_scope(planned, path, literal);
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) = (left, identity_cast_path(right)) {
        return path_literal_has_compatible_contract_in_scope(planned, path, literal);
    }
    false
}

fn identity_cast_path(expr: &ResolvedExpr) -> Option<&crate::ResolvedPath> {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic: true,
            args,
            ..
        } if function_id == "cast" && identity_cast_args_are_kernel_safe(args) => {
            let [ResolvedExpr::Path(path), ResolvedExpr::Literal(_)] = args.as_slice() else {
                return None;
            };
            Some(path)
        }
        _ => None,
    }
}

fn path_or_identity_cast_path(expr: &ResolvedExpr) -> Option<&crate::ResolvedPath> {
    match expr {
        ResolvedExpr::Path(path) => Some(path),
        _ => identity_cast_path(expr),
    }
}

fn identity_cast_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(target)] = args else {
        return false;
    };
    let crate::ResolvedLiteralValue::String(target) = &target.typed_value else {
        return false;
    };
    normalized_cast_type(path.logical_type.as_str()) == normalized_cast_type(target)
        && path.physical_kind != "execution_code"
}

fn identity_cast_bool_args_are_kernel_safe(args: &[ResolvedExpr]) -> bool {
    let [ResolvedExpr::Path(path), ResolvedExpr::Literal(_)] = args else {
        return false;
    };
    matches!(path.logical_type.as_str(), "bool" | "boolean")
        && identity_cast_args_are_kernel_safe(args)
}

fn normalized_cast_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "string" | "utf8" => "utf8",
        other => other,
    }
}

fn path_literal_has_compatible_contract(
    path: &crate::ResolvedPath,
    literal: &crate::ResolvedLiteral,
) -> bool {
    if matches!(literal.typed_value, crate::ResolvedLiteralValue::Null) {
        return true;
    }
    if let Some(system) = &path.system_field {
        return match system {
            crate::ResolvedSystemField::Goid => literal_is_string_like(literal),
            crate::ResolvedSystemField::BranchKey | crate::ResolvedSystemField::Csn => {
                literal_is_integer_like(literal)
            }
            crate::ResolvedSystemField::TimestampUs => literal_is_temporal_or_integer_like(literal),
            _ => false,
        };
    }
    match normalized_path_type(path.logical_type.as_str()) {
        "bool" => matches!(literal.typed_value, crate::ResolvedLiteralValue::Boolean(_)),
        "utf8" => literal_is_string_like(literal),
        "uuid" => literal_is_uuid_like(literal),
        "numeric" => literal_is_numeric_like(literal),
        _ => false,
    }
}

fn path_literal_has_compatible_contract_in_scope(
    planned: &crate::PlannedQuery,
    path: &crate::ResolvedPath,
    literal: &crate::ResolvedLiteral,
) -> bool {
    path_literal_has_compatible_contract(path, literal)
        && (path.physical_kind != "file_code"
            || dataset_has_exact_code_domain_bridge(&planned.resolved.operation_context.dataset))
}

fn normalized_path_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "utf8" | "string" | "json" => "utf8",
        "uuid" => "uuid",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "decimal128" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "numeric",
        other => other,
    }
}

fn literal_is_string_like(literal: &crate::ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        crate::ResolvedLiteralValue::String(_)
            | crate::ResolvedLiteralValue::Uuid { .. }
            | crate::ResolvedLiteralValue::Binary { .. }
    )
}

fn literal_is_uuid_like(literal: &crate::ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        crate::ResolvedLiteralValue::Uuid { .. } | crate::ResolvedLiteralValue::String(_)
    )
}

fn literal_is_integer_like(literal: &crate::ResolvedLiteral) -> bool {
    matches!(
        literal.typed_value,
        crate::ResolvedLiteralValue::SignedInteger(_)
            | crate::ResolvedLiteralValue::UnsignedInteger(_)
    )
}

fn literal_is_temporal_or_integer_like(literal: &crate::ResolvedLiteral) -> bool {
    literal_is_integer_like(literal)
        || matches!(
            literal.typed_value,
            crate::ResolvedLiteralValue::TimestampMicros { .. }
        )
}

fn literal_is_numeric_like(literal: &crate::ResolvedLiteral) -> bool {
    literal_is_integer_like(literal)
        || matches!(
            literal.typed_value,
            crate::ResolvedLiteralValue::Decimal { .. }
                | crate::ResolvedLiteralValue::TimestampMicros { .. }
        )
}

fn string_compare_function_id(expr: &ResolvedExpr) -> Option<&str> {
    match expr {
        ResolvedExpr::FunctionCall { function_id, .. }
            if matches!(
                function_id.as_str(),
                "lower" | "lowercase" | "upper" | "uppercase" | "trim"
            ) =>
        {
            Some(function_id.as_str())
        }
        _ => None,
    }
}

fn bool_expr_is_kernel_safe(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Path(path) => matches!(path.logical_type.as_str(), "bool" | "boolean"),
        ResolvedExpr::FunctionCall {
            function_id,
            deterministic,
            args,
            contract,
            ..
        } => {
            *deterministic
                && function_contract_is_coded_safe(contract)
                && match function_id.as_str() {
                    "startsWith" => starts_with_args_are_kernel_safe(args),
                    "isNull" | "isNotNull" => null_check_args_are_kernel_safe(args),
                    "identity" => identity_bool_args_are_kernel_safe(args),
                    "coalesce" => coalesce_bool_args_are_kernel_safe(args),
                    _ => false,
                }
        }
        _ => false,
    }
}

fn expression_contains_association_or_evidence(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => true,
        ResolvedExpr::FunctionCall { args, .. } => {
            args.iter().any(expression_contains_association_or_evidence)
        }
        ResolvedExpr::AggregateCall { arg, .. } => arg
            .as_deref()
            .is_some_and(expression_contains_association_or_evidence),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_association_or_evidence(Some(predicate))
                || expression_contains_association_or_evidence(then_expr)
                || expression_contains_association_or_evidence(else_expr)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coded_operator_contract_serializes_existing_string_shape() {
        let contract = CodedOperatorContract::new(
            "root_scan",
            CodedRepresentationClass::CodePure,
            true,
            false,
            "exact coded root scan",
        )
        .with_row_grain("visible_rows_after_reconstruction")
        .with_required_metadata(&["code_domains"])
        .with_fallback_boundary("materialized_residual_verification");
        let value = serde_json::to_value(&contract).unwrap();

        assert_eq!(
            value["contract_version"],
            crate::CODED_OPERATOR_CONTRACT_VERSION
        );
        assert_eq!(value["operator"], "root_scan");
        assert_eq!(value["representation_class"], "code_pure");
        assert_eq!(value["row_grain"], "visible_rows_after_reconstruction");
        assert_eq!(value["required_metadata"], json!(["code_domains"]));
        assert_eq!(
            value["fallback_boundary"],
            "materialized_residual_verification"
        );
    }
}
