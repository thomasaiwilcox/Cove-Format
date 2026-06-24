use crate::{
    CoveOLogicalPlan, LogicalPlanDependencySet, LogicalPredicateForm, MetadataDisclosurePolicy,
};
use serde_json::{json, Value};

pub fn logical_plan_json(plan: &CoveOLogicalPlan, disclosure: MetadataDisclosurePolicy) -> Value {
    let mut value = serde_json::to_value(plan).expect("logical plan serializes");
    if disclosure != MetadataDisclosurePolicy::AllowProtected {
        crate::explain::redact_explain_value(&mut value);
    }
    value
}

pub fn predicate_forms_json(
    forms: &[LogicalPredicateForm],
    disclosure: MetadataDisclosurePolicy,
) -> Value {
    let mut value = serde_json::to_value(forms).expect("predicate forms serialize");
    if disclosure != MetadataDisclosurePolicy::AllowProtected {
        crate::explain::redact_explain_value(&mut value);
    }
    value
}

pub fn dependencies_json(
    dependencies: &LogicalPlanDependencySet,
    disclosure: MetadataDisclosurePolicy,
) -> Value {
    if disclosure == MetadataDisclosurePolicy::AllowProtected {
        serde_json::to_value(dependencies).expect("dependencies serialize")
    } else {
        json!({
            "object_type_count": dependencies.object_type_ids.len(),
            "property_count": dependencies.property_ids.len(),
            "association_type_count": dependencies.association_type_ids.len(),
            "projection_count": dependencies.projection_ids.len(),
            "projection_column_count": dependencies.projection_columns.len(),
            "projection_contract_count": dependencies.projection_contracts.len(),
            "evidence_field_count": dependencies.evidence_fields.len(),
            "system_fields": dependencies.system_fields,
            "deterministic_function_count": dependencies.deterministic_function_ids.len(),
            "aggregate_kinds": dependencies.aggregate_kinds,
            "code_domain_count": dependencies.code_domains.len(),
            "redacted": true,
        })
    }
}

pub fn logical_plan_text(plan: &CoveOLogicalPlan, disclosure: MetadataDisclosurePolicy) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "logical_plan fingerprint={}",
        plan.logical_plan_fingerprint
    ));
    lines.push(format!(
        "context root={:?} grain={:?} output={:?}",
        plan.context.root_kind, plan.context.scan_grain, plan.context.output_mode
    ));
    lines.push(format!("canonical_order={:?}", plan.canonical_order));
    for node in &plan.nodes {
        let name = serde_json::to_value(&node.kind)
            .ok()
            .and_then(|value| {
                value
                    .as_object()
                    .and_then(|object| object.get("kind"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "unknown".into());
        if disclosure == MetadataDisclosurePolicy::AllowProtected {
            lines.push(format!("{} {name} {:?}", node.id.0, node.kind));
        } else {
            lines.push(format!("{} {name}", node.id.0));
        }
    }
    if !plan.decode_boundaries.is_empty() {
        lines.push(format!(
            "decode_boundaries count={}",
            plan.decode_boundaries.len()
        ));
    }
    if !plan.residual_predicates.is_empty() {
        lines.push(format!(
            "residual_predicates count={}",
            plan.residual_predicates.len()
        ));
    }
    lines.join("\n")
}
