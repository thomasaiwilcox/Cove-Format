use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    time::Instant,
};

use crate::{GraphAlgorithmKind, GraphTraversalDistinctPolicy, GraphTraversalMode};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GraphNodeId(String);

impl GraphNodeId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GraphEdgeId(String);

impl GraphEdgeId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphEdge {
    edge_id: GraphEdgeId,
    source: GraphNodeId,
    target: GraphNodeId,
}

impl GraphEdge {
    pub(crate) fn new(edge_id: GraphEdgeId, source: GraphNodeId, target: GraphNodeId) -> Self {
        Self {
            edge_id,
            source,
            target,
        }
    }

    pub(crate) fn edge_id(&self) -> &GraphEdgeId {
        &self.edge_id
    }

    pub(crate) fn source(&self) -> &GraphNodeId {
        &self.source
    }

    pub(crate) fn target(&self) -> &GraphNodeId {
        &self.target
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GraphAdjacency {
    outgoing: BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    incoming: BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
}

impl GraphAdjacency {
    pub(crate) fn from_edges(edges: &[GraphEdge]) -> Self {
        let mut graph = Self::default();
        for edge in edges {
            graph.add_edge(edge.source.clone(), edge.target.clone());
        }
        graph.normalize();
        graph
    }

    fn add_edge(&mut self, source: GraphNodeId, target: GraphNodeId) {
        self.outgoing
            .entry(source.clone())
            .or_default()
            .push(target.clone());
        self.incoming.entry(target).or_default().push(source);
    }

    fn normalize(&mut self) {
        for targets in self.outgoing.values_mut() {
            targets.sort();
            targets.dedup();
        }
        for sources in self.incoming.values_mut() {
            sources.sort();
            sources.dedup();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphPath {
    start_goid: GraphNodeId,
    current_goid: GraphNodeId,
    node_goids: Vec<GraphNodeId>,
    edge_goids: Vec<GraphEdgeId>,
    depth: u32,
}

impl GraphPath {
    pub(crate) fn new(start_goid: GraphNodeId) -> Self {
        Self {
            start_goid: start_goid.clone(),
            current_goid: start_goid.clone(),
            node_goids: vec![start_goid],
            edge_goids: Vec::new(),
            depth: 0,
        }
    }

    pub(crate) fn from_parts(
        start_goid: GraphNodeId,
        current_goid: GraphNodeId,
        node_goids: Vec<GraphNodeId>,
        edge_goids: Vec<GraphEdgeId>,
        depth: u32,
    ) -> Self {
        Self {
            start_goid,
            current_goid,
            node_goids,
            edge_goids,
            depth,
        }
    }

    pub(crate) fn start_goid(&self) -> &GraphNodeId {
        &self.start_goid
    }

    pub(crate) fn current_goid(&self) -> &GraphNodeId {
        &self.current_goid
    }

    pub(crate) fn node_goids(&self) -> &[GraphNodeId] {
        &self.node_goids
    }

    pub(crate) fn edge_goids(&self) -> &[GraphEdgeId] {
        &self.edge_goids
    }

    pub(crate) fn depth(&self) -> u32 {
        self.depth
    }

    fn extend(&self, edge_goid: GraphEdgeId, target_goid: GraphNodeId) -> Self {
        let mut node_goids = self.node_goids.clone();
        node_goids.push(target_goid.clone());
        let mut edge_goids = self.edge_goids.clone();
        edge_goids.push(edge_goid);
        Self {
            start_goid: self.start_goid.clone(),
            current_goid: target_goid,
            node_goids,
            edge_goids,
            depth: self.depth + 1,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GraphTraversalRow<T> {
    payload: T,
    path: GraphPath,
}

impl<T> GraphTraversalRow<T> {
    pub(crate) fn new(payload: T, path: GraphPath) -> Self {
        Self { payload, path }
    }

    pub(crate) fn path(&self) -> &GraphPath {
        &self.path
    }

    pub(crate) fn into_payload(self) -> T {
        self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphTraversalLimits {
    pub(crate) max_fanout_per_node: usize,
    pub(crate) max_paths: usize,
    pub(crate) max_frontier: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphTraversalSpec {
    pub(crate) min_depth: u32,
    pub(crate) max_depth: u32,
    pub(crate) mode: GraphTraversalMode,
    pub(crate) distinct: GraphTraversalDistinctPolicy,
    pub(crate) limits: GraphTraversalLimits,
}

#[derive(Debug, Clone)]
pub(crate) struct GraphAlgorithmSpec {
    pub(crate) kind: GraphAlgorithmKind,
    pub(crate) variant: String,
    pub(crate) max_depth: u32,
    pub(crate) max_paths: usize,
    pub(crate) max_iterations: usize,
    pub(crate) tolerance: f64,
}

#[derive(Debug, Clone)]
pub(crate) enum GraphAlgorithmOutput {
    Reachable {
        reachable: bool,
        reachable_count: usize,
    },
    ShortestPath {
        shortest_distance: Option<u32>,
    },
    ConnectedComponent {
        component_id: usize,
    },
    Degree {
        out_degree: usize,
        in_degree: usize,
        degree: usize,
    },
    PageRank {
        pagerank: f64,
    },
    Hits {
        authority: f64,
        hub: f64,
    },
    Centrality {
        centrality: f64,
    },
    TriangleCount {
        triangle_count: usize,
    },
    ClusteringCoefficient {
        clustering_coefficient: f64,
    },
    Community {
        community_id: usize,
    },
    SpanningTree {
        tree_parent: Option<GraphNodeId>,
        tree_depth: u32,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct GraphWeightedEdge {
    pub(crate) source: GraphNodeId,
    pub(crate) target: GraphNodeId,
    pub(crate) weight: f64,
}

impl GraphWeightedEdge {
    pub(crate) fn new(source: GraphNodeId, target: GraphNodeId, weight: f64) -> Self {
        Self {
            source,
            target,
            weight,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphExecutionBudget {
    started: Instant,
    maximum_execution_time_ms: u64,
}

impl GraphExecutionBudget {
    pub(crate) fn new(started: Instant, maximum_execution_time_ms: u64) -> Self {
        Self {
            started,
            maximum_execution_time_ms,
        }
    }

    fn check_time(self) -> Result<(), GraphExecutionError> {
        let elapsed = self.started.elapsed().as_millis();
        if elapsed as u64 > self.maximum_execution_time_ms {
            return Err(GraphExecutionError::ResourceLimitExceeded {
                limit: "maximum_execution_time_ms",
                actual: elapsed as usize,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GraphExecutionError {
    ResourceLimitExceeded { limit: &'static str, actual: usize },
}

impl fmt::Display for GraphExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimitExceeded { limit, .. } => {
                write!(f, "{limit} budget exceeded during graph execution")
            }
        }
    }
}

impl Error for GraphExecutionError {}

pub(crate) fn expand_traversal_rows<T, F>(
    rows: &[GraphTraversalRow<T>],
    edges: &[GraphEdge],
    spec: &GraphTraversalSpec,
    budget: GraphExecutionBudget,
    mut render_next: F,
) -> Result<Vec<GraphTraversalRow<T>>, GraphExecutionError>
where
    T: Clone,
    F: FnMut(&T, &GraphEdge, &GraphPath) -> Option<T>,
{
    let mut emitted = Vec::new();
    let mut seen = BTreeSet::<String>::new();
    if spec.min_depth == 0 {
        for row in rows {
            push_emitted_row(row.clone(), spec.distinct, &mut seen, &mut emitted);
        }
    }
    let mut frontier = rows.to_vec();
    for _depth in 1..=spec.max_depth {
        budget.check_time()?;
        let mut next = Vec::new();
        for row in &frontier {
            let mut fanout = 0usize;
            for edge in edges
                .iter()
                .filter(|edge| edge.source() == row.path.current_goid())
            {
                if !traversal_mode_allows(row.path(), edge.edge_id(), edge.target(), spec.mode) {
                    continue;
                }
                fanout += 1;
                if fanout > spec.limits.max_fanout_per_node {
                    return Err(GraphExecutionError::ResourceLimitExceeded {
                        limit: "maximum_graph_traversal_fanout",
                        actual: fanout,
                    });
                }
                let next_path = row
                    .path
                    .extend(edge.edge_id().clone(), edge.target().clone());
                let Some(payload) = render_next(&row.payload, edge, &next_path) else {
                    continue;
                };
                let candidate = GraphTraversalRow::new(payload, next_path);
                if candidate.path.depth() >= spec.min_depth {
                    push_emitted_row(candidate.clone(), spec.distinct, &mut seen, &mut emitted);
                    if emitted.len() > spec.limits.max_paths {
                        return Err(GraphExecutionError::ResourceLimitExceeded {
                            limit: "maximum_graph_traversal_paths",
                            actual: emitted.len(),
                        });
                    }
                }
                next.push(candidate);
                if next.len() > spec.limits.max_frontier {
                    return Err(GraphExecutionError::ResourceLimitExceeded {
                        limit: "maximum_graph_traversal_frontier",
                        actual: next.len(),
                    });
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(emitted)
}

pub(crate) fn run_graph_algorithm(
    adjacency: &GraphAdjacency,
    start_nodes: &BTreeSet<GraphNodeId>,
    spec: &GraphAlgorithmSpec,
    target_nodes: Option<&BTreeSet<GraphNodeId>>,
    weighted_edges: &[GraphWeightedEdge],
    budget: GraphExecutionBudget,
) -> Result<BTreeMap<GraphNodeId, GraphAlgorithmOutput>, GraphExecutionError> {
    let graph_nodes = graph_algorithm_nodes(adjacency, start_nodes);
    let component_ids = if matches!(
        spec.kind,
        GraphAlgorithmKind::ConnectedComponents | GraphAlgorithmKind::Community
    ) {
        Some(graph_component_ids(adjacency, &spec.variant))
    } else {
        None
    };
    let betweenness_scores =
        if spec.kind == GraphAlgorithmKind::Centrality && spec.variant == "betweenness" {
            Some(graph_betweenness_centrality_scores(adjacency, &graph_nodes))
        } else {
            None
        };
    let community_ids =
        if spec.kind == GraphAlgorithmKind::Community && spec.variant == "label_propagation" {
            Some(graph_label_propagation_communities(
                adjacency,
                &graph_nodes,
                spec.max_iterations,
            ))
        } else {
            None
        };
    let spanning_forest = if spec.kind == GraphAlgorithmKind::SpanningTree {
        Some(graph_spanning_forest(
            adjacency,
            &graph_nodes,
            &spec.variant,
            weighted_edges,
            budget,
        )?)
    } else {
        None
    };
    let pagerank_scores = if matches!(spec.kind, GraphAlgorithmKind::PageRank) {
        Some(graph_pagerank_scores(adjacency, &graph_nodes, spec))
    } else {
        None
    };
    let hits_scores = if matches!(spec.kind, GraphAlgorithmKind::Hits) {
        Some(graph_hits_scores(adjacency, &graph_nodes, spec))
    } else {
        None
    };

    let mut out = BTreeMap::new();
    for start in start_nodes {
        budget.check_time()?;
        let reachable =
            graph_reachable_distances(adjacency, start, spec.max_depth, spec.max_paths, budget)?;
        let output = match spec.kind {
            GraphAlgorithmKind::Reachable => GraphAlgorithmOutput::Reachable {
                reachable: !reachable.is_empty(),
                reachable_count: reachable.len(),
            },
            GraphAlgorithmKind::ShortestPath => GraphAlgorithmOutput::ShortestPath {
                shortest_distance: shortest_target_distance(&reachable, target_nodes),
            },
            GraphAlgorithmKind::ConnectedComponents => GraphAlgorithmOutput::ConnectedComponent {
                component_id: component_ids
                    .as_ref()
                    .and_then(|components| components.get(start))
                    .copied()
                    .unwrap_or(0),
            },
            GraphAlgorithmKind::Degree => {
                let out_degree = adjacency.outgoing.get(start).map(Vec::len).unwrap_or(0);
                let in_degree = adjacency.incoming.get(start).map(Vec::len).unwrap_or(0);
                let degree = match spec.variant.as_str() {
                    "in" => in_degree,
                    "total" => out_degree + in_degree,
                    _ => out_degree,
                };
                GraphAlgorithmOutput::Degree {
                    out_degree,
                    in_degree,
                    degree,
                }
            }
            GraphAlgorithmKind::PageRank => GraphAlgorithmOutput::PageRank {
                pagerank: pagerank_scores
                    .as_ref()
                    .and_then(|scores| scores.get(start))
                    .copied()
                    .unwrap_or(0.0),
            },
            GraphAlgorithmKind::Hits => {
                let (authority, hub) = hits_scores
                    .as_ref()
                    .and_then(|scores| scores.get(start))
                    .copied()
                    .unwrap_or((0.0, 0.0));
                GraphAlgorithmOutput::Hits { authority, hub }
            }
            GraphAlgorithmKind::Centrality => {
                let centrality = match spec.variant.as_str() {
                    "degree" => graph_degree_centrality(adjacency, start, graph_nodes.len()),
                    "betweenness" => betweenness_scores
                        .as_ref()
                        .and_then(|scores| scores.get(start))
                        .copied()
                        .unwrap_or(0.0),
                    _ => graph_closeness_centrality(
                        adjacency,
                        start,
                        graph_nodes.len(),
                        spec,
                        budget,
                    )?,
                };
                GraphAlgorithmOutput::Centrality { centrality }
            }
            GraphAlgorithmKind::TriangleCount => GraphAlgorithmOutput::TriangleCount {
                triangle_count: triangle_count_for(adjacency, start),
            },
            GraphAlgorithmKind::ClusteringCoefficient => {
                GraphAlgorithmOutput::ClusteringCoefficient {
                    clustering_coefficient: clustering_coefficient_for(adjacency, start),
                }
            }
            GraphAlgorithmKind::Community => GraphAlgorithmOutput::Community {
                community_id: community_ids
                    .as_ref()
                    .and_then(|communities| communities.get(start))
                    .copied()
                    .or_else(|| {
                        component_ids
                            .as_ref()
                            .and_then(|components| components.get(start))
                            .copied()
                    })
                    .unwrap_or_else(|| community_id_for(start)),
            },
            GraphAlgorithmKind::SpanningTree => {
                let (tree_parent, tree_depth) = spanning_forest
                    .as_ref()
                    .and_then(|forest| forest.get(start))
                    .cloned()
                    .unwrap_or((None, 0));
                GraphAlgorithmOutput::SpanningTree {
                    tree_parent,
                    tree_depth,
                }
            }
            GraphAlgorithmKind::AllPaths | GraphAlgorithmKind::KShortestPaths => continue,
        };
        out.insert(start.clone(), output);
    }
    Ok(out)
}

fn traversal_mode_allows(
    path: &GraphPath,
    edge_goid: &GraphEdgeId,
    target_goid: &GraphNodeId,
    mode: GraphTraversalMode,
) -> bool {
    match mode {
        GraphTraversalMode::Walk => true,
        GraphTraversalMode::Trail => !path.edge_goids.iter().any(|edge| edge == edge_goid),
        GraphTraversalMode::SimplePath => {
            !path.edge_goids.iter().any(|edge| edge == edge_goid)
                && !path.node_goids.iter().any(|node| node == target_goid)
        }
    }
}

fn push_emitted_row<T: Clone>(
    row: GraphTraversalRow<T>,
    distinct: GraphTraversalDistinctPolicy,
    seen: &mut BTreeSet<String>,
    emitted: &mut Vec<GraphTraversalRow<T>>,
) {
    let key = match distinct {
        GraphTraversalDistinctPolicy::None => None,
        GraphTraversalDistinctPolicy::Path => Some(format!(
            "{}|{}",
            row.path.start_goid.as_str(),
            row.path
                .edge_goids
                .iter()
                .map(GraphEdgeId::as_str)
                .collect::<Vec<_>>()
                .join(">")
        )),
        GraphTraversalDistinctPolicy::EndNode => Some(format!(
            "{}|{}",
            row.path.start_goid.as_str(),
            row.path.current_goid.as_str()
        )),
    };
    if let Some(key) = key {
        if !seen.insert(key) {
            return;
        }
    }
    emitted.push(row);
}

fn graph_reachable_distances(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    max_depth: u32,
    max_paths: usize,
    budget: GraphExecutionBudget,
) -> Result<BTreeMap<GraphNodeId, u32>, GraphExecutionError> {
    let mut distances = BTreeMap::new();
    let mut frontier = vec![(start.clone(), 0u32)];
    let mut seen = BTreeSet::from([start.clone()]);
    while let Some((node, depth)) = frontier.pop() {
        budget.check_time()?;
        if depth >= max_depth {
            continue;
        }
        if let Some(targets) = adjacency.outgoing.get(&node) {
            for target in targets {
                if !seen.insert(target.clone()) {
                    continue;
                }
                let next_depth = depth + 1;
                distances.insert(target.clone(), next_depth);
                if distances.len() > max_paths {
                    return Err(GraphExecutionError::ResourceLimitExceeded {
                        limit: "maximum_graph_traversal_paths",
                        actual: distances.len(),
                    });
                }
                frontier.push((target.clone(), next_depth));
            }
        }
    }
    Ok(distances)
}

fn graph_algorithm_nodes(
    adjacency: &GraphAdjacency,
    start_nodes: &BTreeSet<GraphNodeId>,
) -> BTreeSet<GraphNodeId> {
    let mut nodes = start_nodes.clone();
    for (source, targets) in &adjacency.outgoing {
        nodes.insert(source.clone());
        nodes.extend(targets.iter().cloned());
    }
    for (target, sources) in &adjacency.incoming {
        nodes.insert(target.clone());
        nodes.extend(sources.iter().cloned());
    }
    nodes
}

fn graph_pagerank_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
    spec: &GraphAlgorithmSpec,
) -> BTreeMap<GraphNodeId, f64> {
    let count = nodes.len();
    if count == 0 {
        return BTreeMap::new();
    }
    let damping = 0.85f64;
    let base = (1.0 - damping) / count as f64;
    let mut scores = nodes
        .iter()
        .map(|node| (node.clone(), 1.0 / count as f64))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..spec.max_iterations {
        let mut next = nodes
            .iter()
            .map(|node| (node.clone(), base))
            .collect::<BTreeMap<_, _>>();
        let mut sink_score = 0.0f64;
        for node in nodes {
            let score = *scores.get(node).unwrap_or(&0.0);
            let outgoing = adjacency
                .outgoing
                .get(node)
                .map(|targets| {
                    targets
                        .iter()
                        .filter(|target| nodes.contains(*target))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if outgoing.is_empty() {
                sink_score += score;
                continue;
            }
            let share = damping * score / outgoing.len() as f64;
            for target in outgoing {
                *next.entry(target.clone()).or_insert(base) += share;
            }
        }
        if sink_score > 0.0 {
            let share = damping * sink_score / count as f64;
            for value in next.values_mut() {
                *value += share;
            }
        }
        let delta = nodes
            .iter()
            .map(|node| {
                (next.get(node).copied().unwrap_or(0.0) - scores.get(node).copied().unwrap_or(0.0))
                    .abs()
            })
            .sum::<f64>();
        scores = next;
        if delta <= spec.tolerance {
            break;
        }
    }
    scores
}

fn graph_hits_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
    spec: &GraphAlgorithmSpec,
) -> BTreeMap<GraphNodeId, (f64, f64)> {
    if nodes.is_empty() {
        return BTreeMap::new();
    }
    let mut authority = nodes
        .iter()
        .map(|node| (node.clone(), 1.0f64))
        .collect::<BTreeMap<_, _>>();
    let mut hub = authority.clone();
    for _ in 0..spec.max_iterations {
        let mut next_authority = BTreeMap::new();
        for node in nodes {
            let value = adjacency
                .incoming
                .get(node)
                .into_iter()
                .flat_map(|sources| sources.iter())
                .filter(|source| nodes.contains(*source))
                .map(|source| hub.get(source).copied().unwrap_or(0.0))
                .sum::<f64>();
            next_authority.insert(node.clone(), value);
        }
        normalize_scores(&mut next_authority);
        let mut next_hub = BTreeMap::new();
        for node in nodes {
            let value = adjacency
                .outgoing
                .get(node)
                .into_iter()
                .flat_map(|targets| targets.iter())
                .filter(|target| nodes.contains(*target))
                .map(|target| next_authority.get(target).copied().unwrap_or(0.0))
                .sum::<f64>();
            next_hub.insert(node.clone(), value);
        }
        normalize_scores(&mut next_hub);
        let delta = nodes
            .iter()
            .map(|node| {
                (next_authority.get(node).copied().unwrap_or(0.0)
                    - authority.get(node).copied().unwrap_or(0.0))
                .abs()
                    + (next_hub.get(node).copied().unwrap_or(0.0)
                        - hub.get(node).copied().unwrap_or(0.0))
                    .abs()
            })
            .sum::<f64>();
        authority = next_authority;
        hub = next_hub;
        if delta <= spec.tolerance {
            break;
        }
    }
    nodes
        .iter()
        .map(|node| {
            (
                node.clone(),
                (
                    authority.get(node).copied().unwrap_or(0.0),
                    hub.get(node).copied().unwrap_or(0.0),
                ),
            )
        })
        .collect()
}

fn normalize_scores(scores: &mut BTreeMap<GraphNodeId, f64>) {
    let norm = scores
        .values()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return;
    }
    for value in scores.values_mut() {
        *value /= norm;
    }
}

fn shortest_target_distance(
    reachable: &BTreeMap<GraphNodeId, u32>,
    target_nodes: Option<&BTreeSet<GraphNodeId>>,
) -> Option<u32> {
    if let Some(target_nodes) = target_nodes {
        reachable
            .iter()
            .filter(|(goid, _)| target_nodes.contains(*goid))
            .map(|(_, distance)| *distance)
            .min()
    } else {
        reachable.values().copied().min()
    }
}

fn graph_closeness_centrality(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    node_count: usize,
    spec: &GraphAlgorithmSpec,
    budget: GraphExecutionBudget,
) -> Result<f64, GraphExecutionError> {
    if node_count <= 1 {
        return Ok(0.0);
    }
    let reachable =
        graph_reachable_distances(adjacency, start, spec.max_depth, spec.max_paths, budget)?;
    let distance_sum = reachable
        .values()
        .map(|distance| *distance as f64)
        .sum::<f64>();
    if distance_sum <= f64::EPSILON {
        return Ok(0.0);
    }
    Ok(reachable.len() as f64 / distance_sum)
}

fn graph_component_ids(adjacency: &GraphAdjacency, kind: &str) -> BTreeMap<GraphNodeId, usize> {
    if kind == "strong" {
        return graph_strong_component_ids(adjacency);
    }
    graph_weak_component_ids(adjacency)
}

fn graph_weak_component_ids(adjacency: &GraphAdjacency) -> BTreeMap<GraphNodeId, usize> {
    let mut nodes = BTreeSet::new();
    for (source, targets) in &adjacency.outgoing {
        nodes.insert(source.clone());
        nodes.extend(targets.iter().cloned());
    }
    let mut components = BTreeMap::new();
    let mut component_id = 0usize;
    for node in nodes {
        if components.contains_key(&node) {
            continue;
        }
        component_id += 1;
        let mut stack = vec![node.clone()];
        while let Some(current) = stack.pop() {
            if components.insert(current.clone(), component_id).is_some() {
                continue;
            }
            if let Some(next) = adjacency.outgoing.get(&current) {
                stack.extend(next.iter().cloned());
            }
            if let Some(prev) = adjacency.incoming.get(&current) {
                stack.extend(prev.iter().cloned());
            }
        }
    }
    components
}

fn graph_strong_component_ids(adjacency: &GraphAdjacency) -> BTreeMap<GraphNodeId, usize> {
    let nodes = graph_algorithm_nodes(adjacency, &BTreeSet::new());
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::new();
    for node in &nodes {
        graph_scc_finish_order(node, adjacency, &mut visited, &mut finish_order);
    }
    let mut components = BTreeMap::new();
    let mut component_id = 0usize;
    for node in finish_order.into_iter().rev() {
        if components.contains_key(&node) {
            continue;
        }
        component_id += 1;
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if components.insert(current.clone(), component_id).is_some() {
                continue;
            }
            if let Some(sources) = adjacency.incoming.get(&current) {
                for source in sources.iter().rev() {
                    if !components.contains_key(source) {
                        stack.push(source.clone());
                    }
                }
            }
        }
    }
    components
}

fn graph_scc_finish_order(
    node: &GraphNodeId,
    adjacency: &GraphAdjacency,
    visited: &mut BTreeSet<GraphNodeId>,
    finish_order: &mut Vec<GraphNodeId>,
) {
    if !visited.insert(node.clone()) {
        return;
    }
    if let Some(targets) = adjacency.outgoing.get(node) {
        for target in targets {
            graph_scc_finish_order(target, adjacency, visited, finish_order);
        }
    }
    finish_order.push(node.clone());
}

fn graph_degree_centrality(
    adjacency: &GraphAdjacency,
    node: &GraphNodeId,
    node_count: usize,
) -> f64 {
    if node_count <= 1 {
        return 0.0;
    }
    let out_degree = adjacency.outgoing.get(node).map(Vec::len).unwrap_or(0);
    let in_degree = adjacency.incoming.get(node).map(Vec::len).unwrap_or(0);
    (out_degree + in_degree) as f64 / (node_count - 1) as f64
}

fn graph_betweenness_centrality_scores(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
) -> BTreeMap<GraphNodeId, f64> {
    let mut scores = nodes
        .iter()
        .map(|node| (node.clone(), 0.0f64))
        .collect::<BTreeMap<_, _>>();
    for source in nodes {
        let mut stack = Vec::<GraphNodeId>::new();
        let mut predecessors = nodes
            .iter()
            .map(|node| (node.clone(), Vec::<GraphNodeId>::new()))
            .collect::<BTreeMap<_, _>>();
        let mut sigma = nodes
            .iter()
            .map(|node| (node.clone(), 0.0f64))
            .collect::<BTreeMap<_, _>>();
        let mut distance = nodes
            .iter()
            .map(|node| (node.clone(), -1i64))
            .collect::<BTreeMap<_, _>>();
        sigma.insert(source.clone(), 1.0);
        distance.insert(source.clone(), 0);
        let mut queue = VecDeque::from([source.clone()]);
        while let Some(current) = queue.pop_front() {
            stack.push(current.clone());
            let current_distance = distance.get(&current).copied().unwrap_or(-1);
            for target in adjacency.outgoing.get(&current).into_iter().flatten() {
                if !nodes.contains(target) {
                    continue;
                }
                if distance.get(target).copied().unwrap_or(-1) < 0 {
                    distance.insert(target.clone(), current_distance + 1);
                    queue.push_back(target.clone());
                }
                if distance.get(target).copied().unwrap_or(-1) == current_distance + 1 {
                    let current_sigma = sigma.get(&current).copied().unwrap_or(0.0);
                    *sigma.entry(target.clone()).or_insert(0.0) += current_sigma;
                    predecessors
                        .entry(target.clone())
                        .or_default()
                        .push(current.clone());
                }
            }
        }
        let mut dependency = nodes
            .iter()
            .map(|node| (node.clone(), 0.0f64))
            .collect::<BTreeMap<_, _>>();
        while let Some(node) = stack.pop() {
            for predecessor in predecessors.get(&node).into_iter().flatten() {
                let numerator = sigma.get(predecessor).copied().unwrap_or(0.0);
                let denominator = sigma.get(&node).copied().unwrap_or(0.0);
                if denominator <= f64::EPSILON {
                    continue;
                }
                let contribution =
                    numerator / denominator * (1.0 + dependency.get(&node).copied().unwrap_or(0.0));
                *dependency.entry(predecessor.clone()).or_insert(0.0) += contribution;
            }
            if &node != source {
                *scores.entry(node.clone()).or_insert(0.0) +=
                    dependency.get(&node).copied().unwrap_or(0.0);
            }
        }
    }
    scores
}

fn graph_label_propagation_communities(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
    max_iterations: usize,
) -> BTreeMap<GraphNodeId, usize> {
    let mut labels = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.clone(), index + 1))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..max_iterations.max(1) {
        let mut changed = false;
        let mut next = labels.clone();
        for node in nodes {
            let mut counts = BTreeMap::<usize, usize>::new();
            for neighbor in adjacency
                .outgoing
                .get(node)
                .into_iter()
                .flatten()
                .chain(adjacency.incoming.get(node).into_iter().flatten())
            {
                if let Some(label) = labels.get(neighbor) {
                    *counts.entry(*label).or_insert(0) += 1;
                }
            }
            let Some((label, _)) = counts.into_iter().max_by(
                |(left_label, left_count), (right_label, right_count)| {
                    left_count
                        .cmp(right_count)
                        .then_with(|| right_label.cmp(left_label))
                },
            ) else {
                continue;
            };
            if next.get(node).copied() != Some(label) {
                next.insert(node.clone(), label);
                changed = true;
            }
        }
        labels = next;
        if !changed {
            break;
        }
    }
    labels
}

fn graph_spanning_forest(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
    variant: &str,
    weighted_edges: &[GraphWeightedEdge],
    budget: GraphExecutionBudget,
) -> Result<BTreeMap<GraphNodeId, (Option<GraphNodeId>, u32)>, GraphExecutionError> {
    match variant {
        "dfs" => Ok(graph_search_spanning_forest(adjacency, nodes, true)),
        "min_weight" => graph_min_weight_spanning_forest(nodes, weighted_edges, budget),
        _ => Ok(graph_search_spanning_forest(adjacency, nodes, false)),
    }
}

fn graph_search_spanning_forest(
    adjacency: &GraphAdjacency,
    nodes: &BTreeSet<GraphNodeId>,
    depth_first: bool,
) -> BTreeMap<GraphNodeId, (Option<GraphNodeId>, u32)> {
    let mut forest = BTreeMap::<GraphNodeId, (Option<GraphNodeId>, u32)>::new();
    for root in nodes {
        if forest.contains_key(root) {
            continue;
        }
        forest.insert(root.clone(), (None, 0));
        if depth_first {
            let mut stack = vec![root.clone()];
            while let Some(node) = stack.pop() {
                let depth = forest.get(&node).map(|(_, depth)| *depth).unwrap_or(0);
                let Some(targets) = adjacency.outgoing.get(&node) else {
                    continue;
                };
                for target in targets.iter().rev() {
                    if forest.contains_key(target) {
                        continue;
                    }
                    forest.insert(target.clone(), (Some(node.clone()), depth + 1));
                    stack.push(target.clone());
                }
            }
        } else {
            let mut queue = VecDeque::from([root.clone()]);
            while let Some(node) = queue.pop_front() {
                let depth = forest.get(&node).map(|(_, depth)| *depth).unwrap_or(0);
                let Some(targets) = adjacency.outgoing.get(&node) else {
                    continue;
                };
                for target in targets {
                    if forest.contains_key(target) {
                        continue;
                    }
                    forest.insert(target.clone(), (Some(node.clone()), depth + 1));
                    queue.push_back(target.clone());
                }
            }
        }
    }
    forest
}

fn graph_min_weight_spanning_forest(
    nodes: &BTreeSet<GraphNodeId>,
    weighted_edges: &[GraphWeightedEdge],
    budget: GraphExecutionBudget,
) -> Result<BTreeMap<GraphNodeId, (Option<GraphNodeId>, u32)>, GraphExecutionError> {
    let mut disjoint = nodes
        .iter()
        .map(|node| (node.clone(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut edges = weighted_edges.to_vec();
    edges.sort_by(|left, right| {
        left.weight
            .partial_cmp(&right.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
    });
    let mut tree = GraphAdjacency::default();
    for edge in edges {
        budget.check_time()?;
        let source_root = graph_find_root(&mut disjoint, &edge.source);
        let target_root = graph_find_root(&mut disjoint, &edge.target);
        if source_root == target_root {
            continue;
        }
        disjoint.insert(target_root, source_root);
        tree.add_edge(edge.source.clone(), edge.target.clone());
        tree.add_edge(edge.target, edge.source);
    }
    tree.normalize();
    Ok(graph_search_spanning_forest(&tree, nodes, false))
}

fn graph_find_root(
    parents: &mut BTreeMap<GraphNodeId, GraphNodeId>,
    node: &GraphNodeId,
) -> GraphNodeId {
    let parent = parents.get(node).cloned().unwrap_or_else(|| node.clone());
    if &parent == node {
        return parent;
    }
    let root = graph_find_root(parents, &parent);
    parents.insert(node.clone(), root.clone());
    root
}

fn triangle_count_for(adjacency: &GraphAdjacency, node: &GraphNodeId) -> usize {
    let Some(neighbors) = adjacency.outgoing.get(node) else {
        return 0;
    };
    let neighbor_set = neighbors.iter().collect::<BTreeSet<_>>();
    let mut count = 0usize;
    for neighbor in neighbors {
        if let Some(second_hop) = adjacency.outgoing.get(neighbor) {
            count += second_hop
                .iter()
                .filter(|candidate| neighbor_set.contains(candidate))
                .count();
        }
    }
    count / 2
}

fn clustering_coefficient_for(adjacency: &GraphAdjacency, node: &GraphNodeId) -> f64 {
    let degree = adjacency.outgoing.get(node).map(Vec::len).unwrap_or(0);
    if degree < 2 {
        return 0.0;
    }
    let triangles = triangle_count_for(adjacency, node) as f64;
    let possible = (degree * (degree - 1) / 2) as f64;
    triangles / possible
}

fn community_id_for(goid: &GraphNodeId) -> usize {
    goid.as_str().as_bytes().iter().fold(0usize, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(*byte as usize)
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Instant};

    use crate::{GraphAlgorithmKind, GraphTraversalDistinctPolicy, GraphTraversalMode};

    use super::{
        expand_traversal_rows, run_graph_algorithm, GraphAdjacency, GraphAlgorithmOutput,
        GraphAlgorithmSpec, GraphEdge, GraphEdgeId, GraphExecutionBudget, GraphExecutionError,
        GraphNodeId, GraphPath, GraphTraversalLimits, GraphTraversalRow, GraphTraversalSpec,
        GraphWeightedEdge,
    };

    fn node(value: &str) -> GraphNodeId {
        GraphNodeId::new(value)
    }

    fn edge(id: &str, source: &str, target: &str) -> GraphEdge {
        GraphEdge::new(GraphEdgeId::new(id), node(source), node(target))
    }

    fn budget() -> GraphExecutionBudget {
        GraphExecutionBudget::new(Instant::now(), 60_000)
    }

    fn algorithm(kind: GraphAlgorithmKind, variant: &str) -> GraphAlgorithmSpec {
        GraphAlgorithmSpec {
            kind,
            variant: variant.into(),
            max_depth: 8,
            max_paths: 100,
            max_iterations: 20,
            tolerance: 1e-9,
        }
    }

    #[test]
    fn traversal_expands_paths_and_preserves_distinct_policy() {
        let rows = vec![GraphTraversalRow::new(0usize, GraphPath::new(node("a")))];
        let edges = vec![edge("ab", "a", "b"), edge("bc", "b", "c")];
        let spec = GraphTraversalSpec {
            min_depth: 1,
            max_depth: 2,
            mode: GraphTraversalMode::Walk,
            distinct: GraphTraversalDistinctPolicy::Path,
            limits: GraphTraversalLimits {
                max_fanout_per_node: 10,
                max_paths: 10,
                max_frontier: 10,
            },
        };

        let expanded = expand_traversal_rows(&rows, &edges, &spec, budget(), |payload, _, _| {
            Some(*payload)
        })
        .unwrap();

        let paths = expanded
            .iter()
            .map(|row| {
                row.path()
                    .node_goids()
                    .iter()
                    .map(GraphNodeId::as_str)
                    .collect::<Vec<_>>()
                    .join(">")
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["a>b", "a>b>c"]);
    }

    #[test]
    fn traversal_modes_reject_reused_edges_and_nodes() {
        let rows = vec![GraphTraversalRow::new(0usize, GraphPath::new(node("a")))];
        let edges = vec![edge("ab", "a", "b"), edge("ba", "b", "a")];
        let spec = GraphTraversalSpec {
            min_depth: 1,
            max_depth: 2,
            mode: GraphTraversalMode::SimplePath,
            distinct: GraphTraversalDistinctPolicy::Path,
            limits: GraphTraversalLimits {
                max_fanout_per_node: 10,
                max_paths: 10,
                max_frontier: 10,
            },
        };

        let expanded = expand_traversal_rows(&rows, &edges, &spec, budget(), |payload, _, _| {
            Some(*payload)
        })
        .unwrap();

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].path().current_goid().as_str(), "b");
    }

    #[test]
    fn graph_algorithms_compute_deterministic_degree_and_components() {
        let edges = vec![edge("ab", "a", "b"), edge("bc", "b", "c")];
        let adjacency = GraphAdjacency::from_edges(&edges);
        let starts = BTreeSet::from([node("a"), node("b"), node("c")]);

        let degree = run_graph_algorithm(
            &adjacency,
            &starts,
            &algorithm(GraphAlgorithmKind::Degree, "total"),
            None,
            &[],
            budget(),
        )
        .unwrap();
        let GraphAlgorithmOutput::Degree { degree, .. } = degree.get(&node("b")).unwrap() else {
            panic!("expected degree output");
        };
        assert_eq!(*degree, 2);

        let components = run_graph_algorithm(
            &adjacency,
            &starts,
            &algorithm(GraphAlgorithmKind::ConnectedComponents, "weak"),
            None,
            &[],
            budget(),
        )
        .unwrap();
        let GraphAlgorithmOutput::ConnectedComponent { component_id } =
            components.get(&node("c")).unwrap()
        else {
            panic!("expected component output");
        };
        assert_eq!(*component_id, 1);
    }

    #[test]
    fn iterative_scores_are_finite_and_ordered_for_chain() {
        let edges = vec![edge("ab", "a", "b"), edge("bc", "b", "c")];
        let adjacency = GraphAdjacency::from_edges(&edges);
        let starts = BTreeSet::from([node("a"), node("b"), node("c")]);

        let pagerank = run_graph_algorithm(
            &adjacency,
            &starts,
            &algorithm(GraphAlgorithmKind::PageRank, ""),
            None,
            &[],
            budget(),
        )
        .unwrap();
        let rank = |id: &str| match pagerank.get(&node(id)).unwrap() {
            GraphAlgorithmOutput::PageRank { pagerank } => *pagerank,
            _ => panic!("expected pagerank"),
        };
        assert!(rank("a").is_finite());
        assert!(rank("a") < rank("b"));
        assert!(rank("b") < rank("c"));

        let hits = run_graph_algorithm(
            &adjacency,
            &starts,
            &algorithm(GraphAlgorithmKind::Hits, ""),
            None,
            &[],
            budget(),
        )
        .unwrap();
        let GraphAlgorithmOutput::Hits { authority, hub } = hits.get(&node("b")).unwrap() else {
            panic!("expected hits output");
        };
        assert!(authority.is_finite());
        assert!(hub.is_finite());
    }

    #[test]
    fn traversal_reports_fanout_path_and_frontier_limits() {
        let rows = vec![GraphTraversalRow::new(0usize, GraphPath::new(node("a")))];
        let edges = vec![edge("ab", "a", "b"), edge("ac", "a", "c")];
        let spec = |limits| GraphTraversalSpec {
            min_depth: 1,
            max_depth: 1,
            mode: GraphTraversalMode::Walk,
            distinct: GraphTraversalDistinctPolicy::Path,
            limits,
        };
        let render = |payload: &usize, _: &GraphEdge, _: &GraphPath| Some(*payload);

        let fanout = expand_traversal_rows(
            &rows,
            &edges,
            &spec(GraphTraversalLimits {
                max_fanout_per_node: 1,
                max_paths: 10,
                max_frontier: 10,
            }),
            budget(),
            render,
        )
        .unwrap_err();
        assert_eq!(
            fanout,
            GraphExecutionError::ResourceLimitExceeded {
                limit: "maximum_graph_traversal_fanout",
                actual: 2,
            }
        );

        let paths = expand_traversal_rows(
            &rows,
            &edges,
            &spec(GraphTraversalLimits {
                max_fanout_per_node: 10,
                max_paths: 1,
                max_frontier: 10,
            }),
            budget(),
            render,
        )
        .unwrap_err();
        assert_eq!(
            paths,
            GraphExecutionError::ResourceLimitExceeded {
                limit: "maximum_graph_traversal_paths",
                actual: 2,
            }
        );

        let frontier = expand_traversal_rows(
            &rows,
            &edges,
            &spec(GraphTraversalLimits {
                max_fanout_per_node: 10,
                max_paths: 10,
                max_frontier: 1,
            }),
            budget(),
            render,
        )
        .unwrap_err();
        assert_eq!(
            frontier,
            GraphExecutionError::ResourceLimitExceeded {
                limit: "maximum_graph_traversal_frontier",
                actual: 2,
            }
        );
    }

    #[test]
    fn min_weight_spanning_tree_uses_weight_then_node_order() {
        let adjacency = GraphAdjacency::from_edges(&[
            edge("ab", "a", "b"),
            edge("ac", "a", "c"),
            edge("bc", "b", "c"),
        ]);
        let starts = BTreeSet::from([node("a"), node("b"), node("c")]);
        let weighted = vec![
            GraphWeightedEdge::new(node("a"), node("b"), 2.0),
            GraphWeightedEdge::new(node("a"), node("c"), 1.0),
            GraphWeightedEdge::new(node("b"), node("c"), 3.0),
        ];

        let output = run_graph_algorithm(
            &adjacency,
            &starts,
            &algorithm(GraphAlgorithmKind::SpanningTree, "min_weight"),
            None,
            &weighted,
            budget(),
        )
        .unwrap();
        let GraphAlgorithmOutput::SpanningTree { tree_parent, .. } =
            output.get(&node("c")).unwrap()
        else {
            panic!("expected spanning tree output");
        };
        assert_eq!(tree_parent.as_ref().map(GraphNodeId::as_str), Some("a"));
    }
}
