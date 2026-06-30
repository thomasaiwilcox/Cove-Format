use super::*;

pub(super) fn synthetic_projection_for_table_contract(
    contract: &TableSurfaceContract,
) -> ResolvedProjectionRoot {
    ResolvedProjectionRoot {
        projection_id: contract.table_id.clone(),
        mapping_id: format!("table-surface:{}", contract.table_id),
        mapping_version: contract.contract_version.clone(),
        output_table: Some(contract.table_name.clone()),
        row_grain: Some(contract.row_grain.clone()),
        anchor: None,
        temporal_mode: Some(format!("{:?}", contract.temporal_authority).to_ascii_lowercase()),
        columns: contract
            .logical_column_map
            .iter()
            .map(|column| ResolvedProjectionColumn {
                name: column.name.clone(),
                value: column
                    .source_path
                    .clone()
                    .unwrap_or_else(|| column.name.clone()),
                logical_type: column.logical_type.clone(),
                nested_shape: None,
                conflict_policy: "table_surface_contract".into(),
                missing_policy: if column.nullable {
                    "missing_is_null".into()
                } else {
                    "reject".into()
                },
                source_property_id: None,
            })
            .collect(),
        assertion_ids: Vec::new(),
        multi_value_policy: Some("reject".into()),
        missing_policy: contract.null_missing_nan_policy.clone(),
        ordering: contract.canonical_order.clone(),
        evidence_policy: "table_surface_contract".into(),
        output_modes: vec!["json_rows".into(), "arrow_record_batch".into()],
        column_count: contract.logical_column_map.len(),
    }
}

pub(super) fn registered_table_rows_for_recursive_cte(
    table: &ResolvedTableRoot,
) -> Option<Vec<TableSurfaceRow>> {
    match &table.execution_authority {
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows }
        | TableExecutionAuthority::ExternalRows { rows, .. } => Some(rows.clone()),
        TableExecutionAuthority::DeterministicProjection { .. } => None,
    }
}

pub(super) fn recursive_cte_key_for_row(
    row: &TableSurfaceRow,
    key: &ResolvedExpr,
    options: &ResolveOptions,
) -> Result<String, BuildResolvedQueryError> {
    let ResolvedExpr::Path(path) = key else {
        return Err(profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            "withRecursive key must resolve to a table column path for materialized fixpoint execution",
            options,
        ));
    };
    let field = path
        .projection_column
        .as_deref()
        .unwrap_or(path.display_name.as_str());
    let value = row
        .get(field)
        .or_else(|| {
            field
                .rsplit_once('.')
                .and_then(|(_, unqualified)| row.get(unqualified))
        })
        .unwrap_or(&serde_json::Value::Null);
    serde_json::to_string(value).map_err(|error| {
        profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            format!("withRecursive key could not be serialized: {error}"),
            options,
        )
    })
}

