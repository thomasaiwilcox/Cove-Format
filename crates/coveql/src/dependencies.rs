use crate::{
    AstAggregateName, CodeDomainId, FunctionExecutionClass, RedactionPolicy, ResolvedExpr,
    ResolvedLiteral, ResolvedLiteralValue, ResolvedPath, ResolvedPredicate, ResolvedRoot,
    ResolvedSystemField, VisibilityPolicy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProjectionPushdownStatus {
    #[default]
    FullyPushdownSafe,
    PartiallyPushdownSafe,
    ResidualRequired,
    Disabled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDependencyContract {
    pub contract_version: String,
    pub projection_id: String,
    pub projection_version: Option<String>,
    pub mapping_id: Option<String>,
    pub mapping_version: Option<String>,
    pub output_table: Option<String>,
    pub row_grain: Option<String>,
    pub anchor_object_type: Option<String>,
    pub anchor_association_type: Option<String>,
    pub temporal_mode: Option<String>,
    pub assertion_ids: Vec<String>,
    pub map_columns: Vec<ProjectionDependencyColumn>,
    pub columns: Vec<ProjectionDependencyColumn>,
    pub selected_columns: BTreeSet<String>,
    pub pushed_columns: BTreeSet<String>,
    pub pushed_predicates: Vec<String>,
    pub residual_predicates: Vec<String>,
    pub source_object_types: BTreeSet<String>,
    pub source_association_types: BTreeSet<String>,
    pub source_properties: BTreeSet<u32>,
    pub source_evidence_fields: BTreeSet<String>,
    pub deterministic_functions: BTreeSet<String>,
    pub aggregate_kinds: BTreeSet<String>,
    pub function_requirements: Vec<ProjectionDependencyRequirement>,
    pub aggregate_requirements: Vec<ProjectionDependencyRequirement>,
    pub ordering: Vec<String>,
    pub evidence_policy: Option<String>,
    pub output_modes: Vec<String>,
    pub missing_policy: Option<String>,
    pub multi_value_policy: Option<String>,
    pub domain_contracts: Vec<CodeDomainId>,
    pub domain_policy: String,
    pub collation_policy: String,
    pub null_policy: String,
    pub visibility_policy: String,
    pub redaction_policy: String,
    pub pushdown_status: ProjectionPushdownStatus,
    pub residual_required_fields: BTreeSet<String>,
    pub output_compatibility: Vec<String>,
    pub pushdown_safe: bool,
    pub residual_required: bool,
    pub residual_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDependencyColumn {
    pub name: String,
    pub value: String,
    pub logical_type: Option<String>,
    pub nested_shape: Option<String>,
    pub conflict_policy: String,
    pub missing_policy: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDependencyRequirement {
    pub id: String,
    pub input_columns: BTreeSet<String>,
    pub pushdown_safe: bool,
    pub residual_required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalPlanDependencySet {
    pub object_type_ids: BTreeSet<u32>,
    pub object_type_names: BTreeSet<String>,
    pub property_ids: BTreeSet<u32>,
    pub association_type_ids: BTreeSet<u32>,
    pub projection_ids: BTreeSet<String>,
    pub projection_columns: BTreeSet<String>,
    pub projection_contracts: Vec<ProjectionDependencyContract>,
    pub evidence_fields: BTreeSet<String>,
    pub system_fields: BTreeSet<ResolvedSystemField>,
    pub deterministic_function_ids: BTreeSet<String>,
    pub aggregate_kinds: BTreeSet<String>,
    pub code_domains: Vec<CodeDomainId>,
    pub temporal_role_bindings: BTreeSet<String>,
}

impl LogicalPlanDependencySet {
    pub fn from_resolved_query(resolved: &crate::ResolvedQuery) -> Self {
        let mut dependencies = Self::default();
        dependencies.record_root(&resolved.root);
        if let Some(binding) = &resolved.temporal.role_binding {
            dependencies.temporal_role_bindings.insert(binding.clone());
        }

        if let Some(predicate) = &resolved.method_chain.where_predicate {
            dependencies.record_predicate(predicate);
        }
        for cte in &resolved.method_chain.ctes {
            dependencies.record_table_root(&cte.table);
            if let Some(key) = &cte.key {
                dependencies.record_expr(key);
            }
        }
        for lookup in &resolved.method_chain.lookups {
            dependencies.record_table_root(&lookup.right);
            dependencies.record_predicate(&lookup.on);
        }
        for traversal in &resolved.method_chain.traversals {
            dependencies.record_graph_traversal(traversal);
        }
        for algorithm in &resolved.method_chain.graph_algorithms {
            dependencies.record_graph_algorithm(algorithm);
        }
        if let Some(select) = &resolved.method_chain.select {
            for item in select {
                dependencies.record_expr(&item.expr);
            }
        }
        if let Some(order) = &resolved.method_chain.order_by {
            dependencies.record_expr(&order.expr);
        }
        if let Some(group_by) = &resolved.method_chain.group_by {
            for expr in group_by {
                dependencies.record_expr(expr);
            }
        }
        dependencies.record_projection_contracts(resolved);

        dependencies
    }

    pub fn record_root(&mut self, root: &ResolvedRoot) {
        match root {
            ResolvedRoot::Object(object) => {
                self.object_type_ids.insert(object.object_type_id);
                self.object_type_names.insert(object.type_name.clone());
            }
            ResolvedRoot::Association(association) => {
                self.object_type_ids.insert(association.object_type_id);
                self.object_type_names.insert(association.type_name.clone());
                self.association_type_ids.insert(association.object_type_id);
                if let Some(id) = association.source_property_id {
                    self.property_ids.insert(id);
                }
                if let Some(id) = association.target_property_id {
                    self.property_ids.insert(id);
                }
            }
            ResolvedRoot::Node(node) => {
                self.object_type_ids.insert(node.object.object_type_id);
                self.object_type_names.insert(node.object.type_name.clone());
            }
            ResolvedRoot::Edge(edge) => {
                let association = &edge.association;
                self.object_type_ids.insert(association.object_type_id);
                self.object_type_names.insert(association.type_name.clone());
                self.association_type_ids.insert(association.object_type_id);
                if let Some(id) = association.source_property_id {
                    self.property_ids.insert(id);
                }
                if let Some(id) = association.target_property_id {
                    self.property_ids.insert(id);
                }
            }
            ResolvedRoot::Projection(projection) => {
                self.projection_ids.insert(projection.projection_id.clone());
                self.projection_columns
                    .extend(projection.columns.iter().map(|column| column.name.clone()));
            }
            ResolvedRoot::Table(table) => {
                self.record_table_root(table);
            }
            ResolvedRoot::Evidence(evidence) => {
                self.evidence_fields.insert("evidence_index".into());
                if let Some(mapping_id) = &evidence.mapping_id {
                    self.evidence_fields.insert(format!("mapping:{mapping_id}"));
                }
                if let Some(mapping_version) = &evidence.mapping_version {
                    self.evidence_fields
                        .insert(format!("mapping_version:{mapping_version}"));
                }
            }
        }
    }

    fn record_table_root(&mut self, table: &crate::ResolvedTableRoot) {
        self.projection_ids
            .insert(table.projection.projection_id.clone());
        self.projection_columns.extend(
            table
                .projection
                .columns
                .iter()
                .map(|column| column.name.clone()),
        );
    }

    pub fn record_predicate(&mut self, predicate: &ResolvedPredicate) {
        match predicate {
            ResolvedPredicate::Compare { left, right, .. } => {
                self.record_expr(left);
                self.record_expr(right);
            }
            ResolvedPredicate::InList { expr, .. }
            | ResolvedPredicate::NullCheck { expr, .. }
            | ResolvedPredicate::Exists(expr)
            | ResolvedPredicate::BoolExpr(expr) => {
                self.record_expr(expr);
            }
            ResolvedPredicate::Not(inner) => self.record_predicate(inner),
            ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
                for part in parts {
                    self.record_predicate(part);
                }
            }
        }
    }

    pub fn record_expr(&mut self, expr: &ResolvedExpr) {
        match expr {
            ResolvedExpr::Path(path) => self.record_path(path),
            ResolvedExpr::Literal(_) => {}
            ResolvedExpr::FunctionCall {
                function_id, args, ..
            } => {
                self.deterministic_function_ids.insert(function_id.clone());
                for arg in args {
                    self.record_expr(arg);
                }
            }
            ResolvedExpr::AggregateCall { name, arg, .. } => {
                self.aggregate_kinds.insert(aggregate_name(*name).into());
                if let Some(arg) = arg {
                    self.record_expr(arg);
                }
            }
            ResolvedExpr::Association(association) => {
                self.record_association_root(association);
            }
            ResolvedExpr::Evidence(evidence) => {
                self.evidence_fields.insert("evidence_index".into());
                if let Some(mapping_id) = &evidence.mapping_id {
                    self.evidence_fields.insert(format!("mapping:{mapping_id}"));
                }
                if let Some(mapping_version) = &evidence.mapping_version {
                    self.evidence_fields
                        .insert(format!("mapping_version:{mapping_version}"));
                }
            }
            ResolvedExpr::TableExists(exists) => {
                self.record_table_root(&exists.right);
                self.record_predicate(&exists.on);
            }
            ResolvedExpr::Conditional {
                predicate,
                then_expr,
                else_expr,
                ..
            } => {
                self.record_predicate(predicate);
                self.record_expr(then_expr);
                self.record_expr(else_expr);
            }
        }
    }

    fn record_graph_traversal(&mut self, traversal: &crate::ResolvedGraphTraversal) {
        self.record_association_root(&traversal.edge.association);
        if let Some(target) = &traversal.target {
            self.object_type_ids.insert(target.object.object_type_id);
            self.object_type_names
                .insert(target.object.type_name.clone());
        }
    }

    fn record_graph_algorithm(&mut self, algorithm: &crate::ResolvedGraphAlgorithm) {
        if let Some(edge) = &algorithm.edge {
            self.record_association_root(&edge.association);
        }
        if let Some(target) = &algorithm.target {
            self.object_type_ids.insert(target.object.object_type_id);
            self.object_type_names
                .insert(target.object.type_name.clone());
        }
        if let Some(weight) = &algorithm.weight {
            self.record_expr(weight);
        }
    }

    fn record_association_root(&mut self, association: &crate::ResolvedAssociationRoot) {
        self.object_type_ids.insert(association.object_type_id);
        self.object_type_names.insert(association.type_name.clone());
        self.association_type_ids.insert(association.object_type_id);
        if let Some(id) = association.source_property_id {
            self.property_ids.insert(id);
        }
        if let Some(id) = association.target_property_id {
            self.property_ids.insert(id);
        }
        if let Some(id) = association.association_type_property_id {
            self.property_ids.insert(id);
        }
        if let Some(id) = association.valid_from_property_id {
            self.property_ids.insert(id);
        }
        if let Some(id) = association.valid_to_property_id {
            self.property_ids.insert(id);
        }
        if let Some(id) = association.target_node_object_type_id {
            self.object_type_ids.insert(id);
        }
        if let Some(label) = &association.target_node_label {
            self.object_type_names.insert(label.clone());
        }
    }

    fn record_path(&mut self, path: &ResolvedPath) {
        if let Some(object_type_id) = path.object_type_id {
            self.object_type_ids.insert(object_type_id);
        }
        if let Some(property_id) = path.property_id {
            self.property_ids.insert(property_id);
        }
        if let Some(association_type_id) = path.association_type_id {
            self.association_type_ids.insert(association_type_id);
        }
        if let Some(projection_id) = &path.projection_id {
            self.projection_ids.insert(projection_id.clone());
        }
        if path.projection_column.is_some() {
            self.projection_columns.insert(projection_column_name(path));
        }
        if let Some(field) = &path.evidence_field_id {
            self.evidence_fields.insert(field.clone());
        }
        if let Some(system_field) = &path.system_field {
            self.system_fields.insert(system_field.clone());
        }
        self.code_domains.push(path.code_domain_id.clone());
    }

    fn record_projection_contracts(&mut self, resolved: &crate::ResolvedQuery) {
        if let Some(projection) = projection_contract_root(&resolved.root) {
            let selected_columns =
                selected_projection_columns(&resolved.method_chain.select, projection);
            let pushed_columns = projection_pushed_columns(resolved, &selected_columns);
            let (pushed_predicates, residual_predicates) = projection_predicate_pushdown_contract(
                resolved.method_chain.where_predicate.as_ref(),
            );
            let mut residual_reasons = Vec::new();
            if projection_select_contains_residual_expr(&resolved.method_chain.select) {
                residual_reasons.push(
                    "projection select contains non-column expressions; residual output evaluation required"
                        .into(),
                );
            }
            if resolved.method_chain.group_by.is_some() {
                residual_reasons
                    .push("projection grouping requires residual/materialized execution".into());
            }
            if resolved.method_chain.order_by.is_some() {
                residual_reasons.push(
                    "projection ordering requires residual/materialized sort semantics".into(),
                );
            }
            if resolved
                .method_chain
                .where_predicate
                .as_ref()
                .is_some_and(|predicate| !projection_predicate_is_pushdown_safe(predicate))
            {
                residual_reasons.push(
                    "projection predicate is not fully pushdown-safe and remains residual".into(),
                );
            }
            let pushdown_safe = residual_reasons.is_empty();
            let residual_required = !residual_reasons.is_empty();
            let residual_required_fields =
                projection_residual_required_fields(resolved, &selected_columns, residual_required);
            let pushdown_status =
                projection_pushdown_status(pushdown_safe, residual_required, &pushed_columns);
            let function_requirements = projection_function_requirements(resolved);
            let aggregate_requirements = projection_aggregate_requirements(resolved);
            self.projection_contracts
                .push(ProjectionDependencyContract {
                    contract_version: crate::PROJECTION_DEPENDENCY_CONTRACT_VERSION.into(),
                    projection_id: projection.projection_id.clone(),
                    projection_version: Some(projection.mapping_version.clone()),
                    mapping_id: Some(projection.mapping_id.clone()),
                    mapping_version: Some(projection.mapping_version.clone()),
                    output_table: projection.output_table.clone(),
                    row_grain: projection.row_grain.clone(),
                    anchor_object_type: projection
                        .anchor
                        .as_ref()
                        .and_then(|anchor| anchor.object_type.clone()),
                    anchor_association_type: projection
                        .anchor
                        .as_ref()
                        .and_then(|anchor| anchor.association_type.clone()),
                    temporal_mode: projection.temporal_mode.clone(),
                    assertion_ids: projection.assertion_ids.clone(),
                    map_columns: projection_dependency_columns(projection),
                    columns: projection_dependency_columns(projection),
                    source_properties: projection_source_properties(
                        projection,
                        &pushed_columns,
                        &self.property_ids,
                    ),
                    selected_columns,
                    pushed_columns,
                    pushed_predicates,
                    residual_predicates,
                    source_object_types: projection
                        .anchor
                        .as_ref()
                        .and_then(|anchor| anchor.object_type.clone())
                        .into_iter()
                        .collect(),
                    source_association_types: projection
                        .anchor
                        .as_ref()
                        .and_then(|anchor| anchor.association_type.clone())
                        .into_iter()
                        .collect(),
                    source_evidence_fields: self.evidence_fields.clone(),
                    deterministic_functions: self.deterministic_function_ids.clone(),
                    aggregate_kinds: self.aggregate_kinds.clone(),
                    function_requirements,
                    aggregate_requirements,
                    ordering: projection.ordering.clone(),
                    evidence_policy: Some(projection.evidence_policy.clone()),
                    output_modes: projection.output_modes.clone(),
                    missing_policy: Some(projection.missing_policy.clone()),
                    multi_value_policy: projection.multi_value_policy.clone(),
                    domain_contracts: self.code_domains.clone(),
                    domain_policy: "same_projection_contract_or_materialized".into(),
                    collation_policy: "declared_collation_or_materialized_sort".into(),
                    null_policy: "cove_null_semantics_preserved".into(),
                    visibility_policy: visibility_policy_name(
                        &resolved.operation_context.security.visibility_policy,
                    ),
                    redaction_policy: redaction_policy_name(
                        &resolved.operation_context.security.redaction_policy,
                    ),
                    pushdown_status,
                    residual_required_fields,
                    output_compatibility: projection.output_modes.clone(),
                    pushdown_safe,
                    residual_required,
                    residual_reasons,
                });
        }
        for projection_id in self.projection_ids.clone() {
            if self
                .projection_contracts
                .iter()
                .any(|contract| contract.projection_id == projection_id)
            {
                continue;
            }
            self.projection_contracts
                .push(ProjectionDependencyContract {
                    contract_version: crate::PROJECTION_DEPENDENCY_CONTRACT_VERSION.into(),
                    projection_id,
                    selected_columns: self.projection_columns.clone(),
                    pushed_columns: BTreeSet::new(),
                    pushed_predicates: Vec::new(),
                    residual_predicates: resolved
                        .method_chain
                        .where_predicate
                        .as_ref()
                        .map(|predicate| vec![projection_predicate_summary(predicate)])
                        .unwrap_or_default(),
                    source_properties: self.property_ids.clone(),
                    source_evidence_fields: self.evidence_fields.clone(),
                    deterministic_functions: self.deterministic_function_ids.clone(),
                    aggregate_kinds: self.aggregate_kinds.clone(),
                    function_requirements: projection_function_requirements(resolved),
                    aggregate_requirements: projection_aggregate_requirements(resolved),
                    domain_contracts: self.code_domains.clone(),
                    domain_policy: "projection_catalog_unavailable".into(),
                    collation_policy: "materialized_only".into(),
                    null_policy: "materialized_only".into(),
                    visibility_policy: visibility_policy_name(
                        &resolved.operation_context.security.visibility_policy,
                    ),
                    redaction_policy: redaction_policy_name(
                        &resolved.operation_context.security.redaction_policy,
                    ),
                    pushdown_status: ProjectionPushdownStatus::Disabled,
                    residual_required_fields: self.projection_columns.clone(),
                    output_compatibility: Vec::new(),
                    pushdown_safe: false,
                    residual_required: true,
                    residual_reasons: vec![
                        "projection catalog entry was not available in resolved root".into(),
                    ],
                    ..ProjectionDependencyContract::default()
                });
        }
    }
}

fn projection_contract_root(root: &ResolvedRoot) -> Option<&crate::ResolvedProjectionRoot> {
    match root {
        ResolvedRoot::Projection(projection) => Some(projection),
        ResolvedRoot::Table(table) => Some(&table.projection),
        _ => None,
    }
}

fn projection_pushdown_status(
    pushdown_safe: bool,
    residual_required: bool,
    pushed_columns: &BTreeSet<String>,
) -> ProjectionPushdownStatus {
    if pushdown_safe {
        ProjectionPushdownStatus::FullyPushdownSafe
    } else if residual_required && !pushed_columns.is_empty() {
        ProjectionPushdownStatus::PartiallyPushdownSafe
    } else if residual_required {
        ProjectionPushdownStatus::ResidualRequired
    } else {
        ProjectionPushdownStatus::Disabled
    }
}

fn projection_residual_required_fields(
    resolved: &crate::ResolvedQuery,
    selected_columns: &BTreeSet<String>,
    residual_required: bool,
) -> BTreeSet<String> {
    if !residual_required {
        return BTreeSet::new();
    }
    let mut fields = selected_columns.clone();
    if let Some(predicate) = &resolved.method_chain.where_predicate {
        collect_projection_predicate_columns(predicate, &mut fields);
    }
    if let Some(order) = &resolved.method_chain.order_by {
        collect_projection_expr_column(&order.expr, &mut fields);
    }
    fields
}

fn projection_dependency_columns(
    projection: &crate::ResolvedProjectionRoot,
) -> Vec<ProjectionDependencyColumn> {
    projection
        .columns
        .iter()
        .map(|column| ProjectionDependencyColumn {
            name: column.name.clone(),
            value: column.value.clone(),
            logical_type: column.logical_type.clone(),
            nested_shape: column.nested_shape.clone(),
            conflict_policy: column.conflict_policy.clone(),
            missing_policy: column.missing_policy.clone(),
        })
        .collect()
}

fn projection_source_properties(
    projection: &crate::ResolvedProjectionRoot,
    pushed_columns: &BTreeSet<String>,
    observed_properties: &BTreeSet<u32>,
) -> BTreeSet<u32> {
    let mut out = observed_properties.clone();
    for column in &projection.columns {
        if pushed_columns.is_empty() || pushed_columns.contains(&column.name) {
            if let Some(property_id) = column.source_property_id {
                out.insert(property_id);
            }
        }
    }
    out
}

fn projection_predicate_pushdown_contract(
    predicate: Option<&ResolvedPredicate>,
) -> (Vec<String>, Vec<String>) {
    let Some(predicate) = predicate else {
        return (Vec::new(), Vec::new());
    };
    if projection_predicate_is_pushdown_safe(predicate) {
        let mut pushed = Vec::new();
        collect_projection_predicate_summaries(predicate, &mut pushed);
        (pushed, Vec::new())
    } else {
        (Vec::new(), vec![projection_predicate_summary(predicate)])
    }
}

fn collect_projection_predicate_summaries(predicate: &ResolvedPredicate, out: &mut Vec<String>) {
    match predicate {
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_projection_predicate_summaries(part, out);
            }
        }
        ResolvedPredicate::Not(inner) => {
            collect_projection_negated_predicate_summaries(inner, out);
        }
        ResolvedPredicate::Or(parts) => {
            if let Some(summary) = projection_same_column_equality_or_summary(parts) {
                out.push(summary);
            } else {
                out.push(projection_predicate_summary(predicate));
            }
        }
        _ => out.push(projection_predicate_summary(predicate)),
    }
}

fn collect_projection_negated_predicate_summaries(
    predicate: &ResolvedPredicate,
    out: &mut Vec<String>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => out.push(format!(
            "compare:{:?}:{}",
            negated_compare_op(*op),
            projection_predicate_columns_summary([left, right])
        )),
        ResolvedPredicate::InList { expr, values } => out.push(format!(
            "not_in:{}:{} literals",
            projection_predicate_columns_summary([expr]),
            values.len()
        )),
        ResolvedPredicate::NullCheck { expr, negated } => out.push(format!(
            "{}:{}",
            if !*negated { "is_not_null" } else { "is_null" },
            projection_predicate_columns_summary([expr])
        )),
        ResolvedPredicate::BoolExpr(expr) => out.push(format!(
            "not_bool:{}",
            projection_predicate_columns_summary([expr])
        )),
        ResolvedPredicate::Not(inner) => collect_projection_predicate_summaries(inner, out),
        _ => out.push(format!("not:{}", projection_predicate_summary(predicate))),
    }
}

fn projection_predicate_summary(predicate: &ResolvedPredicate) -> String {
    match predicate {
        ResolvedPredicate::Compare { left, op, right } => format!(
            "compare:{op:?}:{}",
            projection_predicate_columns_summary([left, right])
        ),
        ResolvedPredicate::InList { expr, values } => format!(
            "in:{}:{} literals",
            projection_predicate_columns_summary([expr]),
            values.len()
        ),
        ResolvedPredicate::NullCheck { expr, negated } => format!(
            "{}:{}",
            if *negated { "is_not_null" } else { "is_null" },
            projection_predicate_columns_summary([expr])
        ),
        ResolvedPredicate::Exists(expr) => {
            format!("exists:{}", projection_predicate_columns_summary([expr]))
        }
        ResolvedPredicate::BoolExpr(expr) => {
            format!("bool:{}", projection_predicate_columns_summary([expr]))
        }
        ResolvedPredicate::Not(inner) => {
            format!("not:{}", projection_predicate_summary(inner))
        }
        ResolvedPredicate::And(parts) => format!("and:{} terms", parts.len()),
        ResolvedPredicate::Or(parts) => format!("or:{} terms", parts.len()),
    }
}

fn projection_predicate_columns_summary<'a>(
    exprs: impl IntoIterator<Item = &'a ResolvedExpr>,
) -> String {
    let mut columns = BTreeSet::new();
    for expr in exprs {
        collect_projection_expr_column(expr, &mut columns);
    }
    if columns.is_empty() {
        "expression".into()
    } else {
        columns.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn visibility_policy_name(policy: &VisibilityPolicy) -> String {
    match policy {
        VisibilityPolicy::AllRows => "all_rows".into(),
        VisibilityPolicy::ExternalOverlay(reference) => format!("external_overlay:{reference}"),
    }
}

fn redaction_policy_name(policy: &RedactionPolicy) -> String {
    match policy {
        RedactionPolicy::ProtectedValuesRedacted => "protected_values_redacted".into(),
        RedactionPolicy::RefuseProtectedValues => "refuse_protected_values".into(),
    }
}

fn selected_projection_columns(
    select: &Option<Vec<crate::ResolvedSelectItem>>,
    projection: &crate::ResolvedProjectionRoot,
) -> BTreeSet<String> {
    let Some(select) = select else {
        return projection
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
    };
    let mut columns = BTreeSet::new();
    for item in select {
        collect_projection_expr_column(&item.expr, &mut columns);
    }
    columns
}

fn projection_select_contains_residual_expr(
    select: &Option<Vec<crate::ResolvedSelectItem>>,
) -> bool {
    select.as_ref().is_some_and(|select| {
        select
            .iter()
            .any(|item| !matches!(&item.expr, ResolvedExpr::Path(path) if path.projection_column.is_some()))
    })
}

fn projection_pushed_columns(
    resolved: &crate::ResolvedQuery,
    selected_columns: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut pushed = selected_columns.clone();
    if let Some(predicate) = &resolved.method_chain.where_predicate {
        collect_projection_predicate_columns(predicate, &mut pushed);
    }
    if let Some(order) = &resolved.method_chain.order_by {
        collect_projection_expr_column(&order.expr, &mut pushed);
    }
    if let Some(group_by) = &resolved.method_chain.group_by {
        for expr in group_by {
            collect_projection_expr_column(expr, &mut pushed);
        }
    }
    pushed
}

fn projection_function_requirements(
    resolved: &crate::ResolvedQuery,
) -> Vec<ProjectionDependencyRequirement> {
    let mut requirements = Vec::new();
    if let Some(predicate) = &resolved.method_chain.where_predicate {
        collect_function_requirements_from_predicate(predicate, &mut requirements);
    }
    if let Some(select) = &resolved.method_chain.select {
        for item in select {
            collect_function_requirements_from_expr(&item.expr, &mut requirements);
        }
    }
    if let Some(order) = &resolved.method_chain.order_by {
        collect_function_requirements_from_expr(&order.expr, &mut requirements);
    }
    if let Some(group_by) = &resolved.method_chain.group_by {
        for expr in group_by {
            collect_function_requirements_from_expr(expr, &mut requirements);
        }
    }
    requirements
}

fn projection_aggregate_requirements(
    resolved: &crate::ResolvedQuery,
) -> Vec<ProjectionDependencyRequirement> {
    let mut requirements = Vec::new();
    if let Some(select) = &resolved.method_chain.select {
        for item in select {
            collect_aggregate_requirements_from_expr(&item.expr, &mut requirements);
        }
    }
    requirements
}

fn collect_function_requirements_from_predicate(
    predicate: &ResolvedPredicate,
    out: &mut Vec<ProjectionDependencyRequirement>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_function_requirements_from_expr(left, out);
            collect_function_requirements_from_expr(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_function_requirements_from_expr(expr, out),
        ResolvedPredicate::Not(inner) => collect_function_requirements_from_predicate(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_function_requirements_from_predicate(part, out);
            }
        }
    }
}

fn collect_function_requirements_from_expr(
    expr: &ResolvedExpr,
    out: &mut Vec<ProjectionDependencyRequirement>,
) {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id,
            contract,
            args,
            ..
        } => {
            let mut input_columns = BTreeSet::new();
            for arg in args {
                collect_projection_expr_column(arg, &mut input_columns);
                collect_function_requirements_from_expr(arg, out);
            }
            let declared_coded_safe =
                matches!(contract.execution_class, FunctionExecutionClass::CodedSafe);
            out.push(ProjectionDependencyRequirement {
                id: function_id.clone(),
                input_columns,
                pushdown_safe: false,
                residual_required: true,
                reason: if declared_coded_safe {
                    "function declares a coded-safe contract, but projection execution still requires a precomputed projection column or residual verification".into()
                } else {
                    "function requires materialized projection values for CoveQL-equivalent output evaluation".into()
                },
            });
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_function_requirements_from_expr(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_function_requirements_from_predicate(predicate, out);
            collect_function_requirements_from_expr(then_expr, out);
            collect_function_requirements_from_expr(else_expr, out);
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => {}
        ResolvedExpr::TableExists(exists) => {
            collect_function_requirements_from_predicate(&exists.on, out);
        }
    }
}

fn collect_aggregate_requirements_from_expr(
    expr: &ResolvedExpr,
    out: &mut Vec<ProjectionDependencyRequirement>,
) {
    match expr {
        ResolvedExpr::AggregateCall { name, arg, .. } => {
            let mut input_columns = BTreeSet::new();
            if let Some(arg) = arg {
                collect_projection_expr_column(arg, &mut input_columns);
                collect_aggregate_requirements_from_expr(arg, out);
            }
            out.push(ProjectionDependencyRequirement {
                id: aggregate_name(*name).into(),
                input_columns,
                pushdown_safe: false,
                residual_required: true,
                reason: "aggregate requires materialized projection rows plus exact grouping, null, duplicate-row, and disclosure semantics".into(),
            });
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_aggregate_requirements_from_expr(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_aggregate_requirements_from_predicate(predicate, out);
            collect_aggregate_requirements_from_expr(then_expr, out);
            collect_aggregate_requirements_from_expr(else_expr, out);
        }
        ResolvedExpr::Path(_)
        | ResolvedExpr::Literal(_)
        | ResolvedExpr::Association(_)
        | ResolvedExpr::Evidence(_) => {}
        ResolvedExpr::TableExists(exists) => {
            collect_aggregate_requirements_from_predicate(&exists.on, out);
        }
    }
}

fn collect_aggregate_requirements_from_predicate(
    predicate: &ResolvedPredicate,
    out: &mut Vec<ProjectionDependencyRequirement>,
) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_aggregate_requirements_from_expr(left, out);
            collect_aggregate_requirements_from_expr(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_aggregate_requirements_from_expr(expr, out),
        ResolvedPredicate::Not(inner) => collect_aggregate_requirements_from_predicate(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_aggregate_requirements_from_predicate(part, out);
            }
        }
    }
}

