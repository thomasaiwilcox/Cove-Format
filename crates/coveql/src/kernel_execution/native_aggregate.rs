use super::*;

pub(crate) fn aggregate_output_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
    }
}

pub(super) fn try_native_bool_group_count_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeBoolGroupCountReport,
    )>,
    BuildExecutionError,
> {
    if !native_bool_group_count_shape(planned) {
        return Ok(None);
    }
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    if let Some(result) = try_native_dense_bool_group_star_aggregate_result(
        rows, group_path, select, planned, options, started,
    )? {
        return Ok(Some(result));
    }
    let mut groups = BTreeMap::<String, (Value, Vec<usize>)>::new();
    for (row_index, row) in rows.iter().enumerate() {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        let value = row.value_for_path(group_path);
        if !native_direct_group_count_value_is_supported(&value, group_path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct grouped aggregate kernel received a value outside its proven typed lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            ));
        }
        let key = serde_json::to_string(std::slice::from_ref(&value)).map_err(|err| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                format!("native direct group key could not be serialized: {err}"),
                json!({}),
            )
        })?;
        groups
            .entry(key)
            .and_modify(|(_, row_indices)| row_indices.push(row_index))
            .or_insert((value, vec![row_index]));
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }
    let mut json_rows = Vec::with_capacity(groups.len());
    let mut aggregate_name = "count";
    let mut aggregate_strategy = "none";
    let mut values_seen = 0usize;
    for (_key, (group_value, row_indices)) in groups {
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name, arg, star, ..
                } => {
                    aggregate_name = aggregate_operator_name(*name);
                    let aggregate_eval = native_grouped_direct_aggregate_eval(
                        *name,
                        *star,
                        arg.as_deref(),
                        &row_indices,
                        rows,
                    )?;
                    aggregate_strategy = aggregate_eval.strategy;
                    values_seen += aggregate_eval.values_seen;
                    object.insert(key, aggregate_eval.value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }
    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeBoolGroupCountReport {
            aggregate: aggregate_name,
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            values_seen,
            group_strategy: "row_index_single_pass",
            aggregate_strategy,
        },
    )))
}

pub(super) fn try_native_dense_bool_group_star_aggregate_result(
    rows: &[ExecutionRow],
    group_path: &ResolvedPath,
    select: &[crate::ResolvedSelectItem],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeBoolGroupCountReport,
    )>,
    BuildExecutionError,
> {
    if !matches!(group_path.logical_type.as_str(), "bool" | "boolean") {
        return Ok(None);
    }
    let Some(aggregate_name) = select.iter().find_map(|item| match &item.expr {
        ResolvedExpr::AggregateCall {
            name,
            star: true,
            arg: None,
            ..
        } if matches!(name, AstAggregateName::Count | AstAggregateName::Exists) => Some(*name),
        _ => None,
    }) else {
        return Ok(None);
    };

    let mut false_count = 0usize;
    let mut null_count = 0usize;
    let mut true_count = 0usize;
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        match row.value_for_path(group_path) {
            Value::Bool(false) => false_count += 1,
            Value::Bool(true) => true_count += 1,
            Value::Null => null_count += 1,
            value => return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native dense bool group aggregate received a value outside its proven typed lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            )),
        }
    }

    let mut json_rows = Vec::with_capacity(3);
    for (group_value, group_rows) in [
        (Value::Bool(false), false_count),
        (Value::Null, null_count),
        (Value::Bool(true), true_count),
    ] {
        if group_rows == 0 {
            continue;
        }
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name,
                    star: true,
                    arg: None,
                    ..
                } if *name == aggregate_name => {
                    let value = match aggregate_name {
                        AstAggregateName::Count => json!(group_rows as u64),
                        AstAggregateName::Exists => Value::Bool(group_rows > 0),
                        _ => unreachable!("dense bool group star aggregate was prefiltered"),
                    };
                    object.insert(key, value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }

    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeBoolGroupCountReport {
            aggregate: aggregate_operator_name(aggregate_name),
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            values_seen: rows.len(),
            group_strategy: "dense_bool_star",
            aggregate_strategy: "row_count",
        },
    )))
}

