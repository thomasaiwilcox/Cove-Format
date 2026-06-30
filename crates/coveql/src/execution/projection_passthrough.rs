use super::*;

pub(crate) fn execute_projection_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let pushed_filters = projection_filters_for_plan(planned);
    let output_columns = projection_output_columns_for_plan(planned);
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns,
        pushed_filters: pushed_filters.clone(),
        batch_size: options.batch_size,
        candidate_projection_rows: None,
    };
    let batches = projected_record_batches_from_cove_o_bytes(
        bytes,
        options.mapping_path.as_deref(),
        projection_id,
        &projection_options,
    )
    .map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("COVE-MAP projection readback failed: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    if let Some(batches) = projection_arrow_passthrough_batches(planned, &batches, &pushed_filters)
        .map_err(|err| {
            exec_error(
                "E_ARROW_OUTPUT",
                format!("projection Arrow passthrough failed: {err}"),
                json!({ "projection_id": projection_id }),
            )
        })?
    {
        let row_count = batches.iter().map(RecordBatch::num_rows).sum();
        return Ok((
            CoveQlExecutionResult::ArrowRecordBatches(batches),
            ExecutionRowCounts {
                input_rows: row_count,
                filtered_rows: row_count,
                output_rows: row_count,
            },
        ));
    }
    let rows = record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    let rows = rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    finish_materialized_rows(rows, &[], &[], &[], planned, options, started)
}

pub(super) fn projection_arrow_passthrough_batches(
    planned: &PlannedQuery,
    batches: &[RecordBatch],
    pushed_filters: &[ProjectionFilter],
) -> Result<Option<Vec<RecordBatch>>, ProjectionArrowPassthroughError> {
    if !projection_arrow_passthrough_shape(planned) {
        return Ok(None);
    }
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Ok(None);
    };
    if !projection_arrow_filters_are_exact(planned, schema.as_ref(), pushed_filters) {
        return Ok(None);
    }
    let Some(final_projection) = projection_arrow_final_projection(planned, schema.as_ref()) else {
        return Ok(None);
    };
    if let Some(indices) = final_projection {
        return batches
            .iter()
            .map(|batch| batch.project(&indices).map_err(Into::into))
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    Ok(Some(batches.to_vec()))
}

pub(super) fn projection_arrow_passthrough_shape(planned: &PlannedQuery) -> bool {
    if !matches!(
        planned.resolved.output_mode,
        CoveQlOutputMode::ArrowRecordBatch { .. }
    ) {
        return false;
    }
    let chain = &planned.resolved.method_chain;
    chain.lookups.is_empty()
        && chain.joins.is_empty()
        && chain.set_operations.is_empty()
        && chain.windows.is_empty()
        && chain.traversals.is_empty()
        && chain.graph_algorithms.is_empty()
        && chain.group_by.is_none()
        && chain.order_by.is_none()
        && chain.skip.is_none()
        && chain.take.is_none()
        && chain.history.is_none()
        && chain.changes.is_none()
}

pub(super) fn projection_arrow_filters_are_exact(
    planned: &PlannedQuery,
    schema: &arrow_schema::Schema,
    pushed_filters: &[ProjectionFilter],
) -> bool {
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return true;
    };
    if pushed_filters.is_empty() || projection_filters_for_predicate(predicate).is_none() {
        return false;
    }
    let root = projection_contract_root(&planned.resolved.root);
    pushed_filters.iter().all(|filter| {
        let predicate = projection_predicate_for_schema(schema, filter);
        let form = classify_predicate(&predicate, &root, "projection_readback");
        form.representation.proof_state == PredicateProofState::ProvenExact
            && form.residual_reason.is_none()
    })
}

pub(super) fn projection_contract_root(root: &ResolvedRoot) -> ResolvedRoot {
    match root {
        ResolvedRoot::Table(root) => ResolvedRoot::Projection(root.projection.clone()),
        ResolvedRoot::Projection(root) => ResolvedRoot::Projection(root.clone()),
        other => other.clone(),
    }
}

