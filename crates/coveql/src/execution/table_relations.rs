use super::*;

pub(super) fn table_relation_context_required(planned: &PlannedQuery) -> bool {
    !planned.resolved.method_chain.lookups.is_empty()
        || !planned.resolved.method_chain.joins.is_empty()
        || !planned.resolved.method_chain.set_operations.is_empty()
        || !planned.resolved.method_chain.windows.is_empty()
        || matches!(
            &planned.resolved.root,
            ResolvedRoot::Table(root) if table_root_uses_registered_rows(root)
        )
        || planned
            .resolved
            .method_chain
            .where_predicate
            .as_ref()
            .is_some_and(predicate_contains_table_exists)
}

pub(super) fn table_root_uses_registered_rows(root: &crate::ResolvedTableRoot) -> bool {
    matches!(
        &root.execution_authority,
        TableExecutionAuthority::MaterializedRows { .. }
            | TableExecutionAuthority::RawRows { .. }
            | TableExecutionAuthority::ExternalRows { .. }
    )
}

pub(super) fn predicate_contains_table_exists(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::TableExists(_)) => true,
        ResolvedPredicate::Compare { left, right, .. } => {
            expr_contains_table_exists(left) || expr_contains_table_exists(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => expr_contains_table_exists(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_table_exists(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_table_exists)
        }
    }
}

pub(super) fn expr_contains_table_exists(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::TableExists(_) => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(expr_contains_table_exists),
        ResolvedExpr::AggregateCall { arg, .. } => {
            arg.as_deref().is_some_and(expr_contains_table_exists)
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_table_exists(predicate)
                || expr_contains_table_exists(then_expr)
                || expr_contains_table_exists(else_expr)
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => false,
    }
}

