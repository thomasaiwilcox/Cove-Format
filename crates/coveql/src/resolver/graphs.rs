use super::*;

pub(super) fn graph_algorithm_kind(name: &str) -> Option<GraphAlgorithmKind> {
    match name {
        "reachable" => Some(GraphAlgorithmKind::Reachable),
        "shortestPath" => Some(GraphAlgorithmKind::ShortestPath),
        "allPaths" => Some(GraphAlgorithmKind::AllPaths),
        "kShortestPaths" => Some(GraphAlgorithmKind::KShortestPaths),
        "connectedComponents" => Some(GraphAlgorithmKind::ConnectedComponents),
        "degree" => Some(GraphAlgorithmKind::Degree),
        "pageRank" => Some(GraphAlgorithmKind::PageRank),
        "hits" => Some(GraphAlgorithmKind::Hits),
        "centrality" => Some(GraphAlgorithmKind::Centrality),
        "triangleCount" => Some(GraphAlgorithmKind::TriangleCount),
        "clusteringCoefficient" => Some(GraphAlgorithmKind::ClusteringCoefficient),
        "community" => Some(GraphAlgorithmKind::Community),
        "spanningTree" => Some(GraphAlgorithmKind::SpanningTree),
        _ => None,
    }
}

pub(super) fn graph_algorithm_output_logical_type(field: &str) -> Option<&'static str> {
    match field {
        "reachable" => Some("bool"),
        "reachable_count" | "shortest_distance" | "component_id" | "out_degree" | "in_degree"
        | "degree" | "authority" | "hub" | "centrality" | "triangle_count" | "community_id"
        | "tree_depth" | "path_count" => Some("uint64"),
        "pagerank" | "clustering_coefficient" => Some("float64"),
        "tree_parent" => Some("utf8"),
        _ => None,
    }
}

