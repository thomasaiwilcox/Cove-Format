use crate::{
    parse_and_resolve_query, parse_query, parse_resolve_and_plan_query, AstAggregateName,
    AstAssociationDirection, AstAssociationRole, AstChangeBound, AstChangeMode, AstEvidenceGrain,
    AstHistoryMode, AstNullOrdering, AstOrderDirection, AstTimeBound, AstTimeRole,
    BuildLogicalPlanError, BuildResolvedQueryError, ExplainMode, OqlDiagnostic, ParseOptions,
    ParsedQuery, PlanOptions, PlannedQuery, ResolveOptions, ResolvedQuery,
};
use cove_core::reader::ValidationOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveOqlQueryBuilder {
    root: String,
    methods: Vec<String>,
}

impl CoveOqlQueryBuilder {
    pub fn object(type_name: impl AsRef<str>) -> Self {
        Self::new(oql_identifier(type_name.as_ref()))
    }

    pub fn association(type_name: impl AsRef<str>) -> Self {
        Self::new(format!(
            "association({})",
            oql_identifier(type_name.as_ref())
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
            oql_identifier(projection_id.as_ref())
        ))
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
            oql_identifier(projection_id.as_ref())
        ))
    }

    pub fn evidence_projection_with_grain(
        projection_id: impl AsRef<str>,
        grain: AstEvidenceGrain,
    ) -> Self {
        Self::new(format!(
            "evidence(projection({}), grain: {})",
            oql_identifier(projection_id.as_ref()),
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

    pub fn as_of_csn(mut self, csn: u64) -> Self {
        self.methods.push(format!("asOf(csn: {csn})"));
        self
    }

    pub fn as_of_timestamp(mut self, role: AstTimeRole, timestamp: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "asOf({}: {})",
            time_role_name(role),
            oql_string_literal(timestamp.as_ref())
        ));
        self
    }

    pub fn as_of_time(mut self, timestamp: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "asOf(time: {})",
            oql_string_literal(timestamp.as_ref())
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
            .push(format!("branch({})", oql_identifier(branch.as_ref())));
        self
    }

    pub fn branch_string(mut self, branch: impl AsRef<str>) -> Self {
        self.methods
            .push(format!("branch({})", oql_string_literal(branch.as_ref())));
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

    pub fn changes_csn_final_objects(self, from: u64, to: u64) -> Self {
        self.changes_csn(from, to, AstChangeMode::FinalObjects)
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
            oql_string_literal(from.as_ref()),
            time_role_name(role),
            oql_string_literal(to.as_ref()),
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
            oql_string_literal(from.as_ref()),
            time_role_name(role),
            oql_string_literal(to.as_ref())
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
            oql_string_literal(from.as_ref()),
            oql_string_literal(to.as_ref()),
            change_mode_name(mode)
        ));
        self
    }

    pub fn changes_time_default(mut self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.methods.push(format!(
            "changes(time: {}, time: {})",
            oql_string_literal(from.as_ref()),
            oql_string_literal(to.as_ref())
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

    pub fn changes_time_final_objects(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Self {
        self.changes_time(from, to, AstChangeMode::FinalObjects)
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

    pub fn changes_timestamp_final_objects(
        self,
        role: AstTimeRole,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
    ) -> Self {
        self.changes_timestamp(role, from, to, AstChangeMode::FinalObjects)
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

    pub fn changes_bounds_final_objects(self, from: AstChangeBound, to: AstChangeBound) -> Self {
        self.changes_bounds(from, to, AstChangeMode::FinalObjects)
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
            oql_identifier(alias.as_ref())
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
            oql_identifier(alias.as_ref()),
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
            oql_identifier(alias.as_ref()),
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
        select_items.push(format!("{}: count(*)", oql_identifier(alias.as_ref())));
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
            oql_identifier(alias.as_ref()),
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
            oql_identifier(alias.as_ref()),
            aggregate_function_name(name),
            arg.as_ref()
        ));
        self.methods
            .push(format!("select({})", select_items.join(", ")));
        self
    }

    pub fn explain(mut self, mode: ExplainMode) -> Self {
        self.methods.push(format!(
            "explain({})",
            oql_string_literal(explain_mode_name(mode))
        ));
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

    pub fn parse(&self, options: ParseOptions) -> Result<ParsedQuery, Vec<OqlDiagnostic>> {
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

pub fn oql_identifier(value: &str) -> String {
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

pub fn oql_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string literal cannot fail")
}

pub fn oql_association_expr(
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    association_expression(type_name, role, role_name)
}

pub fn oql_directed_association_expr(
    direction: AstAssociationDirection,
    type_name: impl AsRef<str>,
    role: Option<AstAssociationRole>,
    role_name: Option<impl AsRef<str>>,
) -> String {
    directed_association_expression(direction, type_name, role, role_name)
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
    let mut args = vec![oql_identifier(type_name.as_ref())];
    if let (Some(role), Some(role_name)) = (role, role_name) {
        args.push(format!(
            "{}: {}",
            association_role_name(role),
            oql_identifier(role_name.as_ref())
        ));
    }
    format!("association({})", args.join(", "))
}

fn time_bound(bound: AstTimeBound) -> String {
    match bound {
        AstTimeBound::Csn(csn) => format!("csn: {csn}"),
        AstTimeBound::Timestamp { role, timestamp } => {
            format!(
                "{}: {}",
                time_role_name(role),
                oql_string_literal(&timestamp)
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
                oql_string_literal(&timestamp)
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
            | "evidence"
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
        AstEvidenceGrain::Source => "source",
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
        AstChangeMode::FinalObjects => "final_objects",
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

fn explain_mode_name(mode: ExplainMode) -> &'static str {
    match mode {
        ExplainMode::Public => "public",
        ExplainMode::Developer => "developer",
        ExplainMode::Proof => "proof",
        ExplainMode::Coded => "coded",
        ExplainMode::Forensic => "forensic",
    }
}