pub(super) fn try_native_grouped_helper_aggregate_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeGroupedHelperAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_grouped_helper_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(group_path) = native_bool_group_count_path(planned) else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let mut groups = BTreeMap::<String, (Value, Vec<usize>)>::new();
    for (row_index, row) in rows.iter().enumerate() {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, group_path) {
            return Ok(None);
        }
        let value = row.value_for_path(group_path);
        if !native_direct_group_count_value_is_supported(&value, group_path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped helper aggregate kernel received a value outside its proven typed group lane",
                json!({
                    "group_property": group_path.display_name,
                    "logical_type": group_path.logical_type,
                    "value": value,
                }),
            ));
        }
        let key = serde_json::to_string(std::slice::from_ref(&value)).map_err(|err| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                format!("native grouped helper key could not be serialized: {err}"),
                json!({}),
            )
        })?;
        groups
            .entry(key)
            .and_modify(|(_, row_indices)| row_indices.push(row_index))
            .or_insert((value, vec![row_index]));
    }
    if groups.len() > options.resource_budget.maximum_groups {
        return Err(resource_error("maximum_groups", groups.len()));
    }

    let association_edges = AssociationEdgeTable::from_rows_for_plan(planned, associations);
    let evidence_index = EvidenceGrainIndex::from_rows(evidence_rows);
    let mut json_rows = Vec::with_capacity(groups.len());
    let mut aggregate_name = "count";
    let mut helper_kind = "helper";
    let mut values_seen = 0usize;
    for (_key, (group_value, row_indices)) in groups {
        let mut object = serde_json::Map::new();
        for item in select {
            let key = native_bool_group_count_output_name(&item.expr, item.alias.as_deref());
            match &item.expr {
                ResolvedExpr::Path(_) => {
                    object.insert(key, group_value.clone());
                }
                ResolvedExpr::AggregateCall {
                    name,
                    arg: Some(arg),
                    star: false,
                    ..
                } => {
                    aggregate_name = aggregate_operator_name(*name);
                    let Some((aggregate_value, group_helper_kind, group_values_seen)) =
                        native_helper_aggregate_value_for_indices(
                            *name,
                            arg,
                            &row_indices,
                            rows,
                            &association_edges,
                            &evidence_index,
                        )?
                    else {
                        return Ok(None);
                    };
                    helper_kind = group_helper_kind;
                    values_seen += group_values_seen;
                    object.insert(key, aggregate_value);
                }
                _ => return Ok(None),
            }
        }
        json_rows.push(Value::Object(object));
    }
    let group_count = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeGroupedHelperAggregateReport {
            aggregate: aggregate_name,
            helper_kind,
            group_property: group_path.display_name.clone(),
            group_logical_type: group_path.logical_type.clone(),
            group_count,
            rows_counted: rows.len(),
            helper_values_seen: values_seen,
        },
    )))
}

pub(super) fn object_path_is_redacted(row: &MaterializedObjectRow, path: &ResolvedPath) -> bool {
    if row.redacted_properties.contains(&path.display_name) {
        return true;
    }
    path.property_id
        .and_then(|property_id| row.property_ids.get(&property_id))
        .is_some_and(|name| row.redacted_properties.contains(name))
}

pub(super) fn native_bool_group_count_output_name(
    expr: &ResolvedExpr,
    alias: Option<&str>,
) -> String {
    if let Some(alias) = alias {
        return alias.into();
    }
    match expr {
        ResolvedExpr::Path(path) => path.display_name.clone(),
        ResolvedExpr::AggregateCall { name, .. } => aggregate_output_name(*name).into(),
        _ => "expr".into(),
    }
}

pub(super) fn native_direct_group_count_value_is_supported(
    value: &Value,
    path: &ResolvedPath,
) -> bool {
    if value.is_null() {
        return true;
    }
    if path.physical_kind == "file_code" {
        return match path.logical_type.as_str() {
            "bool" | "boolean" => value.as_bool().is_some(),
            "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
            | "float32" | "float64" | "decimal64" | "decimal128" | "date_days"
            | "timestamp_micros" | "timestamp_nanos" => parse_decimal_value(value).is_some(),
            "utf8" | "string" | "uuid" | "binary" => value.as_str().is_some(),
            _ => false,
        };
    }
    match path.logical_type.as_str() {
        "bool" | "boolean" => value.as_bool().is_some(),
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "decimal128" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => parse_decimal_value(value).is_some(),
        _ => false,
    }
}

