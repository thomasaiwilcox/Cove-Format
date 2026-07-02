use super::*;

pub(super) fn aggregate_rows(
    rows: &[ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
    options: &ExecutionOptions,
) -> Result<Vec<Value>, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact
        | AggregateDisclosurePolicy::AllowMaterializedOnly
        | AggregateDisclosurePolicy::AllowThresholded
        | AggregateDisclosurePolicy::AllowRedacted => {}
        AggregateDisclosurePolicy::Reject => {
            return Err(exec_error(
                "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
                "aggregate disclosure is rejected by the active security context",
                json!({}),
            ))
        }
    }
    let group_by = planned
        .resolved
        .method_chain
        .group_by
        .clone()
        .unwrap_or_default();
    let mut groups = BTreeMap::<String, Vec<&ExecutionRow>>::new();
    for row in rows {
        let key_values = group_by
            .iter()
            .map(|expr| eval_expr(expr, row, context))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("group expression evaluation failed: {}", err.message),
                    json!({}),
                )
            })?;
        let key = serde_json::to_string(&key_values).unwrap_or_else(|_| "[]".into());
        groups.entry(key).or_default().push(row);
    }
    if group_by.is_empty() {
        groups
            .entry("[]".into())
            .or_insert_with(|| rows.iter().collect());
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }
    let select = planned
        .resolved
        .method_chain
        .select
        .as_ref()
        .ok_or_else(|| {
            exec_error(
                "E_AGGREGATE",
                "aggregate query requires select output",
                json!({}),
            )
        })?;
    let mut out = Vec::new();
    for group_rows in groups.into_values() {
        let representative = group_rows.first().copied();
        let mut object = serde_json::Map::new();
        for item in select {
            let key = item
                .alias
                .clone()
                .unwrap_or_else(|| output_name_for_expr(&item.expr));
            let value = match &item.expr {
                ResolvedExpr::AggregateCall {
                    name, arg, star, ..
                } => aggregate_disclosed_value(
                    *name,
                    arg.as_deref(),
                    *star,
                    &group_rows,
                    context,
                    planned,
                )?,
                expr => match representative {
                    Some(row) => eval_expr(expr, row, context).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!("grouped expression evaluation failed: {}", err.message),
                            json!({ "column": key }),
                        )
                    })?,
                    None => Value::Null,
                },
            };
            object.insert(key, value);
        }
        out.push(Value::Object(object));
    }
    Ok(out)
}

pub(super) fn aggregate_disclosed_value(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    star: bool,
    rows: &[&ExecutionRow],
    context: &EvalContext<'_>,
    planned: &PlannedQuery,
) -> Result<Value, BuildExecutionError> {
    match planned
        .resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
    {
        AggregateDisclosurePolicy::AllowExact
        | AggregateDisclosurePolicy::AllowMaterializedOnly => {
            aggregate_value(name, arg, star, rows, context)
        }
        AggregateDisclosurePolicy::AllowThresholded => {
            let threshold = planned
                .resolved
                .operation_context
                .security
                .aggregate_disclosure_threshold
                .unwrap_or(1);
            if rows.len() as u64 >= threshold {
                aggregate_value(name, arg, star, rows, context)
            } else {
                Ok(aggregate_policy_marker(
                    "thresholded",
                    "suppressed",
                    Some(threshold),
                ))
            }
        }
        AggregateDisclosurePolicy::AllowRedacted => {
            Ok(aggregate_policy_marker("redacted", "redacted", None))
        }
        AggregateDisclosurePolicy::Reject => Err(exec_error(
            "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
            "aggregate disclosure is rejected by the active security context",
            json!({}),
        )),
    }
}

pub(super) fn aggregate_policy_marker(policy: &str, status: &str, threshold: Option<u64>) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("policy".into(), Value::String(policy.into()));
    object.insert("status".into(), Value::String(status.into()));
    if let Some(threshold) = threshold {
        object.insert("threshold".into(), json!(threshold));
    }
    Value::Object(object)
}

