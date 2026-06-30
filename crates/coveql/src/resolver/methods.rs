use super::*;

impl Resolver {
    pub(super) fn resolve_methods(
        &mut self,
        parsed: &ParsedQuery,
        root: &ResolvedRoot,
        settings: &ChainSettings,
    ) -> Result<ResolvedMethodChain, BuildResolvedQueryError> {
        let mut chain = ResolvedMethodChain::default();
        if let AstRoot::Path(path) = &parsed.root.node {
            for relationship in &path.relationships {
                let traversal = self.resolve_graph_traversal_expr(
                    relationship,
                    root,
                    1,
                    1,
                    GraphTraversalMode::Walk,
                    GraphTraversalDistinctPolicy::None,
                    None,
                )?;
                if let Some(target) = &traversal.target {
                    self.graph_node_scope.push(target.clone());
                }
                self.graph_edge_scope.push(traversal.edge.clone());
                chain.traversals.push(traversal);
            }
        }
        for method in &parsed.methods {
            match &method.node {
                AstMethod::Where(predicate) => {
                    let resolved = self.resolve_predicate(predicate, root)?;
                    chain.where_predicate = Some(match chain.where_predicate.take() {
                        Some(existing) => ResolvedPredicate::And(vec![existing, resolved]),
                        None => resolved,
                    });
                }
                AstMethod::Select(items) => {
                    chain.select = Some(
                        items
                            .iter()
                            .map(|item| {
                                Ok(ResolvedSelectItem {
                                    alias: item.alias.as_ref().map(|alias| alias.name.clone()),
                                    expr: self.resolve_expr(&item.expr, root)?,
                                })
                            })
                            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
                    );
                }
                AstMethod::OrderBy(order) => {
                    chain.order_by = Some(ResolvedOrderClause {
                        expr: self.resolve_expr(&order.expr, root)?,
                        direction: order.direction,
                        nulls: order.nulls,
                        uses_default_ordering: order.nulls == AstNullOrdering::Default,
                    });
                }
                AstMethod::GroupBy(exprs) => {
                    chain.group_by = Some(
                        exprs
                            .iter()
                            .map(|expr| self.resolve_expr(expr, root))
                            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
                    );
                }
                AstMethod::Take(value) => chain.take = Some(*value),
                AstMethod::Skip(value) => chain.skip = Some(*value),
                AstMethod::Explain(mode) => chain.explain = Some(*mode),
                AstMethod::History(mode) => chain.history = Some(*mode),
                AstMethod::Changes { from, to, mode } => {
                    chain.changes = Some(ResolvedChanges {
                        from: self.resolve_change_bound(from)?,
                        to: self.resolve_change_bound(to)?,
                        mode: *mode,
                    });
                }
                AstMethod::ProfileCall { name, args } if name.name == "with" => {
                    let cte = self.resolve_common_table_expression(args, false)?;
                    self.cte_table_scope
                        .insert(cte.name.clone(), cte.table.clone());
                    chain.ctes.push(cte);
                }
                AstMethod::ProfileCall { name, args } if name.name == "withRecursive" => {
                    let cte = self.resolve_common_table_expression(args, true)?;
                    self.cte_table_scope
                        .insert(cte.name.clone(), cte.table.clone());
                    chain.ctes.push(cte);
                }
                AstMethod::ProfileCall { name, args } if name.name == "lookup" => {
                    let lookup = self.resolve_table_lookup(args, root)?;
                    self.lookup_scope.push(lookup.right.clone());
                    chain.lookups.push(lookup);
                }
                AstMethod::ProfileCall { name, args } if name.name == "join" => {
                    let join = self.resolve_table_join(args, root, None)?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args } if name.name == "semiJoin" => {
                    let join = self.resolve_table_join(args, root, Some(TableJoinKind::Semi))?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args } if name.name == "antiJoin" => {
                    let join = self.resolve_table_join(args, root, Some(TableJoinKind::Anti))?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args }
                    if matches!(name.name.as_str(), "union" | "intersect" | "except") =>
                {
                    chain
                        .set_operations
                        .push(self.resolve_set_operation(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, args } if name.name == "window" => {
                    chain.windows.push(self.resolve_window_spec(args, root)?);
                }
                AstMethod::ProfileCall { name, args } if name.name == "traverse" => {
                    let traversal = self.resolve_graph_traversal(args, root)?;
                    if let Some(target) = &traversal.target {
                        self.graph_node_scope.push(target.clone());
                    }
                    self.graph_edge_scope.push(traversal.edge.clone());
                    chain.traversals.push(traversal);
                }
                AstMethod::ProfileCall { name, args }
                    if graph_algorithm_kind(&name.name).is_some() =>
                {
                    chain
                        .graph_algorithms
                        .push(self.resolve_graph_algorithm(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, args }
                    if ai_operation_for_method(&name.name).is_some() =>
                {
                    chain
                        .ai_operations
                        .push(self.resolve_ai_operation(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, .. } => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!(
                            "CoveQL profile method {} is not declared by the active profile contract for this root grain",
                            name.name
                        ),
                        &self.options,
                    ));
                }
                AstMethod::AsOf(_) | AstMethod::Branch(_) | AstMethod::IncludeTombstones(_) => {}
            }
        }

        if chain.select.is_none() {
            self.diagnostics.push(warning(
                "W_DEFAULT_SELECT",
                "query has no explicit select; Phase 1 records default output shape only",
                "resolve",
                &self.options.security,
            ));
        }
        if !chain.ai_operations.is_empty() {
            self.diagnostics.push(warning(
                "W_AI_SIDECAR_REQUIRED",
                "CoveQL-AI operation selected; payload access remains sidecar-gated until source binding, privacy, redaction, and integrity validate",
                "resolve",
                &self.options.security,
            ));
        }
        if matches!(settings.output_mode, CoveQlOutputMode::ExplainJson) {
            chain.explain.get_or_insert(ExplainMode::Public);
        }
        Ok(chain)
    }
}
