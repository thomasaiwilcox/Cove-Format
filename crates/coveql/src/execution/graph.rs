use super::*;

pub(super) fn execute_graph_traverse_root(
    bytes: &[u8],
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    root: &crate::ResolvedGraphNodeRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let read_result = read_object_surface_from_bytes_with_pushdown_options(
        bytes,
        &CoveObjectReadWithPushdownOptions {
            read: object_read_options(planned),
            pushdown: CoveObjectReadPushdownOptions::default(),
        },
    )
    .map_err(|err| {
        exec_error(
            "E_READBACK",
            format!("COVE-O materialized readback failed: {err}"),
            json!({}),
        )
    })?;
    let surface = read_result.surface;
    execute_graph_traverse_surface_root(&surface, planned, options, started, root)
}

pub(super) fn execute_graph_traverse_surface_root(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    options: &ExecutionOptions,
    started: Instant,
    root: &crate::ResolvedGraphNodeRoot,
) -> Result<(CoveQlExecutionResult, ExecutionRowCounts), BuildExecutionError> {
    check_time(&options.resource_budget, started)?;
    let states = object_states_for_temporal_context(surface, planned)?;
    let object_rows = states
        .iter()
        .map(MaterializedObjectRow::from_state)
        .collect::<Vec<_>>();
    let visible_object_rows = filter_object_context_rows(&object_rows, planned, options);
    let associations = states
        .iter()
        .filter_map(MaterializedAssociationRow::from_state)
        .collect::<Vec<_>>();
    let visible_associations = filter_association_context_rows(&associations, planned, options);
    let mut by_type_and_goid = BTreeMap::<(u32, String), MaterializedObjectRow>::new();
    let object_goids = object_rows
        .iter()
        .map(|row| row.goid.clone())
        .collect::<BTreeSet<_>>();
    let mut visible_object_goids = BTreeSet::<String>::new();
    for row in &visible_object_rows {
        visible_object_goids.insert(row.goid.clone());
        by_type_and_goid.insert((row.object_type_id, row.goid.clone()), row.clone());
    }
    let mut rows = visible_object_rows
        .iter()
        .filter(|row| row.object_type_id == root.object.object_type_id)
        .map(|row| {
            let state = GraphPath::new(GraphNodeId::new(row.goid.clone()));
            with_graph_path_state(namespace_graph_node_projection_row(row, root, true), &state)
        })
        .collect::<Vec<_>>();
    let graph_context = GraphMaterializedExecutionContext {
        root,
        visible_associations: &visible_associations,
        by_type_and_goid: &by_type_and_goid,
        object_goids: &object_goids,
        visible_object_goids: &visible_object_goids,
        planned,
        options,
        started,
    };
    for traversal in &planned.resolved.method_chain.traversals {
        rows = expand_graph_traversal_rows(&rows, traversal, &graph_context)?;
    }
    if !planned.resolved.method_chain.graph_algorithms.is_empty() {
        rows = apply_graph_algorithms(
            rows,
            &planned.resolved.method_chain.graph_algorithms,
            &graph_context,
        )?;
    }
    let rows = rows
        .into_iter()
        .map(ExecutionRow::Projection)
        .collect::<Vec<_>>();
    let context = EvalContext::for_plan_with_objects(
        &visible_associations,
        &[],
        &visible_object_rows,
        planned,
    );
    finish_materialized_rows_with_context(rows, planned, options, started, &context)
}

struct GraphMaterializedExecutionContext<'a> {
    root: &'a crate::ResolvedGraphNodeRoot,
    visible_associations: &'a [MaterializedAssociationRow],
    by_type_and_goid: &'a BTreeMap<(u32, String), MaterializedObjectRow>,
    object_goids: &'a BTreeSet<String>,
    visible_object_goids: &'a BTreeSet<String>,
    planned: &'a PlannedQuery,
    options: &'a ExecutionOptions,
    started: Instant,
}

fn apply_graph_algorithms(
    rows: Vec<MaterializedProjectionRow>,
    algorithms: &[crate::ResolvedGraphAlgorithm],
    context: &GraphMaterializedExecutionContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let mut rows = rows;
    for algorithm in algorithms {
        check_time(&context.options.resource_budget, context.started)?;
        rows = apply_graph_algorithm(rows, algorithm, context)?;
    }
    Ok(rows)
}

