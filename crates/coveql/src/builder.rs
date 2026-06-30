use crate::{
    parse_and_resolve_query, parse_query, parse_resolve_and_plan_query, AstAggregateName,
    AstAssociationDirection, AstAssociationRole, AstChangeBound, AstChangeMode, AstEvidenceGrain,
    AstHistoryMode, AstNullOrdering, AstOrderDirection, AstTimeBound, AstTimeRole,
    BuildLogicalPlanError, BuildResolvedQueryError, CoveQlDiagnostic, ExplainMode, ParseOptions,
    ParsedQuery, PlanOptions, PlannedQuery, ResolveOptions, ResolvedQuery, TableLookupCardinality,
    TableLookupDuplicatePolicy, TableLookupUnmatchedPolicy,
};
use cove_core::reader::ValidationOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveQlQueryBuilder {
    root: String,
    methods: Vec<String>,
}

impl CoveQlQueryBuilder {
    pub fn object(type_name: impl AsRef<str>) -> Self {
        Self::new(format!("object({})", coveql_identifier(type_name.as_ref())))
    }

    pub fn association(type_name: impl AsRef<str>) -> Self {
        Self::new(format!(
            "association({})",
            coveql_identifier(type_name.as_ref())
        ))
    }

    pub fn association_with_role(
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
    ) -> Self {
        Self::new(association_expression(
            type_name,
            Some(role),
            Some(role_name),
        ))
    }

    pub fn association_with_direction(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
    ) -> Self {
        Self::new(directed_association_expression(
            direction,
            type_name,
            None::<AstAssociationRole>,
            None::<&str>,
        ))
    }

    pub fn association_with_direction_and_role(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
    ) -> Self {
        Self::new(directed_association_expression(
            direction,
            type_name,
            Some(role),
            Some(role_name),
        ))
    }

    pub fn projection(projection_id: impl AsRef<str>) -> Self {
        Self::new(format!(
            "projection({})",
            coveql_identifier(projection_id.as_ref())
        ))
    }

    pub fn table(table_name: impl AsRef<str>) -> Self {
        Self::new(format!("table({})", coveql_identifier(table_name.as_ref())))
    }

    pub fn node(label: impl AsRef<str>) -> Self {
        Self::new(format!("node({})", coveql_identifier(label.as_ref())))
    }

    pub fn edge(label: impl AsRef<str>) -> Self {
        Self::new(format!("edge({})", coveql_identifier(label.as_ref())))
    }

    pub fn path(path_expr: impl AsRef<str>) -> Self {
        Self::new(format!("path({})", path_expr.as_ref()))
    }

    pub fn alias(mut self, alias: impl AsRef<str>) -> Self {
        self.root
            .push_str(&format!(" as {}", coveql_identifier(alias.as_ref())));
        self
    }

    pub fn evidence() -> Self {
        Self::new("evidence()")
    }

    pub fn evidence_with_grain(grain: AstEvidenceGrain) -> Self {
        Self::new(format!("evidence(grain: {})", evidence_grain_name(grain)))
    }

    pub fn evidence_self() -> Self {
        Self::new("evidence(self)")
    }

