use super::*;

pub(super) fn apply_materialized_windows(
    mut rows: Vec<MaterializedProjectionRow>,
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let window_functions = window_functions_for_plan(planned);
    for window in &planned.resolved.method_chain.windows {
        let mut partitions = BTreeMap::<String, Vec<usize>>::new();
        for (index, row) in rows.iter().enumerate() {
            let exec_row = ExecutionRow::Projection(row.clone());
            let key_values = window
                .partition_by
                .iter()
                .map(|expr| eval_expr(expr, &exec_row, context))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| {
                    exec_error(
                        "E_EXPRESSION",
                        format!(
                            "window partition expression evaluation failed: {}",
                            err.message
                        ),
                        json!({}),
                    )
                })?;
            let key = serde_json::to_string(&key_values).unwrap_or_else(|_| "[]".into());
            partitions.entry(key).or_default().push(index);
        }
        for indices in partitions.into_values() {
            let mut ordered = indices;
            if let Some(order) = &window.order_by {
                ordered.sort_by(|left, right| {
                    let left_row = ExecutionRow::Projection(rows[*left].clone());
                    let right_row = ExecutionRow::Projection(rows[*right].clone());
                    let left_value =
                        eval_expr(&order.expr, &left_row, context).unwrap_or(Value::Null);
                    let right_value =
                        eval_expr(&order.expr, &right_row, context).unwrap_or(Value::Null);
                    compare_sort_values(
                        &left_value,
                        &right_value,
                        order.direction,
                        order.nulls,
                        expr_logical_type(&order.expr),
                        expr_collation_id(&order.expr),
                    )
                    .then_with(|| {
                        materialized_projection_row_key(&rows[*left])
                            .cmp(&materialized_projection_row_key(&rows[*right]))
                    })
                });
            }
            let mut previous_order_key: Option<String> = None;
            let mut rank = 1usize;
            let mut dense_rank = 1usize;
            for (position, row_index) in ordered.iter().enumerate() {
                let order_key = if let Some(order) = &window.order_by {
                    let exec_row = ExecutionRow::Projection(rows[*row_index].clone());
                    let value = eval_expr(&order.expr, &exec_row, context).unwrap_or(Value::Null);
                    stable_value_key(&value)
                } else {
                    position.to_string()
                };
                if let Some(previous) = &previous_order_key {
                    if previous != &order_key {
                        rank = position + 1;
                        dense_rank += 1;
                    }
                }
                previous_order_key = Some(order_key);
                let row_number = json!((position + 1) as u64);
                let rank_value = json!(rank as u64);
                let dense_rank_value = json!(dense_rank as u64);
                rows[*row_index]
                    .values
                    .insert("row_number".into(), row_number.clone());
                rows[*row_index]
                    .values
                    .insert("rank".into(), rank_value.clone());
                rows[*row_index]
                    .values
                    .insert("dense_rank".into(), dense_rank_value.clone());
                for function in &window_functions {
                    let value = match function.name.as_str() {
                        "row_number" => row_number.clone(),
                        "rank" => rank_value.clone(),
                        "dense_rank" => dense_rank_value.clone(),
                        _ => evaluate_window_function(
                            function, &ordered, position, window, &rows, context,
                        )?,
                    };
                    rows[*row_index].values.insert(function.key.clone(), value);
                }
            }
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowFunctionSpec {
    key: String,
    name: String,
    args: Vec<ResolvedExpr>,
}

fn window_functions_for_plan(planned: &PlannedQuery) -> Vec<WindowFunctionSpec> {
    let mut functions = BTreeMap::<String, WindowFunctionSpec>::new();
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_window_functions_from_expr(&item.expr, &mut functions);
        }
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_window_functions_from_predicate(predicate, &mut functions);
    }
    if let Some(order) = &planned.resolved.method_chain.order_by {
        collect_window_functions_from_expr(&order.expr, &mut functions);
    }
    functions.into_values().collect()
}

fn collect_window_functions_from_predicate(
    predicate: &ResolvedPredicate,
    out: &mut BTreeMap<String, WindowFunctionSpec>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_window_functions_from_expr(left, out);
            collect_window_functions_from_expr(right, out);
        }
        ResolvedPredicate::InList { expr, .. } | ResolvedPredicate::NullCheck { expr, .. } => {
            collect_window_functions_from_expr(expr, out);
        }
        ResolvedPredicate::Exists(expr) | ResolvedPredicate::BoolExpr(expr) => {
            collect_window_functions_from_expr(expr, out);
        }
        ResolvedPredicate::Not(inner) => collect_window_functions_from_predicate(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_window_functions_from_predicate(part, out);
            }
        }
    }
}

fn collect_window_functions_from_expr(
    expr: &ResolvedExpr,
    out: &mut BTreeMap<String, WindowFunctionSpec>,
) {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id, args, ..
        } => {
            for arg in args {
                collect_window_functions_from_expr(arg, out);
            }
            if is_materialized_window_function(function_id) {
                let key = window_function_key(function_id, args);
                out.entry(key.clone())
                    .or_insert_with(|| WindowFunctionSpec {
                        key,
                        name: function_id.clone(),
                        args: args.clone(),
                    });
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_window_functions_from_predicate(predicate, out);
            collect_window_functions_from_expr(then_expr, out);
            collect_window_functions_from_expr(else_expr, out);
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_window_functions_from_expr(arg, out);
            }
        }
        ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_)
        | ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::TableExists(_) => {}
    }
}

