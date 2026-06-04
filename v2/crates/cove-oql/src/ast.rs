use crate::{
    BranchContext, CoveOqlOutputMode, FallbackPolicy, OperationContext, ResourceBudgetPolicy,
    ResourceUseEstimate, SecurityContext, TemporalContext, TombstoneContext,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, fmt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn merge(self, other: SourceSpan) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spanned<T> {
    pub node: T,
    #[serde(skip)]
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: SourceSpan) -> Self {
        Self { node, span }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseOptions {
    pub allow_implicit_language_version: bool,
    pub required_language_version: Option<String>,
    pub resource_budget: ResourceBudgetPolicy,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            allow_implicit_language_version: true,
            required_language_version: None,
            resource_budget: ResourceBudgetPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveOptions {
    pub security: SecurityContext,
    pub fallback_policy: FallbackPolicy,
    pub resource_budget: ResourceBudgetPolicy,
    pub output_mode: Option<CoveOqlOutputMode>,
    pub cache_hook: Option<crate::CacheHookRef>,
    pub execution_code_mapping_requested: bool,
    pub branch_aliases: BTreeMap<String, u64>,
    pub ambiguous_branch_aliases: BTreeMap<String, Vec<u64>>,
    pub temporal_role_bindings: BTreeMap<crate::TemporalRole, String>,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            security: SecurityContext::default(),
            fallback_policy: FallbackPolicy::default(),
            resource_budget: ResourceBudgetPolicy::default(),
            output_mode: None,
            cache_hook: None,
            execution_code_mapping_requested: false,
            branch_aliases: BTreeMap::new(),
            ambiguous_branch_aliases: BTreeMap::new(),
            temporal_role_bindings: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub language_version: String,
    pub root: Spanned<AstRoot>,
    pub methods: Vec<Spanned<AstMethod>>,
    #[serde(skip)]
    pub span: SourceSpan,
    pub resource_use: ResourceUseEstimate,
    pub query_text_fingerprint: String,
    pub parsed_ast_fingerprint: String,
}

impl ParsedQuery {
    pub fn to_canonical_query(&self) -> String {
        let mut query = format!(
            "# cove-oql:{}\n{}",
            self.language_version,
            render_root(&self.root.node)
        );
        for method in &self.methods {
            query.push('.');
            query.push_str(&render_method(&method.node));
        }
        query
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstIdentifier {
    pub name: String,
    #[serde(skip)]
    pub quoted: bool,
}

impl AstIdentifier {
    pub fn new(name: impl Into<String>, quoted: bool) -> Self {
        Self {
            name: name.into(),
            quoted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstRoot {
    Object(AstIdentifier),
    Association(AstAssociationExpr),
    Evidence(AstEvidenceExpr),
    Projection(AstIdentifier),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstMethod {
    Where(Spanned<AstPredicate>),
    Select(Vec<AstSelectItem>),
    AsOf(AstTimeBound),
    Branch(AstBranchSelector),
    IncludeTombstones(bool),
    History(AstHistoryMode),
    Changes {
        from: AstChangeBound,
        to: AstChangeBound,
        mode: AstChangeMode,
    },
    OrderBy(AstOrderClause),
    Take(u64),
    Skip(u64),
    GroupBy(Vec<Spanned<AstExpr>>),
    Explain(crate::ExplainMode),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstSelectItem {
    pub alias: Option<AstIdentifier>,
    pub expr: Spanned<AstExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstOrderClause {
    pub expr: Spanned<AstExpr>,
    pub direction: AstOrderDirection,
    pub nulls: AstNullOrdering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstOrderDirection {
    Asc,
    Desc,
}

impl Default for AstOrderDirection {
    fn default() -> Self {
        Self::Asc
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstNullOrdering {
    Default,
    NullsFirst,
    NullsLast,
}

impl Default for AstNullOrdering {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstPredicate {
    Compare {
        left: Spanned<AstExpr>,
        op: AstCompareOp,
        right: Spanned<AstExpr>,
    },
    InList {
        expr: Spanned<AstExpr>,
        values: Vec<Spanned<AstLiteral>>,
    },
    NullCheck {
        expr: Spanned<AstExpr>,
        negated: bool,
    },
    Exists(Spanned<AstExpr>),
    BoolExpr(Spanned<AstExpr>),
    Not(Box<Spanned<AstPredicate>>),
    And(Vec<Spanned<AstPredicate>>),
    Or(Vec<Spanned<AstPredicate>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstCompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstExpr {
    Path(AstPath),
    Literal(AstLiteral),
    FunctionCall {
        name: AstIdentifier,
        args: Vec<Spanned<AstExpr>>,
    },
    AggregateCall {
        name: AstAggregateName,
        arg: Option<Box<Spanned<AstExpr>>>,
        star: bool,
    },
    Association(AstAssociationExpr),
    Evidence(AstEvidenceExpr),
    Conditional {
        predicate: Box<Spanned<AstPredicate>>,
        then_expr: Box<Spanned<AstExpr>>,
        else_expr: Box<Spanned<AstExpr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstPath {
    pub parts: Vec<AstIdentifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstLiteral {
    Null,
    Boolean(bool),
    String(String),
    Integer(String),
    Decimal(String),
    Timestamp(String),
    Uuid(String),
    Binary(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstAggregateName {
    Count,
    Min,
    Max,
    Sum,
    Avg,
    Exists,
    DistinctCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstAssociationExpr {
    pub type_name: AstIdentifier,
    pub direction: Option<AstAssociationDirection>,
    pub role: Option<AstAssociationRole>,
    pub role_name: Option<AstIdentifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstAssociationDirection {
    In,
    Out,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstAssociationRole {
    Role,
    From,
    To,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstEvidenceExpr {
    pub target: Option<AstEvidenceTarget>,
    pub grain: Option<AstEvidenceGrain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstEvidenceTarget {
    SelfTarget,
    Path(AstPath),
    Association(Box<AstAssociationExpr>),
    Projection(AstIdentifier),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstEvidenceGrain {
    Object,
    Property,
    Association,
    Row,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstBranchSelector {
    Identifier(AstIdentifier),
    String(String),
    UInt(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstTimeBound {
    Csn(u64),
    Timestamp {
        role: AstTimeRole,
        timestamp: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstTimeRole {
    Time,
    CommitTime,
    ValidTime,
    ObservedTime,
    SourceEventTime,
    AssociationValidTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstChangeBound {
    Csn(u64),
    Timestamp {
        role: AstTimeRole,
        timestamp: String,
    },
}

impl AstChangeBound {
    pub fn bound_kind(&self) -> &'static str {
        match self {
            AstChangeBound::Csn(_) => "csn",
            AstChangeBound::Timestamp { role, .. } => match role {
                AstTimeRole::Time | AstTimeRole::CommitTime => "commit_time",
                AstTimeRole::ValidTime => "valid_time",
                AstTimeRole::ObservedTime => "observed_time",
                AstTimeRole::SourceEventTime => "source_event_time",
                AstTimeRole::AssociationValidTime => "association_valid_time",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstHistoryMode {
    Records,
    States,
    RecordsAndStates,
}

impl Default for AstHistoryMode {
    fn default() -> Self {
        Self::States
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AstChangeMode {
    Records,
    StateTransitions,
    PropertyDiffs,
    FinalObjects,
}

impl Default for AstChangeMode {
    fn default() -> Self {
        Self::Records
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseError {
    pub diagnostic: crate::OqlDiagnostic,
    pub span: Option<SourceSpan>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.diagnostic.code, self.diagnostic.message)
    }
}

impl Error for ParseError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveError {
    pub diagnostic: crate::OqlDiagnostic,
    pub span: Option<SourceSpan>,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.diagnostic.code, self.diagnostic.message)
    }
}

impl Error for ResolveError {}

fn render_root(root: &AstRoot) -> String {
    match root {
        AstRoot::Object(identifier) => render_identifier(identifier),
        AstRoot::Association(association) => render_association_expr(association),
        AstRoot::Evidence(evidence) => render_evidence_expr(evidence),
        AstRoot::Projection(identifier) => format!("projection({})", render_identifier(identifier)),
    }
}

fn render_method(method: &AstMethod) -> String {
    match method {
        AstMethod::Where(predicate) => format!("where({})", render_predicate(predicate)),
        AstMethod::Select(items) => {
            let items = items
                .iter()
                .map(render_select_item)
                .collect::<Vec<_>>()
                .join(", ");
            format!("select({items})")
        }
        AstMethod::AsOf(bound) => format!("asOf({})", render_time_bound(bound)),
        AstMethod::Branch(selector) => format!("branch({})", render_branch_selector(selector)),
        AstMethod::IncludeTombstones(include) => format!("includeTombstones({include})"),
        AstMethod::History(mode) => format!("history(mode: {})", render_history_mode(*mode)),
        AstMethod::Changes { from, to, mode } => format!(
            "changes({}, {}, mode: {})",
            render_change_bound("from", from),
            render_change_bound("to", to),
            render_change_mode(*mode)
        ),
        AstMethod::OrderBy(order) => {
            let mut parts = vec![render_expr(&order.expr)];
            if order.direction != AstOrderDirection::default()
                || order.nulls != AstNullOrdering::default()
            {
                parts.push(render_order_direction(order.direction).into());
            }
            if order.nulls != AstNullOrdering::default() {
                parts.push(render_null_ordering(order.nulls).into());
            }
            format!("orderBy({})", parts.join(", "))
        }
        AstMethod::Take(rows) => format!("take({rows})"),
        AstMethod::Skip(rows) => format!("skip({rows})"),
        AstMethod::GroupBy(exprs) => {
            let exprs = exprs.iter().map(render_expr).collect::<Vec<_>>().join(", ");
            format!("groupBy({exprs})")
        }
        AstMethod::Explain(mode) => format!("explain({})", render_explain_mode(*mode)),
    }
}

fn render_select_item(item: &AstSelectItem) -> String {
    match &item.alias {
        Some(alias) => format!("{}: {}", render_identifier(alias), render_expr(&item.expr)),
        None => render_expr(&item.expr),
    }
}

fn render_predicate(predicate: &Spanned<AstPredicate>) -> String {
    match &predicate.node {
        AstPredicate::Compare { left, op, right } => {
            format!(
                "{} {} {}",
                render_expr(left),
                render_compare_op(*op),
                render_expr(right)
            )
        }
        AstPredicate::InList { expr, values } => {
            let values = values
                .iter()
                .map(render_literal)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} in [{values}]", render_expr(expr))
        }
        AstPredicate::NullCheck { expr, negated } => {
            let name = if *negated { "isNotNull" } else { "isNull" };
            format!("{}.{}()", render_expr(expr), name)
        }
        AstPredicate::Exists(expr) => format!("exists({})", render_expr(expr)),
        AstPredicate::BoolExpr(expr) => render_expr(expr),
        AstPredicate::Not(inner) => format!("!({})", render_predicate(inner)),
        AstPredicate::And(parts) => parts
            .iter()
            .map(|part| format!("({})", render_predicate(part)))
            .collect::<Vec<_>>()
            .join(" && "),
        AstPredicate::Or(parts) => parts
            .iter()
            .map(|part| format!("({})", render_predicate(part)))
            .collect::<Vec<_>>()
            .join(" || "),
    }
}

fn render_expr(expr: &Spanned<AstExpr>) -> String {
    match &expr.node {
        AstExpr::Path(path) => render_path(path),
        AstExpr::Literal(literal) => render_literal_node(literal),
        AstExpr::FunctionCall { name, args } => {
            let args = args.iter().map(render_expr).collect::<Vec<_>>().join(", ");
            format!("{}({args})", render_identifier(name))
        }
        AstExpr::AggregateCall { name, arg, star } => {
            let arg = if *star {
                "*".into()
            } else {
                arg.as_deref().map(render_expr).unwrap_or_default()
            };
            format!("{}({arg})", render_aggregate_name(*name))
        }
        AstExpr::Association(association) => render_association_expr(association),
        AstExpr::Evidence(evidence) => render_evidence_expr(evidence),
        AstExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
        } => format!(
            "if({}, {}, {})",
            render_predicate(predicate),
            render_expr(then_expr),
            render_expr(else_expr)
        ),
    }
}

fn render_literal(literal: &Spanned<AstLiteral>) -> String {
    render_literal_node(&literal.node)
}

fn render_literal_node(literal: &AstLiteral) -> String {
    match literal {
        AstLiteral::Null => "null".into(),
        AstLiteral::Boolean(value) => value.to_string(),
        AstLiteral::String(value) | AstLiteral::Timestamp(value) => render_string(value),
        AstLiteral::Integer(value) | AstLiteral::Decimal(value) => value.clone(),
        AstLiteral::Uuid(value) => format!("uuid{}", render_string(value)),
        AstLiteral::Binary(value) => format!("x{}", render_string(value)),
    }
}

fn render_association_expr(association: &AstAssociationExpr) -> String {
    let mut args = vec![render_identifier(&association.type_name)];
    if let (Some(role), Some(role_name)) = (association.role, association.role_name.as_ref()) {
        args.push(format!(
            "{}: {}",
            render_association_role(role),
            render_identifier(role_name)
        ));
    }
    let rendered = format!("association({})", args.join(", "));
    match association.direction {
        Some(direction) => format!("{}({rendered})", render_association_direction(direction)),
        None => rendered,
    }
}

fn render_evidence_expr(evidence: &AstEvidenceExpr) -> String {
    let mut args = Vec::new();
    if let Some(target) = &evidence.target {
        args.push(render_evidence_target(target));
    }
    if let Some(grain) = evidence.grain {
        args.push(format!("grain: {}", render_evidence_grain(grain)));
    }
    format!("evidence({})", args.join(", "))
}

fn render_evidence_target(target: &AstEvidenceTarget) -> String {
    match target {
        AstEvidenceTarget::SelfTarget => "self".into(),
        AstEvidenceTarget::Path(path) => render_path(path),
        AstEvidenceTarget::Association(association) => render_association_expr(association),
        AstEvidenceTarget::Projection(identifier) => {
            format!("projection({})", render_identifier(identifier))
        }
    }
}

fn render_path(path: &AstPath) -> String {
    path.parts
        .iter()
        .map(render_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn render_identifier(identifier: &AstIdentifier) -> String {
    if is_plain_identifier(&identifier.name) && !identifier_requires_quote(&identifier.name) {
        return identifier.name.clone();
    }
    let mut out = String::with_capacity(identifier.name.len() + 2);
    out.push('`');
    for ch in identifier.name.chars() {
        if matches!(ch, '`' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('`');
    out
}

fn render_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string literal cannot fail")
}

fn render_time_bound(bound: &AstTimeBound) -> String {
    match bound {
        AstTimeBound::Csn(csn) => format!("csn: {csn}"),
        AstTimeBound::Timestamp { role, timestamp } => {
            format!("{}: {}", render_time_role(*role), render_string(timestamp))
        }
    }
}

fn render_change_bound(label: &str, bound: &AstChangeBound) -> String {
    match bound {
        AstChangeBound::Csn(csn) => format!("{label}: {csn}"),
        AstChangeBound::Timestamp { role, timestamp } => {
            format!("{}: {}", render_time_role(*role), render_string(timestamp))
        }
    }
}

fn render_branch_selector(selector: &AstBranchSelector) -> String {
    match selector {
        AstBranchSelector::Identifier(identifier) => render_identifier(identifier),
        AstBranchSelector::String(value) => render_string(value),
        AstBranchSelector::UInt(value) => value.to_string(),
    }
}

fn render_compare_op(op: AstCompareOp) -> &'static str {
    match op {
        AstCompareOp::Eq => "==",
        AstCompareOp::Ne => "!=",
        AstCompareOp::Lt => "<",
        AstCompareOp::Le => "<=",
        AstCompareOp::Gt => ">",
        AstCompareOp::Ge => ">=",
    }
}

fn render_aggregate_name(name: AstAggregateName) -> &'static str {
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

fn render_association_direction(direction: AstAssociationDirection) -> &'static str {
    match direction {
        AstAssociationDirection::In => "in",
        AstAssociationDirection::Out => "out",
        AstAssociationDirection::Either => "either",
    }
}

fn render_association_role(role: AstAssociationRole) -> &'static str {
    match role {
        AstAssociationRole::Role => "role",
        AstAssociationRole::From => "from",
        AstAssociationRole::To => "to",
    }
}

fn render_evidence_grain(grain: AstEvidenceGrain) -> &'static str {
    match grain {
        AstEvidenceGrain::Object => "object",
        AstEvidenceGrain::Property => "property",
        AstEvidenceGrain::Association => "association",
        AstEvidenceGrain::Row => "row",
        AstEvidenceGrain::Source => "source",
    }
}

fn render_time_role(role: AstTimeRole) -> &'static str {
    match role {
        AstTimeRole::Time => "time",
        AstTimeRole::CommitTime => "commit_time",
        AstTimeRole::ValidTime => "valid_time",
        AstTimeRole::ObservedTime => "observed_time",
        AstTimeRole::SourceEventTime => "source_event_time",
        AstTimeRole::AssociationValidTime => "association_valid_time",
    }
}

fn render_history_mode(mode: AstHistoryMode) -> &'static str {
    match mode {
        AstHistoryMode::Records => "records",
        AstHistoryMode::States => "states",
        AstHistoryMode::RecordsAndStates => "records_and_states",
    }
}

fn render_change_mode(mode: AstChangeMode) -> &'static str {
    match mode {
        AstChangeMode::Records => "records",
        AstChangeMode::StateTransitions => "state_transitions",
        AstChangeMode::PropertyDiffs => "property_diffs",
        AstChangeMode::FinalObjects => "final_objects",
    }
}

fn render_order_direction(direction: AstOrderDirection) -> &'static str {
    match direction {
        AstOrderDirection::Asc => "asc",
        AstOrderDirection::Desc => "desc",
    }
}

fn render_null_ordering(nulls: AstNullOrdering) -> &'static str {
    match nulls {
        AstNullOrdering::Default => "nulls_last",
        AstNullOrdering::NullsFirst => "nulls_first",
        AstNullOrdering::NullsLast => "nulls_last",
    }
}

fn render_explain_mode(mode: crate::ExplainMode) -> &'static str {
    match mode {
        crate::ExplainMode::Public => "public",
        crate::ExplainMode::Developer => "developer",
        crate::ExplainMode::Proof => "proof",
        crate::ExplainMode::Coded => "coded",
        crate::ExplainMode::Forensic => "forensic",
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

fn identifier_requires_quote(value: &str) -> bool {
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

#[derive(Debug, Clone)]
pub struct ResolvedQuery {
    pub parsed: ParsedQuery,
    pub operation_context: OperationContext,
    pub root: ResolvedRoot,
    pub method_chain: ResolvedMethodChain,
    pub output_mode: CoveOqlOutputMode,
    pub temporal: TemporalContext,
    pub branch: BranchContext,
    pub tombstone: TombstoneContext,
    pub visibility_reference: VisibilityReference,
    pub redaction_reference: RedactionReference,
    pub diagnostic_policy: DiagnosticPolicy,
    pub resource_use: ResourceUseEstimate,
    pub resolved_query_fingerprint: String,
    pub diagnostics: Vec<crate::OqlDiagnostic>,
}

impl ResolvedQuery {
    pub fn explain_json(&self) -> Value {
        crate::explain::resolved_query_explain_json(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedRoot {
    Object(ResolvedObjectRoot),
    Association(ResolvedAssociationRoot),
    Projection(ResolvedProjectionRoot),
    Evidence(ResolvedEvidenceRoot),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedObjectRoot {
    pub object_type_id: u32,
    pub type_name: String,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAssociationRoot {
    pub object_type_id: u32,
    pub type_name: String,
    pub flags: u32,
    pub source_property_id: Option<u32>,
    pub target_property_id: Option<u32>,
    pub association_type_property_id: Option<u32>,
    pub valid_from_property_id: Option<u32>,
    pub valid_to_property_id: Option<u32>,
    pub direction: Option<AstAssociationDirection>,
    pub role: Option<String>,
    pub endpoint_role: AssociationEndpointRole,
    pub disclosure_outcome: AssociationDisclosureOutcome,
    pub object_relative: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociationEndpointRole {
    Source,
    Target,
    Either,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociationDisclosureOutcome {
    Public,
    ProtectedEndpoint,
    ProtectedExistence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjectionRoot {
    pub projection_id: String,
    pub mapping_id: String,
    pub mapping_version: String,
    pub output_table: Option<String>,
    pub row_grain: Option<String>,
    pub anchor: Option<ResolvedProjectionAnchor>,
    pub temporal_mode: Option<String>,
    pub columns: Vec<ResolvedProjectionColumn>,
    pub assertion_ids: Vec<String>,
    pub multi_value_policy: Option<String>,
    pub missing_policy: String,
    pub ordering: Vec<String>,
    pub evidence_policy: String,
    pub output_modes: Vec<String>,
    pub column_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjectionAnchor {
    pub object_type: Option<String>,
    pub association_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjectionColumn {
    pub name: String,
    pub value: String,
    pub logical_type: Option<String>,
    pub nested_shape: Option<String>,
    pub conflict_policy: String,
    pub missing_policy: String,
    pub source_property_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEvidenceRoot {
    pub target: Option<ResolvedEvidenceTarget>,
    pub grain: AstEvidenceGrain,
    pub mapping_id: Option<String>,
    pub mapping_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedEvidenceTarget {
    CurrentRoot,
    ObjectType {
        object_type_id: u32,
        type_name: String,
    },
    AssociationType {
        object_type_id: u32,
        type_name: String,
    },
    Projection {
        projection_id: String,
    },
    Property {
        object_type_id: Option<u32>,
        property_id: u32,
        property_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResolvedMethodChain {
    pub where_predicate: Option<ResolvedPredicate>,
    pub select: Option<Vec<ResolvedSelectItem>>,
    pub order_by: Option<ResolvedOrderClause>,
    pub group_by: Option<Vec<ResolvedExpr>>,
    pub take: Option<u64>,
    pub skip: Option<u64>,
    pub explain: Option<crate::ExplainMode>,
    pub history: Option<AstHistoryMode>,
    pub changes: Option<ResolvedChanges>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedChanges {
    pub from: ResolvedTimeBound,
    pub to: ResolvedTimeBound,
    pub mode: AstChangeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSelectItem {
    pub alias: Option<String>,
    pub expr: ResolvedExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedOrderClause {
    pub expr: ResolvedExpr,
    pub direction: AstOrderDirection,
    pub nulls: AstNullOrdering,
    pub uses_default_ordering: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedPredicate {
    Compare {
        left: ResolvedExpr,
        op: AstCompareOp,
        right: ResolvedExpr,
    },
    InList {
        expr: ResolvedExpr,
        values: Vec<ResolvedLiteral>,
    },
    NullCheck {
        expr: ResolvedExpr,
        negated: bool,
    },
    Exists(ResolvedExpr),
    BoolExpr(ResolvedExpr),
    Not(Box<ResolvedPredicate>),
    And(Vec<ResolvedPredicate>),
    Or(Vec<ResolvedPredicate>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedExpr {
    Path(ResolvedPath),
    Literal(ResolvedLiteral),
    FunctionCall {
        function_id: String,
        deterministic: bool,
        logical_type: String,
        physical_kind: String,
        contract: ResolvedFunctionContract,
        args: Vec<ResolvedExpr>,
    },
    AggregateCall {
        name: AstAggregateName,
        arg: Option<Box<ResolvedExpr>>,
        star: bool,
        logical_type: String,
        aggregate_disclosure: String,
    },
    Association(ResolvedAssociationRoot),
    Evidence(ResolvedEvidenceRoot),
    Conditional {
        predicate: Box<ResolvedPredicate>,
        then_expr: Box<ResolvedExpr>,
        else_expr: Box<ResolvedExpr>,
        logical_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedFunctionContract {
    pub function_id: String,
    pub version: String,
    pub deterministic: bool,
    pub dependency: String,
    pub execution_class: FunctionExecutionClass,
    pub unicode_or_collation_contract: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionExecutionClass {
    MaterializedOnly,
    CodedSafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPath {
    pub display_name: String,
    pub root_kind: ResolvedPathRootKind,
    pub object_type_id: Option<u32>,
    pub property_id: Option<u32>,
    pub association_type_id: Option<u32>,
    pub evidence_field_id: Option<String>,
    pub projection_id: Option<String>,
    pub projection_column: Option<String>,
    pub system_field: Option<ResolvedSystemField>,
    pub logical_type: String,
    pub physical_kind: String,
    pub collation_id: Option<u16>,
    pub nullable: bool,
    pub null_policy: String,
    pub temporal_role: Option<crate::TemporalRole>,
    pub code_domain_id: CodeDomainId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedPathRootKind {
    Object,
    Association,
    Projection,
    Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedSystemField {
    Goid,
    ObjectType,
    BranchKey,
    TimestampUs,
    Csn,
    RecordKind,
    SourceGoid,
    TargetGoid,
    AssociationType,
    ValidFrom,
    ValidTo,
    SourceId,
    SourceRowIdentity,
    RuleId,
    AssertionId,
    OutputObjectId,
    ObservedSchemaFingerprint,
    ObservedSnapshotDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CodeDomainId {
    Placeholder {
        root: String,
        object_type_id: Option<u32>,
        property_id: Option<u32>,
        projection_id: Option<String>,
        field: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedLiteralValue {
    Null,
    Boolean(bool),
    String(String),
    SignedInteger(i64),
    UnsignedInteger(u64),
    BigInteger(String),
    Decimal {
        canonical: String,
        precision: u32,
        scale: u32,
    },
    TimestampMicros {
        micros: i64,
        canonical_rfc3339: String,
    },
    Uuid {
        canonical_hex: String,
        bytes: [u8; 16],
    },
    Binary {
        canonical_hex: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLiteral {
    pub literal: AstLiteral,
    pub logical_type: String,
    pub canonical: String,
    pub typed_value: ResolvedLiteralValue,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedTimeBound {
    Csn(u64),
    TimestampMicros {
        role: crate::TemporalRole,
        binding: Option<String>,
        timestamp_micros: i64,
        canonical_rfc3339: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityReference {
    pub policy: crate::VisibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionReference {
    pub policy: crate::RedactionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticPolicy {
    pub metadata_disclosure: crate::MetadataDisclosurePolicy,
    pub explain_policy: crate::ExplainDisclosurePolicy,
}