fn apply_graph_algorithm(
    rows: Vec<MaterializedProjectionRow>,
    algorithm: &crate::ResolvedGraphAlgorithm,
    context: &GraphMaterializedExecutionContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    match algorithm.kind {
        GraphAlgorithmKind::AllPaths | GraphAlgorithmKind::KShortestPaths => {
            let Some(edge) = &algorithm.edge else {
                return Ok(rows
                    .into_iter()
                    .map(|mut row| {
                        row.values.insert("path_count".into(), json!(0u64));
                        row
                    })
                    .collect());
            };
            let traversal = crate::ResolvedGraphTraversal {
                direction: algorithm.direction,
                edge: edge.clone(),
                target: algorithm.target.clone(),
                min_depth: 1,
                max_depth: algorithm
                    .max_depth
                    .unwrap_or(algorithm.contract.max_depth)
                    .max(1),
                mode: GraphTraversalMode::SimplePath,
                distinct: GraphTraversalDistinctPolicy::Path,
                contract: Some(GraphTraversalContract {
                    contract_version: algorithm.contract.contract_version.clone(),
                    allow_variable_length: true,
                    supported_modes: vec![
                        GraphTraversalMode::Walk,
                        GraphTraversalMode::Trail,
                        GraphTraversalMode::SimplePath,
                    ],
                    supported_distinct_policies: vec![
                        GraphTraversalDistinctPolicy::None,
                        GraphTraversalDistinctPolicy::Path,
                        GraphTraversalDistinctPolicy::EndNode,
                    ],
                    max_depth: algorithm.contract.max_depth,
                    max_fanout_per_node: context
                        .options
                        .resource_budget
                        .maximum_graph_traversal_fanout,
                    max_paths: algorithm.max_paths.unwrap_or(algorithm.contract.max_paths),
                    max_frontier: context
                        .options
                        .resource_budget
                        .maximum_graph_traversal_frontier,
                    path_identity: vec![
                        "start_goid".into(),
                        "edge_goids".into(),
                        "node_goids".into(),
                    ],
                    hidden_endpoint_policy: algorithm.contract.disclosure_policy.clone(),
                    ordering_policy: algorithm.contract.ordering_policy.clone(),
                    execution_authority: "materialized_graph_algorithm_oracle".into(),
                }),
            };
            return expand_graph_traversal_rows(&rows, &traversal, context);
        }
        _ => {}
    }

    let edges = graph_visible_edges(
        algorithm.edge.as_ref(),
        algorithm.direction,
        context.visible_associations,
        context.object_goids,
        context.visible_object_goids,
        context.planned,
    );
    let adjacency = graph_execution::GraphAdjacency::from_edges(&edges);
    let start_nodes = rows
        .iter()
        .filter_map(|row| graph_row_current_node_id(row, context.root))
        .collect::<BTreeSet<_>>();
    let target_nodes = graph_algorithm_target_nodes(algorithm, context);
    let weighted_edges = if algorithm.kind == GraphAlgorithmKind::SpanningTree
        && algorithm.variant == "min_weight"
    {
        graph_visible_weighted_edges(algorithm, context)?
    } else {
        Vec::new()
    };
    let outputs = graph_execution::run_graph_algorithm(
        &adjacency,
        &start_nodes,
        &graph_algorithm_spec(algorithm),
        target_nodes.as_ref(),
        &weighted_edges,
        graph_execution_budget(context),
    )
    .map_err(map_graph_execution_error)?;

    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        let Some(start) = graph_row_current_node_id(&row, context.root) else {
            out.push(row);
            continue;
        };
        if let Some(output) = outputs.get(&start) {
            append_graph_algorithm_output(&mut row, output);
        }
        out.push(row);
    }
    Ok(out)
}