pub(super) fn projection_arrow_final_projection(
    planned: &PlannedQuery,
    schema: &arrow_schema::Schema,
) -> Option<Option<Vec<usize>>> {
    let Some(select) = &planned.resolved.method_chain.select else {
        return Some(None);
    };
    let mut indices = Vec::with_capacity(select.len());
    for item in select {
        let ResolvedExpr::Path(path) = &item.expr else {
            return None;
        };
        path.projection_column.as_ref()?;
        let column = projection_column(path);
        if item.alias.as_ref().is_some_and(|alias| alias != &column) {
            return None;
        }
        let index = schema.index_of(&column).ok()?;
        indices.push(index);
    }
    Some(Some(indices))
}

pub(super) fn projection_predicate_for_schema(
    schema: &arrow_schema::Schema,
    filter: &ProjectionFilter,
) -> ResolvedPredicate {
    match filter {
        ProjectionFilter::Compare {
            column,
            op,
            literal,
        } => ResolvedPredicate::Compare {
            left: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            op: projection_ast_op(*op),
            right: ResolvedExpr::Literal(projection_filter_resolved_literal(literal)),
        },
        ProjectionFilter::InList { column, literals } => ResolvedPredicate::InList {
            expr: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            values: literals
                .iter()
                .map(projection_filter_resolved_literal)
                .collect(),
        },
        ProjectionFilter::IsNull { column, negated } => ResolvedPredicate::NullCheck {
            expr: ResolvedExpr::Path(projection_filter_resolved_path(schema, column)),
            negated: *negated,
        },
    }
}

pub(super) fn projection_filter_resolved_path(
    schema: &arrow_schema::Schema,
    column: &str,
) -> ResolvedPath {
    let (logical_type, nullable) = schema
        .field_with_name(column)
        .map(|field| {
            (
                logical_type_for_arrow(field.data_type()).to_string(),
                field.is_nullable(),
            )
        })
        .unwrap_or_else(|_| ("utf8".into(), true));
    ResolvedPath {
        display_name: column.into(),
        root_kind: ResolvedPathRootKind::Projection,
        object_type_id: None,
        property_id: None,
        association_type_id: None,
        evidence_field_id: None,
        projection_id: None,
        projection_column: Some(column.into()),
        system_field: None,
        logical_type: logical_type.clone(),
        physical_kind: physical_kind_for_logical_name(&logical_type).into(),
        collation_id: None,
        nullable,
        null_policy: if nullable { "allow" } else { "reject" }.into(),
        temporal_role: None,
        code_domain_id: CodeDomainId::Placeholder {
            root: "projection".into(),
            object_type_id: None,
            property_id: None,
            projection_id: None,
            field: Some(column.into()),
        },
    }
}