impl Resolver {
    pub(super) fn resolve_common_table_expression(
        &mut self,
        args: &[AstProfileArgument],
        recursive: bool,
    ) -> Result<ResolvedCommonTableExpression, BuildResolvedQueryError> {
        let mut name = None;
        let mut query = None;
        let mut step = None;
        let mut max_iterations = None;
        let mut key_expr = None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("name"), AstProfileArgumentValue::Expr(expr)) => {
                    name = Some(self.profile_enum_literal("name", expr)?);
                }
                (Some("query" | "seed"), AstProfileArgumentValue::Expr(expr)) => {
                    query = Some(expr);
                }
                (Some("step"), AstProfileArgumentValue::Expr(expr)) => {
                    step = Some(expr);
                }
                (Some("maxIterations" | "max_iterations"), AstProfileArgumentValue::Expr(expr)) => {
                    max_iterations = Some(self.resolve_usize_profile_arg("maxIterations", expr)?);
                }
                (Some("key"), AstProfileArgumentValue::Expr(expr)) => {
                    key_expr = Some(expr);
                }
                (Some(candidate), AstProfileArgumentValue::Expr(expr)) if query.is_none() => {
                    name = Some(candidate.to_string());
                    query = Some(expr);
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported CTE argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "with requires a named table binding, such as with(alias: table(name))",
                        &self.options,
                    ));
                }
            }
        }
        let query = query.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "with requires a table(...) query root binding",
                &self.options,
            )
        })?;
        let AstExpr::RootBinding(binding) = &query.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "with query must be a table(...) root binding",
                &self.options,
            ));
        };
        let mut resolved = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut resolved, binding.alias.as_ref());
        let mut table = match resolved {
            ResolvedRoot::Table(table) => table,
            _ => {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "with query must resolve to a CoveQL/Table surface",
                    &self.options,
                ))
            }
        };
        let name = name
            .or_else(|| binding.alias.as_ref().map(|alias| alias.name.clone()))
            .ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "with requires a CTE name or an aliased table query",
                    &self.options,
                )
            })?;
        if recursive && max_iterations.is_none() {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "withRecursive requires finite maxIterations",
                &self.options,
            ));
        }
        let step_table = if let Some(step) = step {
            let AstExpr::RootBinding(binding) = &step.node else {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive step must be a table(...) root binding",
                    &self.options,
                ));
            };
            let mut resolved = self.resolve_root(&binding.root.node)?;
            apply_root_alias(&mut resolved, binding.alias.as_ref());
            match resolved {
                ResolvedRoot::Table(table) => Some(table),
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "withRecursive step must resolve to a CoveQL/Table surface",
                        &self.options,
                    ))
                }
            }
        } else {
            None
        };
        let key = key_expr
            .map(|expr| self.resolve_expr(expr, &ResolvedRoot::Table(table.clone())))
            .transpose()?;
        if let (true, Some(step_table_ref)) = (recursive, step_table.as_ref()) {
            let key = key.as_ref().ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive with a step table requires a key expression",
                    &self.options,
                )
            })?;
            let max_iterations = max_iterations.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive requires finite maxIterations",
                    &self.options,
                )
            })?;
            table = self.materialized_recursive_cte_table(
                name.as_str(),
                table,
                step_table_ref,
                key,
                max_iterations,
            )?;
        }
        Ok(ResolvedCommonTableExpression {
            name,
            table,
            recursive,
            max_iterations,
            step_table,
            key,
            execution_authority: if recursive {
                "bounded_materialized_recursive_cte_authority".into()
            } else {
                "materialized_cte_table_authority".into()
            },
        })
    }

    pub(super) fn materialized_recursive_cte_table(
        &self,
        cte_name: &str,
        mut seed: ResolvedTableRoot,
        step: &ResolvedTableRoot,
        key: &ResolvedExpr,
        max_iterations: usize,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        let mut rows = registered_table_rows_for_recursive_cte(&seed).ok_or_else(|| {
            profile_rejection(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "withRecursive seed must be a registered materialized/raw/external row authority",
                &self.options,
            )
        })?;
        let step_rows = registered_table_rows_for_recursive_cte(step).ok_or_else(|| {
            profile_rejection(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "withRecursive step must be a registered materialized/raw/external row authority",
                &self.options,
            )
        })?;
        let mut seen = BTreeSet::new();
        for row in &rows {
            seen.insert(recursive_cte_key_for_row(row, key, &self.options)?);
        }
        for _ in 0..max_iterations {
            let mut added = 0usize;
            for row in &step_rows {
                let key = recursive_cte_key_for_row(row, key, &self.options)?;
                if seen.insert(key) {
                    rows.push(row.clone());
                    added += 1;
                }
            }
            if added == 0 {
                break;
            }
        }
        seed.table_name = cte_name.to_string();
        seed.table_id = format!("cte:{cte_name}");
        seed.table_surface_contract.table_name = seed.table_name.clone();
        seed.table_surface_contract.table_id = seed.table_id.clone();
        seed.execution_authority = TableExecutionAuthority::MaterializedRows { rows };
        Ok(seed)
    }

    pub(super) fn resolve_table_lookup(
        &mut self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedTableLookup, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "lookup requires a declared CoveQL bridge from the current root grain to CoveQL/Table",
                &self.options,
            ));
        }
        let mut target = None;
        let mut on = None;
        let mut cardinality = TableLookupCardinality::One;
        let mut cardinality_explicit = false;
        let mut unmatched_policy = TableLookupUnmatchedPolicy::Nulls;
        let mut duplicate_policy = TableLookupDuplicatePolicy::Reject;
        let mut duplicate_explicit = false;
        let mut nulls_match = false;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some("cardinality"), AstProfileArgumentValue::Expr(expr))
                    if !cardinality_explicit =>
                {
                    cardinality = self.resolve_lookup_cardinality(expr)?;
                    cardinality_explicit = true;
                }
                (Some("unmatched"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = self.resolve_lookup_unmatched_policy(expr)?;
                }
                (Some("required"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = if self.resolve_bool_profile_arg("required", expr)? {
                        TableLookupUnmatchedPolicy::Reject
                    } else {
                        TableLookupUnmatchedPolicy::Nulls
                    };
                }
                (Some("duplicate" | "duplicate_policy"), AstProfileArgumentValue::Expr(expr))
                    if !duplicate_explicit =>
                {
                    duplicate_policy = self.resolve_lookup_duplicate_policy(expr)?;
                    duplicate_explicit = true;
                }
                (Some("nulls_match"), AstProfileArgumentValue::Expr(expr)) => {
                    nulls_match = self.resolve_bool_profile_arg("nulls_match", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported lookup argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "lookup requires one table root binding and one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let target = target.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires a right-hand table root binding",
                &self.options,
            )
        })?;
        let on = on.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires an on: predicate",
                &self.options,
            )
        })?;
        let AstExpr::RootBinding(binding) = &target.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup target must be a table(...) root binding",
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup target must resolve to a table surface",
                &self.options,
            ));
        };
        self.lookup_scope.push(right.clone());
        let on = self.resolve_predicate(on, root)?;
        self.lookup_scope.pop();
        self.validate_table_lookup_predicate(&on)?;
        if cardinality == TableLookupCardinality::Many && !duplicate_explicit {
            duplicate_policy = TableLookupDuplicatePolicy::EmitAll;
        }
        if cardinality == TableLookupCardinality::One
            && duplicate_policy == TableLookupDuplicatePolicy::EmitAll
        {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup duplicate: many requires cardinality: many in CoveQL 0.1",
                &self.options,
            ));
        }
        Ok(ResolvedTableLookup {
            right,
            on,
            join_kind: TableLookupJoinKind::LeftPreserving,
            cardinality,
            unmatched_policy,
            duplicate_policy,
            nulls_match,
        })
    }

    pub(super) fn resolve_table_join(
        &mut self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
        forced_kind: Option<TableJoinKind>,
    ) -> Result<ResolvedTableJoin, BuildResolvedQueryError> {
        let ResolvedRoot::Table(left) = root else {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "cross-profile join requires a registered CoveQL bridge and materialized cross-profile join authority",
                &self.options,
            ));
        };
        let mut target = None;
        let mut on = None;
        let mut join_kind = forced_kind.unwrap_or(TableJoinKind::Inner);
        let mut cardinality = TableLookupCardinality::Many;
        let mut unmatched_policy = TableLookupUnmatchedPolicy::Nulls;
        let mut duplicate_policy = TableLookupDuplicatePolicy::EmitAll;
        let mut nulls_match = false;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some("kind"), AstProfileArgumentValue::Expr(expr)) if forced_kind.is_none() => {
                    join_kind = self.resolve_join_kind(expr)?;
                }
                (Some("cardinality"), AstProfileArgumentValue::Expr(expr)) => {
                    cardinality = self.resolve_lookup_cardinality(expr)?;
                }
                (Some("unmatched"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = self.resolve_lookup_unmatched_policy(expr)?;
                }
                (Some("duplicate" | "duplicate_policy"), AstProfileArgumentValue::Expr(expr)) => {
                    duplicate_policy = self.resolve_lookup_duplicate_policy(expr)?;
                }
                (Some("nulls_match"), AstProfileArgumentValue::Expr(expr)) => {
                    nulls_match = self.resolve_bool_profile_arg("nulls_match", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported join argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "join requires one table root binding and one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let right = self.resolve_table_binding_arg(
            target.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "join requires a right-hand table root binding",
                    &self.options,
                )
            })?,
            "join",
        )?;
        self.lookup_scope.push(right.clone());
        let on = self.resolve_predicate(
            on.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "join requires an on: predicate",
                    &self.options,
                )
            })?,
            root,
        )?;
        self.lookup_scope.pop();
        self.validate_table_lookup_predicate(&on)?;
        let bridge_contract = self.resolve_table_table_bridge(left, &right)?;
        Ok(ResolvedTableJoin {
            right,
            on,
            join_kind,
            cardinality,
            unmatched_policy,
            duplicate_policy,
            nulls_match,
            bridge_contract,
        })
    }

    pub(super) fn resolve_table_table_bridge(
        &self,
        left: &ResolvedTableRoot,
        right: &ResolvedTableRoot,
    ) -> Result<Option<CoveQlBridgeRegistration>, BuildResolvedQueryError> {
        let bridge = self
            .options
            .bridge_contracts
            .iter()
            .find(|bridge| {
                bridge.source_profile == CoveQlProfileId::Table
                    && bridge.target_profile == CoveQlProfileId::Table
                    && bridge.source_grain == left.row_grain
                    && bridge.target_grain == right.row_grain
                    && bridge.exact
            })
            .cloned();
        let left_domains = &left.table_surface_contract.code_domain_contexts;
        let right_domains = &right.table_surface_contract.code_domain_contexts;
        let domains_need_bridge =
            !left_domains.is_empty() && !right_domains.is_empty() && left_domains != right_domains;
        let authority_needs_bridge = matches!(
            (left.authority_kind, right.authority_kind),
            (
                TableSurfaceAuthorityKind::ExternalRegisteredTable,
                TableSurfaceAuthorityKind::ExternalRegisteredTable
            )
        ) && left.table_id != right.table_id;
        if (domains_need_bridge || authority_needs_bridge) && bridge.is_none() {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                format!(
                    "table join from {} to {} requires an exact CoveQL table bridge for distinct code domains or external authorities",
                    left.table_name, right.table_name
                ),
                &self.options,
            ));
        }
        Ok(bridge)
    }

    pub(super) fn resolve_table_binding_arg(
        &mut self,
        expr: &Spanned<AstExpr>,
        method: &str,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        let AstExpr::RootBinding(binding) = &expr.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("{method} target must be a table(...) root binding"),
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("{method} target must resolve to a table surface"),
                &self.options,
            ));
        };
        Ok(right)
    }

    pub(super) fn resolve_join_kind(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableJoinKind, BuildResolvedQueryError> {
        match self.profile_enum_literal("kind", expr)?.as_str() {
            "inner" => Ok(TableJoinKind::Inner),
            "left" => Ok(TableJoinKind::Left),
            "right" => Ok(TableJoinKind::Right),
            "full" => Ok(TableJoinKind::Full),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown join kind enum literal {value}"),
                &self.options,
            )),
        }
    }

    pub(super) fn resolve_set_operation(
        &mut self,
        name: &str,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedSetOperation, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "set operations are currently declared for CoveQL/Table roots",
                &self.options,
            ));
        }
        let mut target = None;
        let mut all = true;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("all"), AstProfileArgumentValue::Expr(expr)) => {
                    all = self.resolve_bool_profile_arg("all", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported {name} argument on set operation"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "set operation requires one table root binding",
                        &self.options,
                    ));
                }
            }
        }
        let kind = match name {
            "union" => SetOperationKind::Union,
            "intersect" => SetOperationKind::Intersect,
            "except" => SetOperationKind::Except,
            _ => unreachable!("guarded by caller"),
        };
        let right = self.resolve_table_binding_arg(
            target.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "set operation requires a right-hand table root binding",
                    &self.options,
                )
            })?,
            name,
        )?;
        Ok(ResolvedSetOperation { kind, right, all })
    }

    pub(super) fn resolve_window_spec(
        &self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedWindowSpec, BuildResolvedQueryError> {
        let mut partition_by = Vec::new();
        let mut order_by = None;
        let mut frame = WindowFrameKind::Rows;
        let mut start = "unbounded_preceding".to_string();
        let mut end = "current_row".to_string();
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("partitionBy"), AstProfileArgumentValue::Expr(expr)) => {
                    partition_by.push(self.resolve_expr(expr, root)?);
                }
                (Some("orderBy"), AstProfileArgumentValue::Expr(expr)) => {
                    order_by = Some(ResolvedOrderClause {
                        expr: self.resolve_expr(expr, root)?,
                        direction: AstOrderDirection::Asc,
                        nulls: AstNullOrdering::Default,
                        uses_default_ordering: true,
                    });
                }
                (Some("frame"), AstProfileArgumentValue::Expr(expr)) => {
                    frame = match self.profile_enum_literal("frame", expr)?.as_str() {
                        "rows" => WindowFrameKind::Rows,
                        "range" => WindowFrameKind::Range,
                        value => {
                            return Err(profile_rejection(
                                "E_UNKNOWN_ENUM_LITERAL",
                                format!("unsupported window frame {value}"),
                                &self.options,
                            ))
                        }
                    };
                }
                (Some("start"), AstProfileArgumentValue::Expr(expr)) => {
                    start = self.profile_enum_literal("start", expr)?;
                }
                (Some("end"), AstProfileArgumentValue::Expr(expr)) => {
                    end = self.profile_enum_literal("end", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported window argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "window accepts named partitionBy/orderBy/frame/start/end arguments",
                        &self.options,
                    ));
                }
            }
        }
        Ok(ResolvedWindowSpec {
            partition_by,
            order_by,
            frame,
            start,
            end,
        })
    }

    pub(super) fn resolve_lookup_cardinality(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupCardinality, BuildResolvedQueryError> {
        match self.profile_enum_literal("cardinality", expr)?.as_str() {
            "one" => Ok(TableLookupCardinality::One),
            "many" => Ok(TableLookupCardinality::Many),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup cardinality enum literal {value}"),
                &self.options,
            )),
        }
    }

    pub(super) fn resolve_lookup_unmatched_policy(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupUnmatchedPolicy, BuildResolvedQueryError> {
        match self.profile_enum_literal("unmatched", expr)?.as_str() {
            "nulls" => Ok(TableLookupUnmatchedPolicy::Nulls),
            "reject" => Ok(TableLookupUnmatchedPolicy::Reject),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup unmatched enum literal {value}"),
                &self.options,
            )),
        }
    }

    pub(super) fn resolve_lookup_duplicate_policy(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupDuplicatePolicy, BuildResolvedQueryError> {
        match self.profile_enum_literal("duplicate", expr)?.as_str() {
            "reject" => Ok(TableLookupDuplicatePolicy::Reject),
            "many" | "emit_all" => Ok(TableLookupDuplicatePolicy::EmitAll),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup duplicate enum literal {value}"),
                &self.options,
            )),
        }
    }

    pub(super) fn resolve_bool_profile_arg(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<bool, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Literal(AstLiteral::Boolean(value)) => Ok(*value),
            _ => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("lookup argument {name} requires a boolean literal"),
                &self.options,
            )),
        }
    }

    pub(super) fn profile_enum_literal(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<String, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Path(path) if path.parts.len() == 1 => Ok(path.parts[0].name.clone()),
            AstExpr::Literal(AstLiteral::String(value)) => Ok(value.clone()),
            _ => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("lookup argument {name} requires an enum literal"),
                &self.options,
            )),
        }
    }

    pub(super) fn validate_table_lookup_predicate(
        &self,
        predicate: &ResolvedPredicate,
    ) -> Result<(), BuildResolvedQueryError> {
        match predicate {
            ResolvedPredicate::Compare { left, op, right }
                if *op == AstCompareOp::Eq
                    && matches!(left, ResolvedExpr::Path(_))
                    && matches!(right, ResolvedExpr::Path(_)) =>
            {
                Ok(())
            }
            _ => Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires an equality on: predicate between table paths",
                &self.options,
            )),
        }
    }

    pub(super) fn resolve_table_root(
        &self,
        table_name: &str,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        if let Some(table) = self.cte_table_scope.get(table_name) {
            return Ok(table.clone());
        }
        if let Some(authority) = self.table_authority_for_name(table_name) {
            self.validate_registered_table_authority(authority)?;
            let projection = synthetic_projection_for_table_contract(&authority.contract);
            return Ok(ResolvedTableRoot {
                table_name: authority.contract.table_name.clone(),
                binding_name: None,
                table_id: authority.contract.table_id.clone(),
                authority_kind: authority.contract.authority_kind,
                row_grain: authority.contract.row_grain.clone(),
                row_identity: authority.contract.row_identity.clone(),
                canonical_order: authority.contract.canonical_order.clone(),
                temporal_authority: authority.contract.temporal_authority,
                evidence_capabilities: authority.contract.evidence_capabilities.clone(),
                table_surface_contract: authority.contract.clone(),
                execution_authority: authority.execution_authority.clone(),
                projection,
            });
        }
        let catalog = self.projection_catalog().map_err(|_| {
            self.unknown(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surfaces require COVE-MAP projection metadata",
                Some(table_name),
            )
        })?;
        let projection_id = catalog
            .projections
            .iter()
            .find(|entry| {
                entry.output_table.as_deref() == Some(table_name)
                    || entry.projection_id == table_name
            })
            .map(|entry| entry.projection_id.clone())
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_TABLE_SURFACE",
                    "unknown CoveQL/Table surface",
                    Some(table_name),
                )
            })?;
        let projection = self.resolve_projection_root(&projection_id)?;
        let mut row_identity = projection.ordering.clone();
        if row_identity.is_empty() {
            row_identity.push("projection_row_identity".into());
        }
        let row_grain = projection
            .row_grain
            .clone()
            .unwrap_or_else(|| "visible_table_row".into());
        let mut canonical_order = projection.ordering.clone();
        if canonical_order.is_empty() {
            canonical_order = row_identity.clone();
        }
        let temporal_authority = if matches!(
            projection.temporal_mode.as_deref(),
            Some("as_of" | "recomputable" | "recomputable_projection_at_temporal_cut")
        ) {
            TableTemporalAuthority::RecomputableProjectionAtTemporalCut
        } else {
            TableTemporalAuthority::MaterializedSnapshotOnly
        };
        let auto_contract = self.projection_table_surface_contract(
            table_name,
            &projection,
            &row_grain,
            &row_identity,
            &canonical_order,
            temporal_authority,
        );
        let table_surface_contract = self
            .table_surface_contract_override(table_name, &projection, &auto_contract)?
            .unwrap_or(auto_contract);
        Ok(ResolvedTableRoot {
            table_name: table_name.into(),
            binding_name: None,
            table_id: format!("projection:{}", projection.projection_id),
            authority_kind: table_surface_contract.authority_kind,
            row_grain: table_surface_contract.row_grain.clone(),
            row_identity: table_surface_contract.row_identity.clone(),
            canonical_order: table_surface_contract.canonical_order.clone(),
            temporal_authority: table_surface_contract.temporal_authority,
            evidence_capabilities: table_surface_contract.evidence_capabilities.clone(),
            table_surface_contract,
            execution_authority: TableExecutionAuthority::DeterministicProjection {
                projection_id: projection.projection_id.clone(),
            },
            projection,
        })
    }

    pub(super) fn table_authority_for_name(
        &self,
        table_name: &str,
    ) -> Option<&TableSurfaceAuthority> {
        self.options
            .table_authorities
            .get(table_name)
            .or_else(|| {
                self.options
                    .table_authorities
                    .values()
                    .find(|authority| authority.contract.table_name == table_name)
            })
            .or_else(|| {
                self.options
                    .table_authorities
                    .values()
                    .find(|authority| authority.contract.table_id == table_name)
            })
    }

    pub(super) fn validate_registered_table_authority(
        &self,
        authority: &TableSurfaceAuthority,
    ) -> Result<(), BuildResolvedQueryError> {
        self.validate_table_surface_contract_fields(&authority.contract)?;
        match (
            authority.contract.authority_kind,
            &authority.execution_authority,
        ) {
            (
                TableSurfaceAuthorityKind::DeterministicProjection,
                TableExecutionAuthority::DeterministicProjection { .. },
            )
            | (
                TableSurfaceAuthorityKind::MaterializedTable,
                TableExecutionAuthority::MaterializedRows { .. },
            )
            | (TableSurfaceAuthorityKind::RawTable, TableExecutionAuthority::RawRows { .. })
            | (
                TableSurfaceAuthorityKind::ExternalRegisteredTable,
                TableExecutionAuthority::ExternalRows { .. },
            ) => Ok(()),
            _ => Err(self.unknown(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "table authority kind and execution authority do not match",
                Some(&authority.contract.table_name),
            )),
        }
    }

    pub(super) fn projection_table_surface_contract(
        &self,
        table_name: &str,
        projection: &ResolvedProjectionRoot,
        row_grain: &str,
        row_identity: &[String],
        canonical_order: &[String],
        temporal_authority: TableTemporalAuthority,
    ) -> TableSurfaceContract {
        let table_id = format!("projection:{}", projection.projection_id);
        let logical_column_map = projection
            .columns
            .iter()
            .map(|column| TableSurfaceColumnContract {
                name: column.name.clone(),
                logical_type: column.logical_type.clone(),
                nullable: column.missing_policy != "reject",
                source_path: Some(column.value.clone()),
                code_domain: None,
                collation: None,
            })
            .collect::<Vec<_>>();
        TableSurfaceContract {
            table_id,
            table_name: table_name.into(),
            contract_version: crate::COVEQL_PROFILE_CONTRACT_VERSION.into(),
            authority_kind: TableSurfaceAuthorityKind::DeterministicProjection,
            authority_fingerprint: projection.projection_id.clone(),
            schema_fingerprint: projection.projection_id.clone(),
            logical_column_map,
            row_grain: row_grain.into(),
            row_identity: row_identity.to_vec(),
            canonical_order: canonical_order.to_vec(),
            visibility_authority: "cove_o_visibility".into(),
            redaction_authority: "cove_o_redaction".into(),
            temporal_authority,
            evidence_capabilities: vec![
                AstEvidenceGrain::Row,
                AstEvidenceGrain::Column,
                AstEvidenceGrain::Projection,
                AstEvidenceGrain::Source,
            ],
            null_missing_nan_policy: "projection_contract".into(),
            collation_policy: "projection_contract".into(),
            code_domain_contexts: Vec::new(),
            code_domain_bridges: Vec::new(),
            projection_dependency_contract_id: Some(projection.projection_id.clone()),
            datafusion_interop_contract: Some("coveql_table_projection_provider".into()),
        }
    }

    pub(super) fn table_surface_contract_override(
        &self,
        table_name: &str,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<Option<TableSurfaceContract>, BuildResolvedQueryError> {
        let supplied = self
            .options
            .table_surface_contracts
            .get(table_name)
            .or_else(|| {
                self.options
                    .table_surface_contracts
                    .get(&projection.projection_id)
            })
            .or_else(|| {
                self.options
                    .table_surface_contracts
                    .get(&auto_contract.table_id)
            });
        let Some(contract) = supplied else {
            return Ok(None);
        };
        self.validate_table_surface_contract(contract, projection, auto_contract)?;
        Ok(Some(contract.clone()))
    }

    pub(super) fn validate_table_surface_contract(
        &self,
        contract: &TableSurfaceContract,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        self.validate_table_surface_contract_fields(contract)?;
        if contract.authority_kind != TableSurfaceAuthorityKind::DeterministicProjection
            && !self
                .options
                .table_authorities
                .values()
                .any(|authority| authority.contract.table_id == contract.table_id)
        {
            return Err(self.unknown(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "non-projection table surface contracts require a registered materialized, raw, or external table authority",
                Some(&contract.table_name),
            ));
        }
        if contract.authority_kind != TableSurfaceAuthorityKind::DeterministicProjection {
            return Ok(());
        }
        self.validate_projection_backed_table_contract(contract, projection, auto_contract)
    }

    pub(super) fn validate_table_surface_contract_fields(
        &self,
        contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        let required_string_fields = [
            ("contract_version", contract.contract_version.as_str()),
            (
                "authority_fingerprint",
                contract.authority_fingerprint.as_str(),
            ),
            ("schema_fingerprint", contract.schema_fingerprint.as_str()),
            ("row_grain", contract.row_grain.as_str()),
            (
                "visibility_authority",
                contract.visibility_authority.as_str(),
            ),
            ("redaction_authority", contract.redaction_authority.as_str()),
            (
                "null_missing_nan_policy",
                contract.null_missing_nan_policy.as_str(),
            ),
            ("collation_policy", contract.collation_policy.as_str()),
        ];
        for (field, value) in required_string_fields {
            if value.trim().is_empty() {
                return Err(profile_rejection(
                    "E_UNKNOWN_TABLE_SURFACE",
                    format!("table surface contract is missing required field {field}"),
                    &self.options,
                ));
            }
        }
        if contract.row_identity.is_empty() {
            return Err(profile_rejection(
                "E_TABLE_ROW_IDENTITY_MISSING",
                "table surface contract requires a non-empty row_identity",
                &self.options,
            ));
        }
        if contract.canonical_order.is_empty() {
            return Err(profile_rejection(
                "E_TABLE_ORDER_MISSING",
                "table surface contract requires a non-empty canonical_order",
                &self.options,
            ));
        }
        if contract.logical_column_map.is_empty() {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract requires a non-empty logical_column_map",
                &self.options,
            ));
        }
        Ok(())
    }

    pub(super) fn validate_projection_backed_table_contract(
        &self,
        contract: &TableSurfaceContract,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        if contract.table_id != auto_contract.table_id {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract table_id must match the deterministic projection authority",
                &self.options,
            ));
        }
        if contract.row_grain != auto_contract.row_grain {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract row_grain must match the deterministic projection authority",
                &self.options,
            ));
        }
        if contract.temporal_authority != auto_contract.temporal_authority {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract temporal authority does not match the projection authority",
                &self.options,
            ));
        }
        if contract.projection_dependency_contract_id.as_deref()
            != Some(projection.projection_id.as_str())
        {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract must reference the projection dependency contract it executes",
                &self.options,
            ));
        }
        for column in &projection.columns {
            if !contract
                .logical_column_map
                .iter()
                .any(|contract_column| contract_column.name == column.name)
            {
                return Err(profile_rejection(
                    "E_UNKNOWN_TABLE_SURFACE",
                    format!(
                        "table surface contract is missing logical column {}",
                        column.name
                    ),
                    &self.options,
                ));
            }
        }
        Ok(())
    }
}