fn append_graph_algorithm_output(
    row: &mut MaterializedProjectionRow,
    output: &GraphAlgorithmOutput,
) {
    match output {
        GraphAlgorithmOutput::Reachable {
            reachable,
            reachable_count,
        } => {
            row.values.insert("reachable".into(), json!(*reachable));
            row.values
                .insert("reachable_count".into(), json!(*reachable_count as u64));
        }
        GraphAlgorithmOutput::ShortestPath { shortest_distance } => {
            row.values.insert(
                "shortest_distance".into(),
                shortest_distance.map_or(Value::Null, |distance| json!(distance)),
            );
        }
        GraphAlgorithmOutput::ConnectedComponent { component_id } => {
            row.values
                .insert("component_id".into(), json!(*component_id as u64));
        }
        GraphAlgorithmOutput::Degree {
            out_degree,
            in_degree,
            degree,
        } => {
            row.values
                .insert("out_degree".into(), json!(*out_degree as u64));
            row.values
                .insert("in_degree".into(), json!(*in_degree as u64));
            row.values.insert("degree".into(), json!(*degree as u64));
        }
        GraphAlgorithmOutput::PageRank { pagerank } => {
            row.values.insert("pagerank".into(), json!(*pagerank));
        }
        GraphAlgorithmOutput::Hits { authority, hub } => {
            row.values.insert("authority".into(), json!(*authority));
            row.values.insert("hub".into(), json!(*hub));
        }
        GraphAlgorithmOutput::Centrality { centrality } => {
            row.values.insert("centrality".into(), json!(*centrality));
        }
        GraphAlgorithmOutput::TriangleCount { triangle_count } => {
            row.values
                .insert("triangle_count".into(), json!(*triangle_count as u64));
        }
        GraphAlgorithmOutput::ClusteringCoefficient {
            clustering_coefficient,
        } => {
            row.values.insert(
                "clustering_coefficient".into(),
                json!(*clustering_coefficient),
            );
        }
        GraphAlgorithmOutput::Community { community_id } => {
            row.values
                .insert("community_id".into(), json!(*community_id as u64));
        }
        GraphAlgorithmOutput::SpanningTree {
            tree_parent,
            tree_depth,
        } => {
            row.values.insert(
                "tree_parent".into(),
                tree_parent
                    .as_ref()
                    .map(|parent| Value::String(parent.as_str().to_string()))
                    .unwrap_or(Value::Null),
            );
            row.values.insert("tree_depth".into(), json!(*tree_depth));
        }
    }
}

fn graph_algorithm_spec(algorithm: &crate::ResolvedGraphAlgorithm) -> GraphAlgorithmSpec {
    GraphAlgorithmSpec {
        kind: algorithm.kind,
        variant: algorithm.variant.clone(),
        max_depth: algorithm.max_depth.unwrap_or(algorithm.contract.max_depth),
        max_paths: algorithm.max_paths.unwrap_or(algorithm.contract.max_paths),
        max_iterations: graph_algorithm_iterations(algorithm),
        tolerance: graph_algorithm_tolerance(algorithm),
    }
}

fn graph_algorithm_iterations(algorithm: &crate::ResolvedGraphAlgorithm) -> usize {
    algorithm
        .max_iterations
        .unwrap_or(20)
        .min(algorithm.contract.max_iterations)
        .max(1)
}

fn graph_algorithm_tolerance(algorithm: &crate::ResolvedGraphAlgorithm) -> f64 {
    algorithm
        .tolerance
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1e-9)
}

fn graph_algorithm_target_nodes(
    algorithm: &crate::ResolvedGraphAlgorithm,
    context: &GraphMaterializedExecutionContext<'_>,
) -> Option<BTreeSet<GraphNodeId>> {
    let target = algorithm.target.as_ref()?;
    Some(
        context
            .by_type_and_goid
            .keys()
            .filter(|(object_type_id, _)| *object_type_id == target.object.object_type_id)
            .map(|(_, goid)| GraphNodeId::new(goid.clone()))
            .collect(),
    )
}

fn graph_visible_edges(
    edge: Option<&crate::ResolvedGraphEdgeRoot>,
    direction: crate::AstAssociationDirection,
    associations: &[MaterializedAssociationRow],
    object_goids: &BTreeSet<String>,
    visible_object_goids: &BTreeSet<String>,
    planned: &PlannedQuery,
) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for association in associations {
        if !graph_edge_matches(association, edge, planned) {
            continue;
        }
        let (Some(source), Some(target)) = (&association.source_goid, &association.target_goid)
        else {
            continue;
        };
        if (object_goids.contains(source) && !visible_object_goids.contains(source))
            || (object_goids.contains(target) && !visible_object_goids.contains(target))
        {
            continue;
        }
        for (source, target) in graph_oriented_edge_pairs(source, target, direction) {
            edges.push(GraphEdge::new(
                GraphEdgeId::new(association.goid.clone()),
                source,
                target,
            ));
        }
    }
    edges
}