pub(super) fn projection_filter_resolved_literal(
    literal: &ProjectionFilterLiteral,
) -> ResolvedLiteral {
    match literal {
        ProjectionFilterLiteral::Null => ResolvedLiteral {
            literal: crate::AstLiteral::Null,
            logical_type: "null".into(),
            canonical: "null".into(),
            typed_value: ResolvedLiteralValue::Null,
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Boolean(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Boolean(*value),
            logical_type: "bool".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Boolean(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Int64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Integer(value.to_string()),
            logical_type: "int64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::SignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::UInt64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Integer(value.to_string()),
            logical_type: "uint64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::UnsignedInteger(*value),
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Float64(value) => ResolvedLiteral {
            literal: crate::AstLiteral::Decimal(value.to_string()),
            logical_type: "float64".into(),
            canonical: value.to_string(),
            typed_value: ResolvedLiteralValue::Decimal {
                canonical: value.to_string(),
                precision: 0,
                scale: 0,
            },
            precision: None,
            scale: None,
        },
        ProjectionFilterLiteral::Utf8(value) => ResolvedLiteral {
            literal: crate::AstLiteral::String(value.clone()),
            logical_type: "utf8".into(),
            canonical: value.clone(),
            typed_value: ResolvedLiteralValue::String(value.clone()),
            precision: None,
            scale: None,
        },
    }
}

pub(super) fn projection_ast_op(op: ProjectionFilterOp) -> AstCompareOp {
    match op {
        ProjectionFilterOp::Eq => AstCompareOp::Eq,
        ProjectionFilterOp::Ne => AstCompareOp::Ne,
        ProjectionFilterOp::Lt => AstCompareOp::Lt,
        ProjectionFilterOp::LtEq => AstCompareOp::Le,
        ProjectionFilterOp::Gt => AstCompareOp::Gt,
        ProjectionFilterOp::GtEq => AstCompareOp::Ge,
    }
}

pub(super) fn logical_type_for_arrow(data_type: &DataType) -> &'static str {
    match data_type {
        DataType::Boolean => "bool",
        DataType::Int8 => "int8",
        DataType::Int16 => "int16",
        DataType::Int32 => "int32",
        DataType::Int64 => "int64",
        DataType::UInt8 => "uint8",
        DataType::UInt16 => "uint16",
        DataType::UInt32 => "uint32",
        DataType::UInt64 => "uint64",
        DataType::Float32 => "float32",
        DataType::Float64 => "float64",
        DataType::Date32 => "date_days",
        DataType::Timestamp(_, _) => "timestamp_micros",
        _ => "utf8",
    }
}

pub(super) fn physical_kind_for_logical_name(logical: &str) -> &'static str {
    match logical {
        "bool" | "boolean" => "boolean",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "num_code",
        "uuid" | "decimal128" => "fixed_bytes",
        "list" => "list",
        "struct" => "struct",
        "map" => "map",
        _ => "var_bytes",
    }
}

pub(crate) fn projection_root_execution_rows(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let pushed_filters = projection_filters_for_plan(planned);
    let output_columns = projection_output_columns_for_plan(planned);
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns,
        pushed_filters,
        batch_size: options.batch_size,
        candidate_projection_rows: None,
    };
    let batches = projected_record_batches_from_cove_o_bytes(
        bytes,
        options.mapping_path.as_deref(),
        projection_id,
        &projection_options,
    )
    .map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("COVE-MAP projection readback failed: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    let rows = record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    Ok(rows.into_iter().map(ExecutionRow::Projection).collect())
}

pub(super) fn projection_rows_all_columns(
    bytes: &[u8],
    options: &ExecutionOptions,
    started: Instant,
    projection_id: &str,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let projection_options = ProjectionBatchOptions {
        max_rows: None,
        output_columns: None,
        pushed_filters: Vec::new(),
        batch_size: options.batch_size,
        candidate_projection_rows: None,
    };
    let batches = projected_record_batches_from_cove_o_bytes(
        bytes,
        options.mapping_path.as_deref(),
        projection_id,
        &projection_options,
    )
    .map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("COVE-MAP projection readback failed: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })?;
    record_batches_to_projection_rows(projection_id, &batches).map_err(|err| {
        exec_error(
            "E_PROJECTION_READBACK",
            format!("cannot materialize projection rows: {err}"),
            json!({ "projection_id": projection_id }),
        )
    })
}

pub(crate) fn projection_readback_pushdown_report(
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    row_counts: &ExecutionRowCounts,
) -> PushdownReport {
    let output_columns = projection_output_columns_for_plan(planned);
    let pushed_filters = projection_filters_for_plan(planned);
    pushdown::projection_readback_report(
        planned,
        &options.pushdown,
        output_columns.as_deref(),
        pushed_filters.len(),
        row_counts.input_rows,
        row_counts.output_rows,
    )
}

pub(super) fn projection_filters_for_plan(planned: &PlannedQuery) -> Vec<ProjectionFilter> {
    if !matches!(
        planned.resolved.root,
        ResolvedRoot::Projection(_) | ResolvedRoot::Table(_)
    ) {
        return Vec::new();
    }
    let Some(predicate) = &planned.resolved.method_chain.where_predicate else {
        return Vec::new();
    };
    projection_filters_for_predicate(predicate).unwrap_or_default()
}

pub(super) fn projection_output_columns_for_plan(planned: &PlannedQuery) -> Option<Vec<String>> {
    projection_output_columns_for_parts(&planned.resolved.root, &planned.resolved.method_chain)
}

pub(super) fn projection_output_columns_for_parts(
    root: &ResolvedRoot,
    method_chain: &ResolvedMethodChain,
) -> Option<Vec<String>> {
    if !matches!(root, ResolvedRoot::Projection(_) | ResolvedRoot::Table(_)) {
        return None;
    }
    let Some(select) = &method_chain.select else {
        return None;
    };
    let mut columns = BTreeSet::new();
    for item in select {
        collect_projection_expr_column(&item.expr, &mut columns);
    }
    if let Some(predicate) = &method_chain.where_predicate {
        collect_projection_predicate_columns(predicate, &mut columns);
    }
    if let Some(order) = &method_chain.order_by {
        collect_projection_expr_column(&order.expr, &mut columns);
    }
    if let Some(group_by) = &method_chain.group_by {
        for expr in group_by {
            collect_projection_expr_column(expr, &mut columns);
        }
    }
    (!columns.is_empty()).then(|| columns.into_iter().collect())
}

pub(super) fn collect_projection_predicate_columns(
    predicate: &ResolvedPredicate,
    columns: &mut BTreeSet<String>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_projection_expr_column(left, columns);
            collect_projection_expr_column(right, columns);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_projection_expr_column(expr, columns),
        ResolvedPredicate::Not(inner) => collect_projection_predicate_columns(inner, columns),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_projection_predicate_columns(part, columns);
            }
        }
    }
}