    pub fn evidence_self_with_grain(grain: AstEvidenceGrain) -> Self {
        Self::new(format!(
            "evidence(self, grain: {})",
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_path(path: impl AsRef<str>) -> Self {
        Self::new(format!("evidence({})", path.as_ref()))
    }

    pub fn evidence_path_with_grain(path: impl AsRef<str>, grain: AstEvidenceGrain) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            path.as_ref(),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_root_binding(root_binding: impl AsRef<str>) -> Self {
        Self::new(format!("evidence({})", root_binding.as_ref()))
    }

    pub fn evidence_root_binding_with_grain(
        root_binding: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            root_binding.as_ref(),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_association(type_name: impl AsRef<str>) -> Self {
        Self::new(format!(
            "evidence({})",
            association_expression(type_name, None::<AstAssociationRole>, None::<&str>)
        ))
    }

    pub fn evidence_association_with_grain(
        type_name: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            association_expression(type_name, None::<AstAssociationRole>, None::<&str>),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_association_with_role(
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
    ) -> Self {
        Self::new(format!(
            "evidence({})",
            association_expression(type_name, Some(role), Some(role_name))
        ))
    }

    pub fn evidence_association_with_role_and_grain(
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            association_expression(type_name, Some(role), Some(role_name)),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_association_with_direction(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
    ) -> Self {
        Self::new(format!(
            "evidence({})",
            directed_association_expression(
                direction,
                type_name,
                None::<AstAssociationRole>,
                None::<&str>
            )
        ))
    }

    pub fn evidence_association_with_direction_and_grain(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            directed_association_expression(
                direction,
                type_name,
                None::<AstAssociationRole>,
                None::<&str>
            ),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_association_with_direction_and_role(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
    ) -> Self {
        Self::new(format!(
            "evidence({})",
            directed_association_expression(direction, type_name, Some(role), Some(role_name))
        ))
    }

    pub fn evidence_association_with_direction_role_and_grain(
        direction: AstAssociationDirection,
        type_name: impl AsRef<str>,
        role: AstAssociationRole,
        role_name: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence({}, grain: {})",
            directed_association_expression(direction, type_name, Some(role), Some(role_name)),
            evidence_grain_name(grain)
        ))
    }

    pub fn evidence_projection(projection_id: impl AsRef<str>) -> Self {
        Self::new(format!(
            "evidence(projection({}))",
            coveql_identifier(projection_id.as_ref())
        ))
    }

    pub fn evidence_projection_with_grain(
        projection_id: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence(projection({}), grain: {})",
            coveql_identifier(projection_id.as_ref()),
            evidence_grain_name(grain)
        ))
    }

    pub fn where_predicate(mut self, predicate: impl AsRef<str>) -> Self {
        self.methods.push(format!("where({})", predicate.as_ref()));
        self
    }

    pub fn select<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let items = items
            .into_iter()
            .map(|item| item.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.methods.push(format!("select({})", items.join(", ")));
        self
    }

    pub fn lookup(
        self,
        table_name: impl AsRef<str>,
        alias: impl AsRef<str>,
        on: impl AsRef<str>,
    ) -> Self {
        self.lookup_with_options(
            table_name,
            alias,
            on,
            TableLookupCardinality::One,
            TableLookupUnmatchedPolicy::Nulls,
            TableLookupDuplicatePolicy::Reject,
            false,
        )
    }

    pub fn lookup_many(
        self,
        table_name: impl AsRef<str>,
        alias: impl AsRef<str>,
        on: impl AsRef<str>,
    ) -> Self {
        self.lookup_with_options(
            table_name,
            alias,
            on,
            TableLookupCardinality::Many,
            TableLookupUnmatchedPolicy::Nulls,
            TableLookupDuplicatePolicy::EmitAll,
            false,
        )
    }

    pub fn lookup_required(
        self,
        table_name: impl AsRef<str>,
        alias: impl AsRef<str>,
        on: impl AsRef<str>,
    ) -> Self {
        self.lookup_with_options(
            table_name,
            alias,
            on,
            TableLookupCardinality::One,
            TableLookupUnmatchedPolicy::Reject,
            TableLookupDuplicatePolicy::Reject,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lookup_with_options(
        mut self,
        table_name: impl AsRef<str>,
        alias: impl AsRef<str>,
        on: impl AsRef<str>,
        cardinality: TableLookupCardinality,
        unmatched: TableLookupUnmatchedPolicy,
        duplicate: TableLookupDuplicatePolicy,
        nulls_match: bool,
    ) -> Self {
        self.methods.push(format!(
            "lookup(table({}) as {}, on: {}, cardinality: {}, unmatched: {}, duplicate: {}, nulls_match: {})",
            coveql_identifier(table_name.as_ref()),
            coveql_identifier(alias.as_ref()),
            on.as_ref(),
            table_lookup_cardinality_name(cardinality),
            table_lookup_unmatched_name(unmatched),
            table_lookup_duplicate_name(duplicate),
            nulls_match
        ));
        self
    }

    pub fn traverse(mut self, relationship_expr: impl AsRef<str>) -> Self {
        self.methods
            .push(format!("traverse({})", relationship_expr.as_ref()));
        self
    }

    pub fn traverse_to_node(
        self,
        direction: AstAssociationDirection,
        edge_label: impl AsRef<str>,
        edge_alias: impl AsRef<str>,
        target_label: impl AsRef<str>,
        target_alias: impl AsRef<str>,
    ) -> Self {
        self.traverse(coveql_relationship_expr_to_node(
            direction,
            edge_label,
            Some(edge_alias),
            target_label,
            Some(target_alias),
        ))
    }

    pub fn as_of_csn(mut self, csn: u64) -> Self {
        self.methods.push(format!("asOf(csn: {csn})"));
        self
    }

    pub fn as_of_timestamp(mut self, role: AstTimeRole, timestamp: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "asOf({}: {})",
            time_role_name(role),
            coveql_string_literal(timestamp.as_ref())
        ));
        self
    }

    pub fn as_of_time(mut self, timestamp: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "asOf(time: {})",
            coveql_string_literal(timestamp.as_ref())
        ));
        self
    }

    pub fn as_of_commit_time(self, timestamp: impl AsRef<str>) -> Self {
        self.as_of_timestamp(AstTimeRole::CommitTime, timestamp)
    }

    pub fn as_of_bound(mut self, bound: AstTimeBound) -> Self {
        self.methods.push(format!("asOf({})", time_bound(bound)));
        self
    }

    pub fn branch_identifier(mut self, branch: impl AsRef<str>) -> Self {
        self.methods
            .push(format!("branch({})", coveql_identifier(branch.as_ref())));
        self
    }

    pub fn branch_string(mut self, branch: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "branch({})",
            coveql_string_literal(branch.as_ref())
        ));
        self
    }

    pub fn branch_key(mut self, branch_key: u64) -> Self {
        self.methods.push(format!("branch({branch_key})"));
        self
    }

    pub fn branch_reject_ambiguous(self) -> Self {
        self.branch_identifier("reject_ambiguous")
    }

    pub fn include_tombstones(mut self, include: bool) -> Self {
        self.methods.push(format!("includeTombstones({include})"));
        self
    }

    pub fn include_tombstones_enabled(self) -> Self {
        self.include_tombstones(true)
    }

    pub fn include_tombstones_disabled(self) -> Self {
        self.include_tombstones(false)
    }

    pub fn history(mut self, mode: AstHistoryMode) -> Self {
        self.methods
            .push(format!("history(mode: {})", history_mode_name(mode)));
        self
    }

    pub fn history_records(self) -> Self {
        self.history(AstHistoryMode::Records)
    }

    pub fn history_states(self) -> Self {
        self.history(AstHistoryMode::States)
    }

    pub fn history_records_and_states(self) -> Self {
        self.history(AstHistoryMode::RecordsAndStates)
    }

    pub fn history_default(mut self) -> Self {
        self.methods.push("history()".into());
        self
    }

    pub fn changes_csn(mut self, from: u64, to: u64, mode: AstChangeMode) -> Self {
        self.methods.push(format!(
            "changes(from: {from}, to: {to}, mode: {})",
            change_mode_name(mode)
        ));
        self
    }

    pub fn changes_csn_default(mut self, from: u64, to: u64) -> Self {
        self.methods
            .push(format!("changes(from: {from}, to: {to})"));
        self
    }

    pub fn changes_csn_records(self, from: u64, to: u64) -> Self {
        self.changes_csn(from, to, AstChangeMode::Records)
    }

    pub fn changes_csn_state_transitions(self, from: u64, to: u64) -> Self {
        self.changes_csn(from, to, AstChangeMode::StateTransitions)
    }

    pub fn changes_csn_property_diffs(self, from: u64, to: u64) -> Self {
        self.changes_csn(from, to, AstChangeMode::PropertyDiffs)
    }

    pub fn changes_csn_final_rows(self, from: u64, to: u64) -> Self {
        self.changes_csn(from, to, AstChangeMode::FinalRows)
    }

    pub fn changes_csn_final_objects(self, from: u64, to: u64) -> Self {
        self.changes_csn_final_rows(from, to)
    }

    pub fn changes_timestamp(
        mut self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
        mode: AstChangeMode,
    ) -> Self {
        self.methods.push(format!(
            "changes({}: {}, {}: {}, mode: {})",
            time_role_name(role),
            coveql_string_literal(from.as_ref()),
            time_role_name(role),
            coveql_string_literal(to.as_ref()),
            change_mode_name(mode)
        ));
        self
    }

    pub fn changes_timestamp_default(
        mut self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.methods.push(format!(
            "changes({}: {}, {}: {})",
            time_role_name(role),
            coveql_string_literal(from.as_ref()),
            time_role_name(role),
            coveql_string_literal(to.as_ref())
        ));
        self
    }

    pub fn changes_time(
        mut self,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
        mode: AstChangeMode,
    ) -> Self {
        self.methods.push(format!(
            "changes(time: {}, time: {}, mode: {})",
            coveql_string_literal(from.as_ref()),
            coveql_string_literal(to.as_ref()),
            change_mode_name(mode)
        ));
        self
    }

    pub fn changes_time_default(mut self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "changes(time: {}, time: {})",
            coveql_string_literal(from.as_ref()),
            coveql_string_literal(to.as_ref())
        ));
        self
    }

    pub fn changes_time_records(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.changes_time(from, to, AstChangeMode::Records)
    }

    pub fn changes_time_state_transitions(
        self,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_time(from, to, AstChangeMode::StateTransitions)
    }

    pub fn changes_time_property_diffs(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.changes_time(from, to, AstChangeMode::PropertyDiffs)
    }

    pub fn changes_time_final_rows(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.changes_time(from, to, AstChangeMode::FinalRows)
    }

    pub fn changes_time_final_objects(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.changes_time_final_rows(from, to)
    }

    pub fn changes_timestamp_records(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp(role, from, to, AstChangeMode::Records)
    }

    pub fn changes_timestamp_state_transitions(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp(role, from, to, AstChangeMode::StateTransitions)
    }

    pub fn changes_timestamp_property_diffs(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp(role, from, to, AstChangeMode::PropertyDiffs)
    }

    pub fn changes_timestamp_final_rows(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp(role, from, to, AstChangeMode::FinalRows)
    }

    pub fn changes_timestamp_final_objects(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp_final_rows(role, from, to)
    }

    pub fn changes_bounds(
        mut self,
        from: AstChangeBound,
        to: AstChangeBound,
        mode: AstChangeMode,
    ) -> Self {
        self.methods.push(format!(
            "changes({}, {}, mode: {})",
            change_bound(from),
            change_bound(to),
            change_mode_name(mode)
        ));
        self
    }

    pub fn changes_bounds_records(self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.changes_bounds(from, to, AstChangeMode::Records)
    }

    pub fn changes_bounds_state_transitions(
        self,
        from: AstChangeBound,
        to: AstChangeBound,
    ) -> Self {
        self.changes_bounds(from, to, AstChangeMode::StateTransitions)
    }

    pub fn changes_bounds_property_diffs(self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.changes_bounds(from, to, AstChangeMode::PropertyDiffs)
    }

    pub fn changes_bounds_final_rows(self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.changes_bounds(from, to, AstChangeMode::FinalRows)
    }

    pub fn changes_bounds_final_objects(self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.changes_bounds_final_rows(from, to)
    }

    pub fn changes_bounds_default(mut self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.methods.push(format!(
            "changes({}, {})",
            change_bound(from),
            change_bound(to)
        ));
        self
    }

    pub fn order_by_default(mut self, expr: impl AsRef<str>) -> Self {
        self.methods.push(format!("orderBy({})", expr.as_ref()));
        self
    }

    pub fn order_by(
        mut self,
        expr: impl AsRef<str>,
        direction: AstOrderDirection,
        nulls: AstNullOrdering,
    ) -> Self {
        let mut parts = vec![
            expr.as_ref().to_owned(),
            order_direction_name(direction).into(),
        ];
        if nulls != AstNullOrdering::Default {
            parts.push(null_ordering_name(nulls).into());
        }
        self.methods.push(format!("orderBy({})", parts.join(", ")));
        self
    }

    pub fn take(mut self, rows: u64) -> Self {
        self.methods.push(format!("take({rows})"));
        self
    }

    pub fn skip(mut self, rows: u64) -> Self {
        self.methods.push(format!("skip({rows})"));
        self
    }

    pub fn group_by<I, S>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let exprs = exprs
            .into_iter()
            .map(|expr| expr.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.methods.push(format!("groupBy({})", exprs.join(", ")));
        self
    }

    pub fn select_count_star_as(mut self, alias: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "select({}: count(*))",
            coveql_identifier(alias.as_ref())
        ));
        self
    }

    pub fn select_star_aggregate_as(
        mut self,
        alias: impl AsRef<str>,
        name: AstAggregateName,
    ) -> Self {
        self.methods.push(format!(
            "select({}: {}(*))",
            coveql_identifier(alias.as_ref()),
            aggregate_function_name(name)
        ));
        self
    }

    pub fn select_aggregate_as(
        mut self,
        alias: impl AsRef<str>,
        name: AstAggregateName,
        arg: impl AsRef<str>,
    ) -> Self {
        self.methods.push(format!(
            "select({}: {}({}))",
            coveql_identifier(alias.as_ref()),
            aggregate_function_name(name),
            arg.as_ref()
        ));
        self
    }

    pub fn group_by_count_star_as<I, S>(mut self, exprs: I, alias: impl AsRef<str>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let exprs = exprs
            .into_iter()
            .map(|expr| expr.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.methods.push(format!("groupBy({})", exprs.join(", ")));
        let mut select_items = exprs;
        select_items.push(format!("{}: count(*)", coveql_identifier(alias.as_ref())));
        self.methods
            .push(format!("select({})", select_items.join(", ")));
        self
    }

    pub fn group_by_star_aggregate_as<I, S>(
        mut self,
        exprs: I,
        alias: impl AsRef<str>,
        name: AstAggregateName,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let exprs = exprs
            .into_iter()
            .map(|expr| expr.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.methods.push(format!("groupBy({})", exprs.join(", ")));
        let mut select_items = exprs;
        select_items.push(format!(
            "{}: {}(*)",
            coveql_identifier(alias.as_ref()),
            aggregate_function_name(name)
        ));
        self.methods
            .push(format!("select({})", select_items.join(", ")));
        self
    }

    pub fn group_by_aggregate_as<I, S>(
        mut self,
        exprs: I,
        alias: impl AsRef<str>,
        name: AstAggregateName,
        arg: impl AsRef<str>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let exprs = exprs
            .into_iter()
            .map(|expr| expr.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.methods.push(format!("groupBy({})", exprs.join(", ")));
        let mut select_items = exprs;
        select_items.push(format!(
            "{}: {}({})",
            coveql_identifier(alias.as_ref()),
            aggregate_function_name(name),
            arg.as_ref()
        ));
        self.methods
            .push(format!("select({})", select_items.join(", ")));
        self
    }

    pub fn join_table(
        mut self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        on: impl AsRef<str>,
        kind: crate::TableJoinKind,
    ) -> Self {
        self.methods.push(format!(
            "join({}, on: {}, kind: {})",
            coveql_table_binding(table, alias),
            on.as_ref(),
            table_join_kind_name(kind)
        ));
        self
    }

    pub fn with_table(
        mut self,
        name: impl AsRef<str>,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
    ) -> Self {
        self.methods.push(format!(
            "with({}: {})",
            coveql_identifier(name.as_ref()),
            coveql_table_binding(table, alias)
        ));
        self
    }

    pub fn with_recursive_table(
        mut self,
        name: impl AsRef<str>,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        max_iterations: usize,
    ) -> Self {
        self.methods.push(format!(
            "withRecursive(name: {}, seed: {}, maxIterations: {max_iterations})",
            coveql_identifier(name.as_ref()),
            coveql_table_binding(table, alias)
        ));
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_recursive_table_step(
        mut self,
        name: impl AsRef<str>,
        seed_table: impl AsRef<str>,
        seed_alias: Option<impl AsRef<str>>,
        step_table: impl AsRef<str>,
        step_alias: Option<impl AsRef<str>>,
        key: impl AsRef<str>,
        max_iterations: usize,
    ) -> Self {
        self.methods.push(format!(
            "withRecursive(name: {}, seed: {}, step: {}, key: {}, maxIterations: {max_iterations})",
            coveql_identifier(name.as_ref()),
            coveql_table_binding(seed_table, seed_alias),
            coveql_table_binding(step_table, step_alias),
            coveql_identifier(key.as_ref())
        ));
        self
    }

    pub fn semi_join_table(
        mut self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        on: impl AsRef<str>,
    ) -> Self {
        self.methods.push(format!(
            "semiJoin({}, on: {})",
            coveql_table_binding(table, alias),
            on.as_ref()
        ));
        self
    }

    pub fn anti_join_table(
        mut self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        on: impl AsRef<str>,
    ) -> Self {
        self.methods.push(format!(
            "antiJoin({}, on: {})",
            coveql_table_binding(table, alias),
            on.as_ref()
        ));
        self
    }

    pub fn union_table(
        self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        all: bool,
    ) -> Self {
        self.set_operation_table("union", table, alias, all)
    }

    pub fn intersect_table(
        self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        all: bool,
    ) -> Self {
        self.set_operation_table("intersect", table, alias, all)
    }

    pub fn except_table(
        self,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        all: bool,
    ) -> Self {
        self.set_operation_table("except", table, alias, all)
    }

    fn set_operation_table(
        mut self,
        method: &'static str,
        table: impl AsRef<str>,
        alias: Option<impl AsRef<str>>,
        all: bool,
    ) -> Self {
        self.methods.push(format!(
            "{method}({}, all: {all})",
            coveql_table_binding(table, alias)
        ));
        self
    }

    pub fn window(
        mut self,
        partition_by: Option<impl AsRef<str>>,
        order_by: Option<impl AsRef<str>>,
    ) -> Self {
        let mut args = Vec::new();
        if let Some(partition_by) = partition_by {
            args.push(format!("partitionBy: {}", partition_by.as_ref()));
        }
        if let Some(order_by) = order_by {
            args.push(format!("orderBy: {}", order_by.as_ref()));
        }
        self.methods.push(format!("window({})", args.join(", ")));
        self
    }

    pub fn graph_algorithm(
        mut self,
        name: impl AsRef<str>,
        relationship: Option<impl AsRef<str>>,
    ) -> Self {
        let args = relationship
            .map(|relationship| relationship.as_ref().to_string())
            .unwrap_or_default();
        self.methods.push(format!("{}({})", name.as_ref(), args));
        self
    }

    pub fn graph_algorithm_with_args<I, K, V>(
        mut self,
        name: impl AsRef<str>,
        relationship: Option<impl AsRef<str>>,
        args: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut rendered = Vec::new();
        if let Some(relationship) = relationship {
            rendered.push(relationship.as_ref().to_string());
        }
        rendered.extend(args.into_iter().map(|(name, value)| {
            format!("{}: {}", coveql_identifier(name.as_ref()), value.as_ref())
        }));
        self.methods
            .push(format!("{}({})", name.as_ref(), rendered.join(", ")));
        self
    }

    pub fn reachable(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("reachable", relationship)
    }

    pub fn shortest_path(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("shortestPath", relationship)
    }

    pub fn all_paths(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("allPaths", relationship)
    }

    pub fn k_shortest_paths(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("kShortestPaths", relationship)
    }

    pub fn connected_components(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("connectedComponents", relationship)
    }

    pub fn connected_components_kind(
        self,
        relationship: Option<impl AsRef<str>>,
        kind: impl AsRef<str>,
    ) -> Self {
        self.graph_algorithm_with_args("connectedComponents", relationship, [("kind", kind)])
    }

    pub fn degree(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("degree", relationship)
    }

    pub fn degree_kind(self, relationship: Option<impl AsRef<str>>, kind: impl AsRef<str>) -> Self {
        self.graph_algorithm_with_args("degree", relationship, [("kind", kind)])
    }

    pub fn page_rank(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("pageRank", relationship)
    }

    pub fn hits(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("hits", relationship)
    }

    pub fn centrality(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("centrality", relationship)
    }

    pub fn centrality_kind(
        self,
        relationship: Option<impl AsRef<str>>,
        kind: impl AsRef<str>,
    ) -> Self {
        self.graph_algorithm_with_args("centrality", relationship, [("kind", kind)])
    }

    pub fn triangle_count(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("triangleCount", relationship)
    }

    pub fn clustering_coefficient(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("clusteringCoefficient", relationship)
    }

    pub fn community(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("community", relationship)
    }

    pub fn community_kind(
        self,
        relationship: Option<impl AsRef<str>>,
        kind: impl AsRef<str>,
    ) -> Self {
        self.graph_algorithm_with_args("community", relationship, [("kind", kind)])
    }

    pub fn spanning_tree(self, relationship: Option<impl AsRef<str>>) -> Self {
        self.graph_algorithm("spanningTree", relationship)
    }

    pub fn spanning_tree_kind(
        self,
        relationship: Option<impl AsRef<str>>,
        kind: impl AsRef<str>,
    ) -> Self {
        self.graph_algorithm_with_args("spanningTree", relationship, [("kind", kind)])
    }

    pub fn explain(mut self, mode: ExplainMode) -> Self {
        self.methods
            .push(format!("explain({})", coveql_string_literal(mode.as_str())));
        self
    }

    pub fn explain_public(self) -> Self {
        self.explain(ExplainMode::Public)
    }

    pub fn explain_developer(self) -> Self {
        self.explain(ExplainMode::Developer)
    }

    pub fn explain_proof(self) -> Self {
        self.explain(ExplainMode::Proof)
    }

    pub fn explain_coded(self) -> Self {
        self.explain(ExplainMode::Coded)
    }

    pub fn explain_ai(self) -> Self {
        self.explain(ExplainMode::Ai)
    }

    pub fn explain_forensic(self) -> Self {
        self.explain(ExplainMode::Forensic)
    }

    pub fn explain_default(mut self) -> Self {
        self.methods.push("explain()".into());
        self
    }

    pub fn to_query(&self) -> String {
        if self.methods.is_empty() {
            return self.root.clone();
        }
        format!("{}.{}", self.root, self.methods.join("."))
    }

    pub fn parse(&self, options: ParseOptions) -> Result<ParsedQuery, Vec<CoveQlDiagnostic>> {
        parse_query(&self.to_query(), options)
    }

    pub fn resolve(
        &self,
        bytes: &[u8],
        parse_options: ParseOptions,
        resolve_options: ResolveOptions,
        validation_options: ValidationOptions,
    ) -> Result<ResolvedQuery, BuildResolvedQueryError> {
        let query = self.to_query();
        parse_and_resolve_query(
            bytes,
            &query,
            parse_options,
            resolve_options,
            validation_options,
        )
    }

    pub fn plan(
        &self,
        bytes: &[u8],
        parse_options: ParseOptions,
        resolve_options: ResolveOptions,
        plan_options: PlanOptions,
        validation_options: ValidationOptions,
    ) -> Result<PlannedQuery, BuildLogicalPlanError> {
        let query = self.to_query();
        parse_resolve_and_plan_query(
            bytes,
            &query,
            parse_options,
            resolve_options,
            plan_options,
            validation_options,
        )
    }

    fn new(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            methods: Vec::new(),
        }
    }
}

pub fn coveql_identifier(value: &str) -> String {
    if is_plain_identifier(value) && !requires_identifier_quote(value) {
        return value.into();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('`');
    for ch in value.chars() {
        if matches!(ch, '`' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('`');
    out
}

pub fn coveql_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string literal cannot fail")
}

pub fn coveql_association_expr(
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    association_expression(type_name, role, role_name)
}

pub fn coveql_directed_association_expr(
    direction: AstAssociationDirection,
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    directed_association_expression(direction, type_name, role, role_name)
}

pub fn coveql_graph_edge_expr(
    edge_label: impl AsRef<str>,
    alias: Option<impl AsRef<str>>,
) -> String {
    let mut out = format!("edge({})", coveql_identifier(edge_label.as_ref()));
    if let Some(alias) = alias {
        out.push_str(&format!(" as {}", coveql_identifier(alias.as_ref())));
    }
    out
}

pub fn coveql_table_binding(table: impl AsRef<str>, alias: Option<impl AsRef<str>>) -> String {
    let mut out = format!("table({})", coveql_identifier(table.as_ref()));
    if let Some(alias) = alias {
        out.push_str(&format!(" as {}", coveql_identifier(alias.as_ref())));
    }
    out
}

pub fn coveql_relationship_expr(
    direction: AstAssociationDirection,
    edge_label: impl AsRef<str>,
    edge_alias: Option<impl AsRef<str>>,
) -> String {
    format!(
        "{}({})",
        association_direction_name(direction),
        coveql_graph_edge_expr(edge_label, edge_alias)
    )
}

pub fn coveql_relationship_expr_to_node(
    direction: AstAssociationDirection,
    edge_label: impl AsRef<str>,
    edge_alias: Option<impl AsRef<str>>,
    target_label: impl AsRef<str>,
    target_alias: Option<impl AsRef<str>>,
) -> String {
    let mut out = coveql_relationship_expr(direction, edge_label, edge_alias);
    out.push_str(&format!(
        ".to(node({})",
        coveql_identifier(target_label.as_ref())
    ));
    if let Some(target_alias) = target_alias {
        out.push_str(&format!(" as {}", coveql_identifier(target_alias.as_ref())));
    }
    out.push(')');
    out
}

fn directed_association_expression(
    direction: AstAssociationDirection,
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    format!(
        "{}({})",
        association_direction_name(direction),
        association_expression(type_name, role, role_name)
    )
}

fn association_expression(
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    let mut args = vec![coveql_identifier(type_name.as_ref())];
    if let (Some(role), Some(role_name)) = (role, role_name) {
        args.push(format!(
            "{}: {}",
            association_role_name(role),
            coveql_identifier(role_name.as_ref())
        ));
    }
    format!("association({})", args.join(", "))
}

fn table_join_kind_name(kind: crate::TableJoinKind) -> &'static str {
    match kind {
        crate::TableJoinKind::Inner => "inner",
        crate::TableJoinKind::Left => "left",
        crate::TableJoinKind::Right => "right",
        crate::TableJoinKind::Full => "full",
        crate::TableJoinKind::Semi => "semi",
        crate::TableJoinKind::Anti => "anti",
    }
}

fn time_bound(bound: AstTimeBound) -> String {
    match bound {
        AstTimeBound::Csn(csn) => format!("csn: {csn}"),
        AstTimeBound::Timestamp { role, timestamp } => {
            format!(
                "{}: {}",
                time_role_name(role),
                coveql_string_literal(&timestamp)
            )
        }
    }
}

fn change_bound(bound: AstChangeBound) -> String {
    match bound {
        AstChangeBound::Csn(csn) => format!("csn: {csn}"),
        AstChangeBound::Timestamp { role, timestamp } => {
            format!(
                "{}: {}",
                time_role_name(role),
                coveql_string_literal(&timestamp)
            )
        }
    }
}

fn is_plain_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn requires_identifier_quote(value: &str) -> bool {
    matches!(
        value,
        "association"
            | "object"
            | "evidence"
            | "table"
            | "node"
            | "edge"
            | "path"
            | "projection"
            | "where"
            | "select"
            | "asOf"
            | "branch"
            | "includeTombstones"
            | "history"
            | "changes"
            | "orderBy"
            | "take"
            | "skip"
            | "groupBy"
            | "explain"
            | "true"
            | "false"
            | "null"
            | "if"
            | "exists"
            | "in"
            | "out"
            | "either"
            | "as"
            | "lookup"
            | "traverse"
            | "self"
            | "grain"
    )
}

fn association_role_name(role: AstAssociationRole) -> &'static str {
    match role {
        AstAssociationRole::Role => "role",
        AstAssociationRole::From => "from",
        AstAssociationRole::To => "to",
    }
}

fn association_direction_name(direction: AstAssociationDirection) -> &'static str {
    match direction {
        AstAssociationDirection::In => "in",
        AstAssociationDirection::Out => "out",
        AstAssociationDirection::Either => "either",
    }
}

fn evidence_grain_name(grain: AstEvidenceGrain) -> &'static str {
    match grain {
        AstEvidenceGrain::Object => "object",
        AstEvidenceGrain::Property => "property",
        AstEvidenceGrain::Association => "association",
        AstEvidenceGrain::Row => "row",
        AstEvidenceGrain::Column => "column",
        AstEvidenceGrain::Projection => "projection",
        AstEvidenceGrain::Node => "node",
        AstEvidenceGrain::Edge => "edge",
        AstEvidenceGrain::Path => "path",
        AstEvidenceGrain::Source => "source",
    }
}