pub(super) fn is_materialized_window_function(name: &str) -> bool {
    matches!(
        name,
        "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
            | "first_value"
            | "last_value"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "count"
    )
}

fn evaluate_window_function(
    function: &WindowFunctionSpec,
    ordered: &[usize],
    position: usize,
    window: &crate::ResolvedWindowSpec,
    rows: &[MaterializedProjectionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    let (start, end) = window_frame_bounds(window, position, ordered.len())?;
    match function.name.as_str() {
        "lag" | "lead" => {
            let offset = window_offset_arg(function)?;
            let target_position = if function.name == "lag" {
                position.checked_sub(offset)
            } else {
                position
                    .checked_add(offset)
                    .filter(|target| *target < ordered.len())
            };
            let Some(target_position) = target_position else {
                return Ok(Value::Null);
            };
            evaluate_window_arg(&function.args[0], ordered[target_position], rows, context)
        }
        "first_value" => evaluate_window_arg(&function.args[0], ordered[start], rows, context),
        "last_value" => evaluate_window_arg(&function.args[0], ordered[end], rows, context),
        "count" => {
            if function.args.is_empty() {
                return Ok(json!((end - start + 1) as u64));
            }
            let mut count = 0u64;
            for row_index in &ordered[start..=end] {
                if !evaluate_window_arg(&function.args[0], *row_index, rows, context)?.is_null() {
                    count += 1;
                }
            }
            Ok(json!(count))
        }
        "sum" | "avg" | "min" | "max" => {
            let values = ordered[start..=end]
                .iter()
                .map(|row_index| evaluate_window_arg(&function.args[0], *row_index, rows, context))
                .collect::<Result<Vec<_>, _>>()?;
            match function.name.as_str() {
                "sum" => numeric_aggregate(AstAggregateName::Sum, function.args.first(), &values),
                "avg" => numeric_aggregate(AstAggregateName::Avg, function.args.first(), &values),
                "min" => Ok(ordered_aggregate(
                    AstAggregateName::Min,
                    function.args.first(),
                    &values,
                )),
                "max" => Ok(ordered_aggregate(
                    AstAggregateName::Max,
                    function.args.first(),
                    &values,
                )),
                _ => unreachable!("guarded by caller"),
            }
        }
        "row_number" | "rank" | "dense_rank" => unreachable!("handled by caller"),
        _ => Ok(Value::Null),
    }
}

pub(super) fn evaluate_window_arg(
    expr: &ResolvedExpr,
    row_index: usize,
    rows: &[MaterializedProjectionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    eval_expr(
        expr,
        &ExecutionRow::Projection(rows[row_index].clone()),
        context,
    )
    .map_err(|err| {
        exec_error(
            "E_EXPRESSION",
            format!("window expression evaluation failed: {}", err.message),
            json!({}),
        )
    })
}

fn window_offset_arg(function: &WindowFunctionSpec) -> Result<usize, BuildExecutionError> {
    let Some(offset) = function.args.get(1) else {
        return Ok(1);
    };
    let ResolvedExpr::Literal(literal) = offset else {
        return Err(exec_error(
            "E_WINDOW",
            "lag/lead offset must be an integer literal",
            json!({ "function": function.name }),
        ));
    };
    let value = match &literal.typed_value {
        ResolvedLiteralValue::SignedInteger(value) => usize::try_from(*value).ok(),
        ResolvedLiteralValue::UnsignedInteger(value) => usize::try_from(*value).ok(),
        _ => None,
    }
    .ok_or_else(|| {
        exec_error(
            "E_WINDOW",
            "lag/lead offset must be a non-negative integer in range",
            json!({ "function": function.name }),
        )
    })?;
    Ok(value)
}

pub(super) fn window_frame_bounds(
    window: &crate::ResolvedWindowSpec,
    position: usize,
    len: usize,
) -> Result<(usize, usize), BuildExecutionError> {
    if len == 0 {
        return Err(exec_error("E_WINDOW", "empty window partition", json!({})));
    }
    let start = match window.start.as_str() {
        "unbounded_preceding" => 0,
        "current_row" => position,
        value => {
            return Err(exec_error(
                "E_WINDOW",
                format!("unsupported window frame start {value}"),
                json!({ "frame": format!("{:?}", window.frame) }),
            ))
        }
    };
    let end = match window.end.as_str() {
        "current_row" => position,
        "unbounded_following" => len - 1,
        value => {
            return Err(exec_error(
                "E_WINDOW",
                format!("unsupported window frame end {value}"),
                json!({ "frame": format!("{:?}", window.frame) }),
            ))
        }
    };
    if start > end {
        return Err(exec_error(
            "E_WINDOW",
            "window frame start is after frame end",
            json!({ "start": window.start, "end": window.end }),
        ));
    }
    Ok((start, end))
}