pub(super) fn collect_projection_expr_column(expr: &ResolvedExpr, columns: &mut BTreeSet<String>) {
    match expr {
        ResolvedExpr::Path(path) => {
            if path.projection_column.is_some() {
                columns.insert(projection_column(path));
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_projection_expr_column(arg, columns);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_projection_expr_column(arg, columns);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_projection_predicate_columns(predicate, columns);
            collect_projection_expr_column(then_expr, columns);
            collect_projection_expr_column(else_expr, columns);
        }
        ResolvedExpr::TableExists(exists) => {
            collect_projection_predicate_columns(&exists.on, columns);
        }
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) | ResolvedExpr::Literal(_) => {}
    }
}

pub(super) fn projection_filters_for_predicate(
    predicate: &ResolvedPredicate,
) -> Option<Vec<ProjectionFilter>> {
    match predicate {
        ResolvedPredicate::And(parts) => {
            let mut out = Vec::new();
            for part in parts {
                out.extend(projection_filters_for_predicate(part)?);
            }
            Some(out)
        }
        ResolvedPredicate::Compare { left, op, right } => {
            projection_compare_filter(left, *op, right).map(|filter| vec![filter])
        }
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            Some(vec![ProjectionFilter::InList {
                column: projection_column(path),
                literals,
            }])
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let path = projection_path(expr)?;
            Some(vec![ProjectionFilter::IsNull {
                column: projection_column(path),
                negated: *negated,
            }])
        }
        ResolvedPredicate::BoolExpr(expr) => {
            projection_bool_filter(expr, true).map(|filter| vec![filter])
        }
        ResolvedPredicate::Not(inner) => projection_filters_for_negated_predicate(inner),
        ResolvedPredicate::Or(parts) => {
            projection_same_column_equality_or_filter(parts).map(|filter| vec![filter])
        }
        ResolvedPredicate::Exists(_) => None,
    }
}