fn native_grouped_direct_aggregate_eval(
    aggregate: AstAggregateName,
    star: bool,
    arg: Option<&ResolvedExpr>,
    row_indices: &[usize],
    rows: &[ExecutionRow],
) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
    if star {
        let value = match aggregate {
            AstAggregateName::Count => json!(row_indices.len() as u64),
            AstAggregateName::Exists => Value::Bool(!row_indices.is_empty()),
            _ => {
                return Err(exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native grouped aggregate received an unsupported star aggregate",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                ))
            }
        };
        return Ok(NativeDirectAggregateEval {
            value,
            values_seen: row_indices.len(),
            strategy: "row_count",
        });
    }
    let Some(ResolvedExpr::Path(path)) = arg else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native grouped aggregate requires a direct path argument",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        ));
    };
    let mut state = NativeDirectAggregateState::new();
    for row_index in row_indices {
        let Some(row) = rows.get(*row_index) else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate row index was outside the reconstructed row set",
                json!({ "row_index": row_index }),
            ));
        };
        let ExecutionRow::Object(row) = row else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate expected object rows",
                json!({}),
            ));
        };
        if object_path_is_redacted(row, path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native grouped aggregate cannot evaluate a redacted path",
                json!({ "path": path.display_name }),
            ));
        }
        let Some(value) = object_property_value_for_path(row, path) else {
            continue;
        };
        state.accumulate_value(aggregate, path, value)?;
    }
    state.finish(aggregate)
}

pub(super) fn try_native_direct_aggregate_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeDirectAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_direct_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall { star, .. } = &item.expr else {
        return Ok(None);
    };
    let Some(aggregate) = native_direct_aggregate_name(planned) else {
        return Ok(None);
    };
    let counted_path = native_direct_aggregate_path(planned);
    let aggregate_eval = native_direct_aggregate_eval(aggregate, *star, counted_path, rows)?;
    let mut object = serde_json::Map::new();
    object.insert(
        native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
        aggregate_eval.value,
    );
    let (result, row_counts) = finish_json_rows(
        vec![Value::Object(object)],
        rows.len(),
        rows.len(),
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeDirectAggregateReport {
            aggregate: aggregate_operator_name(aggregate),
            counted_path: counted_path.map(|path| path.display_name.clone()),
            rows_counted: rows.len(),
            values_seen: aggregate_eval.values_seen,
            aggregate_strategy: aggregate_eval.strategy,
        },
    )))
}