fn graph_edge_matches(
    association: &MaterializedAssociationRow,
    edge: Option<&crate::ResolvedGraphEdgeRoot>,
    planned: &PlannedQuery,
) -> bool {
    edge.is_none_or(|edge| {
        association.object_type_id == edge.association.object_type_id
            && association_row_matches_temporal(
                association,
                &edge.association,
                association_valid_at(planned),
            )
    })
}

fn graph_visible_weighted_edges(
    algorithm: &crate::ResolvedGraphAlgorithm,
    context: &GraphMaterializedExecutionContext<'_>,
) -> Result<Vec<graph_execution::GraphWeightedEdge>, BuildExecutionError> {
    let Some(weight_expr) = &algorithm.weight else {
        return Err(exec_error(
            "E_GRAPH_ALGORITHM",
            "min_weight spanning tree requires a weight expression",
            json!({}),
        ));
    };
    let eval_context = EvalContext::for_plan(&[], &[], context.planned);
    let mut edges = Vec::new();
    for association in context.visible_associations {
        if !graph_edge_matches(association, algorithm.edge.as_ref(), context.planned) {
            continue;
        }
        let weight_value = eval_expr(
            weight_expr,
            &ExecutionRow::Association(association.clone()),
            &eval_context,
        )
        .map_err(|err| {
            exec_error(
                "E_GRAPH_ALGORITHM",
                format!("spanningTree weight evaluation failed: {}", err.message),
                json!({}),
            )
        })?;
        let weight = match weight_value {
            Value::Number(number) => number_to_f64(&number),
            _ => None,
        }
        .ok_or_else(|| {
            exec_error(
                "E_GRAPH_ALGORITHM",
                "spanningTree weight must evaluate to a numeric value",
                json!({}),
            )
        })?;
        if !weight.is_finite() || weight < 0.0 {
            return Err(exec_error(
                "E_GRAPH_ALGORITHM",
                "spanningTree weight must be finite and non-negative",
                json!({ "weight": weight }),
            ));
        }
        let (Some(source), Some(target)) = (&association.source_goid, &association.target_goid)
        else {
            continue;
        };
        for (source, target) in graph_oriented_edge_pairs(source, target, algorithm.direction) {
            if context.object_goids.contains(source.as_str())
                && !context.visible_object_goids.contains(source.as_str())
            {
                continue;
            }
            if context.object_goids.contains(target.as_str())
                && !context.visible_object_goids.contains(target.as_str())
            {
                continue;
            }
            edges.push(graph_execution::GraphWeightedEdge::new(
                source, target, weight,
            ));
        }
    }
    Ok(edges)
}

fn graph_oriented_edge_pairs(
    source: &str,
    target: &str,
    direction: crate::AstAssociationDirection,
) -> Vec<(GraphNodeId, GraphNodeId)> {
    match direction {
        crate::AstAssociationDirection::Out => {
            vec![(GraphNodeId::new(source), GraphNodeId::new(target))]
        }
        crate::AstAssociationDirection::In => {
            vec![(GraphNodeId::new(target), GraphNodeId::new(source))]
        }
        crate::AstAssociationDirection::Either => vec![
            (GraphNodeId::new(source), GraphNodeId::new(target)),
            (GraphNodeId::new(target), GraphNodeId::new(source)),
        ],
    }
}

fn expand_graph_traversal_rows(
    rows: &[MaterializedProjectionRow],
    traversal: &crate::ResolvedGraphTraversal,
    context: &GraphMaterializedExecutionContext<'_>,
) -> Result<Vec<MaterializedProjectionRow>, BuildExecutionError> {
    let edges = graph_visible_edges(
        Some(&traversal.edge),
        traversal.direction,
        context.visible_associations,
        context.object_goids,
        context.visible_object_goids,
        context.planned,
    );
    let rows = rows
        .iter()
        .filter_map(|row| {
            graph_row_path_state(row, context.root)
                .map(|path| GraphTraversalRow::new(row.clone(), path))
        })
        .collect::<Vec<_>>();
    let associations_by_goid = context
        .visible_associations
        .iter()
        .map(|association| (association.goid.as_str(), association))
        .collect::<BTreeMap<_, _>>();
    let spec = graph_traversal_spec(traversal, context);
    let expanded = graph_execution::expand_traversal_rows(
        &rows,
        &edges,
        &spec,
        graph_execution_budget(context),
        |row, edge, path| {
            let association = associations_by_goid.get(edge.edge_id().as_str())?;
            let mut candidate = merge_lookup_projection_rows(
                row,
                &namespace_graph_edge_projection_row(association, &traversal.edge),
            );
            if let Some(target) = &traversal.target {
                let target_row = context.by_type_and_goid.get(&(
                    target.object.object_type_id,
                    edge.target().as_str().to_string(),
                ))?;
                candidate = merge_lookup_projection_rows(
                    &candidate,
                    &namespace_graph_node_projection_row(target_row, target, false),
                );
            }
            Some(with_graph_path_state(candidate, path))
        },
    )
    .map_err(map_graph_execution_error)?;
    Ok(expanded
        .into_iter()
        .map(GraphTraversalRow::into_payload)
        .collect())
}