fn table_lookup_cardinality_name(cardinality: TableLookupCardinality) -> &'static str {
    match cardinality {
        TableLookupCardinality::One => "one",
        TableLookupCardinality::Many => "many",
    }
}

fn table_lookup_unmatched_name(unmatched: TableLookupUnmatchedPolicy) -> &'static str {
    match unmatched {
        TableLookupUnmatchedPolicy::Nulls => "nulls",
        TableLookupUnmatchedPolicy::Reject => "reject",
    }
}

fn table_lookup_duplicate_name(duplicate: TableLookupDuplicatePolicy) -> &'static str {
    match duplicate {
        TableLookupDuplicatePolicy::Reject => "reject",
        TableLookupDuplicatePolicy::EmitAll => "many",
    }
}

fn history_mode_name(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "records",
        AstHistoryMode::States => "states",
        AstHistoryMode::RecordsAndStates => "records_and_states",
    }
}

fn change_mode_name(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "records",
        AstChangeMode::StateTransitions => "state_transitions",
        AstChangeMode::PropertyDiffs => "property_diffs",
        AstChangeMode::FinalRows => "final_rows",
    }
}

fn time_role_name(role: AstTimeRole) -> &'static str {
    match role {
        AstTimeRole::Time | AstTimeRole::CommitTime => "commit_time",
        AstTimeRole::ValidTime => "valid_time",
        AstTimeRole::ObservedTime => "observed_time",
        AstTimeRole::SourceEventTime => "source_event_time",
        AstTimeRole::AssociationValidTime => "association_valid_time",
    }
}

fn order_direction_name(direction: AstOrderDirection) -> &'static str {
    match direction {
        AstOrderDirection::Asc => "asc",
        AstOrderDirection::Desc => "desc",
    }
}

fn null_ordering_name(nulls: AstNullOrdering) -> &'static str {
    match nulls {
        AstNullOrdering::Default => "default",
        AstNullOrdering::NullsFirst => "nulls_first",
        AstNullOrdering::NullsLast => "nulls_last",
    }
}

fn aggregate_function_name(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count => "count",
        AstAggregateName::Exists => "exists",
        AstAggregateName::DistinctCount => "distinct_count",
        AstAggregateName::Sum => "sum",
        AstAggregateName::Avg => "avg",
        AstAggregateName::Min => "min",
        AstAggregateName::Max => "max",
    }
}
