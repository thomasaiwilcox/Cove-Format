use serde::{Deserialize, Serialize};

use crate::{
    predicate::{
        FilterClassification, LogicalPredicateForm, LogicalPredicateKind, PredicatePlacement,
        PredicateProofState, RepresentationClass,
    },
    AstCompareOp, CodeDomainId, MetadataDisclosurePolicy, PlannedQuery,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPredicateNormalForms {
    pub normal_form_version: String,
    pub ast_forms: Vec<PhysicalPredicateForm>,
    pub cnf_forms: Vec<PhysicalPredicateForm>,
    pub interval_forms: Vec<PhysicalPredicateForm>,
    pub encoded_forms: Vec<PhysicalPredicateForm>,
    pub coverage_forms: Vec<PhysicalPredicateForm>,
    pub residual_forms: Vec<PhysicalPredicateForm>,
    pub decode_boundaries: Vec<String>,
    pub code_domains: Vec<PhysicalCodeDomainDescriptor>,
}

impl PhysicalPredicateNormalForms {
    pub fn form_count(&self) -> usize {
        self.ast_forms.len()
            + self.cnf_forms.len()
            + self.interval_forms.len()
            + self.encoded_forms.len()
            + self.coverage_forms.len()
            + self.residual_forms.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalPredicateForm {
    pub form_id: u32,
    pub kind: PhysicalPredicateFormKind,
    pub representation: PhysicalRepresentationClass,
    pub placement: PredicatePlacement,
    pub classification: FilterClassification,
    pub logical_type: Option<String>,
    pub physical_kind: Option<String>,
    pub collation_id: Option<u16>,
    pub null_policy: Option<String>,
    pub code_domain: Option<PhysicalCodeDomainDescriptor>,
    pub operator: Option<String>,
    pub literal_count: usize,
    pub exact: bool,
    pub proof_state: PredicateProofState,
    pub proof_required: bool,
    pub residual_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalPredicateFormKind {
    Ast,
    CnfConjunct,
    Interval,
    Encoded,
    CoverageCompatible,
    Residual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalRepresentationClass {
    CodePure,
    FileCodeLiteral,
    ExecutionCodeRemapped,
    NumericCoded,
    DictionaryLifted,
    CoverageOnly,
    DecodeBoundary,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalCodeDomainDescriptor {
    pub file_id: String,
    pub snapshot_id: Option<String>,
    pub dictionary_id: Option<String>,
    pub object_type_id: Option<u32>,
    pub property_id: Option<u32>,
    pub projection_id: Option<String>,
    pub field: Option<String>,
    pub logical_type: Option<String>,
    pub physical_kind: Option<String>,
    pub collation_id: Option<u16>,
    pub null_policy: Option<String>,
    pub semantic_domain_id: Option<String>,
    pub dictionary_epoch: Option<u64>,
    pub security_scope: SecurityScopeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalExecutionCodeDomainDescriptor {
    pub engine_profile_id: Option<String>,
    pub code_space_id: Option<String>,
    pub comparison_scope: Option<String>,
    pub lifetime: Option<String>,
    pub epoch: Option<u64>,
    pub null_code_policy: Option<String>,
    pub semantic_domain_id: Option<String>,
    pub security_scope: SecurityScopeDescriptor,
    pub validated: bool,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityScopeDescriptor {
    pub tenant_id: Option<String>,
    pub principal_scope: Option<String>,
    pub visibility_policy: String,
    pub redaction_policy: String,
    pub metadata_disclosure_policy: MetadataDisclosurePolicy,
}

pub(crate) fn build_predicate_normal_forms(
    planned: &PlannedQuery,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
) -> PhysicalPredicateNormalForms {
    let mut out = PhysicalPredicateNormalForms {
        normal_form_version: crate::PREDICATE_NORMAL_FORM_VERSION.into(),
        ..PhysicalPredicateNormalForms::default()
    };
    let mut next_id = 0u32;
    for form in &planned.logical_plan.predicate_forms {
        record_form(
            form,
            planned,
            enable_coverage_candidates,
            enable_execution_code_candidates,
            &mut next_id,
            &mut out,
        );
    }
    out.decode_boundaries = planned.logical_plan.decode_boundaries.clone();
    dedup_code_domains(&mut out.code_domains);
    out
}

fn record_form(
    form: &LogicalPredicateForm,
    planned: &PlannedQuery,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
    next_id: &mut u32,
    out: &mut PhysicalPredicateNormalForms,
) {
    let ast = physical_form(
        *next_id,
        PhysicalPredicateFormKind::Ast,
        form,
        planned,
        enable_coverage_candidates,
        enable_execution_code_candidates,
    );
    *next_id += 1;
    if let Some(domain) = &ast.code_domain {
        out.code_domains.push(domain.clone());
    }
    out.ast_forms.push(ast.clone());
    record_coverage_form(
        form,
        planned,
        enable_coverage_candidates,
        enable_execution_code_candidates,
        next_id,
        out,
    );

    match &form.kind {
        LogicalPredicateKind::And(parts) => {
            for part in parts {
                let cnf = physical_form(
                    *next_id,
                    PhysicalPredicateFormKind::CnfConjunct,
                    part,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                );
                *next_id += 1;
                out.cnf_forms.push(cnf);
                record_coverage_form(
                    part,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                    next_id,
                    out,
                );
            }
        }
        LogicalPredicateKind::Or(parts) => {
            if ast.exact && ast.representation != PhysicalRepresentationClass::DecodeBoundary {
                let encoded = physical_form(
                    *next_id,
                    PhysicalPredicateFormKind::Encoded,
                    form,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                );
                *next_id += 1;
                out.encoded_forms.push(encoded);
            } else if enable_coverage_candidates {
                let mut coverage = ast.clone();
                coverage.form_id = *next_id;
                coverage.kind = PhysicalPredicateFormKind::Residual;
                coverage.representation = if form.placement == PredicatePlacement::PreReconstruction
                    && form.representation.representation == RepresentationClass::CodePure
                {
                    PhysicalRepresentationClass::CodePure
                } else {
                    PhysicalRepresentationClass::CoverageOnly
                };
                coverage.exact = false;
                coverage.proof_required = true;
                coverage.residual_reason = form.residual_reason.clone().or_else(|| {
                    Some("OR requires compatible no-false-negative coverage proof".into())
                });
                *next_id += 1;
                out.residual_forms.push(coverage);
            }
            for part in parts {
                record_form(
                    part,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                    next_id,
                    out,
                );
            }
        }
        LogicalPredicateKind::Not(inner) => {
            if ast.exact && ast.representation != PhysicalRepresentationClass::DecodeBoundary {
                let encoded = physical_form(
                    *next_id,
                    PhysicalPredicateFormKind::Encoded,
                    form,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                );
                *next_id += 1;
                out.encoded_forms.push(encoded);
            } else {
                out.residual_forms.push(residual_form(
                    next_id,
                    inner,
                    planned,
                    "NOT requires complement-proof metadata before coded pruning",
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                ));
            }
        }
        _ => {
            if can_be_interval(form) {
                let interval = physical_form(
                    *next_id,
                    PhysicalPredicateFormKind::Interval,
                    form,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                );
                *next_id += 1;
                out.interval_forms.push(interval);
            }
            if ast.exact && ast.representation != PhysicalRepresentationClass::DecodeBoundary {
                let encoded = physical_form(
                    *next_id,
                    PhysicalPredicateFormKind::Encoded,
                    form,
                    planned,
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                );
                *next_id += 1;
                out.encoded_forms.push(encoded);
            } else {
                out.residual_forms.push(residual_form(
                    next_id,
                    form,
                    planned,
                    form.residual_reason
                        .as_deref()
                        .unwrap_or("predicate remains materialized residual"),
                    enable_coverage_candidates,
                    enable_execution_code_candidates,
                ));
            }
        }
    }
}

fn record_coverage_form(
    form: &LogicalPredicateForm,
    planned: &PlannedQuery,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
    next_id: &mut u32,
    out: &mut PhysicalPredicateNormalForms,
) {
    if !enable_coverage_candidates || !can_be_coverage_compatible(form) {
        return;
    }
    let mut coverage = physical_form(
        *next_id,
        PhysicalPredicateFormKind::CoverageCompatible,
        form,
        planned,
        enable_coverage_candidates,
        enable_execution_code_candidates,
    );
    *next_id += 1;
    coverage.representation = PhysicalRepresentationClass::CoverageOnly;
    coverage.exact = false;
    coverage.proof_state = PredicateProofState::CandidateNeedsResidual;
    coverage.proof_required = true;
    coverage.residual_reason = Some(
        form.residual_reason.clone().unwrap_or_else(|| {
            "coverage-compatible predicate requires validated no-false-negative proof metadata before pruning".into()
        }),
    );
    out.coverage_forms.push(coverage);
}

fn residual_form(
    next_id: &mut u32,
    form: &LogicalPredicateForm,
    planned: &PlannedQuery,
    reason: &str,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
) -> PhysicalPredicateForm {
    let mut residual = physical_form(
        *next_id,
        PhysicalPredicateFormKind::Residual,
        form,
        planned,
        enable_coverage_candidates,
        enable_execution_code_candidates,
    );
    *next_id += 1;
    residual.exact = false;
    residual.proof_state = PredicateProofState::CandidateNeedsResidual;
    residual.proof_required = true;
    residual.residual_reason = Some(reason.into());
    residual
}

fn physical_form(
    form_id: u32,
    kind: PhysicalPredicateFormKind,
    form: &LogicalPredicateForm,
    planned: &PlannedQuery,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
) -> PhysicalPredicateForm {
    let representation = representation_class(
        form,
        kind,
        enable_coverage_candidates,
        enable_execution_code_candidates,
    );
    let exact = form.representation.proof_state == PredicateProofState::ProvenExact
        && form.representation.exact
        && matches!(
            representation,
            PhysicalRepresentationClass::CodePure
                | PhysicalRepresentationClass::FileCodeLiteral
                | PhysicalRepresentationClass::NumericCoded
                | PhysicalRepresentationClass::DictionaryLifted
                | PhysicalRepresentationClass::ExecutionCodeRemapped
        );
    PhysicalPredicateForm {
        form_id,
        kind,
        representation,
        placement: form.placement,
        classification: form.classification,
        logical_type: form.representation.logical_type.clone(),
        physical_kind: form.representation.physical_kind.clone(),
        collation_id: form.representation.collation_id,
        null_policy: form.representation.null_policy.clone(),
        code_domain: form
            .representation
            .code_domain_id
            .as_ref()
            .map(|domain| physical_code_domain(domain, form, planned)),
        operator: Some(operator_name(&form.kind)),
        literal_count: literal_count(&form.kind),
        exact,
        proof_state: form.representation.proof_state,
        proof_required: form.representation.proof_state != PredicateProofState::ProvenExact,
        residual_reason: form.residual_reason.clone(),
    }
}

fn representation_class(
    form: &LogicalPredicateForm,
    kind: PhysicalPredicateFormKind,
    enable_coverage_candidates: bool,
    enable_execution_code_candidates: bool,
) -> PhysicalRepresentationClass {
    if matches!(kind, PhysicalPredicateFormKind::CoverageCompatible) {
        return PhysicalRepresentationClass::CoverageOnly;
    }
    if matches!(kind, PhysicalPredicateFormKind::Residual) {
        if form.representation.physical_kind.as_deref() == Some("file_code") {
            return PhysicalRepresentationClass::FileCodeLiteral;
        }
        return if enable_coverage_candidates && matches!(form.kind, LogicalPredicateKind::Or(_)) {
            PhysicalRepresentationClass::CoverageOnly
        } else {
            PhysicalRepresentationClass::DecodeBoundary
        };
    }
    let physical_kind = form
        .representation
        .physical_kind
        .as_deref()
        .unwrap_or_default();
    if enable_execution_code_candidates
        && matches!(
            form.representation.representation,
            RepresentationClass::CrossSourceBridgeCandidate
        )
    {
        return PhysicalRepresentationClass::ExecutionCodeRemapped;
    }
    match form.representation.representation {
        RepresentationClass::CodePure => {
            if physical_kind == "file_code" {
                PhysicalRepresentationClass::FileCodeLiteral
            } else {
                PhysicalRepresentationClass::CodePure
            }
        }
        RepresentationClass::TypedNumeric => PhysicalRepresentationClass::NumericCoded,
        RepresentationClass::DictionaryLiftedCandidate => {
            PhysicalRepresentationClass::DictionaryLifted
        }
        RepresentationClass::OrdinalMapCandidate => PhysicalRepresentationClass::DecodeBoundary,
        RepresentationClass::DecodeBoundary => {
            if physical_kind == "file_code" {
                PhysicalRepresentationClass::FileCodeLiteral
            } else {
                PhysicalRepresentationClass::DecodeBoundary
            }
        }
        RepresentationClass::CrossSourceBridgeCandidate => PhysicalRepresentationClass::Unsupported,
        RepresentationClass::ResidualMaterialized | RepresentationClass::NonBeneficial => {
            PhysicalRepresentationClass::Unsupported
        }
    }
}

fn physical_code_domain(
    domain: &CodeDomainId,
    form: &LogicalPredicateForm,
    planned: &PlannedQuery,
) -> PhysicalCodeDomainDescriptor {
    let (root, object_type_id, property_id, projection_id, field) = match domain {
        CodeDomainId::Placeholder {
            root,
            object_type_id,
            property_id,
            projection_id,
            field,
        } => (
            root.clone(),
            *object_type_id,
            *property_id,
            projection_id.clone(),
            field.clone(),
        ),
    };
    let (dictionary_id, semantic_domain_id, dictionary_epoch) = physical_dictionary_domain(
        planned,
        &root,
        object_type_id,
        property_id,
        projection_id.as_deref(),
        field.as_deref(),
        form,
    );
    PhysicalCodeDomainDescriptor {
        file_id: physical_code_domain_file_id(planned),
        snapshot_id: planned
            .resolved
            .operation_context
            .snapshot
            .snapshot_id
            .clone(),
        dictionary_id,
        object_type_id,
        property_id,
        projection_id,
        field,
        logical_type: form.representation.logical_type.clone(),
        physical_kind: form.representation.physical_kind.clone(),
        collation_id: form.representation.collation_id,
        null_policy: form.representation.null_policy.clone(),
        semantic_domain_id,
        dictionary_epoch,
        security_scope: security_scope_descriptor(planned),
    }
}

fn physical_code_domain_file_id(planned: &PlannedQuery) -> String {
    let dataset = &planned.resolved.operation_context.dataset;
    if dataset.files.len() > 1 {
        return format!("dataset:{}", dataset.file_membership_fingerprint);
    }
    let file_id = dataset
        .files
        .first()
        .map(|file| file.file_id)
        .unwrap_or(planned.resolved.operation_context.file.file_id);
    crate::hex_lower(&file_id)
}

fn physical_dictionary_domain(
    planned: &PlannedQuery,
    root: &str,
    object_type_id: Option<u32>,
    property_id: Option<u32>,
    projection_id: Option<&str>,
    field: Option<&str>,
    form: &LogicalPredicateForm,
) -> (Option<String>, Option<String>, Option<u64>) {
    if form.representation.physical_kind.as_deref() != Some("file_code") {
        return (None, None, None);
    }

    let dataset = &planned.resolved.operation_context.dataset;
    if let Some(bridge) = dataset
        .code_domain_bridges
        .iter()
        .find(|bridge| bridge.exact)
    {
        let scope = dataset
            .manifest_id
            .as_deref()
            .or(dataset.dataset_id.as_deref())
            .unwrap_or(dataset.file_membership_fingerprint.as_str());
        return (
            Some(format!(
                "{scope}:bridged_dictionary_domain:{}",
                descriptor_component(&bridge.domain_id)
            )),
            Some(bridge.domain_id.clone()),
            bridge.epoch,
        );
    }

    if dataset.files.len() <= 1 {
        let file_id = dataset
            .files
            .first()
            .map(|file| file.file_id)
            .unwrap_or(planned.resolved.operation_context.file.file_id);
        let dictionary_id = format!("file:{}:dictionary", crate::hex_lower(&file_id));
        let dictionary_epoch = dataset
            .dictionary_epochs
            .iter()
            .find(|epoch| epoch.domain_id == dictionary_id)
            .and_then(|epoch| epoch.epoch);
        return (Some(dictionary_id), None, dictionary_epoch);
    }

    (
        Some(format!(
            "dataset:{}:unbridged_file_dictionaries:{}",
            dataset.file_membership_fingerprint,
            physical_domain_path_key(root, object_type_id, property_id, projection_id, field)
        )),
        None,
        None,
    )
}

fn physical_domain_path_key(
    root: &str,
    object_type_id: Option<u32>,
    property_id: Option<u32>,
    projection_id: Option<&str>,
    field: Option<&str>,
) -> String {
    let mut parts = vec![format!("root={}", descriptor_component(root))];
    if let Some(object_type_id) = object_type_id {
        parts.push(format!("object_type={object_type_id}"));
    }
    if let Some(property_id) = property_id {
        parts.push(format!("property={property_id}"));
    }
    if let Some(projection_id) = projection_id {
        parts.push(format!(
            "projection={}",
            descriptor_component(projection_id)
        ));
    }
    if let Some(field) = field {
        parts.push(format!("field={}", descriptor_component(field)));
    }
    parts.join(":")
}

fn descriptor_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn security_scope_descriptor(planned: &PlannedQuery) -> SecurityScopeDescriptor {
    let security = &planned.resolved.operation_context.security;
    let dataset_security = &planned.resolved.operation_context.dataset.security_scope;
    SecurityScopeDescriptor {
        tenant_id: dataset_security.tenant_id.clone(),
        principal_scope: security.principal_or_session.clone(),
        visibility_policy: format!("{:?}", security.visibility_policy),
        redaction_policy: format!("{:?}", security.redaction_policy),
        metadata_disclosure_policy: security.metadata_disclosure_policy,
    }
}

pub(crate) fn default_execution_code_domain(
    planned: &PlannedQuery,
    validated: bool,
    fallback_reason: Option<String>,
) -> PhysicalExecutionCodeDomainDescriptor {
    PhysicalExecutionCodeDomainDescriptor {
        engine_profile_id: None,
        code_space_id: None,
        comparison_scope: None,
        lifetime: None,
        epoch: None,
        null_code_policy: None,
        semantic_domain_id: None,
        security_scope: security_scope_descriptor(planned),
        validated,
        fallback_reason,
    }
}

fn can_be_interval(form: &LogicalPredicateForm) -> bool {
    match &form.kind {
        LogicalPredicateKind::Compare { op, .. } => {
            matches!(
                op,
                AstCompareOp::Eq
                    | AstCompareOp::Lt
                    | AstCompareOp::Le
                    | AstCompareOp::Gt
                    | AstCompareOp::Ge
            ) && is_numeric_datetime(form.representation.logical_type.as_deref())
        }
        LogicalPredicateKind::InList { .. } => true,
        LogicalPredicateKind::NullCheck { .. } => true,
        _ => false,
    }
}

fn can_be_coverage_compatible(form: &LogicalPredicateForm) -> bool {
    let placement_ok = matches!(
        form.placement,
        PredicatePlacement::PreReconstruction
            | PredicatePlacement::Association
            | PredicatePlacement::Evidence
    );
    let classification_ok = matches!(
        form.classification,
        FilterClassification::System
            | FilterClassification::ObjectType
            | FilterClassification::Temporal
            | FilterClassification::Branch
            | FilterClassification::Tombstone
            | FilterClassification::PropertyCodedCandidate
            | FilterClassification::AssociationSemiJoin
            | FilterClassification::EvidenceResidual
    );
    let predicate_shape_ok = matches!(
        &form.kind,
        LogicalPredicateKind::Compare { .. }
            | LogicalPredicateKind::InList { .. }
            | LogicalPredicateKind::NullCheck { .. }
            | LogicalPredicateKind::Exists { .. }
            | LogicalPredicateKind::BoolExpr { .. }
            | LogicalPredicateKind::And(_)
            | LogicalPredicateKind::Or(_)
    );
    placement_ok && classification_ok && predicate_shape_ok
}

fn is_numeric_datetime(logical_type: Option<&str>) -> bool {
    matches!(
        logical_type,
        Some(
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
                | "date_days"
                | "timestamp_micros"
                | "timestamp_nanos"
        )
    )
}

fn operator_name(kind: &LogicalPredicateKind) -> String {
    match kind {
        LogicalPredicateKind::Compare { op, .. } => format!("{op:?}"),
        LogicalPredicateKind::InList { .. } => "in".into(),
        LogicalPredicateKind::NullCheck { negated, .. } => {
            if *negated { "is_not_null" } else { "is_null" }.into()
        }
        LogicalPredicateKind::Exists { .. } => "exists".into(),
        LogicalPredicateKind::BoolExpr { .. } => "bool".into(),
        LogicalPredicateKind::Not(_) => "not".into(),
        LogicalPredicateKind::And(_) => "and".into(),
        LogicalPredicateKind::Or(_) => "or".into(),
    }
}

fn literal_count(kind: &LogicalPredicateKind) -> usize {
    match kind {
        LogicalPredicateKind::InList { literal_count, .. } => *literal_count,
        LogicalPredicateKind::Compare { right, left, .. } => {
            usize::from(left.starts_with("literal:")) + usize::from(right.starts_with("literal:"))
        }
        LogicalPredicateKind::And(parts) | LogicalPredicateKind::Or(parts) => {
            parts.iter().map(|part| literal_count(&part.kind)).sum()
        }
        LogicalPredicateKind::Not(inner) => literal_count(&inner.kind),
        _ => 0,
    }
}

fn dedup_code_domains(domains: &mut Vec<PhysicalCodeDomainDescriptor>) {
    domains.sort_by(|left, right| {
        (
            &left.file_id,
            &left.dictionary_id,
            left.object_type_id,
            left.property_id,
            &left.projection_id,
            &left.field,
        )
            .cmp(&(
                &right.file_id,
                &right.dictionary_id,
                right.object_type_id,
                right.property_id,
                &right.projection_id,
                &right.field,
            ))
    });
    domains.dedup_by(|left, right| {
        left.file_id == right.file_id
            && left.dictionary_id == right.dictionary_id
            && left.object_type_id == right.object_type_id
            && left.property_id == right.property_id
            && left.projection_id == right.projection_id
            && left.field == right.field
    });
}
