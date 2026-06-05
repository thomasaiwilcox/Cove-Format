use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::PlannedQuery;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageReuseReport {
    pub enabled: bool,
    pub projection_dependency_reuse: bool,
    pub evidence_dependency_reuse: bool,
    pub requested_evidence_fields: usize,
    pub requested_projection_columns: usize,
    pub reused_evidence_fields: Vec<String>,
    pub reused_projection_columns: Vec<String>,
    pub fallback_reasons: Vec<String>,
}

impl LineageReuseReport {
    pub fn for_plan(planned: &PlannedQuery) -> Self {
        let requested_evidence_fields = planned.dependencies.evidence_fields.len();
        let requested_projection_columns = planned.dependencies.projection_columns.len();
        let projection_dependency_reuse = !planned.dependencies.projection_ids.is_empty()
            && (requested_evidence_fields > 0 || requested_projection_columns > 0);
        let evidence_dependency_reuse =
            requested_evidence_fields > 0 && !planned.dependencies.projection_ids.is_empty();
        let reused_evidence_fields = planned
            .dependencies
            .evidence_fields
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let reused_projection_columns = planned
            .dependencies
            .projection_columns
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut report = Self {
            enabled: projection_dependency_reuse || evidence_dependency_reuse,
            projection_dependency_reuse,
            evidence_dependency_reuse,
            requested_evidence_fields,
            requested_projection_columns,
            reused_evidence_fields,
            reused_projection_columns,
            fallback_reasons: Vec::new(),
        };
        if requested_evidence_fields > 0 && planned.dependencies.projection_ids.is_empty() {
            report
                .fallback_reasons
                .push("lineage_dependency_without_projection".into());
        }
        report
    }

    pub fn to_json(&self, allow_protected: bool) -> Value {
        if allow_protected {
            return serde_json::to_value(self).unwrap_or(Value::Null);
        }
        json!({
            "enabled": self.enabled,
            "projection_dependency_reuse": self.projection_dependency_reuse,
            "evidence_dependency_reuse": self.evidence_dependency_reuse,
            "requested_evidence_fields_present": self.requested_evidence_fields > 0,
            "requested_projection_columns_present": self.requested_projection_columns > 0,
            "reused_evidence_field_count": self.reused_evidence_fields.len(),
            "reused_projection_column_count": self.reused_projection_columns.len(),
            "fallback_reasons": self.fallback_reasons,
        })
    }
}
