use super::*;

pub(super) fn try_native_root_scan_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    _options: &ExecutionOptions,
    _started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeRootScanReport,
    )>,
    BuildExecutionError,
> {
    if native_association_root_scan_shape(planned) {
        let ResolvedRoot::Association(root) = &planned.resolved.root else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let ExecutionRow::Association(row) = row else {
                return Ok(None);
            };
            if planned.resolved.operation_context.security.redaction_policy
                == crate::RedactionPolicy::RefuseProtectedValues
                && !row.redacted_properties.is_empty()
            {
                return Err(exec_error(
                    "E_SECURITY_DISCLOSURE_FORBIDDEN",
                    "redacted association values are present but the active redaction policy refuses protected values",
                    json!({}),
                ));
            }
            out.push(row.clone());
        }
        let row_counts = ExecutionRowCounts {
            input_rows: rows.len(),
            filtered_rows: rows.len(),
            output_rows: out.len(),
        };
        return Ok(Some((
            CoveQlExecutionResult::AssociationRows(out),
            row_counts,
            NativeRootScanReport {
                root_kind: "association",
                root_type: root.type_name.clone(),
                rows_returned: rows.len(),
            },
        )));
    }
    if native_evidence_root_scan_shape(planned) {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let ExecutionRow::Evidence(row) = row else {
                return Ok(None);
            };
            out.push(row.clone());
        }
        let row_counts = ExecutionRowCounts {
            input_rows: rows.len(),
            filtered_rows: rows.len(),
            output_rows: out.len(),
        };
        return Ok(Some((
            CoveQlExecutionResult::EvidenceRows(out),
            row_counts,
            NativeRootScanReport {
                root_kind: "evidence",
                root_type: "evidence".into(),
                rows_returned: rows.len(),
            },
        )));
    }
    Ok(None)
}

pub(super) fn try_native_projection_root_scan_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeProjectionRootScanReport,
    )>,
    BuildExecutionError,
> {
    if !native_projection_root_scan_shape(planned) {
        return Ok(None);
    }
    let Some(projection_id) = projection_backed_root_id(&planned.resolved.root) else {
        return Ok(None);
    };
    if !rows
        .iter()
        .all(|row| matches!(row, ExecutionRow::Projection(_)))
    {
        return Ok(None);
    }
    let (result, row_counts) =
        finish_materialized_rows(rows.to_vec(), &[], &[], &[], planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeProjectionRootScanReport {
            projection_id: projection_id.to_string(),
            rows_returned: rows.len(),
        },
    )))
}

pub(super) fn try_native_direct_projection_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    runtime_exact_helper_projection: bool,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeDirectProjectionReport,
    )>,
    BuildExecutionError,
> {
    if !(native_direct_projection_shape(planned)
        || native_helper_exists_direct_projection_shape(planned)
        || native_role_bound_direct_projection_shape(planned)
        || native_temporal_direct_projection_shape(planned)
        || runtime_exact_helper_projection)
    {
        return Ok(None);
    }
    for row in rows {
        if planned.resolved.operation_context.security.redaction_policy
            == crate::RedactionPolicy::RefuseProtectedValues
            && execution_row_has_redacted_values_for_native_projection(row)
        {
            return Err(exec_error(
                "E_SECURITY_DISCLOSURE_FORBIDDEN",
                "redacted values are present but the active redaction policy refuses protected values",
                json!({}),
            ));
        }
    }

    let context = EvalContext::for_plan(associations, evidence_rows, planned);
    let filtered_rows = native_direct_projection_filter_rows(rows, planned)?;
    let filtered_row_count = filtered_rows.len();
    let mut sorted_rows = filtered_rows;
    sort_rows(&mut sorted_rows, planned, &context);
    let projected_rows = apply_skip_take(sorted_rows, planned);

    let json_rows = if let Some(select) = &planned.resolved.method_chain.select {
        let mut out = Vec::with_capacity(projected_rows.len());
        for row in &projected_rows {
            let mut object = serde_json::Map::new();
            for item in select {
                let key = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| native_direct_projection_output_name(&item.expr));
                let value = match &item.expr {
                    ResolvedExpr::Path(path) => row.value_for_path(path),
                    ResolvedExpr::Literal(literal) => literal_value(literal).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!(
                                "native direct projection literal evaluation failed: {}",
                                err.message
                            ),
                            json!({ "column": key }),
                        )
                    })?,
                    _ if native_projection_expr_is_exact(&item.expr) => {
                        eval_expr(&item.expr, row, &context).map_err(|err| {
                            exec_error(
                                "E_EXPRESSION",
                                format!(
                                    "native direct projection expression evaluation failed: {}",
                                    err.message
                                ),
                                json!({ "column": key }),
                            )
                        })?
                    }
                    _ => return Ok(None),
                };
                object.insert(key, value);
            }
            out.push(Value::Object(object));
        }
        out
    } else {
        projected_rows.iter().map(ExecutionRow::to_json).collect()
    };
    let column_count = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .map_or(0, Vec::len);
    let rows_projected = json_rows.len();
    let (result, row_counts) = finish_json_rows(
        json_rows,
        rows.len(),
        filtered_row_count,
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeDirectProjectionReport {
            root_kind: native_root_kind_name(&planned.resolved.root),
            column_count,
            rows_projected,
        },
    )))
}

