use super::*;

pub(super) fn enforce_materialized_branch_policy(
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

pub(super) fn execution_row_branch_key(row: &ExecutionRow) -> Option<u64> {
    match row {
        ExecutionRow::Object(row) => Some(row.branch_key),
        ExecutionRow::Association(row) => Some(row.branch_key),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => None,
    }
}

pub(super) fn apply_visibility_overlay(
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

pub(super) fn enforce_redaction_policy(
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

pub(super) fn execution_row_has_redacted_values(row: &ExecutionRow) -> bool {
    match row {
        ExecutionRow::Object(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Association(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => false,
    }
}

pub(super) fn row_visible_in_overlay(row: &ExecutionRow, overlay: &VisibilityOverlay) -> bool {
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

pub(super) fn filter_association_context_rows(
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

pub(super) fn filter_evidence_context_rows(
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

pub(super) fn filter_object_context_rows(
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

pub(super) fn active_visibility_overlay<'a>(
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

pub(super) fn association_row_visible_in_overlay(
    row: &MaterializedAssociationRow,
    overlay: &VisibilityOverlay,
) -> bool {
    overlay.visible_goids.contains(&row.goid) || overlay.visible_record_ids.contains(&row.record_id)
}

pub(super) fn object_row_visible_in_overlay(
    row: &MaterializedObjectRow,
    overlay: &VisibilityOverlay,
) -> bool {
    overlay.visible_goids.contains(&row.goid) || overlay.visible_record_ids.contains(&row.record_id)
}

pub(super) fn evidence_row_visible_in_overlay(
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

pub(super) fn evidence_field_visible_in_overlay(
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