impl Resolver {
    pub(super) fn resolve_graph_traversal(
        &self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedGraphTraversal, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "traverse is declared for CoveQL/Graph node or path grains",
                &self.options,
            ));
        }
        let mut relationship = None;
        let mut min_depth = 1u32;
        let mut max_depth = 1u32;
        let mut mode = GraphTraversalMode::Walk;
        let mut distinct = GraphTraversalDistinctPolicy::None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if relationship.is_none() => {
                    let AstExpr::Relationship(candidate) = &expr.node else {
                        return Err(profile_rejection(
                            "E_UNSUPPORTED_PROFILE_METHOD",
                            "traverse accepts one positional relationship expression",
                            &self.options,
                        ));
                    };
                    relationship = Some(candidate);
                }
                (Some("min"), AstProfileArgumentValue::Expr(expr)) => {
                    min_depth = self.resolve_traversal_depth_arg("min", expr)?;
                }
                (Some("max"), AstProfileArgumentValue::Expr(expr)) => {
                    max_depth = self.resolve_traversal_depth_arg("max", expr)?;
                }
                (Some("mode"), AstProfileArgumentValue::Expr(expr)) => {
                    let mode_literal = self.profile_enum_literal("mode", expr)?;
                    mode = mode_literal.parse().map_err(|_| {
                        profile_rejection(
                            "E_UNKNOWN_ENUM_LITERAL",
                            format!("unsupported traverse mode {mode_literal}"),
                            &self.options,
                        )
                    })?;
                }
                (Some("distinct"), AstProfileArgumentValue::Expr(expr)) => {
                    let distinct_literal = self.profile_enum_literal("distinct", expr)?;
                    distinct = distinct_literal.parse().map_err(|_| {
                        profile_rejection(
                            "E_UNKNOWN_ENUM_LITERAL",
                            format!("unsupported traverse distinct policy {distinct_literal}"),
                            &self.options,
                        )
                    })?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported traverse argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "traverse requires one positional relationship expression",
                        &self.options,
                    ));
                }
            }
        }
        let relationship = relationship.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "traverse requires one positional relationship expression",
                &self.options,
            )
        })?;
        if min_depth > max_depth {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "traverse min depth must be less than or equal to max depth",
                &self.options,
            ));
        }
        let requires_contract = min_depth != 1
            || max_depth != 1
            || mode != GraphTraversalMode::Walk
            || distinct != GraphTraversalDistinctPolicy::None;
        let contract = if requires_contract {
            let contract = self
                .options
                .graph_traversal_contract
                .as_ref()
                .ok_or_else(|| {
                    profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "variable-length traversal requires an explicit CoveQL/Graph traversal contract; CoveQL 0.1 executes min: 1, max: 1, mode: walk",
                        &self.options,
                    )
                })?;
            self.validate_graph_traversal_contract(contract, min_depth, max_depth, mode, distinct)?;
            Some(contract.clone())
        } else if let Some(contract) = &self.options.graph_traversal_contract {
            self.validate_graph_traversal_contract(contract, min_depth, max_depth, mode, distinct)?;
            Some(contract.clone())
        } else {
            None
        };
        self.resolve_graph_traversal_expr(
            relationship,
            root,
            min_depth,
            max_depth,
            mode,
            distinct,
            contract,
        )
    }

    pub(super) fn resolve_graph_algorithm(
        &self,
        name: &str,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedGraphAlgorithm, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "graph algorithms are declared for CoveQL/Graph node roots",
                &self.options,
            ));
        }
        let Some(kind) = graph_algorithm_kind(name) else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("unsupported CoveQL/Graph algorithm {name}"),
                &self.options,
            ));
        };
        let mut relationship = None;
        let mut direction = AstAssociationDirection::Out;
        let mut weight = None;
        let mut max_depth = None;
        let mut max_paths = None;
        let mut max_iterations = None;
        let mut tolerance = None;
        let mut approx = false;
        let mut variant = None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if relationship.is_none() => {
                    let AstExpr::Relationship(candidate) = &expr.node else {
                        return Err(profile_rejection(
                            "E_UNSUPPORTED_PROFILE_METHOD",
                            "graph algorithm positional argument must be a relationship expression",
                            &self.options,
                        ));
                    };
                    relationship = Some(candidate);
                }
                (Some("direction"), AstProfileArgumentValue::Expr(expr)) => {
                    direction = match self.profile_enum_literal("direction", expr)?.as_str() {
                        "out" => AstAssociationDirection::Out,
                        "in" => AstAssociationDirection::In,
                        "either" => AstAssociationDirection::Either,
                        value => {
                            return Err(profile_rejection(
                                "E_UNKNOWN_ENUM_LITERAL",
                                format!("unsupported graph algorithm direction {value}"),
                                &self.options,
                            ))
                        }
                    };
                }
                (Some("weight"), AstProfileArgumentValue::Expr(expr)) => {
                    weight = Some(self.resolve_expr(expr, root)?);
                }
                (Some("maxDepth" | "max_depth"), AstProfileArgumentValue::Expr(expr)) => {
                    max_depth = Some(self.resolve_traversal_depth_arg("maxDepth", expr)?);
                }
                (Some("maxPaths" | "max_paths"), AstProfileArgumentValue::Expr(expr)) => {
                    max_paths = Some(self.resolve_usize_profile_arg("maxPaths", expr)?);
                }
                (Some("maxIterations" | "max_iterations"), AstProfileArgumentValue::Expr(expr)) => {
                    max_iterations = Some(self.resolve_usize_profile_arg("maxIterations", expr)?);
                }
                (Some("tolerance"), AstProfileArgumentValue::Expr(expr)) => {
                    tolerance = Some(self.profile_enum_literal("tolerance", expr)?);
                }
                (Some("approx"), AstProfileArgumentValue::Expr(expr)) => {
                    approx = self.resolve_bool_profile_arg("approx", expr)?;
                }
                (Some("kind"), AstProfileArgumentValue::Expr(expr)) => {
                    variant = Some(self.profile_enum_literal("kind", expr)?);
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported graph algorithm argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "graph algorithm accepts one relationship expression plus named policy arguments",
                        &self.options,
                    ));
                }
            }
        }
        let (edge, target, direction) = if let Some(relationship) = relationship {
            let traversal = self.resolve_graph_traversal_expr(
                relationship,
                root,
                1,
                max_depth.unwrap_or(1).max(1),
                GraphTraversalMode::Walk,
                GraphTraversalDistinctPolicy::None,
                self.graph_traversal_contract_for(root, max_depth.unwrap_or(1)),
            )?;
            (Some(traversal.edge), traversal.target, traversal.direction)
        } else {
            (None, None, direction)
        };
        let contract = self.graph_algorithm_contract_for(kind, root);
        let variant =
            self.validate_graph_algorithm_variant(kind, variant.as_deref(), weight.is_some())?;
        self.validate_graph_algorithm_contract(
            &contract,
            kind,
            max_depth,
            max_paths,
            max_iterations,
        )?;
        Ok(ResolvedGraphAlgorithm {
            kind,
            variant,
            direction,
            edge,
            target,
            weight,
            max_depth,
            max_paths,
            max_iterations,
            tolerance,
            approx,
            contract,
        })
    }

    pub(super) fn graph_traversal_contract_for(
        &self,
        root: &ResolvedRoot,
        max_depth: u32,
    ) -> Option<GraphTraversalContract> {
        let root_label = match root {
            ResolvedRoot::Node(node) => Some(node.label.as_str()),
            _ => None,
        };
        root_label
            .and_then(|label| self.options.graph_traversal_contracts.get(label))
            .or_else(|| self.options.graph_traversal_contracts.get("default"))
            .or(self.options.graph_traversal_contract.as_ref())
            .cloned()
            .or_else(|| {
                (max_depth <= 1).then(|| GraphTraversalContract {
                    contract_version: crate::COVEQL_PROFILE_CONTRACT_VERSION.into(),
                    allow_variable_length: false,
                    supported_modes: vec![GraphTraversalMode::Walk],
                    supported_distinct_policies: vec![GraphTraversalDistinctPolicy::None],
                    max_depth: 1,
                    max_fanout_per_node: self
                        .options
                        .resource_budget
                        .maximum_graph_traversal_fanout,
                    max_paths: self.options.resource_budget.maximum_graph_traversal_paths,
                    max_frontier: self
                        .options
                        .resource_budget
                        .maximum_graph_traversal_frontier,
                    path_identity: vec![
                        "start_goid".into(),
                        "edge_goids".into(),
                        "node_goids".into(),
                    ],
                    hidden_endpoint_policy: "suppress_path".into(),
                    ordering_policy: "depth_start_edge_target".into(),
                    execution_authority: "materialized_visible_graph_oracle".into(),
                })
            })
    }

    pub(super) fn validate_graph_algorithm_variant(
        &self,
        kind: GraphAlgorithmKind,
        variant: Option<&str>,
        has_weight: bool,
    ) -> Result<String, BuildResolvedQueryError> {
        let default = match kind {
            GraphAlgorithmKind::ConnectedComponents => "weak",
            GraphAlgorithmKind::Degree => "out",
            GraphAlgorithmKind::Centrality => "closeness",
            GraphAlgorithmKind::Community => "label_propagation",
            GraphAlgorithmKind::SpanningTree => "bfs",
            _ => "default",
        };
        let variant = variant.unwrap_or(default);
        let valid = match kind {
            GraphAlgorithmKind::ConnectedComponents => matches!(variant, "weak" | "strong"),
            GraphAlgorithmKind::Degree => matches!(variant, "in" | "out" | "total"),
            GraphAlgorithmKind::Centrality => {
                matches!(variant, "degree" | "closeness" | "betweenness")
            }
            GraphAlgorithmKind::Community => variant == "label_propagation",
            GraphAlgorithmKind::SpanningTree => matches!(variant, "bfs" | "dfs" | "min_weight"),
            _ => variant == "default",
        };
        if !valid {
            return Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unsupported {} kind {variant}", kind.as_str()),
                &self.options,
            ));
        }
        if kind == GraphAlgorithmKind::SpanningTree && variant == "min_weight" && !has_weight {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "spanningTree(kind: min_weight) requires a visible non-negative numeric weight expression",
                &self.options,
            ));
        }
        Ok(variant.to_string())
    }

    pub(super) fn graph_algorithm_contract_for(
        &self,
        kind: GraphAlgorithmKind,
        root: &ResolvedRoot,
    ) -> GraphAlgorithmContract {
        let root_label = match root {
            ResolvedRoot::Node(node) => Some(node.label.as_str()),
            _ => None,
        };
        root_label
            .and_then(|label| self.options.graph_algorithm_contracts.get(label))
            .or_else(|| self.options.graph_algorithm_contracts.get(kind.as_str()))
            .or_else(|| self.options.graph_algorithm_contracts.get("default"))
            .cloned()
            .unwrap_or_else(|| GraphAlgorithmContract {
                contract_version: crate::COVEQL_PROFILE_CONTRACT_VERSION.into(),
                allowed_algorithms: vec![
                    GraphAlgorithmKind::Reachable,
                    GraphAlgorithmKind::ShortestPath,
                    GraphAlgorithmKind::AllPaths,
                    GraphAlgorithmKind::KShortestPaths,
                    GraphAlgorithmKind::ConnectedComponents,
                    GraphAlgorithmKind::Degree,
                    GraphAlgorithmKind::PageRank,
                    GraphAlgorithmKind::Hits,
                    GraphAlgorithmKind::Centrality,
                    GraphAlgorithmKind::TriangleCount,
                    GraphAlgorithmKind::ClusteringCoefficient,
                    GraphAlgorithmKind::Community,
                    GraphAlgorithmKind::SpanningTree,
                ],
                direction_policy: "directed_default_allow_either".into(),
                weight_policy: "visible_non_negative_numeric_or_unweighted".into(),
                temporal_policy: "single_temporal_cut".into(),
                visibility_authority: "cove_o_visibility".into(),
                redaction_authority: "cove_o_redaction".into(),
                max_depth: self.options.resource_budget.maximum_graph_traversal_depth,
                max_paths: self.options.resource_budget.maximum_graph_traversal_paths,
                max_iterations: self.options.resource_budget.maximum_graph_traversal_paths,
                disclosure_policy: "redaction_safe_no_partial_results".into(),
                ordering_policy: "deterministic_identity_order".into(),
            })
    }

    pub(super) fn validate_graph_algorithm_contract(
        &self,
        contract: &GraphAlgorithmContract,
        kind: GraphAlgorithmKind,
        max_depth: Option<u32>,
        max_paths: Option<usize>,
        max_iterations: Option<usize>,
    ) -> Result<(), BuildResolvedQueryError> {
        if !contract.allowed_algorithms.contains(&kind) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("graph algorithm contract does not allow {}", kind.as_str()),
                &self.options,
            ));
        }
        if max_depth.is_some_and(|value| value > contract.max_depth) {
            return Err(profile_rejection(
                "E_RESOURCE_BUDGET_EXCEEDED",
                "graph algorithm maxDepth exceeds contract max_depth",
                &self.options,
            ));
        }
        if max_paths.is_some_and(|value| value > contract.max_paths) {
            return Err(profile_rejection(
                "E_RESOURCE_BUDGET_EXCEEDED",
                "graph algorithm maxPaths exceeds contract max_paths",
                &self.options,
            ));
        }
        if max_iterations.is_some_and(|value| value > contract.max_iterations) {
            return Err(profile_rejection(
                "E_RESOURCE_BUDGET_EXCEEDED",
                "graph algorithm maxIterations exceeds contract max_iterations",
                &self.options,
            ));
        }
        if contract.contract_version.trim().is_empty()
            || contract.visibility_authority.trim().is_empty()
            || contract.redaction_authority.trim().is_empty()
            || contract.disclosure_policy.trim().is_empty()
            || contract.ordering_policy.trim().is_empty()
        {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "graph algorithm contract is missing required authority, disclosure, ordering, or version fields",
                &self.options,
            ));
        }
        Ok(())
    }

    pub(super) fn resolve_usize_profile_arg(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<usize, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Literal(AstLiteral::Integer(value)) => value.parse::<usize>().map_err(|_| {
                profile_rejection(
                    "E_UNKNOWN_ENUM_LITERAL",
                    format!("{name} requires a non-negative integer in range"),
                    &self.options,
                )
            }),
            _ => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("{name} requires a non-negative integer literal"),
                &self.options,
            )),
        }
    }

    pub(super) fn validate_graph_traversal_contract(
        &self,
        contract: &GraphTraversalContract,
        min_depth: u32,
        max_depth: u32,
        mode: GraphTraversalMode,
        distinct: GraphTraversalDistinctPolicy,
    ) -> Result<(), BuildResolvedQueryError> {
        if max_depth > 1 && !contract.allow_variable_length {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "graph traversal contract does not allow variable-length traversal",
                &self.options,
            ));
        }
        if max_depth > contract.max_depth {
            return Err(profile_rejection(
                "E_RESOURCE_BUDGET_EXCEEDED",
                format!(
                    "traverse max depth {max_depth} exceeds GraphTraversalContract max_depth {}",
                    contract.max_depth
                ),
                &self.options,
            ));
        }
        if max_depth > self.options.resource_budget.maximum_graph_traversal_depth {
            return Err(profile_rejection(
                "E_RESOURCE_BUDGET_EXCEEDED",
                format!(
                    "traverse max depth {max_depth} exceeds maximum_graph_traversal_depth {}",
                    self.options.resource_budget.maximum_graph_traversal_depth
                ),
                &self.options,
            ));
        }
        if !contract.supported_modes.contains(&mode) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!(
                    "graph traversal contract does not support mode {}",
                    mode.as_str()
                ),
                &self.options,
            ));
        }
        if !contract.supported_distinct_policies.contains(&distinct) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!(
                    "graph traversal contract does not support distinct policy {}",
                    distinct.as_str()
                ),
                &self.options,
            ));
        }
        if contract.max_fanout_per_node == 0
            || contract.max_paths == 0
            || contract.max_frontier == 0
            || contract.path_identity.is_empty()
            || contract.hidden_endpoint_policy.trim().is_empty()
            || contract.ordering_policy.trim().is_empty()
            || contract.execution_authority.trim().is_empty()
            || contract.contract_version.trim().is_empty()
        {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "graph traversal contract is missing required identity, ordering, budget, authority, or hidden-endpoint fields",
                &self.options,
            ));
        }
        if min_depth == 0 && distinct == GraphTraversalDistinctPolicy::EndNode {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "distinct: end_node is not valid with min: 0 because start rows have no traversed end node",
                &self.options,
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_graph_traversal_expr(
        &self,
        relationship: &AstRelationshipExpr,
        root: &ResolvedRoot,
        min_depth: u32,
        max_depth: u32,
        mode: GraphTraversalMode,
        distinct: GraphTraversalDistinctPolicy,
        contract: Option<GraphTraversalContract>,
    ) -> Result<ResolvedGraphTraversal, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "traverse is declared for CoveQL/Graph node or path grains",
                &self.options,
            ));
        }
        if relationship
            .target
            .as_ref()
            .is_some_and(|target| !matches!(target.root.node, AstRoot::Node(_)))
        {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "traverse targets must be node(...) bindings",
                &self.options,
            ));
        }
        let mut edge = self.resolve_graph_edge_root(&relationship.edge)?;
        if let Some(alias) = &relationship.edge_alias {
            edge.binding_name = Some(alias.name.clone());
        }
        let target = relationship
            .target
            .as_ref()
            .map(|target| {
                let mut resolved = self.resolve_root(&target.root.node)?;
                apply_root_alias(&mut resolved, target.alias.as_ref());
                let ResolvedRoot::Node(node) = resolved else {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "traverse target must resolve to a graph node",
                        &self.options,
                    ));
                };
                Ok(node)
            })
            .transpose()?;
        Ok(ResolvedGraphTraversal {
            direction: relationship.direction,
            edge,
            target,
            min_depth,
            max_depth,
            mode,
            distinct,
            contract,
        })
    }

    pub(super) fn resolve_traversal_depth_arg(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<u32, BuildResolvedQueryError> {
        let AstExpr::Literal(AstLiteral::Integer(value)) = &expr.node else {
            return Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("traverse argument {name} requires an unsigned integer literal"),
                &self.options,
            ));
        };
        value.parse::<u32>().map_err(|_| {
            profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("traverse argument {name} requires an unsigned integer literal"),
                &self.options,
            )
        })
    }

    pub(super) fn resolve_graph_node_root(
        &self,
        identifier: &AstIdentifier,
    ) -> Result<ResolvedGraphNodeRoot, BuildResolvedQueryError> {
        let object = self.resolve_object_type(&identifier.name).map_err(|_| {
            self.unknown(
                "E_UNKNOWN_GRAPH_LABEL",
                "unknown graph node label",
                Some(&identifier.name),
            )
        })?;
        Ok(ResolvedGraphNodeRoot {
            label: identifier.name.clone(),
            binding_name: None,
            object: resolved_object_root(object),
        })
    }

    pub(super) fn resolve_graph_edge_root(
        &self,
        edge: &AstGraphEdgeExpr,
    ) -> Result<ResolvedGraphEdgeRoot, BuildResolvedQueryError> {
        let association = AstAssociationExpr {
            type_name: edge.type_name.clone(),
            direction: None,
            role: edge.role,
            role_name: edge.role_name.clone(),
        };
        let resolved = self
            .resolve_association_root(&association, None, AssociationResolveUsage::RootScan)
            .map_err(|_| {
                self.unknown(
                    "E_UNKNOWN_GRAPH_LABEL",
                    "unknown graph edge label",
                    Some(&edge.type_name.name),
                )
            })?;
        Ok(ResolvedGraphEdgeRoot {
            label: edge.type_name.name.clone(),
            binding_name: None,
            association: resolved,
        })
    }

    pub(super) fn resolve_association_endpoint_role(
        &self,
        association: &AstAssociationExpr,
        source_property_id: Option<u32>,
        target_property_id: Option<u32>,
        object_relative: bool,
    ) -> Result<AssociationEndpointRole, BuildResolvedQueryError> {
        if let Some(role) = association.role {
            return match role {
                crate::AstAssociationRole::From => Ok(AssociationEndpointRole::Source),
                crate::AstAssociationRole::To => Ok(AssociationEndpointRole::Target),
                crate::AstAssociationRole::Role => match (
                    source_property_id.is_some(),
                    target_property_id.is_some(),
                ) {
                    (true, false) => Ok(AssociationEndpointRole::Source),
                    (false, true) => Ok(AssociationEndpointRole::Target),
                    (true, true) => Err(BuildResolvedQueryError {
                        diagnostics: vec![diagnostic(
                            "E_AMBIGUOUS_ASSOCIATION_ROLE",
                            "association role metadata does not prove a unique endpoint role; use from:, to:, out(...), in(...), or either(...)",
                            "resolve",
                            &self.options.security,
                        )],
                        rejections: vec![RejectionReport {
                            kind: RejectionKind::FeatureValidation,
                            reason: "ambiguous association role metadata".into(),
                        }],
                        source: None,
                    }),
                    (false, false) => Err(BuildResolvedQueryError {
                        diagnostics: vec![diagnostic(
                            "E_MISSING_ASSOCIATION_ENDPOINT_FLAGS",
                            "association role resolution requires endpoint properties declared by association flags",
                            "resolve",
                            &self.options.security,
                        )],
                        rejections: vec![RejectionReport {
                            kind: RejectionKind::FeatureValidation,
                            reason: "missing association endpoint flags".into(),
                        }],
                        source: None,
                    }),
                },
            };
        }
        match association.direction {
            Some(AstAssociationDirection::Out) => Ok(AssociationEndpointRole::Source),
            Some(AstAssociationDirection::In) => Ok(AssociationEndpointRole::Target),
            Some(AstAssociationDirection::Either) => Ok(AssociationEndpointRole::Either),
            None if !object_relative => {
                if source_property_id.is_some() && target_property_id.is_some() {
                    Ok(AssociationEndpointRole::Either)
                } else {
                    Ok(AssociationEndpointRole::Unknown)
                }
            }
            None => match (source_property_id.is_some(), target_property_id.is_some()) {
                (true, false) => Ok(AssociationEndpointRole::Source),
                (false, true) => Ok(AssociationEndpointRole::Target),
                (true, true) => Err(BuildResolvedQueryError {
                    diagnostics: vec![diagnostic(
                        "E_AMBIGUOUS_ASSOCIATION_ROLE",
                        "object-relative association traversal must specify out(...), in(...), or either(...) when both endpoint roles are declared",
                        "resolve",
                        &self.options.security,
                    )],
                    rejections: vec![RejectionReport {
                        kind: RejectionKind::FeatureValidation,
                        reason: "ambiguous association endpoint role".into(),
                    }],
                    source: None,
                }),
                (false, false) => Err(BuildResolvedQueryError {
                    diagnostics: vec![diagnostic(
                        "E_MISSING_ASSOCIATION_ENDPOINT_FLAGS",
                        "object-relative association traversal requires endpoint properties declared by association flags",
                        "resolve",
                        &self.options.security,
                    )],
                    rejections: vec![RejectionReport {
                        kind: RejectionKind::FeatureValidation,
                        reason: "missing association endpoint flags".into(),
                    }],
                    source: None,
                }),
            },
        }
    }
}