pub(super) fn native_direct_projection_filter_rows(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let Some(predicate) = planned.resolved.method_chain.where_predicate.as_ref() else {
        return Ok(rows.to_vec());
    };
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Association(_) | ResolvedRoot::Evidence(_)
    ) {
        return Ok(rows.to_vec());
    }
    if !row_root_predicate_is_direct_safe(planned, predicate) {
        return Ok(rows.to_vec());
    }
    rows.iter()
        .filter_map(
            |row| match native_row_root_predicate_matches(row, predicate) {
                Ok(true) => Some(Ok(row.clone())),
                Ok(false) => None,
                Err(err) => Some(Err(err)),
            },
        )
        .collect()
}

pub(super) fn native_row_root_predicate_matches(
    row: &ExecutionRow,
    predicate: &ResolvedPredicate,
) -> Result<bool, BuildExecutionError> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            if let Some((path, literal)) = native_row_root_path_literal_parts(left, right) {
                return native_row_root_compare(row, path, literal, *op, false);
            }
            if let Some((path, literal)) = native_row_root_path_literal_parts(right, left) {
                return native_row_root_compare(row, path, literal, *op, true);
            }
            Ok(false)
        }
        ResolvedPredicate::InList { expr, values } => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            let value = row.value_for_path(path);
            if value.is_null() {
                return Ok(false);
            }
            for literal in values {
                let literal = literal_value(literal).map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!(
                            "native row-root IN predicate literal evaluation failed: {}",
                            err.message
                        ),
                        json!({ "path": path.display_name }),
                    )
                })?;
                if !literal.is_null()
                    && native_row_root_values_compare(&value, &literal, AstCompareOp::Eq, path)
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            Ok(row.value_for_path(path).is_null() ^ *negated)
        }
        ResolvedPredicate::BoolExpr(expr) => {
            let ResolvedExpr::Path(path) = expr else {
                return Ok(false);
            };
            Ok(row.value_for_path(path).as_bool().unwrap_or(false))
        }
        ResolvedPredicate::And(parts) => {
            for part in parts {
                if !native_row_root_predicate_matches(row, part)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ResolvedPredicate::Or(parts) => {
            for part in parts {
                if native_row_root_predicate_matches(row, part)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ResolvedPredicate::Not(_) | ResolvedPredicate::Exists(_) => Ok(false),
    }
}

pub(super) fn runtime_kernel_shape_for_helper_prefilter_proof(
    shape: &KernelShape,
    planned: &crate::PlannedQuery,
    evidence_helper_runtime_exact: bool,
) -> KernelShape {
    if !evidence_helper_runtime_exact
        || !native_evidence_exists_direct_projection_candidate_shape(planned)
    {
        return shape.clone();
    }
    let mut runtime_shape = shape.clone();
    for contract in &mut runtime_shape.operator_contracts {
        if contract.operator == "predicate_exists:evidence" {
            contract.exact = true;
            contract.residual_required = false;
            contract.reason =
                "evidence existence semi-join is the full predicate and the loaded COVE-MAP evidence index proved an exact visible target fast path".into();
            contract.proof_obligation =
                "evidence semi-join is exact because protected metadata disclosure and exact aggregate disclosure are enabled, the helper predicate is positive and owns the whole WHERE clause, hidden evidence entries were filtered from the loaded evidence index, target grain fallback rows are absent, and rows are already reconstructed visible object states".into();
            contract.residual_reason = None;
            contract.fallback_boundary = None;
        }
    }
    if runtime_shape
        .operator_contracts
        .iter()
        .all(|contract| !contract.residual_required)
    {
        runtime_shape.residual_verification_required = false;
        runtime_shape.reason =
            "runtime COVE-MAP evidence index proof promoted helper existence direct projection to exact native authority".into();
    }
    runtime_shape
}

pub(super) fn native_row_root_path_literal_parts<'a>(
    path_expr: &'a ResolvedExpr,
    literal_expr: &'a ResolvedExpr,
) -> Option<(&'a ResolvedPath, &'a ResolvedLiteral)> {
    let ResolvedExpr::Path(path) = path_expr else {
        return None;
    };
    let ResolvedExpr::Literal(literal) = literal_expr else {
        return None;
    };
    Some((path, literal))
}

pub(super) fn native_row_root_compare(
    row: &ExecutionRow,
    path: &ResolvedPath,
    literal: &ResolvedLiteral,
    op: AstCompareOp,
    reversed: bool,
) -> Result<bool, BuildExecutionError> {
    let value = row.value_for_path(path);
    let literal = literal_value(literal).map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!(
                "native row-root comparison literal evaluation failed: {}",
                err.message
            ),
            json!({ "path": path.display_name }),
        )
    })?;
    if value.is_null() || literal.is_null() {
        return Ok(false);
    }
    let op = if reversed {
        match op {
            AstCompareOp::Lt => AstCompareOp::Gt,
            AstCompareOp::Le => AstCompareOp::Ge,
            AstCompareOp::Gt => AstCompareOp::Lt,
            AstCompareOp::Ge => AstCompareOp::Le,
            other => other,
        }
    } else {
        op
    };
    Ok(native_row_root_values_compare(&value, &literal, op, path))
}