pub(super) fn aggregate_value(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    star: bool,
    rows: &[&ExecutionRow],
    context: &EvalContext<'_>,
) -> Result<Value, BuildExecutionError> {
    if !star {
        if let Some(ResolvedExpr::Association(association)) = arg {
            return Ok(match name {
                AstAggregateName::Count => json!(rows
                    .iter()
                    .map(|row| context.association_count_for(row, association) as u64)
                    .sum::<u64>()),
                AstAggregateName::Exists => Value::Bool(
                    rows.iter()
                        .any(|row| context.association_count_for(row, association) > 0),
                ),
                AstAggregateName::DistinctCount => {
                    let mut distinct = BTreeSet::new();
                    for row in rows {
                        distinct.extend(context.association_endpoint_keys_for(row, association));
                    }
                    json!(distinct.len() as u64)
                }
                _ => return Err(exec_error(
                    "E_AGGREGATE",
                    "association helper supports count, exists, and distinctCount aggregates only",
                    json!({}),
                )),
            });
        }
        if let Some(ResolvedExpr::Evidence(evidence)) = arg {
            return Ok(match name {
                AstAggregateName::Count => json!(rows
                    .iter()
                    .map(|row| context.evidence_count_for(row, evidence) as u64)
                    .sum::<u64>()),
                AstAggregateName::Exists => Value::Bool(
                    rows.iter()
                        .any(|row| context.evidence_count_for(row, evidence) > 0),
                ),
                AstAggregateName::DistinctCount => {
                    let mut distinct = BTreeSet::new();
                    for row in rows {
                        distinct.extend(context.evidence_identity_keys_for(row, evidence));
                    }
                    json!(distinct.len() as u64)
                }
                _ => {
                    return Err(exec_error(
                        "E_AGGREGATE",
                        "evidence helper supports count, exists, and distinctCount aggregates only",
                        json!({}),
                    ))
                }
            });
        }
    }
    let values = if star {
        Vec::new()
    } else if let Some(arg) = arg {
        rows.iter()
            .map(|row| eval_expr(arg, row, context))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("aggregate argument evaluation failed: {}", err.message),
                    json!({}),
                )
            })?
    } else {
        Vec::new()
    };
    Ok(match name {
        AstAggregateName::Count => {
            if star {
                json!(rows.len() as u64)
            } else {
                json!(values.iter().filter(|value| !value.is_null()).count() as u64)
            }
        }
        AstAggregateName::Exists => Value::Bool(if star {
            !rows.is_empty()
        } else {
            values.iter().any(|value| !value.is_null())
        }),
        AstAggregateName::DistinctCount => json!(distinct_count(
            values.into_iter().filter(|value| !value.is_null()),
            arg.and_then(expr_logical_type)
        )),
        AstAggregateName::Sum | AstAggregateName::Avg => numeric_aggregate(name, arg, &values)?,
        AstAggregateName::Min | AstAggregateName::Max => ordered_aggregate(name, arg, &values),
    })
}