pub(super) fn projection_filters_for_negated_predicate(
    predicate: &ResolvedPredicate,
) -> Option<Vec<ProjectionFilter>> {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => {
            projection_compare_filter(left, negated_compare_op(*op), right)
                .map(|filter| vec![filter])
        }
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            if literals
                .iter()
                .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
            {
                return None;
            }
            Some(
                literals
                    .into_iter()
                    .map(|literal| ProjectionFilter::Compare {
                        column: projection_column(path),
                        op: ProjectionFilterOp::Ne,
                        literal,
                    })
                    .collect(),
            )
        }
        ResolvedPredicate::NullCheck { expr, negated } => {
            let path = projection_path(expr)?;
            Some(vec![ProjectionFilter::IsNull {
                column: projection_column(path),
                negated: !*negated,
            }])
        }
        ResolvedPredicate::BoolExpr(expr) => {
            projection_bool_filter(expr, false).map(|filter| vec![filter])
        }
        ResolvedPredicate::Not(inner) => projection_filters_for_predicate(inner),
        ResolvedPredicate::And(_) | ResolvedPredicate::Or(_) | ResolvedPredicate::Exists(_) => None,
    }
}

struct ProjectionEqualitySetFilter {
    column: String,
    literals: Vec<ProjectionFilterLiteral>,
}

pub(super) fn projection_same_column_equality_or_filter(
    parts: &[ResolvedPredicate],
) -> Option<ProjectionFilter> {
    let mut parts = parts.iter();
    let first = projection_single_equality_set_filter(parts.next()?)?;
    let mut column = first.column;
    let mut literals = first.literals;
    for part in parts {
        let filter = projection_single_equality_set_filter(part)?;
        if filter.column != column {
            return None;
        }
        column = filter.column;
        for literal in filter.literals {
            if matches!(literal, ProjectionFilterLiteral::Null) {
                return None;
            }
            if !literals.contains(&literal) {
                literals.push(literal);
            }
        }
    }
    if literals.is_empty()
        || literals
            .iter()
            .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
    {
        return None;
    }
    Some(ProjectionFilter::InList { column, literals })
}

fn projection_single_equality_set_filter(
    predicate: &ResolvedPredicate,
) -> Option<ProjectionEqualitySetFilter> {
    match predicate {
        ResolvedPredicate::Compare {
            left,
            op: AstCompareOp::Eq,
            right,
        } => match projection_compare_filter(left, AstCompareOp::Eq, right)? {
            ProjectionFilter::Compare {
                column,
                op: ProjectionFilterOp::Eq,
                literal,
            } if !matches!(literal, ProjectionFilterLiteral::Null) => {
                Some(ProjectionEqualitySetFilter {
                    column,
                    literals: vec![literal],
                })
            }
            _ => None,
        },
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literals = values
                .iter()
                .map(projection_filter_literal)
                .collect::<Option<Vec<_>>>()?;
            if literals.is_empty()
                || literals
                    .iter()
                    .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
            {
                return None;
            }
            Some(ProjectionEqualitySetFilter {
                column: projection_column(path),
                literals,
            })
        }
        ResolvedPredicate::Or(parts) => {
            let ProjectionFilter::InList { column, literals } =
                projection_same_column_equality_or_filter(parts)?
            else {
                return None;
            };
            Some(ProjectionEqualitySetFilter { column, literals })
        }
        _ => None,
    }
}

pub(super) fn projection_bool_filter(expr: &ResolvedExpr, value: bool) -> Option<ProjectionFilter> {
    let path = projection_path(expr)?;
    matches!(path.logical_type.as_str(), "bool" | "boolean").then(|| ProjectionFilter::Compare {
        column: projection_column(path),
        op: ProjectionFilterOp::Eq,
        literal: ProjectionFilterLiteral::Boolean(value),
    })
}

pub(super) fn projection_compare_filter(
    left: &ResolvedExpr,
    op: AstCompareOp,
    right: &ResolvedExpr,
) -> Option<ProjectionFilter> {
    if let (Some(path), ResolvedExpr::Literal(literal)) = (projection_path(left), right) {
        return projection_filter_op(op).and_then(|op| {
            Some(ProjectionFilter::Compare {
                column: projection_column(path),
                op,
                literal: projection_filter_literal(literal)?,
            })
        });
    }
    if let (ResolvedExpr::Literal(literal), Some(path)) = (left, projection_path(right)) {
        return projection_filter_op(invert_compare_op(op)).and_then(|op| {
            Some(ProjectionFilter::Compare {
                column: projection_column(path),
                op,
                literal: projection_filter_literal(literal)?,
            })
        });
    }
    None
}