fn projection_predicate_is_pushdown_safe(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::And(parts) => parts.iter().all(projection_predicate_is_pushdown_safe),
        ResolvedPredicate::Compare { left, op, right } => {
            matches!(
                op,
                crate::AstCompareOp::Eq
                    | crate::AstCompareOp::Ne
                    | crate::AstCompareOp::Lt
                    | crate::AstCompareOp::Le
                    | crate::AstCompareOp::Gt
                    | crate::AstCompareOp::Ge
            ) && ((projection_path(left).is_some() && matches!(right, ResolvedExpr::Literal(_)))
                || (matches!(left, ResolvedExpr::Literal(_)) && projection_path(right).is_some()))
        }
        ResolvedPredicate::InList { expr, .. } | ResolvedPredicate::NullCheck { expr, .. } => {
            projection_path(expr).is_some()
        }
        ResolvedPredicate::BoolExpr(expr) => projection_bool_path(expr).is_some(),
        ResolvedPredicate::Not(inner) => projection_negated_predicate_is_pushdown_safe(inner),
        ResolvedPredicate::Or(parts) => projection_same_column_equality_or_summary(parts).is_some(),
        ResolvedPredicate::Exists(_) => false,
    }
}

fn projection_negated_predicate_is_pushdown_safe(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            (projection_path(left).is_some() && matches!(right, ResolvedExpr::Literal(_)))
                || (matches!(left, ResolvedExpr::Literal(_)) && projection_path(right).is_some())
        }
        ResolvedPredicate::InList { expr, values } => {
            projection_path(expr).is_some()
                && values
                    .iter()
                    .all(|literal| !matches!(literal.typed_value, ResolvedLiteralValue::Null))
        }
        ResolvedPredicate::NullCheck { expr, .. } => projection_path(expr).is_some(),
        ResolvedPredicate::BoolExpr(expr) => projection_bool_path(expr).is_some(),
        ResolvedPredicate::Not(inner) => projection_predicate_is_pushdown_safe(inner),
        ResolvedPredicate::And(_) | ResolvedPredicate::Or(_) | ResolvedPredicate::Exists(_) => {
            false
        }
    }
}