fn graph_traversal_spec(
    traversal: &crate::ResolvedGraphTraversal,
    context: &GraphMaterializedExecutionContext<'_>,
) -> GraphTraversalSpec {
    let fanout_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_fanout_per_node)
        .unwrap_or(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_fanout,
        )
        .min(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_fanout,
        );
    let path_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_paths)
        .unwrap_or(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_paths,
        )
        .min(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_paths,
        );
    let frontier_limit = traversal
        .contract
        .as_ref()
        .map(|contract| contract.max_frontier)
        .unwrap_or(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_frontier,
        )
        .min(
            context
                .options
                .resource_budget
                .maximum_graph_traversal_frontier,
        );
    GraphTraversalSpec {
        min_depth: traversal.min_depth,
        max_depth: traversal.max_depth,
        mode: traversal.mode,
        distinct: traversal.distinct,
        limits: GraphTraversalLimits {
            max_fanout_per_node: fanout_limit,
            max_paths: path_limit,
            max_frontier: frontier_limit,
        },
    }
}

fn graph_row_path_state(
    row: &MaterializedProjectionRow,
    root: &crate::ResolvedGraphNodeRoot,
) -> Option<GraphPath> {
    let current_goid = graph_row_current_node_id(row, root)?;
    let start_goid = row
        .values
        .get(GRAPH_PATH_START_GOID_FIELD)
        .and_then(Value::as_str)
        .map(GraphNodeId::new)
        .unwrap_or_else(|| current_goid.clone());
    let node_goids = graph_string_array_field(row, GRAPH_PATH_NODE_GOIDS_FIELD)
        .map(|values| values.into_iter().map(GraphNodeId::new).collect())
        .unwrap_or_else(|| vec![current_goid.clone()]);
    let edge_goids = graph_string_array_field(row, GRAPH_PATH_EDGE_GOIDS_FIELD)
        .unwrap_or_default()
        .into_iter()
        .map(GraphEdgeId::new)
        .collect::<Vec<_>>();
    let depth = row
        .values
        .get(GRAPH_PATH_DEPTH_FIELD)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(edge_goids.len() as u32);
    Some(GraphPath::from_parts(
        start_goid,
        current_goid,
        node_goids,
        edge_goids,
        depth,
    ))
}

fn graph_string_array_field(row: &MaterializedProjectionRow, field: &str) -> Option<Vec<String>> {
    row.values
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
}

fn graph_row_current_node_id(
    row: &MaterializedProjectionRow,
    root: &crate::ResolvedGraphNodeRoot,
) -> Option<GraphNodeId> {
    graph_row_current_goid(row, root).map(GraphNodeId::new)
}