pub(super) fn numeric_aggregate(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    values: &[Value],
) -> Result<Value, BuildExecutionError> {
    let mut exact_sum: Option<ExactDecimal> = None;
    let mut float_sum = 0.0f64;
    let mut saw_float = false;
    let mut count = 0usize;
    let logical_type = arg.and_then(expr_logical_type);
    for value in values.iter().filter(|value| !value.is_null()) {
        let numeric_is_float = matches!(logical_type, Some("float32" | "float64"))
            || value
                .as_number()
                .is_some_and(|number| number.as_i64().is_none() && number.as_u64().is_none());
        if numeric_is_float {
            let Some(number) = value.as_f64() else {
                return Err(exec_error(
                    "E_AGGREGATE",
                    "sum/avg aggregate requires numeric materialized values",
                    json!({}),
                ));
            };
            saw_float = true;
            float_sum += number;
        } else if let Some(decimal) = parse_decimal_value(value) {
            exact_sum = Some(match exact_sum {
                Some(sum) => sum.checked_add(decimal).ok_or_else(|| {
                    exec_error(
                        "E_AGGREGATE",
                        "sum/avg aggregate overflowed decimal accumulator",
                        json!({}),
                    )
                })?,
                None => decimal,
            });
        } else {
            return Err(exec_error(
                "E_AGGREGATE",
                "sum/avg aggregate requires numeric materialized values",
                json!({}),
            ));
        }
        count += 1;
    }
    if count == 0 {
        return Ok(Value::Null);
    }
    match name {
        AstAggregateName::Sum if saw_float => Ok(json!(float_sum)),
        AstAggregateName::Avg if saw_float => Ok(json!(float_sum / count as f64)),
        AstAggregateName::Sum => Ok(exact_sum
            .ok_or_else(|| exec_error("E_AGGREGATE", "empty numeric accumulator", json!({})))?
            .to_json_sum()),
        AstAggregateName::Avg => Ok(exact_sum
            .ok_or_else(|| exec_error("E_AGGREGATE", "empty numeric accumulator", json!({})))?
            .checked_div_u64(count as u64)
            .ok_or_else(|| {
                exec_error(
                    "E_AGGREGATE",
                    "avg aggregate overflowed decimal accumulator",
                    json!({}),
                )
            })?
            .to_json_sum()),
        _ => unreachable!("guarded by caller"),
    }
}

pub(super) fn ordered_aggregate(
    name: AstAggregateName,
    arg: Option<&ResolvedExpr>,
    values: &[Value],
) -> Value {
    let mut selected: Option<Value> = None;
    let logical_type = arg.and_then(expr_logical_type);
    let collation_id = arg.and_then(expr_collation_id);
    for value in values.iter().filter(|value| !value.is_null()) {
        let replace = match &selected {
            None => true,
            Some(current) => value_ordering_typed(value, current, logical_type, collation_id)
                .is_some_and(|ordering| match name {
                    AstAggregateName::Min => ordering.is_lt(),
                    AstAggregateName::Max => ordering.is_gt(),
                    _ => false,
                }),
        };
        if replace {
            selected = Some(value.clone());
        }
    }
    selected.unwrap_or(Value::Null)
}

pub(crate) fn sort_rows(
    rows: &mut [ExecutionRow],
    planned: &PlannedQuery,
    context: &EvalContext<'_>,
) {
    let Some(order) = &planned.resolved.method_chain.order_by else {
        if planned.logical_plan.default_ordering_applied {
            let fields = default_sort_fields(planned);
            rows.sort_by(|left, right| {
                compare_default_sort_fields(left, right, &fields)
                    .then_with(|| compare_manifest_member_order(left, right, planned))
                    .then_with(|| {
                        stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json()))
                    })
            });
        }
        return;
    };
    let logical_type = expr_logical_type(&order.expr);
    let collation_id = expr_collation_id(&order.expr);
    rows.sort_by(|left, right| {
        let left_value = eval_expr(&order.expr, left, context).unwrap_or(Value::Null);
        let right_value = eval_expr(&order.expr, right, context).unwrap_or(Value::Null);
        compare_sort_values(
            &left_value,
            &right_value,
            order.direction,
            order.nulls,
            logical_type,
            collation_id,
        )
        .then_with(|| compare_manifest_member_order(left, right, planned))
        .then_with(|| stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json())))
    });
}

pub(super) fn compare_manifest_member_order(
    left: &ExecutionRow,
    right: &ExecutionRow,
    planned: &PlannedQuery,
) -> Ordering {
    if planned.resolved.operation_context.dataset.files.len() <= 1 {
        return Ordering::Equal;
    }
    left.dataset_file_ordinal()
        .cmp(&right.dataset_file_ordinal())
}

