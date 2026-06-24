use serde_json::Value;

use crate::{CoveOPhysicalPlan, MetadataDisclosurePolicy};

pub fn physical_plan_json(plan: &CoveOPhysicalPlan, disclosure: MetadataDisclosurePolicy) -> Value {
    let mut value = serde_json::to_value(plan).expect("physical plan serializes");
    if disclosure != MetadataDisclosurePolicy::AllowProtected {
        crate::explain::redact_explain_value(&mut value);
    }
    value
}

pub fn physical_plan_text(
    plan: &CoveOPhysicalPlan,
    disclosure: MetadataDisclosurePolicy,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "physical_plan fingerprint={}",
        plan.physical_plan_fingerprint
    ));
    lines.push(format!(
        "context root={:?} nodes={} predicate_forms={}",
        plan.root_kind,
        plan.nodes.len(),
        plan.predicate_normal_forms.form_count()
    ));
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
    lines.join("\n")
}