pub(super) fn projection_path(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    let ResolvedExpr::Path(path) = expr else {
        return None;
    };
    path.projection_column.as_ref()?;
    Some(path)
}

pub(super) fn projection_column(path: &ResolvedPath) -> String {
    let column = path
        .projection_column
        .clone()
        .unwrap_or_else(|| path.display_name.clone());
    if matches!(path.root_kind, crate::ResolvedPathRootKind::Table) {
        column
            .rsplit_once('.')
            .map(|(_, unqualified)| unqualified.to_string())
            .unwrap_or(column)
    } else {
        column
    }
}

pub(super) fn projection_filter_op(op: AstCompareOp) -> Option<ProjectionFilterOp> {
    match op {
        AstCompareOp::Eq => Some(ProjectionFilterOp::Eq),
        AstCompareOp::Ne => Some(ProjectionFilterOp::Ne),
        AstCompareOp::Lt => Some(ProjectionFilterOp::Lt),
        AstCompareOp::Le => Some(ProjectionFilterOp::LtEq),
        AstCompareOp::Gt => Some(ProjectionFilterOp::Gt),
        AstCompareOp::Ge => Some(ProjectionFilterOp::GtEq),
    }
}

pub(super) fn invert_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Eq,
        AstCompareOp::Ne => AstCompareOp::Ne,
        AstCompareOp::Lt => AstCompareOp::Gt,
        AstCompareOp::Le => AstCompareOp::Ge,
        AstCompareOp::Gt => AstCompareOp::Lt,
        AstCompareOp::Ge => AstCompareOp::Le,
    }
}

pub(super) fn negated_compare_op(op: AstCompareOp) -> AstCompareOp {
    match op {
        AstCompareOp::Eq => AstCompareOp::Ne,
        AstCompareOp::Ne => AstCompareOp::Eq,
        AstCompareOp::Lt => AstCompareOp::Ge,
        AstCompareOp::Le => AstCompareOp::Gt,
        AstCompareOp::Gt => AstCompareOp::Le,
        AstCompareOp::Ge => AstCompareOp::Lt,
    }
}

pub(super) fn projection_filter_literal(
    literal: &ResolvedLiteral,
) -> Option<ProjectionFilterLiteral> {
    match &literal.typed_value {
        ResolvedLiteralValue::Null => Some(ProjectionFilterLiteral::Null),
        ResolvedLiteralValue::Boolean(value) => Some(ProjectionFilterLiteral::Boolean(*value)),
        ResolvedLiteralValue::SignedInteger(value) => Some(ProjectionFilterLiteral::Int64(*value)),
        ResolvedLiteralValue::UnsignedInteger(value) => {
            Some(ProjectionFilterLiteral::UInt64(*value))
        }
        ResolvedLiteralValue::BigInteger(_) => None,
        ResolvedLiteralValue::Decimal { canonical, .. } => canonical
            .parse::<f64>()
            .ok()
            .map(ProjectionFilterLiteral::Float64),
        ResolvedLiteralValue::String(value) => Some(ProjectionFilterLiteral::Utf8(value.clone())),
        ResolvedLiteralValue::TimestampMicros { micros, .. } => {
            Some(ProjectionFilterLiteral::Int64(*micros))
        }
        ResolvedLiteralValue::Uuid { canonical_hex, .. } => {
            Some(ProjectionFilterLiteral::Utf8(canonical_hex.clone()))
        }
        ResolvedLiteralValue::Binary {
            canonical_hex,
            bytes,
        } => Some(ProjectionFilterLiteral::Utf8(binary_materialized_value(
            bytes,
            canonical_hex,
        ))),
    }
}

pub(super) fn binary_materialized_value(bytes: &[u8], canonical_hex: &str) -> String {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .unwrap_or_else(|_| canonical_hex.to_string())
}
