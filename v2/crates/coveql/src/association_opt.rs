use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    materialized::{ExecutionRow, MaterializedAssociationRow},
    AssociationEndpointRole, AstAggregateName, AstAssociationDirection, PlannedQuery,
    ResolvedAssociationRoot, ResolvedExpr, ResolvedPredicate, ResolvedRoot, TemporalMode,
    TemporalRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssociationOptimizationReport {
    pub enabled: bool,
    pub decisions: Vec<AssociationOptimizationDecision>,
    pub endpoint_plans: Vec<AssociationDirectionPlan>,
    pub edge_count: usize,
    pub semi_join_candidates: usize,
    pub anti_join_candidates: usize,
    pub count_fast_path_candidates: usize,
    pub distinct_target_fast_path_candidates: usize,
    pub validity_interval_fast_path_candidates: usize,
    pub fallback_reasons: Vec<String>,
}

impl AssociationOptimizationReport {
    pub fn for_plan(planned: &PlannedQuery, associations: &[MaterializedAssociationRow]) -> Self {
        let association_roots = association_roots(planned);
        if association_roots.is_empty() && planned.dependencies.association_type_ids.is_empty() {
            return Self::default();
        }
        let edge_table = AssociationEdgeTable::from_rows_for_plan(planned, associations);

        let mut report = Self {
            enabled: true,
            edge_count: edge_table.len(),
            ..Self::default()
        };
        let mut seen = BTreeSet::new();
        for association in association_roots {
            if seen.insert(association.object_type_id) {
                let plan = AssociationDirectionPlan::from_resolved(association);
                if !plan.endpoint_flags_complete {
                    report
                        .fallback_reasons
                        .push("missing_endpoint_flags".into());
                    report.decisions.push(AssociationOptimizationDecision::new(
                        "endpoint_flags",
                        "fallback",
                        "association endpoint properties are not fully declared by flags",
                        json!({ "association_type_id": association.object_type_id }),
                        true,
                    ));
                } else {
                    report.decisions.push(AssociationOptimizationDecision::new(
                        "endpoint_flags",
                        "candidate",
                        "association endpoint properties resolved from flags",
                        json!({ "association_type_id": association.object_type_id }),
                        true,
                    ));
                }
                if plan.validity_interval_candidate {
                    report.validity_interval_fast_path_candidates += 1;
                }
                report.endpoint_plans.push(plan);
            }
        }

        report.semi_join_candidates = count_association_exists(
            planned.resolved.method_chain.where_predicate.as_ref(),
            false,
        );
        report.anti_join_candidates =
            count_association_exists(planned.resolved.method_chain.where_predicate.as_ref(), true);
        let (count_candidates, distinct_candidates) = association_aggregate_candidates(planned);
        report.count_fast_path_candidates = count_candidates;
        report.distinct_target_fast_path_candidates = distinct_candidates;

        if report.semi_join_candidates > 0 {
            report.decisions.push(AssociationOptimizationDecision::new(
                "semi_join",
                "candidate",
                "association existence can be evaluated through an endpoint edge table before residual verification",
                json!({ "candidate_count": report.semi_join_candidates }),
                true,
            ));
        }
        if report.anti_join_candidates > 0 {
            report.decisions.push(AssociationOptimizationDecision::new(
                "anti_join",
                "candidate",
                "association non-existence is a candidate only after disclosure policy checks",
                json!({ "candidate_count": report.anti_join_candidates }),
                true,
            ));
        }
        report
    }