fn graph_row_current_goid(
    row: &MaterializedProjectionRow,
    root: &crate::ResolvedGraphNodeRoot,
) -> Option<String> {
    if let Some(goid) = row
        .values
        .get(GRAPH_CURRENT_GOID_FIELD)
        .and_then(Value::as_str)
    {
        return Some(goid.to_string());
    }
    table_binding_labels_for_graph_node(root)
        .into_iter()
        .find_map(|label| {
            row.values
                .get(&format!("{label}.goid"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            row.values
                .get("goid")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn with_graph_current_goid(
    mut row: MaterializedProjectionRow,
    goid: &str,
) -> MaterializedProjectionRow {
    debug_assert!(GRAPH_CURRENT_GOID_FIELD.starts_with(INTERNAL_PROJECTION_FIELD_PREFIX));
    row.values.insert(
        GRAPH_CURRENT_GOID_FIELD.into(),
        Value::String(goid.to_string()),
    );
    row
}

fn with_graph_path_state(
    mut row: MaterializedProjectionRow,
    state: &GraphPath,
) -> MaterializedProjectionRow {
    row = with_graph_current_goid(row, state.current_goid().as_str());
    row.values.insert(
        GRAPH_PATH_START_GOID_FIELD.into(),
        Value::String(state.start_goid().as_str().to_string()),
    );
    row.values.insert(
        GRAPH_PATH_NODE_GOIDS_FIELD.into(),
        Value::Array(
            state
                .node_goids()
                .iter()
                .map(|goid| Value::String(goid.as_str().to_string()))
                .collect(),
        ),
    );
    row.values.insert(
        GRAPH_PATH_EDGE_GOIDS_FIELD.into(),
        Value::Array(
            state
                .edge_goids()
                .iter()
                .map(|goid| Value::String(goid.as_str().to_string()))
                .collect(),
        ),
    );
    row.values
        .insert(GRAPH_PATH_DEPTH_FIELD.into(), json!(state.depth()));
    row
}

fn graph_execution_budget(context: &GraphMaterializedExecutionContext<'_>) -> GraphExecutionBudget {
    GraphExecutionBudget::new(
        context.started,
        context.options.resource_budget.maximum_execution_time_ms,
    )
}

fn map_graph_execution_error(error: GraphExecutionError) -> BuildExecutionError {
    match error {
        GraphExecutionError::ResourceLimitExceeded { limit, actual } => {
            resource_error(limit, actual)
        }
    }
}

fn namespace_graph_node_projection_row(
    row: &MaterializedObjectRow,
    node: &crate::ResolvedGraphNodeRoot,
    include_unqualified: bool,
) -> MaterializedProjectionRow {
    let base = object_projection_values(row);
    let mut values = if include_unqualified {
        base.clone()
    } else {
        BTreeMap::new()
    };
    for label in table_binding_labels_for_graph_node(node) {
        for (field, value) in &base {
            values.insert(format!("{label}.{field}"), value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("node:{}", node.label),
        values,
    }
}

fn namespace_graph_edge_projection_row(
    row: &MaterializedAssociationRow,
    edge: &crate::ResolvedGraphEdgeRoot,
) -> MaterializedProjectionRow {
    let base = association_projection_values(row);
    let mut values = BTreeMap::new();
    for label in table_binding_labels_for_graph_edge(edge) {
        for (field, value) in &base {
            values.insert(format!("{label}.{field}"), value.clone());
        }
    }
    MaterializedProjectionRow {
        projection_id: format!("edge:{}", edge.label),
        values,
    }
}

fn table_binding_labels_for_graph_node(root: &crate::ResolvedGraphNodeRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &root.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &root.label) {
        labels.push(root.label.clone());
    }
    labels
}

fn table_binding_labels_for_graph_edge(root: &crate::ResolvedGraphEdgeRoot) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(alias) = &root.binding_name {
        labels.push(alias.clone());
    }
    if !labels.iter().any(|label| label == &root.label) {
        labels.push(root.label.clone());
    }
    labels
}

fn object_projection_values(row: &MaterializedObjectRow) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    values.insert("object_type_id".into(), json!(row.object_type_id));
    values.insert(
        "object_type_name".into(),
        Value::String(row.object_type_name.clone()),
    );
    values.insert("branch_key".into(), json!(row.branch_key));
    values.insert("goid".into(), Value::String(row.goid.clone()));
    values.insert("record_id".into(), Value::String(row.record_id.clone()));
    values.insert("timestamp_us".into(), json!(row.timestamp_us));
    values.insert("csn".into(), json!(row.csn));
    values.insert("record_kind".into(), Value::String(row.record_kind.clone()));
    for (property, value) in &row.properties {
        values.insert(property.clone(), value.clone());
    }
    values
}

fn association_projection_values(row: &MaterializedAssociationRow) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    values.insert("association_type_id".into(), json!(row.object_type_id));
    values.insert(
        "association_type".into(),
        row.association_type
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert("branch_key".into(), json!(row.branch_key));
    values.insert("goid".into(), Value::String(row.goid.clone()));
    values.insert("association_goid".into(), Value::String(row.goid.clone()));
    values.insert("record_id".into(), Value::String(row.record_id.clone()));
    values.insert(
        "source_goid".into(),
        row.source_goid
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert(
        "target_goid".into(),
        row.target_goid
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    values.insert("timestamp_us".into(), json!(row.timestamp_us));
    values.insert("csn".into(), json!(row.csn));
    for (property, value) in &row.properties {
        values.insert(property.clone(), value.clone());
    }
    values
}