pub(super) fn execute_table_relation_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    table: &crate::ResolvedTableRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    let left_rows = table_rows_all_columns(bytes, options, started, table)?;
    let mut joined_rows = left_rows
        .into_iter()
        .map(|row| namespace_table_projection_row(row, table))
        .collect::<Vec<_>>();
    let context = EvalContext::for_plan(&[], &[], planned);
    for lookup in &planned.resolved.method_chain.lookups {
        let right_rows = table_rows_all_columns(bytes, options, started, &lookup.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &lookup.right))
            .collect::<Vec<_>>();
        let mut next_rows = Vec::new();
        for left in joined_rows {
            let mut matches = Vec::new();
            for right in &right_rows {
                let candidate = merge_lookup_projection_rows(&left, right);
                let row = ExecutionRow::Projection(candidate.clone());
                let predicate_matches =
                    eval_predicate(&lookup.on, &row, &context).map_err(|err| {
                        exec_error(
                            "E_EXPRESSION",
                            format!("lookup predicate evaluation failed: {}", err.message),
                            json!({}),
                        )
                    })?;
                if predicate_matches
                    || (lookup.nulls_match && lookup_join_keys_are_both_null(&lookup.on, &row))
                {
                    matches.push(candidate);
                }
            }
            if matches.is_empty() {
                match lookup.unmatched_policy {
                    TableLookupUnmatchedPolicy::Nulls => next_rows.push(left),
                    TableLookupUnmatchedPolicy::Reject => {
                        return Err(exec_error(
                            "E_LOOKUP_UNMATCHED_ROW",
                            "lookup unmatched: reject encountered a visible left row without a visible matching right row",
                            json!({
                                "right_table": lookup.right.table_name,
                                "cardinality": format!("{:?}", lookup.cardinality),
                            }),
                        ));
                    }
                }
                continue;
            }
            if lookup.cardinality == TableLookupCardinality::One
                && matches.len() > 1
                && lookup.duplicate_policy == TableLookupDuplicatePolicy::Reject
            {
                return Err(exec_error(
                    "E_LOOKUP_DUPLICATE_MATCH",
                    "lookup cardinality: one found more than one visible right-side match",
                    json!({
                        "right_table": lookup.right.table_name,
                        "match_count": matches.len(),
                    }),
                ));
            }
            match lookup.cardinality {
                TableLookupCardinality::One => {
                    if let Some(row) = matches.into_iter().next() {
                        next_rows.push(row);
                    }
                }
                TableLookupCardinality::Many => {
                    next_rows.extend(matches);
                }
            }
        }
        joined_rows = next_rows;
    }
    for join in &planned.resolved.method_chain.joins {
        let right_rows = table_rows_all_columns(bytes, options, started, &join.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &join.right))
            .collect::<Vec<_>>();
        joined_rows = apply_materialized_table_join(joined_rows, &right_rows, join, &context)?;
    }
    for operation in &planned.resolved.method_chain.set_operations {
        let right_rows = table_rows_all_columns(bytes, options, started, &operation.right)?
            .into_iter()
            .map(|row| namespace_table_projection_row(row, &operation.right))
            .collect::<Vec<_>>();
        joined_rows = apply_materialized_set_operation(joined_rows, right_rows, operation);
    }
    if !planned.resolved.method_chain.windows.is_empty() {
        joined_rows = apply_materialized_windows(joined_rows, planned, &context)?;
    }
    let table_exists_storage = table_exists_row_storage(bytes, planned, options, started)?;
    let table_exists_refs = table_exists_storage
        .iter()
        .map(|(table_id, rows)| (table_id.clone(), rows.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let context =
        EvalContext::for_plan_with_table_exists_rows(&[], &[], planned, table_exists_refs);
    let rows = joined_rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

pub(super) fn apply_materialized_table_join(
    left_rows: Vec<MaterializedProjectionRow>,
    right_rows: &[MaterializedProjectionRow],
    join: &crate::ResolvedTableJoin,
    context: &EvalContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let mut out = Vec::new();
    let mut matched_right = BTreeSet::new();
    for left in left_rows {
        let mut matches = Vec::new();
        for (right_index, right) in right_rows.iter().enumerate() {
            let candidate = merge_lookup_projection_rows(&left, right);
            let row = ExecutionRow::Projection(candidate.clone());
            let predicate_matches = eval_predicate(&join.on, &row, context).map_err(|err| {
                exec_error(
                    "E_EXPRESSION",
                    format!("join predicate evaluation failed: {}", err.message),
                    json!({}),
                )
            })?;
            if predicate_matches
                || (join.nulls_match && lookup_join_keys_are_both_null(&join.on, &row))
            {
                matched_right.insert(right_index);
                matches.push(candidate);
            }
        }
        if matches.is_empty() {
            match join.join_kind {
                TableJoinKind::Left | TableJoinKind::Full => out.push(left),
                TableJoinKind::Anti => out.push(left),
                TableJoinKind::Inner | TableJoinKind::Right | TableJoinKind::Semi => {}
            }
            if join.unmatched_policy == TableLookupUnmatchedPolicy::Reject
                && matches!(join.join_kind, TableJoinKind::Inner | TableJoinKind::Semi)
            {
                return Err(exec_error(
                    "E_JOIN_UNMATCHED_ROW",
                    "join unmatched: reject encountered a visible left row without a visible matching right row",
                    json!({ "right_table": join.right.table_name }),
                ));
            }
            continue;
        }
        if join.cardinality == TableLookupCardinality::One
            && matches.len() > 1
            && join.duplicate_policy == TableLookupDuplicatePolicy::Reject
        {
            return Err(exec_error(
                "E_JOIN_DUPLICATE_MATCH",
                "join cardinality: one found more than one visible right-side match",
                json!({ "right_table": join.right.table_name, "match_count": matches.len() }),
            ));
        }
        match join.join_kind {
            TableJoinKind::Semi => out.push(left),
            TableJoinKind::Anti => {}
            TableJoinKind::Inner
            | TableJoinKind::Left
            | TableJoinKind::Right
            | TableJoinKind::Full => {
                if join.cardinality == TableLookupCardinality::One {
                    if let Some(row) = matches.into_iter().next() {
                        out.push(row);
                    }
                } else {
                    out.extend(matches);
                }
            }
        }
    }
    if matches!(join.join_kind, TableJoinKind::Right | TableJoinKind::Full) {
        for (right_index, right) in right_rows.iter().enumerate() {
            if !matched_right.contains(&right_index) {
                out.push(right.clone());
            }
        }
    }
    Ok(out)
}

pub(super) fn apply_materialized_set_operation(
    left_rows: Vec<MaterializedProjectionRow>,
    right_rows: Vec<MaterializedProjectionRow>,
    operation: &crate::ResolvedSetOperation,
) -> Vec<MaterializedProjectionRow> {
    match operation.kind {
        crate::SetOperationKind::Union if operation.all => {
            let mut out = left_rows;
            out.extend(right_rows);
            out
        }
        crate::SetOperationKind::Union => {
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .chain(right_rows)
                .filter(|row| seen.insert(materialized_projection_row_key(row)))
                .collect()
        }
        crate::SetOperationKind::Intersect => {
            let mut right_counts = multiset_counts(&right_rows);
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .filter(|row| {
                    let key = materialized_projection_row_key(row);
                    if operation.all {
                        let Some(count) = right_counts.get_mut(&key) else {
                            return false;
                        };
                        if *count == 0 {
                            return false;
                        }
                        *count -= 1;
                        true
                    } else if right_counts.contains_key(&key) {
                        seen.insert(key)
                    } else {
                        false
                    }
                })
                .collect()
        }
        crate::SetOperationKind::Except => {
            let mut right_counts = multiset_counts(&right_rows);
            let mut seen = BTreeSet::new();
            left_rows
                .into_iter()
                .filter(|row| {
                    let key = materialized_projection_row_key(row);
                    if operation.all {
                        if let Some(count) = right_counts.get_mut(&key) {
                            if *count > 0 {
                                *count -= 1;
                                return false;
                            }
                        }
                        true
                    } else if right_counts.contains_key(&key) {
                        false
                    } else {
                        seen.insert(key)
                    }
                })
                .collect()
        }
    }
}

pub(super) fn multiset_counts(rows: &[MaterializedProjectionRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts
            .entry(materialized_projection_row_key(row))
            .or_insert(0) += 1;
    }
    counts
}

pub(super) fn materialized_projection_row_key(row: &MaterializedProjectionRow) -> String {
    stable_value_key(&Value::Object(
        row.values
            .iter()
            .filter(|(key, _)| {
                !key.contains('.') && !key.starts_with(INTERNAL_PROJECTION_FIELD_PREFIX)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}

pub(super) fn table_rows_all_columns(
    bytes: &[u8],
    options: &ExecutionOptions,
    started: Instant,
    table: &crate::ResolvedTableRoot,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    match &table.execution_authority {
        TableExecutionAuthority::DeterministicProjection { projection_id } => {
            projection_rows_all_columns(bytes, options, started, projection_id)
        }
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows } => Ok(rows
            .iter()
            .map(|values| MaterializedProjectionRow {
                projection_id: table.table_id.clone(),
                values: values.clone(),
            })
            .collect()),
        TableExecutionAuthority::ExternalRows {
            provider_id: _,
            rows,
        } => Ok(rows
            .iter()
            .map(|values| MaterializedProjectionRow {
                projection_id: table.table_id.clone(),
                values: values.clone(),
            })
            .collect()),
    }
}

pub(super) fn lookup_join_keys_are_both_null(
    predicate: &ResolvedPredicate,
    row: &ExecutionRow,
) -> bool {
    let ResolvedPredicate::Compare {
        left,
        op: AstCompareOp::Eq,
        right,
    } = predicate
    else {
        return false;
    };
    let (ResolvedExpr::Path(left), ResolvedExpr::Path(right)) = (left, right) else {
        return false;
    };
    row.value_for_path(left).is_null() && row.value_for_path(right).is_null()
}

pub(super) fn table_exists_row_storage(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
) -> Result<Vec<(String, Vec<MaterializedProjectionRow>)>, BuildExecutionError> {
    let mut roots = Vec::new();
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_table_exists_roots(predicate, &mut roots);
    }
    roots
        .into_iter()
        .map(|root| {
            let rows = table_rows_all_columns(bytes, options, started, &root)?
                .into_iter()
                .map(|row| namespace_table_projection_row(row, &root))
                .collect::<Vec<_>>();
            Ok((root.table_id, rows))
        })
        .collect()
}

pub(super) fn collect_table_exists_roots(
    predicate: &ResolvedPredicate,
    out: &mut Vec<crate::ResolvedTableRoot>,
) {
    match predicate {
        ResolvedPredicate::Exists(ResolvedExpr::TableExists(exists)) => {
            if !out
                .iter()
                .any(|root| root.table_id == exists.right.table_id)
            {
                out.push(exists.right.clone());
            }
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_table_exists_expr_roots(left, out);
            collect_table_exists_expr_roots(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::BoolExpr(expr)
        | ResolvedPredicate::Exists(expr) => collect_table_exists_expr_roots(expr, out),
        ResolvedPredicate::Not(inner) => collect_table_exists_roots(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_table_exists_roots(part, out);
            }
        }
    }
}

pub(super) fn collect_table_exists_expr_roots(
    expr: &ResolvedExpr,
    out: &mut Vec<crate::ResolvedTableRoot>,
) {
    match expr {
        ResolvedExpr::TableExists(exists) => {
            if !out
                .iter()
                .any(|root| root.table_id == exists.right.table_id)
            {
                out.push(exists.right.clone());
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_table_exists_expr_roots(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg.as_deref() {
                collect_table_exists_expr_roots(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_table_exists_roots(predicate, out);
            collect_table_exists_expr_roots(then_expr, out);
            collect_table_exists_expr_roots(else_expr, out);
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => {}
    }
}

pub(super) fn namespace_table_projection_row(
    mut row: MaterializedProjectionRow,
    table: &crate::ResolvedTableRoot,
) -> MaterializedProjectionRow {
    let base = row.values.clone();
    for label in table_binding_labels(table) {
        for (column, value) in &base {
            row.values
                .insert(format!("{label}.{column}"), value.clone());
        }
    }
    row.projection_id = table.table_id.clone();
    row
}

pub(super) fn table_binding_labels(table: &crate::ResolvedTableRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &table.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &table.table_name) {
        labels.push(table.table_name.clone());
    }
    labels
}

pub(super) fn merge_lookup_projection_rows(
    left: &MaterializedProjectionRow,
    right: &MaterializedProjectionRow,
) -> MaterializedProjectionRow {
    let mut values = left.values.clone();
    for (key, value) in &right.values {
        if key.contains('.') {
            values.insert(key.clone(), value.clone());
        } else {
            values.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("lookup:{}:{}", left.projection_id, right.projection_id),
        values,
    }
}