pub(super) fn try_native_helper_aggregate_result(
    rows: &[ExecutionRow],
    associations: &[MaterializedAssociationRow],
    evidence_rows: &[crate::materialized::MaterializedEvidenceRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeHelperAggregateReport,
    )>,
    BuildExecutionError,
> {
    if !native_helper_aggregate_shape(planned) {
        return Ok(None);
    }
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let [item] = select.as_slice() else {
        return Ok(None);
    };
    let ResolvedExpr::AggregateCall {
        name,
        arg: Some(arg),
        star: false,
        ..
    } = &item.expr
    else {
        return Ok(None);
    };

    let (aggregate_value, helper_kind, helper_values_seen) = match arg.as_ref() {
        ResolvedExpr::Association(association) => {
            let edge_table = AssociationEdgeTable::from_rows_for_plan(planned, associations);
            let mut matched_edges = 0u64;
            let mut distinct_endpoints = BTreeSet::new();
            for row in rows {
                let Some(goid) = current_row_goid(row) else {
                    return Ok(None);
                };
                let file_ordinal = current_row_dataset_file_ordinal(row);
                match name {
                    AstAggregateName::Count => {
                        matched_edges +=
                            edge_table.count_for_scoped(file_ordinal, goid, association) as u64;
                    }
                    AstAggregateName::Exists => {
                        if edge_table.exists_for_scoped(file_ordinal, goid, association) {
                            matched_edges += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_endpoints.extend(edge_table.opposite_endpoints_for_scoped(
                            file_ordinal,
                            goid,
                            association,
                        ));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match name {
                AstAggregateName::Count => json!(matched_edges),
                AstAggregateName::Exists => Value::Bool(matched_edges > 0),
                AstAggregateName::DistinctCount => json!(distinct_endpoints.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match name {
                AstAggregateName::DistinctCount => distinct_endpoints.len(),
                _ => usize::try_from(matched_edges).unwrap_or(usize::MAX),
            };
            (value, "association", seen)
        }
        ResolvedExpr::Evidence(evidence) => {
            let evidence_index = EvidenceGrainIndex::from_rows(evidence_rows);
            let mut matched_entries = 0u64;
            let mut distinct_identities = BTreeSet::new();
            for row in rows {
                match name {
                    AstAggregateName::Count => {
                        matched_entries += evidence_index.count_for(row, evidence) as u64;
                    }
                    AstAggregateName::Exists => {
                        if evidence_index.exists_for(row, evidence) {
                            matched_entries += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_identities.extend(evidence_index.identities_for(row, evidence));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match name {
                AstAggregateName::Count => json!(matched_entries),
                AstAggregateName::Exists => Value::Bool(matched_entries > 0),
                AstAggregateName::DistinctCount => json!(distinct_identities.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match name {
                AstAggregateName::DistinctCount => distinct_identities.len(),
                _ => usize::try_from(matched_entries).unwrap_or(usize::MAX),
            };
            (value, "evidence", seen)
        }
        _ => return Ok(None),
    };

    let mut object = serde_json::Map::new();
    object.insert(
        native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
        aggregate_value,
    );
    let (result, row_counts) = finish_json_rows(
        vec![Value::Object(object)],
        rows.len(),
        rows.len(),
        planned,
        options,
        started,
    )?;
    Ok(Some((
        result,
        row_counts,
        NativeHelperAggregateReport {
            aggregate: aggregate_operator_name(*name),
            helper_kind,
            rows_counted: rows.len(),
            helper_values_seen,
        },
    )))
}

pub(super) fn native_helper_aggregate_value_for_indices(
    aggregate: AstAggregateName,
    arg: &ResolvedExpr,
    row_indices: &[usize],
    rows: &[ExecutionRow],
    association_edges: &AssociationEdgeTable,
    evidence_index: &EvidenceGrainIndex,
) -> Result<Option<(Value, &'static str, usize)>, BuildExecutionError> {
    match arg {
        ResolvedExpr::Association(association) => {
            let mut matched_edges = 0u64;
            let mut distinct_endpoints = BTreeSet::new();
            for row_index in row_indices {
                let Some(row) = rows.get(*row_index) else {
                    return Err(exec_error(
                        "E_KERNEL_AGGREGATE",
                        "native helper aggregate row index was outside the reconstructed row set",
                        json!({ "row_index": row_index }),
                    ));
                };
                let Some(goid) = current_row_goid(row) else {
                    return Ok(None);
                };
                let file_ordinal = current_row_dataset_file_ordinal(row);
                match aggregate {
                    AstAggregateName::Count => {
                        matched_edges +=
                            association_edges.count_for_scoped(file_ordinal, goid, association)
                                as u64;
                    }
                    AstAggregateName::Exists => {
                        if association_edges.exists_for_scoped(file_ordinal, goid, association) {
                            matched_edges += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_endpoints.extend(association_edges.opposite_endpoints_for_scoped(
                            file_ordinal,
                            goid,
                            association,
                        ));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match aggregate {
                AstAggregateName::Count => json!(matched_edges),
                AstAggregateName::Exists => Value::Bool(matched_edges > 0),
                AstAggregateName::DistinctCount => json!(distinct_endpoints.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match aggregate {
                AstAggregateName::DistinctCount => distinct_endpoints.len(),
                _ => usize::try_from(matched_edges).unwrap_or(usize::MAX),
            };
            Ok(Some((value, "association", seen)))
        }
        ResolvedExpr::Evidence(evidence) => {
            let mut matched_entries = 0u64;
            let mut distinct_identities = BTreeSet::new();
            for row_index in row_indices {
                let Some(row) = rows.get(*row_index) else {
                    return Err(exec_error(
                        "E_KERNEL_AGGREGATE",
                        "native helper aggregate row index was outside the reconstructed row set",
                        json!({ "row_index": row_index }),
                    ));
                };
                match aggregate {
                    AstAggregateName::Count => {
                        matched_entries += evidence_index.count_for(row, evidence) as u64;
                    }
                    AstAggregateName::Exists => {
                        if evidence_index.exists_for(row, evidence) {
                            matched_entries += 1;
                        }
                    }
                    AstAggregateName::DistinctCount => {
                        distinct_identities.extend(evidence_index.identities_for(row, evidence));
                    }
                    _ => return Ok(None),
                }
            }
            let value = match aggregate {
                AstAggregateName::Count => json!(matched_entries),
                AstAggregateName::Exists => Value::Bool(matched_entries > 0),
                AstAggregateName::DistinctCount => json!(distinct_identities.len() as u64),
                _ => unreachable!("guarded above"),
            };
            let seen = match aggregate {
                AstAggregateName::DistinctCount => distinct_identities.len(),
                _ => usize::try_from(matched_entries).unwrap_or(usize::MAX),
            };
            Ok(Some((value, "evidence", seen)))
        }
        _ => Ok(None),
    }
}

#[derive(Debug)]
struct NativeDirectAggregateEval {
    value: Value,
    values_seen: usize,
    strategy: &'static str,
}

#[derive(Debug)]
struct NativeDirectAggregateState<'a> {
    values_seen: usize,
    distinct: BTreeSet<String>,
    selected: Option<&'a Value>,
    exact_sum: Option<ExactDecimal>,
    float_sum: f64,
    saw_float: bool,
}

impl<'a> NativeDirectAggregateState<'a> {
    fn new() -> Self {
        Self {
            values_seen: 0,
            distinct: BTreeSet::new(),
            selected: None,
            exact_sum: None,
            float_sum: 0.0,
            saw_float: false,
        }
    }

    fn accumulate_value(
        &mut self,
        aggregate: AstAggregateName,
        path: &ResolvedPath,
        value: &'a Value,
    ) -> Result<(), BuildExecutionError> {
        if value.is_null() {
            return Ok(());
        }
        self.values_seen += 1;

        match aggregate {
            AstAggregateName::Count | AstAggregateName::Exists => {}
            AstAggregateName::DistinctCount => {
                self.distinct
                    .insert(logical_value_key(value, Some(path.logical_type.as_str())));
            }
            AstAggregateName::Min | AstAggregateName::Max => {
                let replace = match self.selected {
                    None => true,
                    Some(current) => {
                        value_ordering_typed(
                            value,
                            current,
                            Some(path.logical_type.as_str()),
                            path.collation_id,
                        )
                        .ok_or_else(|| {
                            exec_error(
                                "E_KERNEL_AGGREGATE",
                                "native min/max aggregate could not prove typed value ordering",
                                json!({
                                    "aggregate": aggregate_operator_name(aggregate),
                                    "logical_type": path.logical_type,
                                }),
                            )
                        })?
                        .is_lt()
                            == matches!(aggregate, AstAggregateName::Min)
                    }
                };
                if replace {
                    self.selected = Some(value);
                }
            }
            AstAggregateName::Sum | AstAggregateName::Avg => {
                accumulate_native_direct_numeric_value(
                    aggregate,
                    path,
                    value,
                    &mut self.exact_sum,
                    &mut self.float_sum,
                    &mut self.saw_float,
                )?;
            }
        }
        Ok(())
    }

    fn finish(
        self,
        aggregate: AstAggregateName,
    ) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
        let value = match aggregate {
            AstAggregateName::Count => json!(self.values_seen as u64),
            AstAggregateName::Exists => Value::Bool(self.values_seen > 0),
            AstAggregateName::DistinctCount => json!(self.distinct.len() as u64),
            AstAggregateName::Min | AstAggregateName::Max => {
                self.selected.cloned().unwrap_or(Value::Null)
            }
            AstAggregateName::Sum | AstAggregateName::Avg => {
                finish_native_direct_numeric_aggregate(
                    aggregate,
                    self.exact_sum,
                    self.float_sum,
                    self.saw_float,
                    self.values_seen,
                )?
            }
        };
        let strategy = match aggregate {
            AstAggregateName::Sum | AstAggregateName::Avg => "single_pass_typed_numeric",
            AstAggregateName::Min | AstAggregateName::Max => "single_pass_typed_order",
            AstAggregateName::Count
            | AstAggregateName::Exists
            | AstAggregateName::DistinctCount => "single_pass_value_ref",
        };
        Ok(NativeDirectAggregateEval {
            value,
            values_seen: self.values_seen,
            strategy,
        })
    }
}

fn native_direct_aggregate_eval(
    aggregate: AstAggregateName,
    star: bool,
    path: Option<&ResolvedPath>,
    rows: &[ExecutionRow],
) -> Result<NativeDirectAggregateEval, BuildExecutionError> {
    if star {
        let value = match aggregate {
            AstAggregateName::Count => json!(rows.len() as u64),
            AstAggregateName::Exists => Value::Bool(!rows.is_empty()),
            _ => {
                return Err(exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native direct aggregate received an unsupported star aggregate",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                ))
            }
        };
        return Ok(NativeDirectAggregateEval {
            value,
            values_seen: rows.len(),
            strategy: "row_count",
        });
    }

    let Some(path) = path else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native direct aggregate requires a direct path argument",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        ));
    };

    let mut state = NativeDirectAggregateState::new();
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct aggregate expected object rows",
                json!({}),
            ));
        };
        if object_path_is_redacted(row, path) {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native direct aggregate cannot evaluate a redacted path",
                json!({ "path": path.display_name }),
            ));
        }
        let Some(value) = object_property_value_for_path(row, path) else {
            continue;
        };
        state.accumulate_value(aggregate, path, value)?;
    }
    state.finish(aggregate)
}

pub(super) fn object_property_value_for_path<'a>(
    row: &'a MaterializedObjectRow,
    path: &ResolvedPath,
) -> Option<&'a Value> {
    let property_id = path.property_id?;
    if let Some(name) = row.property_ids.get(&property_id) {
        return row.properties.get(name);
    }
    row.properties.get(&path.display_name)
}

pub(super) fn accumulate_native_direct_numeric_value(
    aggregate: AstAggregateName,
    path: &ResolvedPath,
    value: &Value,
    exact_sum: &mut Option<ExactDecimal>,
    float_sum: &mut f64,
    saw_float: &mut bool,
) -> Result<(), BuildExecutionError> {
    let numeric_is_float = matches!(path.logical_type.as_str(), "float32" | "float64")
        || value
            .as_number()
            .is_some_and(|number| number.as_i64().is_none() && number.as_u64().is_none());
    if numeric_is_float {
        let Some(number) = value.as_f64() else {
            return Err(exec_error(
                "E_KERNEL_AGGREGATE",
                "native sum/avg aggregate requires numeric materialized values",
                json!({
                    "aggregate": aggregate_operator_name(aggregate),
                    "logical_type": path.logical_type,
                }),
            ));
        };
        *saw_float = true;
        *float_sum += number;
        return Ok(());
    }
    let Some(decimal) = parse_decimal_value(value) else {
        return Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native sum/avg aggregate requires numeric materialized values",
            json!({
                "aggregate": aggregate_operator_name(aggregate),
                "logical_type": path.logical_type,
            }),
        ));
    };
    *exact_sum = Some(match exact_sum.take() {
        Some(sum) => sum.checked_add(decimal).ok_or_else(|| {
            exec_error(
                "E_KERNEL_AGGREGATE",
                "native sum/avg aggregate overflowed decimal accumulator",
                json!({
                    "aggregate": aggregate_operator_name(aggregate),
                    "logical_type": path.logical_type,
                }),
            )
        })?,
        None => decimal,
    });
    Ok(())
}

pub(super) fn finish_native_direct_numeric_aggregate(
    aggregate: AstAggregateName,
    exact_sum: Option<ExactDecimal>,
    float_sum: f64,
    saw_float: bool,
    count: usize,
) -> Result<Value, BuildExecutionError> {
    if count == 0 {
        return Ok(Value::Null);
    }
    match aggregate {
        AstAggregateName::Sum if saw_float => Ok(json!(float_sum)),
        AstAggregateName::Avg if saw_float => Ok(json!(float_sum / count as f64)),
        AstAggregateName::Sum => Ok(exact_sum
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "empty native numeric aggregate accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .to_json_sum()),
        AstAggregateName::Avg => Ok(exact_sum
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "empty native numeric aggregate accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .checked_div_u64(count as u64)
            .ok_or_else(|| {
                exec_error(
                    "E_KERNEL_AGGREGATE",
                    "native avg aggregate overflowed decimal accumulator",
                    json!({ "aggregate": aggregate_operator_name(aggregate) }),
                )
            })?
            .to_json_sum()),
        _ => Err(exec_error(
            "E_KERNEL_AGGREGATE",
            "native numeric aggregate received a nonnumeric aggregate",
            json!({ "aggregate": aggregate_operator_name(aggregate) }),
        )),
    }
}
