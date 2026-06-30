use super::*;

pub(crate) fn finish_materialized_rows(
    rows: Vec<ExecutionRow>,
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[MaterializedEvidenceRow],
    object_rows: &[MaterializedObjectRow],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let context_associations = filter_association_context_rows(associations, planned, options);
    let context_evidence_rows = filter_evidence_context_rows(evidence_rows, planned, options);
    let context_object_rows = filter_object_context_rows(object_rows, planned, options);
    let context = EvalContext::for_plan_with_objects(
        &context_associations,
        &context_evidence_rows,
        &context_object_rows,
        planned,
    );
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

pub(super) fn finish_materialized_rows_with_context(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    context: &EvalContext<'_>,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let rows = apply_visibility_overlay(rows, planned, options);
    let rows = enforce_redaction_policy(rows, planned)?;
    let rows = enforce_materialized_branch_policy(rows, planned)?;
    let input_rows = rows.len();
    let mut rows = rows
        .into_iter()
        .filter_map(|row| match predicate_matches(&row, planned, context) {
            Ok(true) => Some(Ok(row)),
            Ok(false) => None,
            Err(err) => Some(Err(err)),
        })
        .collect::<Result<Vec<_>, BuildExecutionError>>()?;
    let filtered_rows = rows.len();
    check_time(&options.resource_budget, started)?;
    if grouped_or_aggregate(planned) {
        let json_rows = aggregate_rows(&rows, planned, context, options)?;
        return finish_json_rows(
            json_rows,
            input_rows,
            filtered_rows,
            planned,
            options,
            started,
        );
    }
    sort_rows(&mut rows, planned, context)?;
    let rows = apply_skip_take(rows, planned);
    let output_rows = rows.len();
    match &planned.resolved.output_mode {
        CoveQlOutputMode::ObjectRows => Ok((
            CoveQlExecutionResult::ObjectRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Object(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveQlOutputMode::AssociationRows => Ok((
            CoveQlExecutionResult::AssociationRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Association(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveQlOutputMode::EvidenceRows => Ok((
            CoveQlExecutionResult::EvidenceRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Evidence(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveQlOutputMode::ProjectionRows => Ok((
            CoveQlExecutionResult::ProjectionRows(
                rows.into_iter()
                    .filter_map(|row| match row {
                        ExecutionRow::Projection(row) => Some(row),
                        _ => None,
                    })
                    .collect(),
            ),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        )),
        CoveQlOutputMode::JsonRows => {
            let json_rows = select_json_rows(&rows, planned, context)?;
            finish_json_rows(
                json_rows,
                input_rows,
                filtered_rows,
                planned,
                options,
                started,
            )
        }
        CoveQlOutputMode::ArrowRecordBatch { .. } => {
            let batch =
                crate::kernel_arrow::execution_rows_to_owned_record_batch(&rows, planned, context)
                    .map_err(|err| {
                        exec_error(
                            "E_ARROW_OUTPUT",
                            format!("Arrow output materialization failed: {err}"),
                            json!({}),
                        )
                    })?;
            Ok((
                CoveQlExecutionResult::ArrowRecordBatches(vec![batch]),
                ExecutionRowCounts {
                    input_rows,
                    filtered_rows,
                    output_rows,
                },
            ))
        }
        CoveQlOutputMode::ExplainJson | CoveQlOutputMode::DataFusionTableProvider => {
            unreachable!("handled before row execution")
        }
    }
}

pub(crate) fn finish_json_rows(
    json_rows: Vec<Value>,
    input_rows: usize,
    filtered_rows: usize,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let output_rows = json_rows.len();
    if matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. }
    ) {
        let batch =
            crate::kernel_arrow::json_rows_to_owned_record_batch_for_plan(&json_rows, planned)
                .map_err(|err| {
                    exec_error(
                        "E_ARROW_OUTPUT",
                        format!("Arrow output materialization failed: {err}"),
                        json!({}),
                    )
                })?;
        Ok((
            CoveQlExecutionResult::ArrowRecordBatches(vec![batch]),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        ))
    } else {
        Ok((
            CoveQlExecutionResult::JsonRows(json_rows),
            ExecutionRowCounts {
                input_rows,
                filtered_rows,
                output_rows,
            },
        ))
    }
}

pub(crate) fn predicate_matches(
    row: &ExecutionRow,
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<bool, BuildExecutionError> {
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Ok(true);
    };
    eval_predicate(predicate, row, context).map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!("materialized predicate evaluation failed: {}", err.message),
            json!({}),
        )
    })
}

pub(crate) fn select_json_rows(
    rows: &[ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<Vec<Value>, BuildExecutionError> {
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(execution_rows_to_json(rows));
    };
    rows.iter()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for item in select {
                if matches!(item.expr, ResolvedExpr::AggregateCall { .. }) {
                    return Err(exec_error(
                        "E_EXPRESSION",
                        "aggregate expression reached row projection without aggregate execution",
                        json!({}),
                    ));
                }
                let key = item
                    .alias
                    .clone()
                    .unwrap_or_else(|| output_name_for_expr(&item.expr));
                let value = eval_expr(&item.expr, row, context).map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!("materialized expression evaluation failed: {}", err.message),
                        json!({ "column": key }),
                    )
                })?;
                object.insert(key, value);
            }
            Ok(Value::Object(object))
        })
        .collect()
}