#[derive(Debug, Clone)]
struct ProjectionEqualitySetSummary {
    column: String,
    literal_keys: BTreeSet<String>,
}

fn projection_same_column_equality_or_summary(parts: &[ResolvedPredicate]) -> Option<String> {
    let summary = projection_same_column_equality_or_set(parts)?;
    Some(format!(
        "in:{}:{} literals",
        summary.column,
        summary.literal_keys.len()
    ))
}

fn projection_same_column_equality_or_set(
    parts: &[ResolvedPredicate],
) -> Option<ProjectionEqualitySetSummary> {
    let mut parts = parts.iter();
    let first = projection_single_equality_set_summary(parts.next()?)?;
    let column = first.column;
    let mut literal_keys = first.literal_keys;
    for part in parts {
        let summary = projection_single_equality_set_summary(part)?;
        if summary.column != column {
            return None;
        }
        literal_keys.extend(summary.literal_keys);
    }
    (!literal_keys.is_empty()).then_some(ProjectionEqualitySetSummary {
        column,
        literal_keys,
    })
}

fn projection_single_equality_set_summary(
    predicate: &ResolvedPredicate,
) -> Option<ProjectionEqualitySetSummary> {
    match predicate {
        ResolvedPredicate::Compare {
            left,
            op: crate::AstCompareOp::Eq,
            right,
        } => {
            if let (Some(path), ResolvedExpr::Literal(literal)) = (projection_path(left), right) {
                return projection_non_null_literal_key(literal).map(|key| {
                    ProjectionEqualitySetSummary {
                        column: projection_column_name(path),
                        literal_keys: BTreeSet::from([key]),
                    }
                });
            }
            if let (ResolvedExpr::Literal(literal), Some(path)) = (left, projection_path(right)) {
                return projection_non_null_literal_key(literal).map(|key| {
                    ProjectionEqualitySetSummary {
                        column: projection_column_name(path),
                        literal_keys: BTreeSet::from([key]),
                    }
                });
            }
            None
        }
        ResolvedPredicate::InList { expr, values } => {
            let path = projection_path(expr)?;
            let literal_keys = values
                .iter()
                .map(projection_non_null_literal_key)
                .collect::<Option<BTreeSet<_>>>()?;
            (!literal_keys.is_empty()).then(|| ProjectionEqualitySetSummary {
                column: projection_column_name(path),
                literal_keys,
            })
        }
        ResolvedPredicate::Or(parts) => projection_same_column_equality_or_set(parts),
        _ => None,
    }
}

