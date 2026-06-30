use super::*;

pub(super) fn try_native_typed_order_result(
    rows: &[ExecutionRow],
    planned: &crate::PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<
    Option<(
        CoveQlExecutionResult,
        ExecutionRowCounts,
        NativeTypedOrderReport,
    )>,
    BuildExecutionError,
> {
    if !native_typed_order_shape(planned) {
        return Ok(None);
    }
    let Some(order_path) = native_typed_order_path(planned) else {
        return Ok(None);
    };
    let Some(order) = &planned.resolved.method_chain.order_by else {
        return Ok(None);
    };
    let Some(select) = &planned.resolved.method_chain.select else {
        return Ok(None);
    };
    let mut ordered = Vec::<&MaterializedObjectRow>::with_capacity(rows.len());
    for row in rows {
        let ExecutionRow::Object(row) = row else {
            return Ok(None);
        };
        if object_path_is_redacted(row, order_path) {
            return Ok(None);
        }
        let value = row.value_for_path(order_path);
        if !native_typed_order_value_is_supported(&value, order_path.logical_type.as_str()) {
            return Ok(None);
        }
        if select.iter().any(|item| {
            matches!(
                &item.expr,
                ResolvedExpr::Path(path) if object_path_is_redacted(row, path)
            )
        }) {
            return Ok(None);
        }
        ordered.push(row);
    }
    ordered.sort_by(|left, right| {
        let left_value = left.value_for_path(order_path);
        let right_value = right.value_for_path(order_path);
        compare_typed_sort_values(
            &left_value,
            &right_value,
            order.direction,
            order.nulls,
            order_path.logical_type.as_str(),
            order_path.collation_id,
        )
        .then_with(|| stable_value_key(&left.to_json()).cmp(&stable_value_key(&right.to_json())))
    });
    let rows_sorted = ordered.len();
    let skip = planned.resolved.method_chain.skip.unwrap_or(0) as usize;
    let take = planned
        .resolved
        .method_chain
        .take
        .map(|value| value as usize)
        .unwrap_or(usize::MAX);
    let mut json_rows = Vec::new();
    for row in ordered.into_iter().skip(skip).take(take) {
        let mut object = serde_json::Map::new();
        for item in select {
            let ResolvedExpr::Path(path) = &item.expr else {
                return Ok(None);
            };
            object.insert(
                native_bool_group_count_output_name(&item.expr, item.alias.as_deref()),
                row.value_for_path(path),
            );
        }
        json_rows.push(Value::Object(object));
    }
    let output_rows = json_rows.len();
    let (result, row_counts) =
        finish_json_rows(json_rows, rows.len(), rows.len(), planned, options, started)?;
    Ok(Some((
        result,
        row_counts,
        NativeTypedOrderReport {
            order_property: order_path.display_name.clone(),
            logical_type: order_path.logical_type.clone(),
            collation_id: order_path.collation_id,
            sort_key_boundary: native_typed_order_sort_key_boundary(order_path),
            rows_sorted,
            output_rows,
        },
    )))
}

pub(super) fn native_typed_order_value_is_supported(value: &Value, logical_type: &str) -> bool {
    value.is_null()
        || match logical_type {
            "bool" | "boolean" => value.as_bool().is_some(),
            "utf8" | "string" => value.as_str().is_some(),
            "int8" | "int16" | "int32" | "int64" | "decimal64" | "date_days"
            | "timestamp_micros" | "timestamp_nanos" => value.as_i64().is_some(),
            "uint8" | "uint16" | "uint32" | "uint64" => value.as_u64().is_some(),
            "float32" | "float64" => value.as_f64().is_some(),
            _ => false,
        }
}

pub(super) fn native_typed_order_sort_key_boundary(path: &ResolvedPath) -> Option<String> {
    (path.physical_kind == "file_code").then(|| match path.collation_id {
        Some(id) if id == CollationKind::Utf8Bytewise.id() => {
            "decoded_filecode_sort_key_under_declared_collation".to_string()
        }
        _ => "decoded_filecode_sort_key_under_default_collation".to_string(),
    })
}

pub(super) fn compare_typed_sort_values(
    left: &Value,
    right: &Value,
    direction: AstOrderDirection,
    nulls: AstNullOrdering,
    logical_type: &str,
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
            let ordering = value_ordering_typed(left, right, Some(logical_type), collation_id)
                .unwrap_or(Ordering::Equal);
            match direction {
                AstOrderDirection::Asc => ordering,
                AstOrderDirection::Desc => ordering.reverse(),
            }
        }
    }
}