pub(super) fn native_row_root_values_compare(
    left: &Value,
    right: &Value,
    op: AstCompareOp,
    path: &ResolvedPath,
) -> bool {
    match op {
        AstCompareOp::Eq => values_equal_typed(left, right, Some(path.logical_type.as_str())),
        AstCompareOp::Ne => !values_equal_typed(left, right, Some(path.logical_type.as_str())),
        AstCompareOp::Lt | AstCompareOp::Le | AstCompareOp::Gt | AstCompareOp::Ge => {
            let Some(ordering) = value_ordering_typed(
                left,
                right,
                Some(path.logical_type.as_str()),
                path.collation_id,
            ) else {
                return false;
            };
            match op {
                AstCompareOp::Lt => ordering.is_lt(),
                AstCompareOp::Le => ordering.is_le(),
                AstCompareOp::Gt => ordering.is_gt(),
                AstCompareOp::Ge => ordering.is_ge(),
                AstCompareOp::Eq | AstCompareOp::Ne => unreachable!(),
            }
        }
    }
}

pub(super) fn execution_row_has_redacted_values_for_native_projection(row: &ExecutionRow) -> bool {
    match row {
        ExecutionRow::Object(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Association(row) => !row.redacted_properties.is_empty(),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => false,
    }
}

pub(super) fn native_direct_projection_output_name(expr: &ResolvedExpr) -> String {
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::Literal(_) => "literal".into(),
        ResolvedExpr::FunctionCall { function_id, .. } => function_id.clone(),
        _ => "expr".into(),
    }
}

pub(super) fn native_root_kind_name(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object",
        ResolvedRoot::Association(_) => "association",
        ResolvedRoot::Node(_) => "node",
        ResolvedRoot::Edge(_) => "edge",
        ResolvedRoot::Table(_) => "table",
        ResolvedRoot::Evidence(_) => "evidence",
        ResolvedRoot::Projection(_) => "projection",
    }
}

pub(super) fn projection_backed_root_id(root: &ResolvedRoot) -> Option<&str> {
    match root {
        ResolvedRoot::Projection(root) => Some(root.projection_id.as_str()),
        ResolvedRoot::Table(root) => Some(root.projection.projection_id.as_str()),
        _ => None,
    }
}