    pub fn to_json(&self, allow_protected: bool) -> Value {
        if allow_protected {
            return serde_json::to_value(self).unwrap_or(Value::Null);
        }
        json!({
            "enabled": self.enabled,
            "decisions": self.decisions.iter().map(|decision| decision.redacted_json()).collect::<Vec<_>>(),
            "endpoint_plan_count": self.endpoint_plans.len(),
            "edge_count": "<redacted>",
            "semi_join_candidates": self.semi_join_candidates > 0,
            "anti_join_candidates": self.anti_join_candidates > 0,
            "count_fast_path_candidates": self.count_fast_path_candidates > 0,
            "distinct_target_fast_path_candidates": self.distinct_target_fast_path_candidates > 0,
            "validity_interval_fast_path_candidates": self.validity_interval_fast_path_candidates > 0,
            "fallback_reasons": self.fallback_reasons,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssociationOptimizationDecision {
    pub kind: String,
    pub outcome: String,
    pub reason: String,
    pub safe_details: Value,
    pub redacted: bool,
}

impl AssociationOptimizationDecision {
    fn new(
        kind: impl Into<String>,
        outcome: impl Into<String>,
        reason: impl Into<String>,
        safe_details: Value,
        redacted: bool,
    ) -> Self {
        Self {
            kind: kind.into(),
            outcome: outcome.into(),
            reason: reason.into(),
            safe_details,
            redacted,
        }
    }

    fn redacted_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "outcome": self.outcome,
            "reason": self.reason,
            "safe_details": if self.redacted { json!("<redacted>") } else { self.safe_details.clone() },
            "redacted": self.redacted,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssociationDirectionPlan {
    pub association_type_id: u32,
    pub direction: Option<AstAssociationDirection>,
    pub endpoint_role: AssociationEndpointRole,
    pub inferred: bool,
    pub endpoint_flags_complete: bool,
    pub source_property_id: Option<u32>,
    pub target_property_id: Option<u32>,
    pub valid_from_property_id: Option<u32>,
    pub valid_to_property_id: Option<u32>,
    pub validity_interval_candidate: bool,
}

impl AssociationDirectionPlan {
    pub fn from_resolved(association: &ResolvedAssociationRoot) -> Self {
        let endpoint_role = association.endpoint_role;
        Self {
            association_type_id: association.object_type_id,
            direction: association.direction,
            endpoint_role,
            inferred: association.direction.is_none()
                && association.object_relative
                && endpoint_role != AssociationEndpointRole::Unknown,
            endpoint_flags_complete: association.source_property_id.is_some()
                && association.target_property_id.is_some(),
            source_property_id: association.source_property_id,
            target_property_id: association.target_property_id,
            valid_from_property_id: association.valid_from_property_id,
            valid_to_property_id: association.valid_to_property_id,
            validity_interval_candidate: association.valid_from_property_id.is_some()
                || association.valid_to_property_id.is_some(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AssociationEdgeTable {
    edges: Vec<AssociationEdge>,
    by_type: BTreeMap<u32, Vec<usize>>,
    by_source: BTreeMap<(u32, Option<usize>, String), Vec<usize>>,
    by_target: BTreeMap<(u32, Option<usize>, String), Vec<usize>>,
    association_valid_at: Option<i64>,
}

impl AssociationEdgeTable {
    #[cfg(test)]
    pub(crate) fn from_rows(rows: &[MaterializedAssociationRow]) -> Self {
        Self::from_rows_with_association_valid_at(rows, None)
    }

    pub(crate) fn from_rows_for_plan(
        planned: &PlannedQuery,
        rows: &[MaterializedAssociationRow],
    ) -> Self {
        Self::from_rows_with_association_valid_at(rows, association_valid_at(planned))
    }

    pub(crate) fn from_rows_with_association_valid_at(
        rows: &[MaterializedAssociationRow],
        association_valid_at: Option<i64>,
    ) -> Self {
        let edges = rows
            .iter()
            .filter_map(AssociationEdge::from_row)
            .collect::<Vec<_>>();
        let mut by_type = BTreeMap::<u32, Vec<usize>>::new();
        let mut by_source = BTreeMap::<(u32, Option<usize>, String), Vec<usize>>::new();
        let mut by_target = BTreeMap::<(u32, Option<usize>, String), Vec<usize>>::new();
        for (index, edge) in edges.iter().enumerate() {
            by_type
                .entry(edge.association_type_id)
                .or_default()
                .push(index);
            by_source
                .entry((
                    edge.association_type_id,
                    edge.dataset_file_ordinal,
                    edge.source_goid.clone(),
                ))
                .or_default()
                .push(index);
            by_target
                .entry((
                    edge.association_type_id,
                    edge.dataset_file_ordinal,
                    edge.target_goid.clone(),
                ))
                .or_default()
                .push(index);
        }
        Self {
            edges,
            by_type,
            by_source,
            by_target,
            association_valid_at,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.by_type.values().map(Vec::len).sum()
    }

    pub(crate) fn exists_for_scoped(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> bool {
        self.exists_for_scoped_with_target_objects(
            dataset_file_ordinal,
            current_goid,
            association,
            &BTreeSet::new(),
        )
    }

    pub(crate) fn exists_for_scoped_with_target_objects(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        visible_object_keys: &BTreeSet<(Option<usize>, u32, String)>,
    ) -> bool {
        self.candidate_edges(dataset_file_ordinal, current_goid, association)
            .any(|edge| {
                edge.matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                ) && edge.target_node_matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                    visible_object_keys,
                )
            })
    }

    #[cfg(test)]
    pub(crate) fn count_for(
        &self,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> usize {
        self.count_for_scoped(None, current_goid, association)
    }

    pub(crate) fn count_for_scoped(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> usize {
        self.count_for_scoped_with_target_objects(
            dataset_file_ordinal,
            current_goid,
            association,
            &BTreeSet::new(),
        )
    }

    pub(crate) fn count_for_scoped_with_target_objects(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        visible_object_keys: &BTreeSet<(Option<usize>, u32, String)>,
    ) -> usize {
        self.candidate_edges(dataset_file_ordinal, current_goid, association)
            .filter(|edge| {
                edge.matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                ) && edge.target_node_matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                    visible_object_keys,
                )
            })
            .count()
    }

    #[cfg(test)]
    pub(crate) fn opposite_endpoints_for(
        &self,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> BTreeSet<String> {
        self.opposite_endpoints_for_scoped(None, current_goid, association)
    }

    pub(crate) fn opposite_endpoints_for_scoped(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> BTreeSet<String> {
        self.opposite_endpoints_for_scoped_with_target_objects(
            dataset_file_ordinal,
            current_goid,
            association,
            &BTreeSet::new(),
        )
    }

    pub(crate) fn opposite_endpoints_for_scoped_with_target_objects(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        visible_object_keys: &BTreeSet<(Option<usize>, u32, String)>,
    ) -> BTreeSet<String> {
        self.candidate_edges(dataset_file_ordinal, current_goid, association)
            .filter(|edge| {
                edge.matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                ) && edge.target_node_matches(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                    visible_object_keys,
                )
            })
            .filter_map(|edge| {
                edge.opposite_endpoint_key(
                    dataset_file_ordinal,
                    current_goid,
                    association,
                    self.association_valid_at,
                )
            })
            .collect()
    }

    fn candidate_edges<'a>(
        &'a self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
    ) -> Box<dyn Iterator<Item = &'a AssociationEdge> + 'a> {
        let key = (
            association.object_type_id,
            dataset_file_ordinal,
            current_goid.to_string(),
        );
        match association.endpoint_role {
            AssociationEndpointRole::Source => Box::new(
                self.by_source
                    .get(&key)
                    .into_iter()
                    .flat_map(|indexes| indexes.iter())
                    .filter_map(|index| self.edges.get(*index)),
            ),
            AssociationEndpointRole::Target => Box::new(
                self.by_target
                    .get(&key)
                    .into_iter()
                    .flat_map(|indexes| indexes.iter())
                    .filter_map(|index| self.edges.get(*index)),
            ),
            AssociationEndpointRole::Either => {
                let mut indexes = BTreeSet::new();
                if let Some(source) = self.by_source.get(&key) {
                    indexes.extend(source.iter().copied());
                }
                if let Some(target) = self.by_target.get(&key) {
                    indexes.extend(target.iter().copied());
                }
                Box::new(
                    indexes
                        .into_iter()
                        .filter_map(|index| self.edges.get(index)),
                )
            }
            AssociationEndpointRole::Unknown => Box::new(std::iter::empty()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssociationEdge {
    dataset_file_ordinal: Option<usize>,
    association_type_id: u32,
    source_goid: String,
    target_goid: String,
    properties_by_id: BTreeMap<u32, Value>,
}

impl AssociationEdge {
    fn from_row(row: &MaterializedAssociationRow) -> Option<Self> {
        Some(Self {
            dataset_file_ordinal: row.dataset_file_ordinal,
            association_type_id: row.object_type_id,
            source_goid: row.source_goid.clone()?,
            target_goid: row.target_goid.clone()?,
            properties_by_id: row
                .property_ids
                .iter()
                .filter_map(|(id, name)| {
                    row.properties.get(name).cloned().map(|value| (*id, value))
                })
                .collect(),
        })
    }

    fn matches(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        association_valid_at: Option<i64>,
    ) -> bool {
        if self.dataset_file_ordinal != dataset_file_ordinal {
            return false;
        }
        if self.association_type_id != association.object_type_id {
            return false;
        }
        if !self.valid_at(association, association_valid_at) {
            return false;
        }
        association_endpoint_matches(
            self.source_goid.as_str(),
            self.target_goid.as_str(),
            current_goid,
            association.endpoint_role,
        )
    }

    fn valid_at(
        &self,
        association: &ResolvedAssociationRoot,
        association_valid_at: Option<i64>,
    ) -> bool {
        let Some(valid_at) = association_valid_at else {
            return true;
        };
        association_valid_interval_contains(
            valid_at,
            association
                .valid_from_property_id
                .and_then(|id| self.properties_by_id.get(&id)),
            association
                .valid_to_property_id
                .and_then(|id| self.properties_by_id.get(&id)),
        )
    }

    fn opposite_endpoint(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        association_valid_at: Option<i64>,
    ) -> Option<&str> {
        if !self.matches(
            dataset_file_ordinal,
            current_goid,
            association,
            association_valid_at,
        ) {
            return None;
        }
        match association.endpoint_role {
            AssociationEndpointRole::Source => Some(self.target_goid.as_str()),
            AssociationEndpointRole::Target => Some(self.source_goid.as_str()),
            AssociationEndpointRole::Either => {
                if self.source_goid == current_goid {
                    Some(self.target_goid.as_str())
                } else if self.target_goid == current_goid {
                    Some(self.source_goid.as_str())
                } else {
                    None
                }
            }
            AssociationEndpointRole::Unknown => None,
        }
    }

    fn opposite_endpoint_key(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        association_valid_at: Option<i64>,
    ) -> Option<String> {
        let endpoint = self.opposite_endpoint(
            dataset_file_ordinal,
            current_goid,
            association,
            association_valid_at,
        )?;
        Some(match self.dataset_file_ordinal {
            Some(ordinal) => format!("{ordinal}:{endpoint}"),
            None => endpoint.to_string(),
        })
    }

    fn target_node_matches(
        &self,
        dataset_file_ordinal: Option<usize>,
        current_goid: &str,
        association: &ResolvedAssociationRoot,
        association_valid_at: Option<i64>,
        visible_object_keys: &BTreeSet<(Option<usize>, u32, String)>,
    ) -> bool {
        let Some(target_type_id) = association.target_node_object_type_id else {
            return true;
        };
        let Some(endpoint) = self.opposite_endpoint(
            dataset_file_ordinal,
            current_goid,
            association,
            association_valid_at,
        ) else {
            return false;
        };
        visible_object_keys.contains(&(dataset_file_ordinal, target_type_id, endpoint.to_string()))
    }
}

#[cfg(test)]
pub(crate) fn association_matches_current(
    row: &MaterializedAssociationRow,
    current_goid: &str,
    association: &ResolvedAssociationRoot,
) -> bool {
    let Some(source) = row.source_goid.as_deref() else {
        return false;
    };
    let Some(target) = row.target_goid.as_deref() else {
        return false;
    };
    row.object_type_id == association.object_type_id
        && association_endpoint_matches(source, target, current_goid, association.endpoint_role)
}

fn association_endpoint_matches(
    source: &str,
    target: &str,
    current_goid: &str,
    endpoint_role: AssociationEndpointRole,
) -> bool {
    match endpoint_role {
        AssociationEndpointRole::Source => source == current_goid,
        AssociationEndpointRole::Target => target == current_goid,
        AssociationEndpointRole::Either => source == current_goid || target == current_goid,
        AssociationEndpointRole::Unknown => false,
    }
}

fn association_roots(planned: &PlannedQuery) -> Vec<&ResolvedAssociationRoot> {
    let mut out = Vec::new();
    if let ResolvedRoot::Association(association) = &planned.resolved.root {
        out.push(association);
    }
    if let Some(predicate) = &planned.resolved.method_chain.where_predicate {
        collect_association_predicates(predicate, &mut out);
    }
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_association_exprs(&item.expr, &mut out);
        }
    }
    out
}

fn collect_association_predicates<'a>(
    predicate: &'a ResolvedPredicate,
    out: &mut Vec<&'a ResolvedAssociationRoot>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_association_exprs(left, out);
            collect_association_exprs(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_association_exprs(expr, out),
        ResolvedPredicate::Not(inner) => collect_association_predicates(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_association_predicates(part, out);
            }
        }
    }
}

fn collect_association_exprs<'a>(
    expr: &'a ResolvedExpr,
    out: &mut Vec<&'a ResolvedAssociationRoot>,
) {
    match expr {
        ResolvedExpr::Association(association) => out.push(association),
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_association_exprs(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_association_exprs(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_association_predicates(predicate, out);
            collect_association_exprs(then_expr, out);
            collect_association_exprs(else_expr, out);
        }
        _ => {}
    }
}

fn count_association_exists(predicate: Option<&ResolvedPredicate>, negated: bool) -> usize {
    fn count(predicate: &ResolvedPredicate, current_negated: bool, target_negated: bool) -> usize {
        match predicate {
            ResolvedPredicate::Exists(ResolvedExpr::Association(_)) => {
                usize::from(current_negated == target_negated)
            }
            ResolvedPredicate::Not(inner) => count(inner, !current_negated, target_negated),
            ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => parts
                .iter()
                .map(|part| count(part, current_negated, target_negated))
                .sum(),
            _ => 0,
        }
    }

    predicate.map_or(0, |predicate| count(predicate, false, negated))
}

fn association_aggregate_candidates(planned: &PlannedQuery) -> (usize, usize) {
    let mut counts = 0usize;
    let mut distinct = 0usize;
    if let Some(select) = &planned.resolved.method_chain.select {
        for item in select {
            collect_association_aggregates(&item.expr, &mut counts, &mut distinct);
        }
    }
    (counts, distinct)
}

fn collect_association_aggregates(expr: &ResolvedExpr, counts: &mut usize, distinct: &mut usize) {
    match expr {
        ResolvedExpr::AggregateCall {
            name, arg, star, ..
        } => {
            if *star
                || arg
                    .as_deref()
                    .is_some_and(|arg| matches!(arg, ResolvedExpr::Association(_)))
            {
                match name {
                    AstAggregateName::Count | AstAggregateName::Exists => *counts += 1,
                    AstAggregateName::DistinctCount => *distinct += 1,
                    _ => {}
                }
            }
            if let Some(arg) = arg {
                collect_association_aggregates(arg, counts, distinct);
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_association_aggregates(arg, counts, distinct);
            }
        }
        ResolvedExpr::Conditional {
            then_expr,
            else_expr,
            ..
        } => {
            collect_association_aggregates(then_expr, counts, distinct);
            collect_association_aggregates(else_expr, counts, distinct);
        }
        _ => {}
    }
}

pub(crate) fn current_row_goid(row: &ExecutionRow) -> Option<&str> {
    match row {
        ExecutionRow::Object(row) => Some(row.goid.as_str()),
        ExecutionRow::Association(row) => Some(row.goid.as_str()),
        ExecutionRow::Evidence(_) | ExecutionRow::Projection(_) => None,
    }
}

pub(crate) fn current_row_dataset_file_ordinal(row: &ExecutionRow) -> Option<usize> {
    row.dataset_file_ordinal()
}

pub(crate) fn association_row_matches_temporal(
    row: &MaterializedAssociationRow,
    association: &ResolvedAssociationRoot,
    association_valid_at: Option<i64>,
) -> bool {
    let Some(valid_at) = association_valid_at else {
        return true;
    };
    association_valid_interval_contains(
        valid_at,
        association
            .valid_from_property_id
            .and_then(|id| row.property_ids.get(&id))
            .and_then(|name| row.properties.get(name)),
        association
            .valid_to_property_id
            .and_then(|id| row.property_ids.get(&id))
            .and_then(|name| row.properties.get(name)),
    )
}

pub(crate) fn association_valid_at(planned: &PlannedQuery) -> Option<i64> {
    if planned.resolved.temporal.role != TemporalRole::AssociationValidTime {
        return None;
    }
    match planned.resolved.temporal.mode {
        TemporalMode::AsOfTimestampMicros(timestamp) => Some(timestamp),
        _ => None,
    }
}

fn association_valid_interval_contains(
    valid_at: i64,
    valid_from: Option<&Value>,
    valid_to: Option<&Value>,
) -> bool {
    let starts_before_or_at = valid_from
        .and_then(timestamp_micros_from_value)
        .map_or(true, |from| from <= valid_at);
    let ends_after = valid_to
        .and_then(timestamp_micros_from_value)
        .map_or(true, |to| valid_at < to);
    starts_before_or_at && ends_after
}

fn timestamp_micros_from_value(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return i64::try_from(value).ok();
    }
    let value = value.as_str()?;
    if let Ok(parsed) = value.parse::<i64>() {
        return Some(parsed);
    }
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    timestamp
        .unix_timestamp()
        .checked_mul(1_000_000)?
        .checked_add(i64::from(timestamp.microsecond()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn root(direction: Option<AstAssociationDirection>) -> ResolvedAssociationRoot {
        ResolvedAssociationRoot {
            object_type_id: 7,
            type_name: "Link".into(),
            flags: 0,
            source_property_id: Some(1),
            target_property_id: Some(2),
            association_type_property_id: None,
            valid_from_property_id: Some(3),
            valid_to_property_id: None,
            direction,
            role: None,
            endpoint_role: match direction {
                Some(AstAssociationDirection::Out) => AssociationEndpointRole::Source,
                Some(AstAssociationDirection::In) => AssociationEndpointRole::Target,
                Some(AstAssociationDirection::Either) => AssociationEndpointRole::Either,
                None => AssociationEndpointRole::Unknown,
            },
            disclosure_outcome: crate::AssociationDisclosureOutcome::Public,
            object_relative: true,
            target_node_object_type_id: None,
            target_node_label: None,
        }
    }

    fn row(source: &str, target: &str) -> MaterializedAssociationRow {
        MaterializedAssociationRow {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: crate::materialized::OutputGrain::AssociationState,
            change: None,
            object_type_id: 7,
            association_type: Some("Link".into()),
            branch_key: 0,
            goid: "edge".into(),
            record_id: "record".into(),
            source_goid: Some(source.into()),
            target_goid: Some(target.into()),
            timestamp_us: 0,
            csn: 0,
            record_kind: "baseline".into(),
            tombstone_status: "live".into(),
            properties: BTreeMap::new(),
            property_ids: BTreeMap::new(),
            redacted_properties: BTreeSet::new(),
        }
    }

    fn row_with_validity(
        source: &str,
        target: &str,
        from: i64,
        to: i64,
    ) -> MaterializedAssociationRow {
        let mut row = row(source, target);
        row.properties.insert("valid_from".into(), json!(from));
        row.properties.insert("valid_to".into(), json!(to));
        row.property_ids.insert(3, "valid_from".into());
        row.property_ids.insert(4, "valid_to".into());
        row
    }

    #[test]
    fn direction_specific_matching_honors_endpoint_role() {
        let edge = row("a", "b");
        assert!(association_matches_current(
            &edge,
            "a",
            &root(Some(AstAssociationDirection::Out))
        ));
        assert!(!association_matches_current(
            &edge,
            "b",
            &root(Some(AstAssociationDirection::Out))
        ));
        assert!(association_matches_current(
            &edge,
            "b",
            &root(Some(AstAssociationDirection::In))
        ));
        assert!(association_matches_current(
            &edge,
            "a",
            &root(Some(AstAssociationDirection::Either))
        ));
    }

    #[test]
    fn direction_plan_records_endpoint_flag_and_validity_candidates() {
        let plan =
            AssociationDirectionPlan::from_resolved(&root(Some(AstAssociationDirection::Either)));
        assert_eq!(plan.endpoint_role, AssociationEndpointRole::Either);
        assert!(!plan.inferred);
        assert!(plan.endpoint_flags_complete);
        assert!(plan.validity_interval_candidate);
    }

    #[test]
    fn edge_table_counts_and_distinct_targets_use_resolved_endpoint_role() {
        let rows = vec![row("a", "b"), row("a", "c"), row("d", "a")];
        let table = AssociationEdgeTable::from_rows(&rows);
        let outgoing = root(Some(AstAssociationDirection::Out));
        let incoming = root(Some(AstAssociationDirection::In));
        let either = root(Some(AstAssociationDirection::Either));

        assert_eq!(table.count_for("a", &outgoing), 2);
        assert_eq!(table.count_for("a", &incoming), 1);
        assert_eq!(table.count_for("a", &either), 3);
        assert_eq!(table.opposite_endpoints_for("a", &either).len(), 3);
    }

    #[test]
    fn edge_table_filters_by_association_valid_interval() {
        let mut association = root(Some(AstAssociationDirection::Out));
        association.valid_to_property_id = Some(4);
        let rows = vec![
            row_with_validity("a", "before", 10, 20),
            row_with_validity("a", "during", 20, 40),
            row_with_validity("a", "after", 40, 50),
        ];
        let table = AssociationEdgeTable::from_rows_with_association_valid_at(&rows, Some(30));

        assert_eq!(table.count_for("a", &association), 1);
        assert_eq!(
            table.opposite_endpoints_for("a", &association),
            ["during".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn edge_table_keeps_manifest_member_scopes_distinct() {
        let association = root(Some(AstAssociationDirection::Out));
        let mut left = row("a", "left");
        left.dataset_file_ordinal = Some(0);
        let mut right = row("a", "right");
        right.dataset_file_ordinal = Some(1);
        let table = AssociationEdgeTable::from_rows(&[left, right]);

        assert_eq!(table.count_for_scoped(Some(0), "a", &association), 1);
        assert_eq!(table.count_for_scoped(Some(1), "a", &association), 1);
        assert_eq!(table.count_for_scoped(None, "a", &association), 0);
        assert_eq!(
            table.opposite_endpoints_for_scoped(Some(0), "a", &association),
            ["0:left".to_string()].into_iter().collect()
        );
        assert_eq!(
            table.opposite_endpoints_for_scoped(Some(1), "a", &association),
            ["1:right".to_string()].into_iter().collect()
        );
    }
}