pub(super) fn compare_sort_values(
    left: &Value,
    right: &Value,
    direction: AstOrderDirection,
    nulls: AstNullOrdering,
    logical_type: Option<&str>,
    collation_id: Option<u16>,
) -> Ordering {
    let nulls = match nulls {
        AstNullOrdering::Default => match direction {
            AstOrderDirection::Asc => AstNullOrdering::NullsLast,
            AstOrderDirection::Desc => AstNullOrdering::NullsFirst,
        },
        explicit => explicit,
    };
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Less,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Greater,
        },
        (false, true) => match nulls {
            AstNullOrdering::NullsFirst => Ordering::Greater,
            AstNullOrdering::NullsLast | AstNullOrdering::Default => Ordering::Less,
        },
        (false, false) => {
            let ordering = value_ordering_typed(left, right, logical_type, collation_id)
                .unwrap_or_else(|| stable_value_key(left).cmp(&stable_value_key(right)));
            match direction {
                AstOrderDirection::Asc => ordering,
                AstOrderDirection::Desc => ordering.reverse(),
            }
        }
    }
}

pub(super) fn default_sort_fields(planned: &PlannedQuery) -> Vec<String> {
    planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            crate::logical_plan::LogicalPlanNodeKind::Sort {
                keys,
                defaulted: true,
                ..
            } => Some(
                keys.iter()
                    .filter_map(|key| key.field.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn compare_default_sort_fields(
    left: &ExecutionRow,
    right: &ExecutionRow,
    fields: &[String],
) -> Ordering {
    for field in fields {
        let left_value = default_sort_field_value(left, field);
        let right_value = default_sort_field_value(right, field);
        let ordering = compare_sort_values(
            &left_value,
            &right_value,
            AstOrderDirection::Asc,
            AstNullOrdering::NullsLast,
            None,
            None,
        );
        if !ordering.is_eq() {
            return ordering;
        }
    }
    Ordering::Equal
}

pub(super) fn default_sort_field_value(row: &ExecutionRow, field: &str) -> Value {
    if field == "dataset_file_ordinal" || field == "manifest_file_ordinal" {
        return row
            .dataset_file_ordinal()
            .map(|ordinal| json!(ordinal))
            .unwrap_or(Value::Null);
    }
    match row {
        ExecutionRow::Object(row) => match field {
            "object_type_id" => json!(row.object_type_id),
            "branch_key" => json!(row.branch_key),
            "goid" | "association_goid" => Value::String(row.goid.clone()),
            "timestamp_us" => json!(row.timestamp_us),
            "csn" => json!(row.csn),
            "record_id" => Value::String(row.record_id.clone()),
            "output_grain" => json!(row.output_grain),
            _ => Value::Null,
        },
        ExecutionRow::Association(row) => match field {
            "object_type_id" | "association_type_id" => json!(row.object_type_id),
            "branch_key" => json!(row.branch_key),
            "goid" | "association_goid" => Value::String(row.goid.clone()),
            "source_goid" => row
                .source_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "target_goid" => row
                .target_goid
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            "timestamp_us" => json!(row.timestamp_us),
            "csn" => json!(row.csn),
            "record_id" => Value::String(row.record_id.clone()),
            "output_grain" => json!(row.output_grain),
            _ => Value::Null,
        },
        ExecutionRow::Evidence(row) => evidence_default_sort_field_value(row, field),
        ExecutionRow::Projection(row) => row.values.get(field).cloned().unwrap_or(Value::Null),
    }
}

pub(super) fn evidence_default_sort_field_value(
    row: &MaterializedEvidenceRow,
    field: &str,
) -> Value {
    row.fields
        .get(field)
        .or_else(|| match field {
            "target_id" => row.fields.get("output_object_id"),
            "source_system" => row.fields.get("source_id"),
            "evidence_id" => row.fields.get("assertion_id"),
            _ => None,
        })
        .cloned()
        .unwrap_or(Value::Null)
}

pub(crate) fn apply_skip_take(
    rows: Vec<ExecutionRow>,
    planned: &PlannedQuery,
) -> Vec<ExecutionRow> {
    let skip = planned.resolved.method_chain.skip.unwrap_or(0) as usize;
    let take = planned
        .resolved
        .method_chain
        .take
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    rows.into_iter().skip(skip).take(take).collect()
}