fn projection_non_null_literal_key(literal: &ResolvedLiteral) -> Option<String> {
    if matches!(literal.typed_value, ResolvedLiteralValue::Null) {
        return None;
    }
    Some(format!("{:?}", literal.typed_value))
}

fn projection_column_name(path: &ResolvedPath) -> String {
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

fn negated_compare_op(op: crate::AstCompareOp) -> crate::AstCompareOp {
    match op {
        crate::AstCompareOp::Eq => crate::AstCompareOp::Ne,
        crate::AstCompareOp::Ne => crate::AstCompareOp::Eq,
        crate::AstCompareOp::Lt => crate::AstCompareOp::Ge,
        crate::AstCompareOp::Le => crate::AstCompareOp::Gt,
        crate::AstCompareOp::Gt => crate::AstCompareOp::Le,
        crate::AstCompareOp::Ge => crate::AstCompareOp::Lt,
    }
}

fn collect_projection_predicate_columns(predicate: &ResolvedPredicate, out: &mut BTreeSet<String>) {
    match predicate {
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_projection_predicate_columns(part, out);
            }
        }
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_projection_expr_column(left, out);
            collect_projection_expr_column(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_projection_expr_column(expr, out),
        ResolvedPredicate::Not(inner) => collect_projection_predicate_columns(inner, out),
    }
}

fn collect_projection_expr_column(expr: &ResolvedExpr, out: &mut BTreeSet<String>) {
    match expr {
        ResolvedExpr::Path(path) => {
            if path.projection_column.is_some() {
                out.insert(projection_column_name(path));
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_projection_expr_column(arg, out);
            }
        }
        ResolvedExpr::AggregateCall { arg, .. } => {
            if let Some(arg) = arg {
                collect_projection_expr_column(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_projection_predicate_columns(predicate, out);
            collect_projection_expr_column(then_expr, out);
            collect_projection_expr_column(else_expr, out);
        }
        ResolvedExpr::TableExists(exists) => {
            collect_projection_predicate_columns(&exists.on, out);
        }
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) | ResolvedExpr::Literal(_) => {}
    }
}

fn projection_path(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    let ResolvedExpr::Path(path) = expr else {
        return None;
    };
    path.projection_column.as_ref()?;
    Some(path)
}

fn projection_bool_path(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    let path = projection_path(expr)?;
    matches!(path.logical_type.as_str(), "bool" | "boolean").then_some(path)
}

fn aggregate_name(name: AstAggregateName) -> &'static str {
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
