use crate::{
    dependencies::LogicalPlanDependencySet,
    plan_printer,
    predicate::{
        LogicalPredicateForm, LogicalPredicateKind, PredicatePlanning, RepresentationClass,
    },
    AggregateDisclosurePolicy, AstHistoryMode, AstNullOrdering, AstOrderDirection,
    BuildResolvedQueryError, CoveQlDiagnostic, CoveQlOutputMode, CoveQlSelectedOperation,
    DiagnosticSeverity, ExplainMode, MetadataDisclosurePolicy, ParseOptions, RedactionReference,
    ResolveOptions, ResolvedCommonTableExpression, ResolvedExpr, ResolvedGraphAlgorithm,
    ResolvedPath, ResolvedPredicate, ResolvedQuery, ResolvedRoot, ResolvedSetOperation,
    ResolvedTableJoin, ResolvedWindowSpec, SecurityContext, TemporalContext, TombstoneContext,
    VisibilityReference,
};
use cove_core::collation::CollationKind;
use cove_core::reader::ValidationOptions;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{error::Error, fmt};

pub type LogicalPlanFingerprint = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOptions {
    pub strict_group_by: bool,
    pub emit_index_only_candidates: bool,
    pub allow_file_code_ordering_without_contract: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            strict_group_by: true,
            emit_index_only_candidates: true,
            allow_file_code_ordering_without_contract: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanContext {
    pub root_kind: LogicalRootKind,
    pub scan_grain: ScanGrain,
    pub temporal: TemporalContext,
    pub branch: crate::BranchContext,
    pub tombstone: TombstoneContext,
    pub output_mode: CoveQlOutputMode,
    pub explain_mode: Option<ExplainMode>,
    pub visibility: VisibilityReference,
    pub redaction: RedactionReference,
    pub security_scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprContext {
    pub root_kind: LogicalRootKind,
    pub scan_grain: ScanGrain,
    pub phase: ExprPhase,
    pub grouping_active: bool,
    pub visibility_applied: bool,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalRootKind {
    Object,
    Association,
    Node,
    Edge,
    Table,
    Projection,
    Evidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanGrain {
    LatestState,
    HistoryRecord,
    HistoryState,
    HistoryRecordsAndStates,
    ChangeRecord,
    ChangeStateTransition,
    ChangePropertyDiff,
    #[serde(rename = "change_final_row", alias = "change_final_object")]
    ChangeFinalObject,
    AssociationState,
    NodeState,
    EdgeState,
    TableRow,
    ProjectionRow,
    EvidenceRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExprPhase {
    PreReconstruction,
    PostReconstruction,
    Grouping,
    Projection,
    Ordering,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LogicalNodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoveOLogicalPlan {
    pub context: PlanContext,
    pub nodes: Vec<LogicalPlanNode>,
    pub predicate_forms: Vec<LogicalPredicateForm>,
    pub decode_boundaries: Vec<String>,
    pub residual_predicates: Vec<LogicalPredicateForm>,
    pub canonical_order: Vec<String>,
    pub default_ordering_applied: bool,
    pub logical_plan_fingerprint: LogicalPlanFingerprint,
}

impl CoveOLogicalPlan {
    pub fn to_json(&self, disclosure: MetadataDisclosurePolicy) -> Value {
        plan_printer::logical_plan_json(self, disclosure)
    }

    pub fn to_text(&self, disclosure: MetadataDisclosurePolicy) -> String {
        plan_printer::logical_plan_text(self, disclosure)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalPlanNode {
    pub id: LogicalNodeId,
    pub kind: LogicalPlanNodeKind,
}

impl LogicalPlanNode {
    fn new(id: LogicalNodeId, kind: LogicalPlanNodeKind) -> Self {
        Self { id, kind }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum LogicalPlanNodeKind {
    RootScan {
        root: ResolvedRoot,
        scan_grain: ScanGrain,
    },
    TemporalScope {
        temporal: TemporalContext,
    },
    BranchScope {
        branch: crate::BranchContext,
    },
    TombstoneScope {
        tombstone: TombstoneContext,
    },
    ScanGrainSelection {
        scan_grain: ScanGrain,
    },
    PreReconstructionFilter {
        predicates: Vec<LogicalPredicateForm>,
    },
    Reconstruct {
        required: bool,
    },
    VisibilityBarrier {
        visibility: VisibilityReference,
    },
    RedactionBarrier {
        redaction: RedactionReference,
    },
    PostReconstructionFilter {
        predicates: Vec<LogicalPredicateForm>,
    },
    AssociationSemiJoin {
        predicates: Vec<LogicalPredicateForm>,
    },
    AssociationAntiJoin {
        predicates: Vec<LogicalPredicateForm>,
    },
    AssociationExpansion {
        associations: Vec<String>,
    },
    EvidenceRead {
        evidence_fields: Vec<String>,
    },
    ProjectionRead {
        projection_ids: Vec<String>,
        projection_columns: Vec<String>,
    },
    CommonTableExpression {
        ctes: Vec<ResolvedCommonTableExpression>,
    },
    TableJoin {
        joins: Vec<ResolvedTableJoin>,
    },
    TableSetOperation {
        operations: Vec<ResolvedSetOperation>,
    },
    Window {
        windows: Vec<ResolvedWindowSpec>,
    },
    GraphAlgorithm {
        algorithms: Vec<ResolvedGraphAlgorithm>,
    },
    AiOperation {
        operations: Vec<crate::ResolvedAiOperation>,
        sidecar_required: bool,
        fallback_behavior: String,
    },
    Aggregate {
        group_by: Vec<ResolvedExpr>,
        aggregates: Vec<ResolvedExpr>,
        disclosure_policy: AggregateDisclosurePolicy,
    },
    SelectProjection {
        items: Vec<SelectProjectionItem>,
    },
    Sort {
        keys: Vec<LogicalSortKey>,
        stable_tiebreaker: Vec<String>,
        defaulted: bool,
    },
    SkipTake {
        skip: Option<u64>,
        take: Option<u64>,
    },
    Explain {
        mode: ExplainMode,
    },
    IndexOnlyCandidate {
        requested: bool,
        permitted: bool,
    },
    FallbackBoundary {
        reasons: Vec<String>,
        optional_metadata_candidates: Vec<String>,
    },
    OutputBoundary {
        output_mode: CoveQlOutputMode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectProjectionItem {
    pub alias: Option<String>,
    pub expr: ResolvedExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalSortKey {
    pub expr: Option<ResolvedExpr>,
    pub field: Option<String>,
    pub direction: AstOrderDirection,
    pub nulls: AstNullOrdering,
    pub representation: RepresentationClass,
    pub collation_id: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalPlanDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub phase: String,
    pub safe_details: Value,
    pub redacted: bool,
}

impl From<CoveQlDiagnostic> for LogicalPlanDiagnostic {
    fn from(diagnostic: CoveQlDiagnostic) -> Self {
        Self {
            code: diagnostic.code,
            severity: diagnostic.severity,
            message: diagnostic.message,
            phase: diagnostic.phase,
            safe_details: diagnostic.safe_details,
            redacted: diagnostic.redacted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedQuery {
    pub resolved: ResolvedQuery,
    pub logical_plan: CoveOLogicalPlan,
    pub dependencies: LogicalPlanDependencySet,
    pub diagnostics: Vec<LogicalPlanDiagnostic>,
    pub predicate_ast_fingerprint: String,
    pub predicate_cnf_fingerprint: String,
    pub projection_dependency_fingerprint: String,
    pub logical_plan_fingerprint: LogicalPlanFingerprint,
}

impl PlannedQuery {
    pub fn explain_json(&self) -> Value {
        crate::explain::planned_query_explain_json(self)
    }

    pub fn explain_text(&self) -> String {
        crate::render_explain_text(&self.explain_json())
    }

    pub fn logical_plan_text(&self) -> String {
        self.logical_plan
            .to_text(self.resolved.diagnostic_policy.metadata_disclosure)
    }
}

#[derive(Debug, Clone)]
pub struct BuildLogicalPlanError {
    pub diagnostics: Vec<LogicalPlanDiagnostic>,
    pub source: Option<String>,
}

impl BuildLogicalPlanError {
    fn single(diagnostic: LogicalPlanDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            source: None,
        }
    }

    fn from_resolve(error: BuildResolvedQueryError) -> Self {
        Self {
            diagnostics: error
                .diagnostics
                .into_iter()
                .map(LogicalPlanDiagnostic::from)
                .collect(),
            source: error.source,
        }
    }

    pub fn explain_json(&self) -> Value {
        crate::explain::error_explain_json(
            "logical_plan",
            self.diagnostics
                .iter()
                .map(|diagnostic| {
                    json!({
                        "code": diagnostic.code,
                        "severity": crate::diagnostic_severity_name(diagnostic.severity),
                        "message": diagnostic.message,
                        "span": Value::Null,
                        "phase": diagnostic.phase,
                        "safe_details": diagnostic.safe_details,
                        "redacted": diagnostic.redacted,
                    })
                })
                .collect(),
            Vec::new(),
        )
    }
}

impl fmt::Display for BuildLogicalPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "CoveQL logical plan build failed")
        }
    }
}

impl Error for BuildLogicalPlanError {}

pub fn parse_resolve_and_plan_query(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    plan_options: PlanOptions,
    validation_options: ValidationOptions,
) -> Result<PlannedQuery, BuildLogicalPlanError> {
    let resolved = crate::resolver::parse_and_resolve_query(
        bytes,
        text,
        parse_options,
        resolve_options,
        validation_options,
    )
    .map_err(BuildLogicalPlanError::from_resolve)?;
    build_logical_plan(resolved, plan_options)
}

pub fn build_logical_plan(
    resolved: ResolvedQuery,
    options: PlanOptions,
) -> Result<PlannedQuery, BuildLogicalPlanError> {
    let context = PlanContext::from_resolved(&resolved);
    let dependencies = LogicalPlanDependencySet::from_resolved_query(&resolved);
    let mut diagnostics = Vec::new();
    validate_grouping(&resolved, &options)?;
    validate_aggregate_filters(&resolved)?;
    validate_aggregate_disclosure(&resolved)?;

    let predicate_plan = PredicatePlanning::from_predicate_with_dataset(
        resolved.method_chain.where_predicate.as_ref(),
        &resolved.root,
        security_scope(&resolved),
        &resolved.operation_context.dataset,
    );
    validate_sort(&resolved, &options)?;

    if !predicate_plan.decode_boundaries.is_empty() {
        diagnostics.push(plan_warning(
            "W_DECODE_BOUNDARY",
            "one or more predicates require materialized residual evaluation",
            &resolved.security_context(),
        ));
    }

    let mut nodes = LogicalNodeBuilder::default();
    nodes.push(LogicalPlanNodeKind::RootScan {
        root: resolved.root.clone(),
        scan_grain: context.scan_grain,
    });
    nodes.push(LogicalPlanNodeKind::BranchScope {
        branch: resolved.branch.clone(),
    });
    nodes.push(LogicalPlanNodeKind::TombstoneScope {
        tombstone: resolved.tombstone.clone(),
    });
    nodes.push(LogicalPlanNodeKind::TemporalScope {
        temporal: resolved.temporal.clone(),
    });
    nodes.push(LogicalPlanNodeKind::ScanGrainSelection {
        scan_grain: context.scan_grain,
    });
    nodes.push(LogicalPlanNodeKind::PreReconstructionFilter {
        predicates: predicate_plan.pre_reconstruction.clone(),
    });
    nodes.push(LogicalPlanNodeKind::Reconstruct {
        required: reconstruction_required(&resolved.root),
    });
    nodes.push(LogicalPlanNodeKind::VisibilityBarrier {
        visibility: resolved.visibility_reference.clone(),
    });
    nodes.push(LogicalPlanNodeKind::RedactionBarrier {
        redaction: resolved.redaction_reference.clone(),
    });
    nodes.push(LogicalPlanNodeKind::PostReconstructionFilter {
        predicates: predicate_plan.post_reconstruction.clone(),
    });
    if !predicate_plan.association.is_empty() {
        nodes.push(LogicalPlanNodeKind::AssociationSemiJoin {
            predicates: predicate_plan.association.clone(),
        });
    }
    let anti_join_predicates = association_anti_join_predicates(&predicate_plan.residual);
    if !anti_join_predicates.is_empty() {
        nodes.push(LogicalPlanNodeKind::AssociationAntiJoin {
            predicates: anti_join_predicates,
        });
    }
    if matches!(resolved.root, ResolvedRoot::Association(_)) {
        nodes.push(LogicalPlanNodeKind::AssociationExpansion {
            associations: dependencies
                .association_type_ids
                .iter()
                .map(u32::to_string)
                .collect(),
        });
    }
    if matches!(resolved.root, ResolvedRoot::Evidence(_))
        || !dependencies.evidence_fields.is_empty()
    {
        nodes.push(LogicalPlanNodeKind::EvidenceRead {
            evidence_fields: dependencies.evidence_fields.iter().cloned().collect(),
        });
    }
    if matches!(resolved.root, ResolvedRoot::Projection(_))
        || !dependencies.projection_ids.is_empty()
    {
        nodes.push(LogicalPlanNodeKind::ProjectionRead {
            projection_ids: dependencies.projection_ids.iter().cloned().collect(),
            projection_columns: dependencies.projection_columns.iter().cloned().collect(),
        });
    }
    if !resolved.method_chain.ctes.is_empty() {
        nodes.push(LogicalPlanNodeKind::CommonTableExpression {
            ctes: resolved.method_chain.ctes.clone(),
        });
    }
    if !resolved.method_chain.joins.is_empty() {
        nodes.push(LogicalPlanNodeKind::TableJoin {
            joins: resolved.method_chain.joins.clone(),
        });
    }
    if !resolved.method_chain.set_operations.is_empty() {
        nodes.push(LogicalPlanNodeKind::TableSetOperation {
            operations: resolved.method_chain.set_operations.clone(),
        });
    }
    if !resolved.method_chain.windows.is_empty() {
        nodes.push(LogicalPlanNodeKind::Window {
            windows: resolved.method_chain.windows.clone(),
        });
    }
    if !resolved.method_chain.graph_algorithms.is_empty() {
        nodes.push(LogicalPlanNodeKind::GraphAlgorithm {
            algorithms: resolved.method_chain.graph_algorithms.clone(),
        });
    }
    if !resolved.method_chain.ai_operations.is_empty() {
        nodes.push(LogicalPlanNodeKind::AiOperation {
            operations: resolved.method_chain.ai_operations.clone(),
            sidecar_required: true,
            fallback_behavior:
                "reject selected AI operation unless required AI sidecar sections validate".into(),
        });
    }
    if grouping_or_aggregates_present(&resolved) {
        nodes.push(LogicalPlanNodeKind::Aggregate {
            group_by: resolved.method_chain.group_by.clone().unwrap_or_default(),
            aggregates: aggregate_exprs(&resolved),
            disclosure_policy: resolved
                .operation_context
                .security
                .aggregate_disclosure_policy,
        });
    }
    nodes.push(LogicalPlanNodeKind::SelectProjection {
        items: select_projection_items(&resolved),
    });
    let (sort_keys, defaulted) = sort_keys(&resolved);
    let canonical_order = canonical_order_fields(&sort_keys);
    nodes.push(LogicalPlanNodeKind::Sort {
        keys: sort_keys,
        stable_tiebreaker: stable_tiebreaker(&resolved.root, context.scan_grain),
        defaulted,
    });
    nodes.push(LogicalPlanNodeKind::SkipTake {
        skip: resolved.method_chain.skip,
        take: resolved.method_chain.take,
    });
    if let Some(mode) = resolved.method_chain.explain {
        nodes.push(LogicalPlanNodeKind::Explain { mode });
    }
    if options.emit_index_only_candidates && index_only_requested(&resolved) {
        nodes.push(LogicalPlanNodeKind::IndexOnlyCandidate {
            requested: true,
            permitted: resolved
                .operation_context
                .security
                .index_only_answer_permission,
        });
    }
    nodes.push(LogicalPlanNodeKind::FallbackBoundary {
        reasons: resolved
            .operation_context
            .fallbacks
            .iter()
            .map(|fallback| fallback.reason.clone())
            .collect(),
        optional_metadata_candidates: resolved
            .operation_context
            .optional_metadata
            .iter()
            .map(|metadata| format!("{:?}:{:?}", metadata.kind, metadata.status))
            .collect(),
    });
    nodes.push(LogicalPlanNodeKind::OutputBoundary {
        output_mode: resolved.output_mode.clone(),
    });

    let mut logical_plan = CoveOLogicalPlan {
        context,
        nodes: nodes.nodes,
        predicate_forms: predicate_plan.all_forms,
        decode_boundaries: predicate_plan.decode_boundaries,
        residual_predicates: predicate_plan.residual,
        canonical_order,
        default_ordering_applied: defaulted,
        logical_plan_fingerprint: String::new(),
    };
    let logical_plan_fingerprint = fingerprint(&logical_plan, &dependencies);
    logical_plan.logical_plan_fingerprint = logical_plan_fingerprint.clone();
    let predicate_ast_fingerprint =
        predicate_ast_fingerprint(resolved.method_chain.where_predicate.as_ref());
    let predicate_cnf_fingerprint =
        predicate_cnf_fingerprint(resolved.method_chain.where_predicate.as_ref());
    let projection_dependency_fingerprint = projection_dependency_fingerprint(&dependencies);

    let mut all_diagnostics = resolved
        .diagnostics
        .iter()
        .cloned()
        .map(LogicalPlanDiagnostic::from)
        .collect::<Vec<_>>();
    all_diagnostics.extend(diagnostics);

    Ok(PlannedQuery {
        resolved,
        logical_plan,
        dependencies,
        diagnostics: all_diagnostics,
        predicate_ast_fingerprint,
        predicate_cnf_fingerprint,
        projection_dependency_fingerprint,
        logical_plan_fingerprint,
    })
}

fn association_anti_join_predicates(forms: &[LogicalPredicateForm]) -> Vec<LogicalPredicateForm> {
    let mut out = Vec::new();
    for form in forms {
        collect_association_anti_join_predicates(form, &mut out);
    }
    out
}

fn collect_association_anti_join_predicates(
    form: &LogicalPredicateForm,
    out: &mut Vec<LogicalPredicateForm>,
) {
    match &form.kind {
        LogicalPredicateKind::Not(child)
            if child.classification
                == crate::predicate::FilterClassification::AssociationSemiJoin =>
        {
            out.push(form.clone());
        }
        LogicalPredicateKind::Not(child) => collect_association_anti_join_predicates(child, out),
        LogicalPredicateKind::And(children) | LogicalPredicateKind::Or(children) => {
            for child in children {
                collect_association_anti_join_predicates(child, out);
            }
        }
        LogicalPredicateKind::Compare { .. }
        | LogicalPredicateKind::InList { .. }
        | LogicalPredicateKind::NullCheck { .. }
        | LogicalPredicateKind::Exists { .. }
        | LogicalPredicateKind::BoolExpr { .. } => {}
    }
}

impl PlanContext {
    fn from_resolved(resolved: &ResolvedQuery) -> Self {
        let root_kind = root_kind(&resolved.root);
        let scan_grain = scan_grain(resolved);
        Self {
            root_kind,
            scan_grain,
            temporal: resolved.temporal.clone(),
            branch: resolved.branch.clone(),
            tombstone: resolved.tombstone.clone(),
            output_mode: resolved.output_mode.clone(),
            explain_mode: resolved.method_chain.explain,
            visibility: resolved.visibility_reference.clone(),
            redaction: resolved.redaction_reference.clone(),
            security_scope: security_scope(resolved),
        }
    }
}

trait ResolvedSecurity {
    fn security_context(&self) -> SecurityContext;
}

impl ResolvedSecurity for ResolvedQuery {
    fn security_context(&self) -> SecurityContext {
        self.operation_context.security.clone()
    }
}

#[derive(Default)]
struct LogicalNodeBuilder {
    nodes: Vec<LogicalPlanNode>,
}

impl LogicalNodeBuilder {
    fn push(&mut self, kind: LogicalPlanNodeKind) {
        let id = LogicalNodeId(self.nodes.len() as u32);
        self.nodes.push(LogicalPlanNode::new(id, kind));
    }
}

fn validate_grouping(
    resolved: &ResolvedQuery,
    options: &PlanOptions,
) -> Result<(), BuildLogicalPlanError> {
    if !options.strict_group_by || resolved.method_chain.group_by.is_none() {
        return Ok(());
    }
    let group_exprs = resolved.method_chain.group_by.as_ref().unwrap();
    let Some(select) = &resolved.method_chain.select else {
        return Ok(());
    };
    for item in select {
        if !expr_is_group_safe(&item.expr, group_exprs) {
            return Err(BuildLogicalPlanError::single(plan_error(
                "E_GROUPING",
                "select expression is neither grouped nor aggregated",
                &resolved.security_context(),
            )));
        }
    }
    Ok(())
}

fn validate_aggregate_disclosure(resolved: &ResolvedQuery) -> Result<(), BuildLogicalPlanError> {
    if aggregate_exprs(resolved).is_empty() {
        return Ok(());
    }
    if resolved
        .operation_context
        .security
        .aggregate_disclosure_policy
        == AggregateDisclosurePolicy::Reject
    {
        return Err(BuildLogicalPlanError::single(plan_error(
            "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
            "aggregate disclosure is rejected by the active security context",
            &resolved.security_context(),
        )));
    }
    Ok(())
}

fn validate_aggregate_filters(resolved: &ResolvedQuery) -> Result<(), BuildLogicalPlanError> {
    let Some(predicate) = &resolved.method_chain.where_predicate else {
        return Ok(());
    };
    if predicate_contains_aggregate(predicate) {
        return Err(BuildLogicalPlanError::single(plan_error(
            "E_UNSUPPORTED_AGGREGATE_FILTER",
            "aggregate expressions are not supported inside where; use select/groupBy aggregates only",
            &resolved.security_context(),
        )));
    }
    Ok(())
}

fn validate_sort(
    resolved: &ResolvedQuery,
    options: &PlanOptions,
) -> Result<(), BuildLogicalPlanError> {
    if options.allow_file_code_ordering_without_contract {
        return Ok(());
    }
    let Some(order) = &resolved.method_chain.order_by else {
        return Ok(());
    };
    if let Some(path) = path_from_expr(&order.expr) {
        if path.physical_kind == "file_code" && !path_has_ordering_collation_contract(path) {
            return Err(BuildLogicalPlanError::single(plan_error(
                "E_UNSAFE_CODE_ORDERING",
                "orderBy over FileCode requires an order-preserving collation contract or materialized sort key",
                &resolved.security_context(),
            )));
        }
    }
    Ok(())
}

fn path_has_ordering_collation_contract(path: &ResolvedPath) -> bool {
    matches!(path.logical_type.as_str(), "utf8" | "string")
        && match path.collation_id {
            None => true,
            Some(id) if id == CollationKind::None.id() => true,
            Some(id) => {
                CollationKind::from_id(id).is_some_and(|kind| kind == CollationKind::Utf8Bytewise)
            }
        }
}

fn expr_is_group_safe(expr: &ResolvedExpr, group_exprs: &[ResolvedExpr]) -> bool {
    if group_exprs.iter().any(|group| expr_equal(expr, group)) {
        return true;
    }
    match expr {
        ResolvedExpr::AggregateCall { .. } => true,
        ResolvedExpr::FunctionCall { args, .. } => {
            args.iter().all(|arg| expr_is_group_safe(arg, group_exprs))
        }
        _ => false,
    }
}

fn expr_equal(left: &ResolvedExpr, right: &ResolvedExpr) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

fn grouping_or_aggregates_present(resolved: &ResolvedQuery) -> bool {
    resolved.method_chain.group_by.is_some() || !aggregate_exprs(resolved).is_empty()
}

fn aggregate_exprs(resolved: &ResolvedQuery) -> Vec<ResolvedExpr> {
    let mut out = Vec::new();
    if let Some(select) = &resolved.method_chain.select {
        for item in select {
            collect_aggregates(&item.expr, &mut out);
        }
    }
    out
}

fn predicate_contains_aggregate(predicate: &ResolvedPredicate) -> bool {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            expr_contains_aggregate(left) || expr_contains_aggregate(right)
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => expr_contains_aggregate(expr),
        ResolvedPredicate::Not(inner) => predicate_contains_aggregate(inner),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            parts.iter().any(predicate_contains_aggregate)
        }
    }
}

fn expr_contains_aggregate(expr: &ResolvedExpr) -> bool {
    match expr {
        ResolvedExpr::AggregateCall { .. } => true,
        ResolvedExpr::FunctionCall { args, .. } => args.iter().any(expr_contains_aggregate),
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            predicate_contains_aggregate(predicate)
                || expr_contains_aggregate(then_expr)
                || expr_contains_aggregate(else_expr)
        }
        _ => false,
    }
}

fn collect_aggregates(expr: &ResolvedExpr, out: &mut Vec<ResolvedExpr>) {
    match expr {
        ResolvedExpr::AggregateCall { arg, .. } => {
            out.push(expr.clone());
            if let Some(arg) = arg {
                collect_aggregates(arg, out);
            }
        }
        ResolvedExpr::FunctionCall { args, .. } => {
            for arg in args {
                collect_aggregates(arg, out);
            }
        }
        ResolvedExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
            ..
        } => {
            collect_aggregates_in_predicate(predicate, out);
            collect_aggregates(then_expr, out);
            collect_aggregates(else_expr, out);
        }
        _ => {}
    }
}

fn collect_aggregates_in_predicate(predicate: &ResolvedPredicate, out: &mut Vec<ResolvedExpr>) {
    match predicate {
        ResolvedPredicate::Compare { left, right, .. } => {
            collect_aggregates(left, out);
            collect_aggregates(right, out);
        }
        ResolvedPredicate::InList { expr, .. }
        | ResolvedPredicate::NullCheck { expr, .. }
        | ResolvedPredicate::Exists(expr)
        | ResolvedPredicate::BoolExpr(expr) => collect_aggregates(expr, out),
        ResolvedPredicate::Not(inner) => collect_aggregates_in_predicate(inner, out),
        ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
            for part in parts {
                collect_aggregates_in_predicate(part, out);
            }
        }
    }
}

fn select_projection_items(resolved: &ResolvedQuery) -> Vec<SelectProjectionItem> {
    resolved
        .method_chain
        .select
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|item| SelectProjectionItem {
                    alias: item.alias.clone(),
                    expr: item.expr.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sort_keys(resolved: &ResolvedQuery) -> (Vec<LogicalSortKey>, bool) {
    if let Some(order) = &resolved.method_chain.order_by {
        let (representation, collation_id) = match path_from_expr(&order.expr) {
            Some(path) if path.physical_kind == "file_code" => {
                (RepresentationClass::DecodeBoundary, path.collation_id)
            }
            Some(path) if path.physical_kind == "num_code" => {
                (RepresentationClass::TypedNumeric, path.collation_id)
            }
            Some(path) => (RepresentationClass::CodePure, path.collation_id),
            None => (RepresentationClass::ResidualMaterialized, None),
        };
        let nulls = match order.nulls {
            AstNullOrdering::Default => match order.direction {
                AstOrderDirection::Asc => AstNullOrdering::NullsLast,
                AstOrderDirection::Desc => AstNullOrdering::NullsFirst,
            },
            explicit => explicit,
        };
        return (
            vec![LogicalSortKey {
                expr: Some(order.expr.clone()),
                field: None,
                direction: order.direction,
                nulls,
                representation,
                collation_id,
            }],
            false,
        );
    }
    (
        default_order_fields(resolved)
            .into_iter()
            .map(|field| LogicalSortKey {
                expr: None,
                field: Some(field),
                direction: AstOrderDirection::Asc,
                nulls: AstNullOrdering::NullsLast,
                representation: RepresentationClass::CodePure,
                collation_id: None,
            })
            .collect(),
        true,
    )
}

fn canonical_order_fields(sort_keys: &[LogicalSortKey]) -> Vec<String> {
    sort_keys
        .iter()
        .map(|key| {
            key.field.clone().unwrap_or_else(|| {
                serde_json::to_string(&key.expr).unwrap_or_else(|_| "expression_order".into())
            })
        })
        .collect()
}

fn path_from_expr(expr: &ResolvedExpr) -> Option<&ResolvedPath> {
    match expr {
        ResolvedExpr::Path(path) => Some(path),
        _ => None,
    }
}

fn stable_tiebreaker(root: &ResolvedRoot, grain: ScanGrain) -> Vec<String> {
    fallback_default_order_fields(root, grain)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn default_order_fields(resolved: &ResolvedQuery) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(projection) = projection_contract_root(&resolved.root) {
        fields.extend(
            projection
                .ordering
                .iter()
                .filter(|field| !field.trim().is_empty())
                .cloned(),
        );
    }
    if let ResolvedRoot::Table(table) = &resolved.root {
        for field in &table.canonical_order {
            if !field.trim().is_empty() && !fields.iter().any(|existing| existing == field) {
                fields.push(field.clone());
            }
        }
        for field in &table.row_identity {
            if !field.trim().is_empty() && !fields.iter().any(|existing| existing == field) {
                fields.push(field.clone());
            }
        }
    }
    for field in fallback_default_order_fields(&resolved.root, scan_grain(resolved)) {
        if !fields.iter().any(|existing| existing == field) {
            fields.push(field.into());
        }
    }
    fields
}

fn fallback_default_order_fields(root: &ResolvedRoot, grain: ScanGrain) -> Vec<&'static str> {
    match (root, grain) {
        (_, ScanGrain::HistoryRecord | ScanGrain::HistoryState) => {
            vec![
                "object_type_id",
                "branch_key",
                "goid",
                "timestamp_us",
                "csn",
                "record_id",
            ]
        }
        (_, ScanGrain::HistoryRecordsAndStates) => {
            vec![
                "object_type_id",
                "branch_key",
                "goid",
                "timestamp_us",
                "csn",
                "record_id",
                "output_grain",
            ]
        }
        (
            _,
            ScanGrain::ChangeRecord
            | ScanGrain::ChangeStateTransition
            | ScanGrain::ChangePropertyDiff
            | ScanGrain::ChangeFinalObject,
        ) => {
            vec![
                "object_type_id",
                "branch_key",
                "goid",
                "timestamp_us",
                "csn",
                "record_id",
            ]
        }
        (ResolvedRoot::Association(_), _) => vec![
            "association_type_id",
            "branch_key",
            "source_goid",
            "target_goid",
            "association_goid",
            "csn",
            "record_id",
        ],
        (ResolvedRoot::Node(_), _) => vec!["node_label", "branch_key", "goid"],
        (ResolvedRoot::Edge(_), _) => vec![
            "edge_label",
            "branch_key",
            "source_goid",
            "target_goid",
            "edge_goid",
            "csn",
            "record_id",
        ],
        (ResolvedRoot::Evidence(_), _) => vec![
            "target_id",
            "grain",
            "source_system",
            "source_row_identity",
            "evidence_id",
        ],
        (ResolvedRoot::Table(table), _) => {
            if table.row_identity.is_empty() {
                vec!["projection_row_identity", "source_canonical_row_identity"]
            } else {
                Vec::new()
            }
        }
        (ResolvedRoot::Projection(_), _) => {
            vec!["projection_row_identity", "source_canonical_row_identity"]
        }
        (ResolvedRoot::Object(_), _) => vec!["object_type_id", "branch_key", "goid"],
    }
}

fn root_kind(root: &ResolvedRoot) -> LogicalRootKind {
    match root {
        ResolvedRoot::Object(_) => LogicalRootKind::Object,
        ResolvedRoot::Association(_) => LogicalRootKind::Association,
        ResolvedRoot::Node(_) => LogicalRootKind::Node,
        ResolvedRoot::Edge(_) => LogicalRootKind::Edge,
        ResolvedRoot::Table(_) => LogicalRootKind::Table,
        ResolvedRoot::Projection(_) => LogicalRootKind::Projection,
        ResolvedRoot::Evidence(_) => LogicalRootKind::Evidence,
    }
}

fn scan_grain(resolved: &ResolvedQuery) -> ScanGrain {
    match &resolved.root {
        ResolvedRoot::Node(_) => ScanGrain::NodeState,
        ResolvedRoot::Edge(_) => ScanGrain::EdgeState,
        ResolvedRoot::Table(_) => ScanGrain::TableRow,
        ResolvedRoot::Projection(_) => ScanGrain::ProjectionRow,
        ResolvedRoot::Evidence(_) => ScanGrain::EvidenceRow,
        ResolvedRoot::Association(_) => {
            if let Some(changes) = &resolved.method_chain.changes {
                scan_grain_for_change_mode(changes.mode)
            } else if matches!(
                resolved.method_chain.history,
                Some(AstHistoryMode::RecordsAndStates)
            ) {
                ScanGrain::HistoryRecordsAndStates
            } else if matches!(resolved.method_chain.history, Some(AstHistoryMode::Records)) {
                ScanGrain::HistoryRecord
            } else if resolved.method_chain.history.is_some() {
                ScanGrain::HistoryState
            } else {
                ScanGrain::AssociationState
            }
        }
        ResolvedRoot::Object(_) => {
            if let Some(changes) = &resolved.method_chain.changes {
                scan_grain_for_change_mode(changes.mode)
            } else if matches!(
                resolved.method_chain.history,
                Some(AstHistoryMode::RecordsAndStates)
            ) {
                ScanGrain::HistoryRecordsAndStates
            } else if matches!(resolved.method_chain.history, Some(AstHistoryMode::Records)) {
                ScanGrain::HistoryRecord
            } else if resolved.method_chain.history.is_some() {
                ScanGrain::HistoryState
            } else {
                ScanGrain::LatestState
            }
        }
    }
}

fn scan_grain_for_change_mode(mode: crate::AstChangeMode) -> ScanGrain {
    match mode {
        crate::AstChangeMode::Records => ScanGrain::ChangeRecord,
        crate::AstChangeMode::StateTransitions => ScanGrain::ChangeStateTransition,
        crate::AstChangeMode::PropertyDiffs => ScanGrain::ChangePropertyDiff,
        crate::AstChangeMode::FinalRows => ScanGrain::ChangeFinalObject,
    }
}

fn reconstruction_required(root: &ResolvedRoot) -> bool {
    matches!(
        root,
        ResolvedRoot::Object(_)
            | ResolvedRoot::Association(_)
            | ResolvedRoot::Node(_)
            | ResolvedRoot::Edge(_)
    )
}

fn projection_contract_root(root: &ResolvedRoot) -> Option<&crate::ResolvedProjectionRoot> {
    match root {
        ResolvedRoot::Projection(projection) => Some(projection),
        ResolvedRoot::Table(table) => Some(&table.projection),
        _ => None,
    }
}

fn index_only_requested(resolved: &ResolvedQuery) -> bool {
    matches!(
        resolved.operation_context.request.selected_operation,
        CoveQlSelectedOperation::IndexOnlyAnswer
            | CoveQlSelectedOperation::Explain {
                target: crate::CoveQlExplainTarget::IndexOnlyAnswer,
                ..
            }
    )
}

fn security_scope(resolved: &ResolvedQuery) -> String {
    format!(
        "visibility={:?};redaction={:?};metadata={:?}",
        resolved.operation_context.security.visibility_policy,
        resolved.operation_context.security.redaction_policy,
        resolved
            .operation_context
            .security
            .metadata_disclosure_policy
    )
}

fn plan_error(
    code: impl Into<String>,
    message: impl Into<String>,
    security: &SecurityContext,
) -> LogicalPlanDiagnostic {
    LogicalPlanDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: "logical_plan".into(),
        safe_details: json!({}),
        redacted: security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected,
    }
}

fn plan_warning(
    code: impl Into<String>,
    message: impl Into<String>,
    security: &SecurityContext,
) -> LogicalPlanDiagnostic {
    let mut diagnostic = plan_error(code, message, security);
    diagnostic.severity = DiagnosticSeverity::Warning;
    diagnostic
}

fn fingerprint(plan: &CoveOLogicalPlan, dependencies: &LogicalPlanDependencySet) -> String {
    let value = json!({
        "context": plan.context,
        "nodes": plan.nodes,
        "predicate_forms": plan.predicate_forms,
        "decode_boundaries": plan.decode_boundaries,
        "residual_predicates": plan.residual_predicates,
        "canonical_order": plan.canonical_order,
        "default_ordering_applied": plan.default_ordering_applied,
        "dependencies": dependencies,
    });
    sha256_hex(
        serde_json::to_string(&value)
            .expect("logical plan fingerprint JSON serializes")
            .as_bytes(),
    )
}

fn predicate_ast_fingerprint(predicate: Option<&ResolvedPredicate>) -> String {
    fingerprint_value(&json!({
        "version": crate::PREDICATE_NORMAL_FORM_VERSION,
        "predicate": predicate,
    }))
}

fn predicate_cnf_fingerprint(predicate: Option<&ResolvedPredicate>) -> String {
    let mut cnf_forms = Vec::new();
    if let Some(predicate) = predicate {
        collect_cnf_fingerprint_forms(predicate, &mut cnf_forms);
    }
    fingerprint_value(&json!({
        "version": crate::PREDICATE_NORMAL_FORM_VERSION,
        "cnf_forms": cnf_forms,
    }))
}

fn projection_dependency_fingerprint(dependencies: &LogicalPlanDependencySet) -> String {
    fingerprint_value(&json!({
        "version": crate::PROJECTION_DEPENDENCY_CONTRACT_VERSION,
        "projection_contracts": &dependencies.projection_contracts,
    }))
}

fn collect_cnf_fingerprint_forms(predicate: &ResolvedPredicate, out: &mut Vec<ResolvedPredicate>) {
    match predicate {
        ResolvedPredicate::And(parts) => {
            for part in parts {
                collect_cnf_fingerprint_forms(part, out);
            }
        }
        _ => out.push(predicate.clone()),
    }
}

fn fingerprint_value(value: &Value) -> String {
    sha256_hex(
        serde_json::to_string(value)
            .expect("fingerprint JSON serializes")
            .as_bytes(),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}
