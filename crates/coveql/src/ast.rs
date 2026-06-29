use crate::{
    BranchContext, CoveQlOutputMode, FallbackPolicy, OperationContext, ResourceBudgetPolicy,
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
    pub output_mode: Option<CoveQlOutputMode>,
    pub cache_hook: Option<crate::CacheHookRef>,
    pub execution_code_mapping_requested: bool,
    pub table_surface_contracts: BTreeMap<String, TableSurfaceContract>,
    pub table_authorities: BTreeMap<String, TableSurfaceAuthority>,
    pub graph_traversal_contract: Option<GraphTraversalContract>,
    pub graph_traversal_contracts: BTreeMap<String, GraphTraversalContract>,
    pub graph_algorithm_contracts: BTreeMap<String, GraphAlgorithmContract>,
    pub bridge_contracts: Vec<CoveQlBridgeRegistration>,
    pub branch_aliases: BTreeMap<String, u64>,
    pub ambiguous_branch_aliases: BTreeMap<String, Vec<u64>>,
    pub temporal_role_bindings: BTreeMap<crate::TemporalRole, String>,
    pub enabled_profiles: Vec<crate::CoveQlProfileId>,
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
            table_surface_contracts: BTreeMap::new(),
            table_authorities: BTreeMap::new(),
            graph_traversal_contract: None,
            graph_traversal_contracts: BTreeMap::new(),
            graph_algorithm_contracts: BTreeMap::new(),
            bridge_contracts: Vec::new(),
            branch_aliases: BTreeMap::new(),
            ambiguous_branch_aliases: BTreeMap::new(),
            temporal_role_bindings: BTreeMap::new(),
            enabled_profiles: vec![
                crate::CoveQlProfileId::Object,
                crate::CoveQlProfileId::Table,
                crate::CoveQlProfileId::Graph,
                crate::CoveQlProfileId::Ai,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSurfaceAuthority {
    pub contract: TableSurfaceContract,
    pub execution_authority: TableExecutionAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum TableExecutionAuthority {
    DeterministicProjection {
        projection_id: String,
    },
    MaterializedRows {
        rows: Vec<TableSurfaceRow>,
    },
    RawRows {
        rows: Vec<TableSurfaceRow>,
    },
    ExternalRows {
        provider_id: String,
        rows: Vec<TableSurfaceRow>,
    },
}

pub type TableSurfaceRow = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveQlBridgeRegistration {
    pub bridge_id: String,
    pub bridge_version: String,
    pub source_profile: crate::CoveQlProfileId,
    pub target_profile: crate::CoveQlProfileId,
    pub source_grain: String,
    pub target_grain: String,
    pub identity_mapping: Vec<CoveQlBridgeIdentityMapping>,
    pub temporal_alignment: String,
    pub null_missing_policy: String,
    pub code_domain_policy: String,
    pub visibility_compatibility: String,
    pub redaction_compatibility: String,
    pub fallback_behavior: String,
    pub exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveQlBridgeIdentityMapping {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub language_version: String,
    pub directives: Vec<AstDirective>,
    pub profiles: Vec<crate::CoveQlProfileId>,
    pub root: Spanned<AstRoot>,
    pub root_alias: Option<AstIdentifier>,
    pub methods: Vec<Spanned<AstMethod>>,
    #[serde(skip)]
    pub span: SourceSpan,
    pub resource_use: ResourceUseEstimate,
    pub query_text_fingerprint: String,
    pub parsed_ast_fingerprint: String,
}

impl ParsedQuery {
    pub fn to_canonical_query(&self) -> String {
        let mut query = format!("# coveql:{}\n", self.language_version);
        if !self.profiles.is_empty() {
            query.push_str("# profiles: ");
            query.push_str(
                &self
                    .profiles
                    .iter()
                    .map(|profile| profile.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            query.push('\n');
        }
        query.push_str(&render_root(&self.root.node));
        if let Some(alias) = &self.root_alias {
            query.push_str(" as ");
            query.push_str(&render_identifier(alias));
        }
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
pub struct AstDirective {
    pub name: AstIdentifier,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstRoot {
    Object(AstIdentifier),
    Association(AstAssociationExpr),
    Evidence(AstEvidenceExpr),
    Projection(AstIdentifier),
    Table(AstIdentifier),
    Node(AstIdentifier),
    Edge(AstGraphEdgeExpr),
    Path(AstPathRootExpr),
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
    ProfileCall {
        name: AstIdentifier,
        args: Vec<AstProfileArgument>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstRootBinding {
    pub root: Box<Spanned<AstRoot>>,
    pub alias: Option<AstIdentifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstProfileArgument {
    pub name: Option<AstIdentifier>,
    pub value: AstProfileArgumentValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AstProfileArgumentValue {
    Expr(Spanned<AstExpr>),
    Predicate(Spanned<AstPredicate>),
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
    Exists {
        target: Spanned<AstExpr>,
        args: Vec<AstProfileArgument>,
    },
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
    Relationship(AstRelationshipExpr),
    RootBinding(Box<AstRootBinding>),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstGraphEdgeExpr {
    pub type_name: AstIdentifier,
    pub role: Option<AstAssociationRole>,
    pub role_name: Option<AstIdentifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstRelationshipExpr {
    pub direction: AstAssociationDirection,
    pub edge: AstGraphEdgeExpr,
    pub edge_alias: Option<AstIdentifier>,
    pub target: Option<Box<AstRootBinding>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstPathRootExpr {
    pub start: AstRootBinding,
    pub relationships: Vec<AstRelationshipExpr>,
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
    RootBinding(Box<AstRootBinding>),
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
    Column,
    Projection,
    Node,
    Edge,
    Path,
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
    FinalRows,
}

impl Default for AstChangeMode {
    fn default() -> Self {
        Self::Records
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseError {
    pub diagnostic: crate::CoveQlDiagnostic,
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
    pub diagnostic: crate::CoveQlDiagnostic,
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
        AstRoot::Object(identifier) => format!("object({})", render_identifier(identifier)),
        AstRoot::Association(association) => render_association_expr(association),
        AstRoot::Evidence(evidence) => render_evidence_expr(evidence),
        AstRoot::Projection(identifier) => format!("projection({})", render_identifier(identifier)),
        AstRoot::Table(identifier) => format!("table({})", render_identifier(identifier)),
        AstRoot::Node(identifier) => format!("node({})", render_identifier(identifier)),
        AstRoot::Edge(edge) => render_graph_edge_expr(edge),
        AstRoot::Path(path) => render_path_root_expr(path),
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
        AstMethod::ProfileCall { name, args } => {
            let args = args
                .iter()
                .map(render_profile_argument)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", render_identifier(name))
        }
    }
}

fn render_root_binding(binding: &AstRootBinding) -> String {
    let mut out = render_root(&binding.root.node);
    if let Some(alias) = &binding.alias {
        out.push_str(" as ");
        out.push_str(&render_identifier(alias));
    }
    out
}

fn render_profile_argument(arg: &AstProfileArgument) -> String {
    let value = match &arg.value {
        AstProfileArgumentValue::Expr(expr) => render_expr(expr),
        AstProfileArgumentValue::Predicate(predicate) => render_predicate(predicate),
    };
    match &arg.name {
        Some(name) => format!("{}: {value}", render_identifier(name)),
        None => value,
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
        AstPredicate::Exists { target, args } => {
            let mut parts = vec![render_expr(target)];
            parts.extend(args.iter().map(render_profile_argument));
            format!("exists({})", parts.join(", "))
        }
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
        AstExpr::Relationship(relationship) => render_relationship_expr(relationship),
        AstExpr::RootBinding(binding) => render_root_binding(binding),
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

fn render_graph_edge_expr(edge: &AstGraphEdgeExpr) -> String {
    let mut args = vec![render_identifier(&edge.type_name)];
    if let (Some(role), Some(role_name)) = (edge.role, edge.role_name.as_ref()) {
        args.push(format!(
            "{}: {}",
            render_association_role(role),
            render_identifier(role_name)
        ));
    }
    format!("edge({})", args.join(", "))
}

fn render_relationship_expr(relationship: &AstRelationshipExpr) -> String {
    let mut inner = render_graph_edge_expr(&relationship.edge);
    if let Some(alias) = &relationship.edge_alias {
        inner.push_str(" as ");
        inner.push_str(&render_identifier(alias));
    }
    let mut out = format!(
        "{}({inner})",
        render_association_direction(relationship.direction)
    );
    if let Some(target) = &relationship.target {
        out.push_str(".to(");
        out.push_str(&render_root_binding(target));
        out.push(')');
    }
    out
}

fn render_path_root_expr(path: &AstPathRootExpr) -> String {
    let mut inner = render_root_binding(&path.start);
    for relationship in &path.relationships {
        inner.push('.');
        inner.push_str(&render_relationship_expr(relationship));
    }
    format!("path({inner})")
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
        AstEvidenceTarget::RootBinding(binding) => render_root_binding(binding),
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
        AstEvidenceGrain::Column => "column",
        AstEvidenceGrain::Projection => "projection",
        AstEvidenceGrain::Node => "node",
        AstEvidenceGrain::Edge => "edge",
        AstEvidenceGrain::Path => "path",
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
        AstChangeMode::FinalRows => "final_rows",
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
        crate::ExplainMode::Ai => "ai",
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

#[derive(Debug, Clone)]
pub struct ResolvedQuery {
    pub parsed: ParsedQuery,
    pub operation_context: OperationContext,
    pub root: ResolvedRoot,
    pub method_chain: ResolvedMethodChain,
    pub output_mode: CoveQlOutputMode,
    pub temporal: TemporalContext,
    pub branch: BranchContext,
    pub tombstone: TombstoneContext,
    pub visibility_reference: VisibilityReference,
    pub redaction_reference: RedactionReference,
    pub diagnostic_policy: DiagnosticPolicy,
    pub resource_use: ResourceUseEstimate,
    pub resolved_query_fingerprint: String,
    pub diagnostics: Vec<crate::CoveQlDiagnostic>,
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
    Node(ResolvedGraphNodeRoot),
    Edge(ResolvedGraphEdgeRoot),
    Projection(ResolvedProjectionRoot),
    Table(ResolvedTableRoot),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_node_object_type_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_node_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraphNodeRoot {
    pub label: String,
    pub binding_name: Option<String>,
    pub object: ResolvedObjectRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraphEdgeRoot {
    pub label: String,
    pub binding_name: Option<String>,
    pub association: ResolvedAssociationRoot,
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
pub struct ResolvedTableRoot {
    pub table_name: String,
    pub binding_name: Option<String>,
    pub table_id: String,
    pub authority_kind: TableSurfaceAuthorityKind,
    pub row_grain: String,
    pub row_identity: Vec<String>,
    pub canonical_order: Vec<String>,
    pub temporal_authority: TableTemporalAuthority,
    pub evidence_capabilities: Vec<AstEvidenceGrain>,
    pub table_surface_contract: TableSurfaceContract,
    pub execution_authority: TableExecutionAuthority,
    pub projection: ResolvedProjectionRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableSurfaceAuthorityKind {
    DeterministicProjection,
    MaterializedTable,
    RawTable,
    ExternalRegisteredTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableTemporalAuthority {
    RecomputableProjectionAtTemporalCut,
    MaterializedSnapshotOnly,
    StaticTableSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSurfaceContract {
    pub table_id: String,
    pub table_name: String,
    pub contract_version: String,
    pub authority_kind: TableSurfaceAuthorityKind,
    pub authority_fingerprint: String,
    pub schema_fingerprint: String,
    pub logical_column_map: Vec<TableSurfaceColumnContract>,
    pub row_grain: String,
    pub row_identity: Vec<String>,
    pub canonical_order: Vec<String>,
    pub visibility_authority: String,
    pub redaction_authority: String,
    pub temporal_authority: TableTemporalAuthority,
    pub evidence_capabilities: Vec<AstEvidenceGrain>,
    pub null_missing_nan_policy: String,
    pub collation_policy: String,
    pub code_domain_contexts: Vec<String>,
    pub code_domain_bridges: Vec<String>,
    pub projection_dependency_contract_id: Option<String>,
    pub datafusion_interop_contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSurfaceColumnContract {
    pub name: String,
    pub logical_type: Option<String>,
    pub nullable: bool,
    pub source_path: Option<String>,
    pub code_domain: Option<String>,
    pub collation: Option<String>,
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
    TableRow {
        table_id: String,
        table_name: String,
        projection_id: String,
    },
    TableColumn {
        table_id: String,
        table_name: String,
        projection_id: String,
        column_name: String,
    },
    GraphNode {
        object_type_id: u32,
        type_name: String,
        label: String,
    },
    GraphEdge {
        object_type_id: u32,
        type_name: String,
        label: String,
    },
    GraphPath {
        start_object_type_id: u32,
        start_label: String,
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
    pub ctes: Vec<ResolvedCommonTableExpression>,
    pub lookups: Vec<ResolvedTableLookup>,
    pub joins: Vec<ResolvedTableJoin>,
    pub set_operations: Vec<ResolvedSetOperation>,
    pub windows: Vec<ResolvedWindowSpec>,
    pub traversals: Vec<ResolvedGraphTraversal>,
    pub graph_algorithms: Vec<ResolvedGraphAlgorithm>,
    pub ai_operations: Vec<ResolvedAiOperation>,
    pub order_by: Option<ResolvedOrderClause>,
    pub group_by: Option<Vec<ResolvedExpr>>,
    pub take: Option<u64>,
    pub skip: Option<u64>,
    pub explain: Option<crate::ExplainMode>,
    pub history: Option<AstHistoryMode>,
    pub changes: Option<ResolvedChanges>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAiOperation {
    pub operation: crate::CoveQlAiOperation,
    pub method_name: String,
    pub args: Vec<ResolvedAiArgument>,
    pub sidecar_required: bool,
    pub authority: String,
    pub policy_scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedAiArgument {
    pub name: Option<String>,
    pub value: ResolvedAiArgumentValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ResolvedAiArgumentValue {
    Expr(ResolvedExpr),
    Predicate(ResolvedPredicate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCommonTableExpression {
    pub name: String,
    pub table: ResolvedTableRoot,
    pub recursive: bool,
    pub max_iterations: Option<usize>,
    pub step_table: Option<ResolvedTableRoot>,
    pub key: Option<ResolvedExpr>,
    pub execution_authority: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTableLookup {
    pub right: ResolvedTableRoot,
    pub on: ResolvedPredicate,
    pub join_kind: TableLookupJoinKind,
    pub cardinality: TableLookupCardinality,
    pub unmatched_policy: TableLookupUnmatchedPolicy,
    pub duplicate_policy: TableLookupDuplicatePolicy,
    pub nulls_match: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTableJoin {
    pub right: ResolvedTableRoot,
    pub on: ResolvedPredicate,
    pub join_kind: TableJoinKind,
    pub cardinality: TableLookupCardinality,
    pub unmatched_policy: TableLookupUnmatchedPolicy,
    pub duplicate_policy: TableLookupDuplicatePolicy,
    pub nulls_match: bool,
    pub bridge_contract: Option<CoveQlBridgeRegistration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableJoinKind {
    Inner,
    Left,
    Right,
    Full,
    Semi,
    Anti,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSetOperation {
    pub kind: SetOperationKind,
    pub right: ResolvedTableRoot,
    pub all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOperationKind {
    Union,
    Intersect,
    Except,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedWindowSpec {
    pub partition_by: Vec<ResolvedExpr>,
    pub order_by: Option<ResolvedOrderClause>,
    pub frame: WindowFrameKind,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowFrameKind {
    Rows,
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTableExists {
    pub right: ResolvedTableRoot,
    pub on: Box<ResolvedPredicate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraphTraversal {
    pub direction: AstAssociationDirection,
    pub edge: ResolvedGraphEdgeRoot,
    pub target: Option<ResolvedGraphNodeRoot>,
    pub min_depth: u32,
    pub max_depth: u32,
    pub mode: GraphTraversalMode,
    pub distinct: GraphTraversalDistinctPolicy,
    pub contract: Option<GraphTraversalContract>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphTraversalMode {
    Walk,
    Trail,
    SimplePath,
}

impl GraphTraversalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Trail => "trail",
            Self::SimplePath => "simple_path",
        }
    }
}

impl std::str::FromStr for GraphTraversalMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "walk" => Ok(Self::Walk),
            "trail" => Ok(Self::Trail),
            "simple_path" => Ok(Self::SimplePath),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphTraversalDistinctPolicy {
    None,
    Path,
    EndNode,
}

impl GraphTraversalDistinctPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Path => "path",
            Self::EndNode => "end_node",
        }
    }
}

impl std::str::FromStr for GraphTraversalDistinctPolicy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "path" => Ok(Self::Path),
            "end_node" => Ok(Self::EndNode),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphTraversalContract {
    pub contract_version: String,
    pub allow_variable_length: bool,
    pub supported_modes: Vec<GraphTraversalMode>,
    pub supported_distinct_policies: Vec<GraphTraversalDistinctPolicy>,
    pub max_depth: u32,
    pub max_fanout_per_node: usize,
    pub max_paths: usize,
    pub max_frontier: usize,
    pub path_identity: Vec<String>,
    pub hidden_endpoint_policy: String,
    pub ordering_policy: String,
    pub execution_authority: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphAlgorithmContract {
    pub contract_version: String,
    pub allowed_algorithms: Vec<GraphAlgorithmKind>,
    pub direction_policy: String,
    pub weight_policy: String,
    pub temporal_policy: String,
    pub visibility_authority: String,
    pub redaction_authority: String,
    pub max_depth: u32,
    pub max_paths: usize,
    pub max_iterations: usize,
    pub disclosure_policy: String,
    pub ordering_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraphAlgorithm {
    pub kind: GraphAlgorithmKind,
    pub variant: String,
    pub direction: AstAssociationDirection,
    pub edge: Option<ResolvedGraphEdgeRoot>,
    pub target: Option<ResolvedGraphNodeRoot>,
    pub weight: Option<ResolvedExpr>,
    pub max_depth: Option<u32>,
    pub max_paths: Option<usize>,
    pub max_iterations: Option<usize>,
    pub tolerance: Option<String>,
    pub approx: bool,
    pub contract: GraphAlgorithmContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphAlgorithmKind {
    Reachable,
    ShortestPath,
    AllPaths,
    KShortestPaths,
    ConnectedComponents,
    Degree,
    PageRank,
    Hits,
    Centrality,
    TriangleCount,
    ClusteringCoefficient,
    Community,
    SpanningTree,
}

impl GraphAlgorithmKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reachable => "reachable",
            Self::ShortestPath => "shortestPath",
            Self::AllPaths => "allPaths",
            Self::KShortestPaths => "kShortestPaths",
            Self::ConnectedComponents => "connectedComponents",
            Self::Degree => "degree",
            Self::PageRank => "pageRank",
            Self::Hits => "hits",
            Self::Centrality => "centrality",
            Self::TriangleCount => "triangleCount",
            Self::ClusteringCoefficient => "clusteringCoefficient",
            Self::Community => "community",
            Self::SpanningTree => "spanningTree",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableLookupJoinKind {
    LeftPreserving,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableLookupCardinality {
    One,
    Many,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableLookupUnmatchedPolicy {
    Nulls,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableLookupDuplicatePolicy {
    Reject,
    EmitAll,
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
    TableExists(ResolvedTableExists),
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
    Node,
    Edge,
    Table,
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
