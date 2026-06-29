use crate::{
    ast::*, build_operation_context, parse_query, BranchContext, BranchSelector, CoveQlAiOperation,
    CoveQlDiagnostic, CoveQlExplainTarget, CoveQlOperationRequest, CoveQlOutputMode,
    CoveQlProfileId, CoveQlSelectedOperation, DiagnosticSeverity, ExplainMode,
    MetadataDisclosurePolicy, RedactionReference, RejectionKind, RejectionReport,
    ResourceUseEstimate, SecurityContext, TemporalContext, TemporalMode, TemporalRole,
    TombstoneContext, VisibilityReference,
};
use cove_core::{
    constants::CovePhysicalKind,
    profile::{
        cove_map::{EmbeddedMapSection, MapEvidenceIndex, MapProjectionCatalog},
        cove_o::{
            read_object_surface_from_bytes_with_options, CoveObjectReadOptions,
            CoveObjectRedactionReadPolicy, CoveObjectSurface, ObjectTypeEntryV1, PropertyEntryV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_LINK_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            PROPERTY_FLAG_ASSOCIATION_TYPE, PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
            PROPERTY_FLAG_ASSOCIATION_VALID_TO,
        },
    },
    reader::ValidationOptions,
    types::logical_type_name,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub fn parse_and_resolve_query(
    bytes: &[u8],
    text: &str,
    parse_options: ParseOptions,
    resolve_options: ResolveOptions,
    validation_options: ValidationOptions,
) -> Result<ResolvedQuery, BuildResolvedQueryError> {
    let parsed = parse_query(text, parse_options).map_err(BuildResolvedQueryError::from_parse)?;
    resolve_query(bytes, parsed, resolve_options, validation_options)
}

pub fn resolve_query(
    bytes: &[u8],
    parsed: ParsedQuery,
    resolve_options: ResolveOptions,
    validation_options: ValidationOptions,
) -> Result<ResolvedQuery, BuildResolvedQueryError> {
    validate_profile_shell(&parsed, &resolve_options)?;
    let mut settings = collect_chain_settings(&parsed, &resolve_options)?;
    let request = operation_request_for(&parsed, &settings, &resolve_options);
    let operation_context = build_operation_context(bytes, request, validation_options)
        .map_err(BuildResolvedQueryError::from_operation_context)?;

    let surface = match read_object_surface_from_bytes_with_options(
        bytes,
        &CoveObjectReadOptions {
            include_records: false,
            include_association_object_types: true,
            include_projection_catalog: true,
            include_function_registry: true,
            include_evidence_index: true,
            redaction_read_policy: CoveObjectRedactionReadPolicy::PreserveMarker,
            ..CoveObjectReadOptions::default()
        },
    ) {
        Ok(surface) => surface,
        Err(_error)
            if can_resolve_registered_table_without_object_surface(&parsed, &resolve_options) =>
        {
            empty_object_surface()
        }
        Err(error) => {
            return Err(BuildResolvedQueryError::single(diagnostic(
                "E_RESOLVE",
                format!("failed to read resolver metadata: {error}"),
                "resolve",
                &resolve_options.security,
            )))
        }
    };

    let mut resolver = Resolver::new(surface, resolve_options.clone());
    resolver.window_function_scope = parsed.methods.iter().any(|method| {
        matches!(
            &method.node,
            AstMethod::ProfileCall { name, .. } if name.name == "window"
        )
    });
    let mut root = resolver.resolve_root(&parsed.root.node)?;
    apply_root_alias(&mut root, parsed.root_alias.as_ref());
    settings.temporal = resolver.resolve_temporal_context(settings.temporal.clone(), &root)?;
    let method_chain = resolver.resolve_methods(&parsed, &root, &settings)?;
    let resolved_query_fingerprint = resolved_fingerprint(
        &root,
        &method_chain,
        &settings.output_mode,
        &settings.temporal,
        &settings.branch,
        &settings.tombstone,
        &parsed.profiles,
    );
    let diagnostics = operation_context
        .diagnostics
        .iter()
        .cloned()
        .chain(resolver.diagnostics)
        .collect();

    Ok(ResolvedQuery {
        parsed,
        operation_context,
        root,
        method_chain,
        output_mode: settings.output_mode,
        temporal: settings.temporal,
        branch: settings.branch,
        tombstone: settings.tombstone,
        visibility_reference: VisibilityReference {
            policy: resolve_options.security.visibility_policy.clone(),
        },
        redaction_reference: RedactionReference {
            policy: resolve_options.security.redaction_policy.clone(),
        },
        diagnostic_policy: DiagnosticPolicy {
            metadata_disclosure: resolve_options.security.metadata_disclosure_policy,
            explain_policy: resolve_options.security.explain_policy,
        },
        resource_use: settings.resource_use,
        resolved_query_fingerprint,
        diagnostics,
    })
}

fn can_resolve_registered_table_without_object_surface(
    parsed: &ParsedQuery,
    options: &ResolveOptions,
) -> bool {
    let AstRoot::Table(identifier) = &parsed.root.node else {
        return false;
    };
    options.table_authorities.contains_key(&identifier.name)
        || options.table_authorities.values().any(|authority| {
            authority.contract.table_name == identifier.name
                || authority.contract.table_id == identifier.name
        })
}

fn empty_object_surface() -> CoveObjectSurface {
    CoveObjectSurface {
        object_types: Vec::new(),
        records: Vec::new(),
        projection_catalog: None,
        evidence_index: None,
        embedded_function_ids: BTreeSet::new(),
        embedded_map_sections: Vec::new(),
    }
}

#[derive(Debug, Clone)]
pub struct BuildResolvedQueryError {
    pub diagnostics: Vec<CoveQlDiagnostic>,
    pub rejections: Vec<RejectionReport>,
    pub source: Option<String>,
}

impl BuildResolvedQueryError {
    fn single(diagnostic: CoveQlDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            rejections: Vec::new(),
            source: None,
        }
    }

    fn from_parse(diagnostics: Vec<CoveQlDiagnostic>) -> Self {
        Self {
            diagnostics,
            rejections: Vec::new(),
            source: None,
        }
    }

    fn from_operation_context(error: crate::BuildOperationContextError) -> Self {
        Self {
            diagnostics: error.diagnostics,
            rejections: error.rejections,
            source: error.source,
        }
    }

    pub fn explain_json(&self) -> serde_json::Value {
        crate::explain::error_explain_json(
            "resolve",
            self.diagnostics
                .iter()
                .map(crate::CoveQlDiagnostic::to_json)
                .collect(),
            self.rejections
                .iter()
                .map(crate::RejectionReport::to_json)
                .collect(),
        )
    }
}

impl fmt::Display for BuildResolvedQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = self.diagnostics.first() {
            write!(f, "{}: {}", diagnostic.code, diagnostic.message)
        } else {
            write!(f, "CoveQL query resolution failed")
        }
    }
}

impl Error for BuildResolvedQueryError {}

#[derive(Debug, Clone)]
struct ChainSettings {
    selected_operation: CoveQlSelectedOperation,
    output_mode: CoveQlOutputMode,
    temporal: TemporalContext,
    branch: BranchContext,
    tombstone: TombstoneContext,
    resource_use: ResourceUseEstimate,
}

fn collect_chain_settings(
    parsed: &ParsedQuery,
    resolve_options: &ResolveOptions,
) -> Result<ChainSettings, BuildResolvedQueryError> {
    let mut temporal = TemporalContext::default();
    let mut branch = BranchContext::default();
    let mut tombstone = TombstoneContext::default();
    let mut seen_select = false;
    let mut seen_as_of = false;
    let mut seen_branch = false;
    let mut seen_tombstones = false;
    let mut seen_history = false;
    let mut seen_changes = false;
    let mut seen_order_by = false;
    let mut seen_take = false;
    let mut seen_skip = false;
    let mut seen_group_by = false;
    let mut seen_relationship_expansion = matches!(parsed.root.node, AstRoot::Path(_));
    let mut explain = None;

    for method in &parsed.methods {
        if explain.is_some() {
            return Err(conflict(
                "explain must be the final method in a CoveQL method chain",
                resolve_options,
            ));
        }
        match &method.node {
            AstMethod::Where(_) => {
                if seen_group_by {
                    return Err(conflict(
                        "where after groupBy requires a profile-declared having/post-aggregate filter",
                        resolve_options,
                    ));
                }
            }
            AstMethod::Select(_) => reject_duplicate(&mut seen_select, "select", resolve_options)?,
            AstMethod::AsOf(bound) => {
                reject_duplicate(&mut seen_as_of, "asOf", resolve_options)?;
                if seen_history || seen_changes {
                    return Err(conflict(
                        "asOf cannot be mixed with history or changes",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "asOf must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                temporal = temporal_for_as_of(bound, resolve_options)?;
            }
            AstMethod::Branch(selector) => {
                reject_duplicate(&mut seen_branch, "branch", resolve_options)?;
                branch = branch_for_selector(selector, resolve_options)?;
            }
            AstMethod::IncludeTombstones(value) => {
                reject_duplicate(&mut seen_tombstones, "includeTombstones", resolve_options)?;
                tombstone.include_tombstones = *value;
            }
            AstMethod::History(mode) => {
                reject_duplicate(&mut seen_history, "history", resolve_options)?;
                if seen_as_of || seen_changes {
                    return Err(conflict(
                        "history cannot be mixed with asOf or changes",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "history must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                temporal.mode = match mode {
                    AstHistoryMode::Records => TemporalMode::HistoryRecords,
                    AstHistoryMode::States => TemporalMode::HistoryStates,
                    AstHistoryMode::RecordsAndStates => TemporalMode::HistoryRecordsAndStates,
                };
            }
            AstMethod::Changes { from, to, mode } => {
                reject_duplicate(&mut seen_changes, "changes", resolve_options)?;
                if seen_as_of || seen_history {
                    return Err(conflict(
                        "changes cannot be mixed with asOf or history",
                        resolve_options,
                    ));
                }
                if seen_relationship_expansion {
                    return Err(conflict(
                        "changes must appear in the temporal phase before lookup, traverse, or path expansion",
                        resolve_options,
                    ));
                }
                if from.bound_kind() != to.bound_kind() {
                    return Err(conflict(
                        "changes bounds must use the same bound kind",
                        resolve_options,
                    ));
                }
                temporal.mode = match mode {
                    AstChangeMode::Records => TemporalMode::ChangesRecords,
                    AstChangeMode::StateTransitions => TemporalMode::ChangesStateTransitions,
                    AstChangeMode::PropertyDiffs => TemporalMode::ChangesPropertyDiffs,
                    AstChangeMode::FinalRows => TemporalMode::ChangesFinalObjects,
                };
            }
            AstMethod::OrderBy(_) => {
                reject_duplicate(&mut seen_order_by, "orderBy", resolve_options)?
            }
            AstMethod::Take(_) => reject_duplicate(&mut seen_take, "take", resolve_options)?,
            AstMethod::Skip(_) => reject_duplicate(&mut seen_skip, "skip", resolve_options)?,
            AstMethod::GroupBy(_) => {
                reject_duplicate(&mut seen_group_by, "groupBy", resolve_options)?
            }
            AstMethod::Explain(mode) => {
                if explain.replace(*mode).is_some() {
                    return Err(duplicate("duplicate explain method", resolve_options));
                }
            }
            AstMethod::ProfileCall { name, .. } if !is_coveql_profile_method(&name.name) => {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    format!(
                        "CoveQL profile method {} is not declared by the active profile contract for this root grain",
                        name.name
                    ),
                    resolve_options,
                ));
            }
            AstMethod::ProfileCall { name, .. } => {
                if seen_group_by {
                    return Err(conflict(
                        format!(
                            "{} cannot appear after groupBy unless its profile contract accepts aggregate grain",
                            name.name
                        ),
                        resolve_options,
                    ));
                }
                if profile_method_expands_relationships(&name.name) {
                    seen_relationship_expansion = true;
                }
            }
        }
    }

    let target = explain_target_for_root(&parsed.root.node);
    let default_output = output_mode_for_root(&parsed.root.node);
    let output_mode = if explain.is_some() {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(CoveQlOutputMode::ExplainJson)
    } else {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(default_output)
    };
    let ai_operation = selected_ai_operation_for_methods(&parsed.methods, explain);
    let selected_operation = if let Some(operation) = ai_operation {
        CoveQlSelectedOperation::Ai {
            root: target,
            operation,
        }
    } else if let Some(mode) = explain {
        CoveQlSelectedOperation::Explain { target, mode }
    } else if let CoveQlOutputMode::ArrowRecordBatch {
        zero_copy_requested,
    } = &output_mode
    {
        CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: *zero_copy_requested,
        }
    } else {
        selected_operation_for_root(&parsed.root.node)
    };

    let mut resource_use = parsed.resource_use.clone();
    resource_use.planning_time_ms = Some(0);
    resource_use.decode_bytes = Some(0);
    resource_use.range_requests = Some(0);

    Ok(ChainSettings {
        selected_operation,
        output_mode,
        temporal,
        branch,
        tombstone,
        resource_use,
    })
}

fn is_coveql_profile_method(name: &str) -> bool {
    matches!(
        name,
        "lookup"
            | "join"
            | "semiJoin"
            | "antiJoin"
            | "union"
            | "intersect"
            | "except"
            | "window"
            | "with"
            | "withRecursive"
            | "traverse"
            | "reachable"
            | "shortestPath"
            | "allPaths"
            | "kShortestPaths"
            | "connectedComponents"
            | "degree"
            | "pageRank"
            | "hits"
            | "centrality"
            | "triangleCount"
            | "clusteringCoefficient"
            | "community"
            | "spanningTree"
            | "similar"
            | "embedding"
            | "chunks"
            | "tokens"
            | "context"
            | "hybrid"
            | "trainingSamples"
            | "split"
            | "pack"
            | "multimodal"
            | "asPromptContext"
            | "rerank"
            | "generatorAudit"
    )
}

fn profile_method_expands_relationships(name: &str) -> bool {
    matches!(
        name,
        "lookup"
            | "join"
            | "semiJoin"
            | "antiJoin"
            | "traverse"
            | "reachable"
            | "shortestPath"
            | "allPaths"
            | "kShortestPaths"
            | "connectedComponents"
            | "degree"
            | "pageRank"
            | "hits"
            | "centrality"
            | "triangleCount"
            | "clusteringCoefficient"
            | "community"
            | "spanningTree"
    )
}

fn graph_algorithm_kind(name: &str) -> Option<GraphAlgorithmKind> {
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

fn graph_algorithm_output_logical_type(field: &str) -> Option<&'static str> {
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

fn operation_request_for(
    parsed: &ParsedQuery,
    settings: &ChainSettings,
    resolve_options: &ResolveOptions,
) -> CoveQlOperationRequest {
    CoveQlOperationRequest {
        selected_operation: settings.selected_operation.clone(),
        output_mode: settings.output_mode.clone(),
        temporal: settings.temporal.clone(),
        branch: settings.branch.clone(),
        tombstone: settings.tombstone.clone(),
        security: resolve_options.security.clone(),
        fallback_policy: resolve_options.fallback_policy,
        resource_budget: resolve_options.resource_budget.clone(),
        resource_use: settings.resource_use.clone(),
        query_text_fingerprint: Some(parsed.query_text_fingerprint.clone()),
        parsed_ast_fingerprint: Some(parsed.parsed_ast_fingerprint.clone()),
        evidence_metadata_requested: ast_requests_evidence_metadata(parsed),
        execution_code_mapping_requested: resolve_options.execution_code_mapping_requested,
        cache_hook: resolve_options.cache_hook.clone(),
    }
}

fn ast_requests_evidence_metadata(parsed: &ParsedQuery) -> bool {
    matches!(parsed.root.node, AstRoot::Evidence(_))
        || parsed
            .methods
            .iter()
            .any(|method| method_requests_evidence_metadata(&method.node))
}

fn method_requests_evidence_metadata(method: &AstMethod) -> bool {
    match method {
        AstMethod::Where(predicate) => predicate_requests_evidence_metadata(&predicate.node),
        AstMethod::Select(items) => items
            .iter()
            .any(|item| expr_requests_evidence_metadata(&item.expr.node)),
        AstMethod::OrderBy(order) => expr_requests_evidence_metadata(&order.expr.node),
        AstMethod::GroupBy(exprs) => exprs
            .iter()
            .any(|expr| expr_requests_evidence_metadata(&expr.node)),
        AstMethod::AsOf(_)
        | AstMethod::Branch(_)
        | AstMethod::IncludeTombstones(_)
        | AstMethod::History(_)
        | AstMethod::Changes { .. }
        | AstMethod::Take(_)
        | AstMethod::Skip(_)
        | AstMethod::Explain(_) => false,
        AstMethod::ProfileCall { args, .. } => args.iter().any(profile_argument_requests_evidence),
    }
}

fn predicate_requests_evidence_metadata(predicate: &AstPredicate) -> bool {
    match predicate {
        AstPredicate::Compare { left, right, .. } => {
            expr_requests_evidence_metadata(&left.node)
                || expr_requests_evidence_metadata(&right.node)
        }
        AstPredicate::InList { expr, .. }
        | AstPredicate::NullCheck { expr, .. }
        | AstPredicate::BoolExpr(expr) => expr_requests_evidence_metadata(&expr.node),
        AstPredicate::Exists { target, args } => {
            expr_requests_evidence_metadata(&target.node)
                || args.iter().any(profile_argument_requests_evidence)
        }
        AstPredicate::Not(inner) => predicate_requests_evidence_metadata(&inner.node),
        AstPredicate::And(parts) | AstPredicate::Or(parts) => parts
            .iter()
            .any(|part| predicate_requests_evidence_metadata(&part.node)),
    }
}

fn expr_requests_evidence_metadata(expr: &AstExpr) -> bool {
    match expr {
        AstExpr::Evidence(_) => true,
        AstExpr::FunctionCall { args, .. } => args
            .iter()
            .any(|arg| expr_requests_evidence_metadata(&arg.node)),
        AstExpr::AggregateCall { arg, .. } => arg
            .as_deref()
            .is_some_and(|arg| expr_requests_evidence_metadata(&arg.node)),
        AstExpr::Association(_)
        | AstExpr::Relationship(_)
        | AstExpr::RootBinding(_)
        | AstExpr::Literal(_)
        | AstExpr::Path(_) => false,
        AstExpr::Conditional {
            predicate,
            then_expr,
            else_expr,
        } => {
            predicate_requests_evidence_metadata(&predicate.node)
                || expr_requests_evidence_metadata(&then_expr.node)
                || expr_requests_evidence_metadata(&else_expr.node)
        }
    }
}

fn profile_argument_requests_evidence(argument: &AstProfileArgument) -> bool {
    match &argument.value {
        AstProfileArgumentValue::Expr(expr) => expr_requests_evidence_metadata(&expr.node),
        AstProfileArgumentValue::Predicate(predicate) => {
            predicate_requests_evidence_metadata(&predicate.node)
        }
    }
}

fn validate_profile_shell(
    parsed: &ParsedQuery,
    resolve_options: &ResolveOptions,
) -> Result<(), BuildResolvedQueryError> {
    for profile in &parsed.profiles {
        if !resolve_options.enabled_profiles.contains(profile) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE",
                format!(
                    "CoveQL/{} is not enabled for this resolver context",
                    profile.as_str()
                ),
                resolve_options,
            ));
        }
    }

    let root_profile = profile_for_root(&parsed.root.node);
    if matches!(
        parsed.root.node,
        AstRoot::Object(_) | AstRoot::Association(_)
    ) && !parsed.profiles.contains(&CoveQlProfileId::Object)
    {
        return Err(profile_rejection(
            "E_UNSUPPORTED_PROFILE",
            "object and association roots require the CoveQL/Object profile",
            resolve_options,
        ));
    }
    if matches!(
        parsed.root.node,
        AstRoot::Projection(_) | AstRoot::Evidence(_)
    ) && parsed
        .profiles
        .iter()
        .filter(|profile| is_host_profile(**profile))
        .count()
        > 1
    {
        return Err(profile_rejection(
            "E_AMBIGUOUS_PROFILE",
            "projection and evidence roots require a single host profile context",
            resolve_options,
        ));
    }
    match root_profile {
        CoveQlProfileId::Object => {}
        CoveQlProfileId::Table => {}
        CoveQlProfileId::Graph => {}
        CoveQlProfileId::Ai => {}
    }
    if let Some(name) = first_unsupported_profile_method(parsed) {
        return Err(profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            format!(
                "CoveQL profile method {name} is not declared by the active profile contract for this root grain"
            ),
            resolve_options,
        ));
    }
    if parsed_requires_ai_profile(parsed) && !parsed.profiles.contains(&CoveQlProfileId::Ai) {
        return Err(profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            "CoveQL-AI methods require the active ai profile",
            resolve_options,
        ));
    }
    Ok(())
}

fn is_host_profile(profile: CoveQlProfileId) -> bool {
    matches!(
        profile,
        CoveQlProfileId::Object | CoveQlProfileId::Table | CoveQlProfileId::Graph
    )
}

fn parsed_requires_ai_profile(parsed: &ParsedQuery) -> bool {
    parsed.methods.iter().any(|method| match &method.node {
        AstMethod::Explain(ExplainMode::Ai) => true,
        AstMethod::ProfileCall { name, .. } => ai_operation_for_method(&name.name).is_some(),
        _ => false,
    })
}

fn first_unsupported_profile_method(parsed: &ParsedQuery) -> Option<&str> {
    parsed.methods.iter().find_map(|method| match &method.node {
        AstMethod::ProfileCall { name, .. } if !is_coveql_profile_method(&name.name) => {
            Some(name.name.as_str())
        }
        _ => None,
    })
}

fn profile_for_root(root: &AstRoot) -> CoveQlProfileId {
    match root {
        AstRoot::Object(_)
        | AstRoot::Association(_)
        | AstRoot::Projection(_)
        | AstRoot::Evidence(_) => CoveQlProfileId::Object,
        AstRoot::Table(_) => CoveQlProfileId::Table,
        AstRoot::Node(_) | AstRoot::Edge(_) | AstRoot::Path(_) => CoveQlProfileId::Graph,
    }
}

fn profile_rejection(
    code: &'static str,
    message: impl Into<String>,
    resolve_options: &ResolveOptions,
) -> BuildResolvedQueryError {
    BuildResolvedQueryError {
        diagnostics: vec![diagnostic(
            code,
            message,
            "resolve",
            &resolve_options.security,
        )],
        rejections: vec![RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: code.into(),
        }],
        source: None,
    }
}

fn apply_root_alias(root: &mut ResolvedRoot, alias: Option<&AstIdentifier>) {
    let Some(alias) = alias else {
        return;
    };
    if let ResolvedRoot::Table(table) = root {
        table.binding_name = Some(alias.name.clone());
    } else if let ResolvedRoot::Node(node) = root {
        node.binding_name = Some(alias.name.clone());
    } else if let ResolvedRoot::Edge(edge) = root {
        edge.binding_name = Some(alias.name.clone());
    }
}

fn reject_duplicate(
    seen: &mut bool,
    method: &'static str,
    resolve_options: &ResolveOptions,
) -> Result<(), BuildResolvedQueryError> {
    if *seen {
        Err(duplicate(
            format!("duplicate or conflicting {method} method"),
            resolve_options,
        ))
    } else {
        *seen = true;
        Ok(())
    }
}

fn duplicate(
    message: impl Into<String>,
    resolve_options: &ResolveOptions,
) -> BuildResolvedQueryError {
    BuildResolvedQueryError {
        diagnostics: vec![diagnostic(
            "E_DUPLICATE_METHOD",
            message,
            "resolve",
            &resolve_options.security,
        )],
        rejections: vec![RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: "duplicate method in method chain".into(),
        }],
        source: None,
    }
}

fn conflict(
    message: impl Into<String>,
    resolve_options: &ResolveOptions,
) -> BuildResolvedQueryError {
    BuildResolvedQueryError {
        diagnostics: vec![diagnostic(
            "E_METHOD_CONFLICT",
            message,
            "resolve",
            &resolve_options.security,
        )],
        rejections: vec![RejectionReport {
            kind: RejectionKind::FeatureValidation,
            reason: "method chain conflict".into(),
        }],
        source: None,
    }
}

fn temporal_for_as_of(
    bound: &AstTimeBound,
    resolve_options: &ResolveOptions,
) -> Result<TemporalContext, BuildResolvedQueryError> {
    match bound {
        AstTimeBound::Csn(csn) => Ok(TemporalContext {
            mode: TemporalMode::AsOfCsn(*csn),
            role: TemporalRole::CommitTime,
            role_binding: None,
        }),
        AstTimeBound::Timestamp { role, timestamp } => {
            let (role, role_binding) = temporal_role(*role, resolve_options)?;
            Ok(TemporalContext {
                mode: TemporalMode::AsOfTimestampMicros(
                    timestamp_micros(timestamp, &resolve_options.security)?.0,
                ),
                role,
                role_binding,
            })
        }
    }
}

fn temporal_role(
    role: AstTimeRole,
    resolve_options: &ResolveOptions,
) -> Result<(TemporalRole, Option<String>), BuildResolvedQueryError> {
    let resolved = match role {
        AstTimeRole::Time | AstTimeRole::CommitTime => {
            return Ok((TemporalRole::CommitTime, None));
        }
        AstTimeRole::AssociationValidTime => {
            return Ok((TemporalRole::AssociationValidTime, None));
        }
        AstTimeRole::ValidTime => TemporalRole::ValidTime,
        AstTimeRole::ObservedTime => TemporalRole::ObservedTime,
        AstTimeRole::SourceEventTime => TemporalRole::SourceEventTime,
    };
    let binding = resolve_options
        .temporal_role_bindings
        .contains_key(&resolved)
        .then(|| resolve_options.temporal_role_bindings[&resolved].clone());
    Ok((resolved, binding))
}

fn branch_for_selector(
    selector: &AstBranchSelector,
    resolve_options: &ResolveOptions,
) -> Result<BranchContext, BuildResolvedQueryError> {
    let ambiguous_alias = |name: &str| {
        resolve_options
            .ambiguous_branch_aliases
            .get(name)
            .is_some_and(|aliases| aliases.len() > 1)
    };
    let ambiguous_error = || {
        BuildResolvedQueryError::single(diagnostic(
            "E_AMBIGUOUS_BRANCH",
            "branch alias is ambiguous for selected query scope",
            "resolve",
            &resolve_options.security,
        ))
    };
    let branch = match selector {
        AstBranchSelector::UInt(value) => BranchSelector::BranchKey(*value),
        AstBranchSelector::Identifier(identifier) if identifier.name == "default" => {
            BranchSelector::Default
        }
        AstBranchSelector::Identifier(identifier) if identifier.name == "reject_ambiguous" => {
            BranchSelector::RejectAmbiguous
        }
        AstBranchSelector::String(value) if value == "default" => BranchSelector::Default,
        AstBranchSelector::String(value) if value == "reject_ambiguous" => {
            BranchSelector::RejectAmbiguous
        }
        AstBranchSelector::Identifier(identifier) => resolve_options
            .branch_aliases
            .get(&identifier.name)
            .copied()
            .map(BranchSelector::BranchKey)
            .ok_or_else(|| {
                if ambiguous_alias(&identifier.name) {
                    ambiguous_error()
                } else {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_UNKNOWN_BRANCH",
                        "unknown branch alias for selected query scope",
                        "resolve",
                        &resolve_options.security,
                    ))
                }
            })?,
        AstBranchSelector::String(value) => resolve_options
            .branch_aliases
            .get(value)
            .copied()
            .map(BranchSelector::BranchKey)
            .ok_or_else(|| {
                if ambiguous_alias(value) {
                    ambiguous_error()
                } else {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_UNKNOWN_BRANCH",
                        "unknown branch alias for selected query scope",
                        "resolve",
                        &resolve_options.security,
                    ))
                }
            })?,
    };
    Ok(BranchContext { selector: branch })
}

fn selected_operation_for_root(root: &AstRoot) -> CoveQlSelectedOperation {
    match root {
        AstRoot::Object(_) => CoveQlSelectedOperation::Object,
        AstRoot::Association(_) => CoveQlSelectedOperation::Association,
        AstRoot::Node(_) => CoveQlSelectedOperation::GraphNode,
        AstRoot::Edge(_) => CoveQlSelectedOperation::GraphEdge,
        AstRoot::Table(_) => CoveQlSelectedOperation::Table,
        AstRoot::Projection(_) => CoveQlSelectedOperation::Projection,
        AstRoot::Evidence(_) => CoveQlSelectedOperation::Evidence,
        AstRoot::Path(_) => CoveQlSelectedOperation::GraphNode,
    }
}

fn explain_target_for_root(root: &AstRoot) -> CoveQlExplainTarget {
    match root {
        AstRoot::Object(_) => CoveQlExplainTarget::Object,
        AstRoot::Association(_) => CoveQlExplainTarget::Association,
        AstRoot::Node(_) => CoveQlExplainTarget::GraphNode,
        AstRoot::Edge(_) => CoveQlExplainTarget::GraphEdge,
        AstRoot::Table(_) => CoveQlExplainTarget::Table,
        AstRoot::Projection(_) => CoveQlExplainTarget::Projection,
        AstRoot::Evidence(_) => CoveQlExplainTarget::Evidence,
        AstRoot::Path(_) => CoveQlExplainTarget::GraphNode,
    }
}

fn output_mode_for_root(root: &AstRoot) -> CoveQlOutputMode {
    match root {
        AstRoot::Object(_) => CoveQlOutputMode::ObjectRows,
        AstRoot::Association(_) => CoveQlOutputMode::AssociationRows,
        AstRoot::Table(_) => CoveQlOutputMode::JsonRows,
        AstRoot::Projection(_) => CoveQlOutputMode::ProjectionRows,
        AstRoot::Evidence(_) => CoveQlOutputMode::EvidenceRows,
        AstRoot::Node(_) | AstRoot::Edge(_) | AstRoot::Path(_) => CoveQlOutputMode::JsonRows,
    }
}

fn selected_ai_operation_for_methods(
    methods: &[Spanned<AstMethod>],
    explain: Option<ExplainMode>,
) -> Option<CoveQlAiOperation> {
    if matches!(explain, Some(ExplainMode::Ai)) {
        return Some(CoveQlAiOperation::Inspect);
    }
    methods
        .iter()
        .filter_map(|method| match &method.node {
            AstMethod::ProfileCall { name, .. } => ai_operation_for_method(&name.name),
            _ => None,
        })
        .max_by_key(|operation| ai_operation_priority(*operation))
}

fn ai_operation_for_method(name: &str) -> Option<CoveQlAiOperation> {
    match name {
        "chunks" => Some(CoveQlAiOperation::ChunkProjection),
        "tokens" => Some(CoveQlAiOperation::TokenProjection),
        "embedding" => Some(CoveQlAiOperation::Embedding),
        "similar" | "hybrid" | "rerank" => Some(CoveQlAiOperation::SemanticSearch),
        "context" | "asPromptContext" => Some(CoveQlAiOperation::RagContext),
        "trainingSamples" | "split" | "pack" => Some(CoveQlAiOperation::TrainingSampleExport),
        "multimodal" => Some(CoveQlAiOperation::MultimodalSequenceRead),
        "generatorAudit" => Some(CoveQlAiOperation::GeneratorAudit),
        _ => None,
    }
}

fn ai_operation_priority(operation: CoveQlAiOperation) -> u8 {
    match operation {
        CoveQlAiOperation::Inspect => 0,
        CoveQlAiOperation::ChunkProjection => 1,
        CoveQlAiOperation::TokenProjection => 2,
        CoveQlAiOperation::Embedding => 3,
        CoveQlAiOperation::SemanticSearch => 4,
        CoveQlAiOperation::RagContext => 5,
        CoveQlAiOperation::TrainingSampleExport => 6,
        CoveQlAiOperation::MultimodalSequenceRead => 7,
        CoveQlAiOperation::GeneratorAudit => 8,
    }
}

fn ai_authority(operation: CoveQlAiOperation) -> &'static str {
    match operation {
        CoveQlAiOperation::Embedding => {
            "sidecar_vector_or_deterministic_runtime_embedding; runtime_float_composition_is_advisory"
        }
        CoveQlAiOperation::SemanticSearch => {
            "validated_exact_flat_vector_scan_or_advisory_candidate_rerank"
        }
        CoveQlAiOperation::RagContext => "validated_chunk_source_binding_with_policy_scoped_context",
        CoveQlAiOperation::TrainingSampleExport => {
            "validated_training_policy_with_policy_withheld_diagnostics"
        }
        CoveQlAiOperation::MultimodalSequenceRead => {
            "validated_multimodal_sequence_asset_tensor_policy"
        }
        CoveQlAiOperation::ChunkProjection => "validated_chunk_profile_and_source_hash_freshness",
        CoveQlAiOperation::TokenProjection => {
            "validated_tokenizer_profile_token_payload_and_source_hash_freshness"
        }
        CoveQlAiOperation::GeneratorAudit => "validated_generator_provenance_and_review_records",
        CoveQlAiOperation::Inspect => "validated_ai_descriptor_summary",
    }
}

fn ai_policy_scope(root: &ResolvedRoot) -> &'static str {
    match root {
        ResolvedRoot::Object(_) => "object_state_visibility_redaction",
        ResolvedRoot::Association(_) => "association_state_visibility_redaction",
        ResolvedRoot::Projection(_) => "projection_row_visibility_redaction",
        ResolvedRoot::Evidence(_) => "evidence_row_visibility_redaction",
        ResolvedRoot::Table(_) => "table_row_visibility_redaction",
        ResolvedRoot::Node(_) => "graph_node_visibility_redaction",
        ResolvedRoot::Edge(_) => "graph_edge_visibility_redaction",
    }
}

fn expr_logical_type(expr: &ResolvedExpr) -> Option<&str> {
    match expr {
        ResolvedExpr::Path(path) => Some(path.logical_type.as_str()),
        ResolvedExpr::Literal(literal) => Some(literal.logical_type.as_str()),
        ResolvedExpr::FunctionCall { logical_type, .. }
        | ResolvedExpr::AggregateCall { logical_type, .. }
        | ResolvedExpr::Conditional { logical_type, .. } => Some(logical_type.as_str()),
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) | ResolvedExpr::TableExists(_) => {
            None
        }
    }
}

fn common_output_logical_type(left: &ResolvedExpr, right: &ResolvedExpr) -> &'static str {
    match (expr_logical_type(left), expr_logical_type(right)) {
        (Some(left), Some(right)) => common_logical_type_names(left, right),
        _ => "unknown",
    }
}

fn common_logical_type_names(left: &str, right: &str) -> &'static str {
    match (left, right) {
        ("null", "null") => "null",
        ("null", other) | (other, "null") => known_logical_type(other),
        (left, right) if left == right => known_logical_type(left),
        ("utf8" | "string", "utf8" | "string") => "utf8",
        (left, right) if is_decimal_type(left) || is_decimal_type(right) => "decimal128",
        (left, right) if is_numeric_type(left) && is_numeric_type(right) => {
            widest_numeric_type(left, right)
        }
        (left, right) if left == "unknown" || right == "unknown" => "unknown",
        _ => "incompatible",
    }
}

fn coalesce_common_logical_type(args: &[ResolvedExpr]) -> Option<&'static str> {
    let mut current = "null";
    for arg in args {
        let arg_type = expr_logical_type(arg)?;
        let next = common_logical_type_names(current, arg_type);
        if next == "incompatible" {
            return None;
        }
        current = next;
    }
    Some(current)
}

fn known_logical_type(logical_type: &str) -> &'static str {
    match logical_type {
        "null" => "null",
        "bool" => "bool",
        "utf8" => "utf8",
        "string" => "string",
        "int8" => "int8",
        "int16" => "int16",
        "int32" => "int32",
        "int64" => "int64",
        "uint8" => "uint8",
        "uint16" => "uint16",
        "uint32" => "uint32",
        "uint64" => "uint64",
        "float32" => "float32",
        "float64" => "float64",
        "decimal64" => "decimal64",
        "decimal128" => "decimal128",
        "timestamp_micros" => "timestamp_micros",
        "timestamp_nanos" => "timestamp_nanos",
        "date_days" => "date_days",
        _ => "unknown",
    }
}

fn is_decimal_type(logical_type: &str) -> bool {
    matches!(logical_type, "decimal64" | "decimal128")
}

fn is_numeric_type(logical_type: &str) -> bool {
    matches!(
        logical_type,
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "decimal128"
            | "timestamp_micros"
            | "timestamp_nanos"
            | "date_days"
    )
}

fn widest_numeric_type(left: &str, right: &str) -> &'static str {
    if matches!(left, "float64" | "float32") || matches!(right, "float64" | "float32") {
        "float64"
    } else if matches!(left, "uint64") || matches!(right, "uint64") {
        "uint64"
    } else {
        "int64"
    }
}

fn integer_literal_value(value: i128) -> ResolvedLiteralValue {
    if let Ok(value) = i64::try_from(value) {
        ResolvedLiteralValue::SignedInteger(value)
    } else if let Ok(value) = u64::try_from(value) {
        ResolvedLiteralValue::UnsignedInteger(value)
    } else {
        ResolvedLiteralValue::BigInteger(value.to_string())
    }
}

fn decode_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(chunk).ok()?;
        out.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(out)
}

fn parse_uuid_literal(value: &str) -> Option<([u8; 16], String)> {
    let mut compact = String::with_capacity(32);
    for (index, ch) in value.chars().enumerate() {
        if ch == '-' {
            if !matches!(index, 8 | 13 | 18 | 23) {
                return None;
            }
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            return None;
        }
        compact.push(ch.to_ascii_lowercase());
    }
    if compact.len() != 32 {
        return None;
    }
    let bytes = decode_hex_bytes(&compact)?;
    let bytes: [u8; 16] = bytes.try_into().ok()?;
    Some((bytes, compact))
}

struct Resolver {
    surface: CoveObjectSurface,
    options: ResolveOptions,
    diagnostics: Vec<CoveQlDiagnostic>,
    lookup_scope: Vec<ResolvedTableRoot>,
    cte_table_scope: BTreeMap<String, ResolvedTableRoot>,
    graph_node_scope: Vec<ResolvedGraphNodeRoot>,
    graph_edge_scope: Vec<ResolvedGraphEdgeRoot>,
    window_function_scope: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssociationResolveUsage {
    RootScan,
    ObjectRelative,
    EvidenceTarget,
}

impl Resolver {
    fn new(surface: CoveObjectSurface, options: ResolveOptions) -> Self {
        Self {
            surface,
            options,
            diagnostics: Vec::new(),
            lookup_scope: Vec::new(),
            cte_table_scope: BTreeMap::new(),
            graph_node_scope: Vec::new(),
            graph_edge_scope: Vec::new(),
            window_function_scope: false,
        }
    }

    fn resolve_root(&self, root: &AstRoot) -> Result<ResolvedRoot, BuildResolvedQueryError> {
        match root {
            AstRoot::Object(identifier) => self
                .resolve_object_type(&identifier.name)
                .map(|object| ResolvedRoot::Object(resolved_object_root(object))),
            AstRoot::Association(association) => self
                .resolve_association_root(association, None, AssociationResolveUsage::RootScan)
                .map(ResolvedRoot::Association),
            AstRoot::Node(identifier) => self
                .resolve_graph_node_root(identifier)
                .map(ResolvedRoot::Node),
            AstRoot::Edge(edge) => self.resolve_graph_edge_root(edge).map(ResolvedRoot::Edge),
            AstRoot::Projection(identifier) => self
                .resolve_projection_root(&identifier.name)
                .map(ResolvedRoot::Projection),
            AstRoot::Table(identifier) => self
                .resolve_table_root(&identifier.name)
                .map(ResolvedRoot::Table),
            AstRoot::Evidence(evidence) => self
                .resolve_evidence_root(evidence, None)
                .map(ResolvedRoot::Evidence),
            AstRoot::Path(path) => self.resolve_path_root_start(path),
        }
    }

    fn resolve_path_root_start(
        &self,
        path: &AstPathRootExpr,
    ) -> Result<ResolvedRoot, BuildResolvedQueryError> {
        let mut root = self.resolve_root(&path.start.root.node)?;
        apply_root_alias(&mut root, path.start.alias.as_ref());
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "path roots require a node(...) start",
                &self.options,
            ));
        }
        Ok(root)
    }

    fn resolve_temporal_context(
        &self,
        mut temporal: TemporalContext,
        root: &ResolvedRoot,
    ) -> Result<TemporalContext, BuildResolvedQueryError> {
        if temporal.role_binding.is_some()
            || matches!(
                temporal.role,
                TemporalRole::CommitTime | TemporalRole::AssociationValidTime
            )
        {
            return Ok(temporal);
        }
        if let Some(binding) = self.infer_temporal_role_binding(temporal.role, root)? {
            temporal.role_binding = Some(binding);
            return Ok(temporal);
        }
        Err(BuildResolvedQueryError::single(diagnostic(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            format!(
                "{:?} requires an exact temporal role binding in ResolveOptions or an unambiguous timestamp property on the root object type",
                temporal.role
            ),
            "resolve",
            &self.options.security,
        )))
    }

    fn infer_temporal_role_binding(
        &self,
        role: TemporalRole,
        root: &ResolvedRoot,
    ) -> Result<Option<String>, BuildResolvedQueryError> {
        let ResolvedRoot::Object(root) = root else {
            return Ok(None);
        };
        let Some(object_type) = self
            .surface
            .object_types
            .iter()
            .find(|object_type| object_type.object_type_id == root.object_type_id)
        else {
            return Ok(None);
        };
        let names = temporal_role_inference_names(role);
        let matches = object_type
            .properties
            .iter()
            .filter(|property| {
                names.contains(&property.property_name.as_str())
                    && logical_type_name(property.logical_type) == "timestamp_micros"
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [property] => Ok(Some(property.property_name.clone())),
            _ => Err(BuildResolvedQueryError::single(diagnostic(
                "E_AMBIGUOUS_TEMPORAL_ROLE",
                format!(
                    "{role:?} matches multiple timestamp properties; pass an explicit temporal role binding"
                ),
                "resolve",
                &self.options.security,
            ))),
        }
    }

    fn resolve_methods(
        &mut self,
        parsed: &ParsedQuery,
        root: &ResolvedRoot,
        settings: &ChainSettings,
    ) -> Result<ResolvedMethodChain, BuildResolvedQueryError> {
        let mut chain = ResolvedMethodChain::default();
        if let AstRoot::Path(path) = &parsed.root.node {
            for relationship in &path.relationships {
                let traversal = self.resolve_graph_traversal_expr(
                    relationship,
                    root,
                    1,
                    1,
                    GraphTraversalMode::Walk,
                    GraphTraversalDistinctPolicy::None,
                    None,
                )?;
                if let Some(target) = &traversal.target {
                    self.graph_node_scope.push(target.clone());
                }
                self.graph_edge_scope.push(traversal.edge.clone());
                chain.traversals.push(traversal);
            }
        }
        for method in &parsed.methods {
            match &method.node {
                AstMethod::Where(predicate) => {
                    let resolved = self.resolve_predicate(predicate, root)?;
                    chain.where_predicate = Some(match chain.where_predicate.take() {
                        Some(existing) => ResolvedPredicate::And(vec![existing, resolved]),
                        None => resolved,
                    });
                }
                AstMethod::Select(items) => {
                    chain.select = Some(
                        items
                            .iter()
                            .map(|item| {
                                Ok(ResolvedSelectItem {
                                    alias: item.alias.as_ref().map(|alias| alias.name.clone()),
                                    expr: self.resolve_expr(&item.expr, root)?,
                                })
                            })
                            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
                    );
                }
                AstMethod::OrderBy(order) => {
                    chain.order_by = Some(ResolvedOrderClause {
                        expr: self.resolve_expr(&order.expr, root)?,
                        direction: order.direction,
                        nulls: order.nulls,
                        uses_default_ordering: order.nulls == AstNullOrdering::Default,
                    });
                }
                AstMethod::GroupBy(exprs) => {
                    chain.group_by = Some(
                        exprs
                            .iter()
                            .map(|expr| self.resolve_expr(expr, root))
                            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
                    );
                }
                AstMethod::Take(value) => chain.take = Some(*value),
                AstMethod::Skip(value) => chain.skip = Some(*value),
                AstMethod::Explain(mode) => chain.explain = Some(*mode),
                AstMethod::History(mode) => chain.history = Some(*mode),
                AstMethod::Changes { from, to, mode } => {
                    chain.changes = Some(ResolvedChanges {
                        from: self.resolve_change_bound(from)?,
                        to: self.resolve_change_bound(to)?,
                        mode: *mode,
                    });
                }
                AstMethod::ProfileCall { name, args } if name.name == "with" => {
                    let cte = self.resolve_common_table_expression(args, false)?;
                    self.cte_table_scope
                        .insert(cte.name.clone(), cte.table.clone());
                    chain.ctes.push(cte);
                }
                AstMethod::ProfileCall { name, args } if name.name == "withRecursive" => {
                    let cte = self.resolve_common_table_expression(args, true)?;
                    self.cte_table_scope
                        .insert(cte.name.clone(), cte.table.clone());
                    chain.ctes.push(cte);
                }
                AstMethod::ProfileCall { name, args } if name.name == "lookup" => {
                    let lookup = self.resolve_table_lookup(args, root)?;
                    self.lookup_scope.push(lookup.right.clone());
                    chain.lookups.push(lookup);
                }
                AstMethod::ProfileCall { name, args } if name.name == "join" => {
                    let join = self.resolve_table_join(args, root, None)?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args } if name.name == "semiJoin" => {
                    let join = self.resolve_table_join(args, root, Some(TableJoinKind::Semi))?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args } if name.name == "antiJoin" => {
                    let join = self.resolve_table_join(args, root, Some(TableJoinKind::Anti))?;
                    self.lookup_scope.push(join.right.clone());
                    chain.joins.push(join);
                }
                AstMethod::ProfileCall { name, args }
                    if matches!(name.name.as_str(), "union" | "intersect" | "except") =>
                {
                    chain
                        .set_operations
                        .push(self.resolve_set_operation(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, args } if name.name == "window" => {
                    chain.windows.push(self.resolve_window_spec(args, root)?);
                }
                AstMethod::ProfileCall { name, args } if name.name == "traverse" => {
                    let traversal = self.resolve_graph_traversal(args, root)?;
                    if let Some(target) = &traversal.target {
                        self.graph_node_scope.push(target.clone());
                    }
                    self.graph_edge_scope.push(traversal.edge.clone());
                    chain.traversals.push(traversal);
                }
                AstMethod::ProfileCall { name, args }
                    if graph_algorithm_kind(&name.name).is_some() =>
                {
                    chain
                        .graph_algorithms
                        .push(self.resolve_graph_algorithm(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, args }
                    if ai_operation_for_method(&name.name).is_some() =>
                {
                    chain
                        .ai_operations
                        .push(self.resolve_ai_operation(&name.name, args, root)?);
                }
                AstMethod::ProfileCall { name, .. } => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!(
                            "CoveQL profile method {} is not declared by the active profile contract for this root grain",
                            name.name
                        ),
                        &self.options,
                    ));
                }
                AstMethod::AsOf(_) | AstMethod::Branch(_) | AstMethod::IncludeTombstones(_) => {}
            }
        }

        if chain.select.is_none() {
            self.diagnostics.push(warning(
                "W_DEFAULT_SELECT",
                "query has no explicit select; Phase 1 records default output shape only",
                "resolve",
                &self.options.security,
            ));
        }
        if !chain.ai_operations.is_empty() {
            self.diagnostics.push(warning(
                "W_AI_SIDECAR_REQUIRED",
                "CoveQL-AI operation selected; payload access remains sidecar-gated until source binding, privacy, redaction, and integrity validate",
                "resolve",
                &self.options.security,
            ));
        }
        if matches!(settings.output_mode, CoveQlOutputMode::ExplainJson) {
            chain.explain.get_or_insert(ExplainMode::Public);
        }
        Ok(chain)
    }

    fn resolve_common_table_expression(
        &mut self,
        args: &[AstProfileArgument],
        recursive: bool,
    ) -> Result<ResolvedCommonTableExpression, BuildResolvedQueryError> {
        let mut name = None;
        let mut query = None;
        let mut step = None;
        let mut max_iterations = None;
        let mut key_expr = None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("name"), AstProfileArgumentValue::Expr(expr)) => {
                    name = Some(self.profile_enum_literal("name", expr)?);
                }
                (Some("query" | "seed"), AstProfileArgumentValue::Expr(expr)) => {
                    query = Some(expr);
                }
                (Some("step"), AstProfileArgumentValue::Expr(expr)) => {
                    step = Some(expr);
                }
                (Some("maxIterations" | "max_iterations"), AstProfileArgumentValue::Expr(expr)) => {
                    max_iterations = Some(self.resolve_usize_profile_arg("maxIterations", expr)?);
                }
                (Some("key"), AstProfileArgumentValue::Expr(expr)) => {
                    key_expr = Some(expr);
                }
                (Some(candidate), AstProfileArgumentValue::Expr(expr)) if query.is_none() => {
                    name = Some(candidate.to_string());
                    query = Some(expr);
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported CTE argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "with requires a named table binding, such as with(alias: table(name))",
                        &self.options,
                    ));
                }
            }
        }
        let query = query.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "with requires a table(...) query root binding",
                &self.options,
            )
        })?;
        let AstExpr::RootBinding(binding) = &query.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "with query must be a table(...) root binding",
                &self.options,
            ));
        };
        let mut resolved = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut resolved, binding.alias.as_ref());
        let mut table = match resolved {
            ResolvedRoot::Table(table) => table,
            _ => {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "with query must resolve to a CoveQL/Table surface",
                    &self.options,
                ))
            }
        };
        let name = name
            .or_else(|| binding.alias.as_ref().map(|alias| alias.name.clone()))
            .ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "with requires a CTE name or an aliased table query",
                    &self.options,
                )
            })?;
        if recursive && max_iterations.is_none() {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "withRecursive requires finite maxIterations",
                &self.options,
            ));
        }
        let step_table = if let Some(step) = step {
            let AstExpr::RootBinding(binding) = &step.node else {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive step must be a table(...) root binding",
                    &self.options,
                ));
            };
            let mut resolved = self.resolve_root(&binding.root.node)?;
            apply_root_alias(&mut resolved, binding.alias.as_ref());
            match resolved {
                ResolvedRoot::Table(table) => Some(table),
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "withRecursive step must resolve to a CoveQL/Table surface",
                        &self.options,
                    ))
                }
            }
        } else {
            None
        };
        let key = key_expr
            .map(|expr| self.resolve_expr(expr, &ResolvedRoot::Table(table.clone())))
            .transpose()?;
        if let (true, Some(step_table_ref)) = (recursive, step_table.as_ref()) {
            let key = key.as_ref().ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive with a step table requires a key expression",
                    &self.options,
                )
            })?;
            let max_iterations = max_iterations.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "withRecursive requires finite maxIterations",
                    &self.options,
                )
            })?;
            table = self.materialized_recursive_cte_table(
                name.as_str(),
                table,
                step_table_ref,
                key,
                max_iterations,
            )?;
        }
        Ok(ResolvedCommonTableExpression {
            name,
            table,
            recursive,
            max_iterations,
            step_table,
            key,
            execution_authority: if recursive {
                "bounded_materialized_recursive_cte_authority".into()
            } else {
                "materialized_cte_table_authority".into()
            },
        })
    }

    fn materialized_recursive_cte_table(
        &self,
        cte_name: &str,
        mut seed: ResolvedTableRoot,
        step: &ResolvedTableRoot,
        key: &ResolvedExpr,
        max_iterations: usize,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        let mut rows = registered_table_rows_for_recursive_cte(&seed).ok_or_else(|| {
            profile_rejection(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "withRecursive seed must be a registered materialized/raw/external row authority",
                &self.options,
            )
        })?;
        let step_rows = registered_table_rows_for_recursive_cte(step).ok_or_else(|| {
            profile_rejection(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "withRecursive step must be a registered materialized/raw/external row authority",
                &self.options,
            )
        })?;
        let mut seen = BTreeSet::new();
        for row in &rows {
            seen.insert(recursive_cte_key_for_row(row, key, &self.options)?);
        }
        for _ in 0..max_iterations {
            let mut added = 0usize;
            for row in &step_rows {
                let key = recursive_cte_key_for_row(row, key, &self.options)?;
                if seen.insert(key) {
                    rows.push(row.clone());
                    added += 1;
                }
            }
            if added == 0 {
                break;
            }
        }
        seed.table_name = cte_name.to_string();
        seed.table_id = format!("cte:{cte_name}");
        seed.table_surface_contract.table_name = seed.table_name.clone();
        seed.table_surface_contract.table_id = seed.table_id.clone();
        seed.execution_authority = TableExecutionAuthority::MaterializedRows { rows };
        Ok(seed)
    }

    fn resolve_table_lookup(
        &mut self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedTableLookup, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "lookup requires a declared CoveQL bridge from the current root grain to CoveQL/Table",
                &self.options,
            ));
        }
        let mut target = None;
        let mut on = None;
        let mut cardinality = TableLookupCardinality::One;
        let mut cardinality_explicit = false;
        let mut unmatched_policy = TableLookupUnmatchedPolicy::Nulls;
        let mut duplicate_policy = TableLookupDuplicatePolicy::Reject;
        let mut duplicate_explicit = false;
        let mut nulls_match = false;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some("cardinality"), AstProfileArgumentValue::Expr(expr))
                    if !cardinality_explicit =>
                {
                    cardinality = self.resolve_lookup_cardinality(expr)?;
                    cardinality_explicit = true;
                }
                (Some("unmatched"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = self.resolve_lookup_unmatched_policy(expr)?;
                }
                (Some("required"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = if self.resolve_bool_profile_arg("required", expr)? {
                        TableLookupUnmatchedPolicy::Reject
                    } else {
                        TableLookupUnmatchedPolicy::Nulls
                    };
                }
                (Some("duplicate" | "duplicate_policy"), AstProfileArgumentValue::Expr(expr))
                    if !duplicate_explicit =>
                {
                    duplicate_policy = self.resolve_lookup_duplicate_policy(expr)?;
                    duplicate_explicit = true;
                }
                (Some("nulls_match"), AstProfileArgumentValue::Expr(expr)) => {
                    nulls_match = self.resolve_bool_profile_arg("nulls_match", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported lookup argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "lookup requires one table root binding and one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let target = target.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires a right-hand table root binding",
                &self.options,
            )
        })?;
        let on = on.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires an on: predicate",
                &self.options,
            )
        })?;
        let AstExpr::RootBinding(binding) = &target.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup target must be a table(...) root binding",
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup target must resolve to a table surface",
                &self.options,
            ));
        };
        self.lookup_scope.push(right.clone());
        let on = self.resolve_predicate(on, root)?;
        self.lookup_scope.pop();
        self.validate_table_lookup_predicate(&on)?;
        if cardinality == TableLookupCardinality::Many && !duplicate_explicit {
            duplicate_policy = TableLookupDuplicatePolicy::EmitAll;
        }
        if cardinality == TableLookupCardinality::One
            && duplicate_policy == TableLookupDuplicatePolicy::EmitAll
        {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup duplicate: many requires cardinality: many in CoveQL 0.1",
                &self.options,
            ));
        }
        Ok(ResolvedTableLookup {
            right,
            on,
            join_kind: TableLookupJoinKind::LeftPreserving,
            cardinality,
            unmatched_policy,
            duplicate_policy,
            nulls_match,
        })
    }

    fn resolve_table_join(
        &mut self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
        forced_kind: Option<TableJoinKind>,
    ) -> Result<ResolvedTableJoin, BuildResolvedQueryError> {
        let ResolvedRoot::Table(left) = root else {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "cross-profile join requires a registered CoveQL bridge and materialized cross-profile join authority",
                &self.options,
            ));
        };
        let mut target = None;
        let mut on = None;
        let mut join_kind = forced_kind.unwrap_or(TableJoinKind::Inner);
        let mut cardinality = TableLookupCardinality::Many;
        let mut unmatched_policy = TableLookupUnmatchedPolicy::Nulls;
        let mut duplicate_policy = TableLookupDuplicatePolicy::EmitAll;
        let mut nulls_match = false;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some("kind"), AstProfileArgumentValue::Expr(expr)) if forced_kind.is_none() => {
                    join_kind = self.resolve_join_kind(expr)?;
                }
                (Some("cardinality"), AstProfileArgumentValue::Expr(expr)) => {
                    cardinality = self.resolve_lookup_cardinality(expr)?;
                }
                (Some("unmatched"), AstProfileArgumentValue::Expr(expr)) => {
                    unmatched_policy = self.resolve_lookup_unmatched_policy(expr)?;
                }
                (Some("duplicate" | "duplicate_policy"), AstProfileArgumentValue::Expr(expr)) => {
                    duplicate_policy = self.resolve_lookup_duplicate_policy(expr)?;
                }
                (Some("nulls_match"), AstProfileArgumentValue::Expr(expr)) => {
                    nulls_match = self.resolve_bool_profile_arg("nulls_match", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported join argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "join requires one table root binding and one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let right = self.resolve_table_binding_arg(
            target.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "join requires a right-hand table root binding",
                    &self.options,
                )
            })?,
            "join",
        )?;
        self.lookup_scope.push(right.clone());
        let on = self.resolve_predicate(
            on.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "join requires an on: predicate",
                    &self.options,
                )
            })?,
            root,
        )?;
        self.lookup_scope.pop();
        self.validate_table_lookup_predicate(&on)?;
        let bridge_contract = self.resolve_table_table_bridge(left, &right)?;
        Ok(ResolvedTableJoin {
            right,
            on,
            join_kind,
            cardinality,
            unmatched_policy,
            duplicate_policy,
            nulls_match,
            bridge_contract,
        })
    }

    fn resolve_table_table_bridge(
        &self,
        left: &ResolvedTableRoot,
        right: &ResolvedTableRoot,
    ) -> Result<Option<CoveQlBridgeRegistration>, BuildResolvedQueryError> {
        let bridge = self
            .options
            .bridge_contracts
            .iter()
            .find(|bridge| {
                bridge.source_profile == CoveQlProfileId::Table
                    && bridge.target_profile == CoveQlProfileId::Table
                    && bridge.source_grain == left.row_grain
                    && bridge.target_grain == right.row_grain
                    && bridge.exact
            })
            .cloned();
        let left_domains = &left.table_surface_contract.code_domain_contexts;
        let right_domains = &right.table_surface_contract.code_domain_contexts;
        let domains_need_bridge =
            !left_domains.is_empty() && !right_domains.is_empty() && left_domains != right_domains;
        let authority_needs_bridge = matches!(
            (left.authority_kind, right.authority_kind),
            (
                TableSurfaceAuthorityKind::ExternalRegisteredTable,
                TableSurfaceAuthorityKind::ExternalRegisteredTable
            )
        ) && left.table_id != right.table_id;
        if (domains_need_bridge || authority_needs_bridge) && bridge.is_none() {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                format!(
                    "table join from {} to {} requires an exact CoveQL table bridge for distinct code domains or external authorities",
                    left.table_name, right.table_name
                ),
                &self.options,
            ));
        }
        Ok(bridge)
    }

    fn resolve_table_binding_arg(
        &mut self,
        expr: &Spanned<AstExpr>,
        method: &str,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        let AstExpr::RootBinding(binding) = &expr.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("{method} target must be a table(...) root binding"),
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                format!("{method} target must resolve to a table surface"),
                &self.options,
            ));
        };
        Ok(right)
    }

    fn resolve_join_kind(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableJoinKind, BuildResolvedQueryError> {
        match self.profile_enum_literal("kind", expr)?.as_str() {
            "inner" => Ok(TableJoinKind::Inner),
            "left" => Ok(TableJoinKind::Left),
            "right" => Ok(TableJoinKind::Right),
            "full" => Ok(TableJoinKind::Full),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown join kind enum literal {value}"),
                &self.options,
            )),
        }
    }

    fn resolve_set_operation(
        &mut self,
        name: &str,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedSetOperation, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "set operations are currently declared for CoveQL/Table roots",
                &self.options,
            ));
        }
        let mut target = None;
        let mut all = true;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (None, AstProfileArgumentValue::Expr(expr)) if target.is_none() => {
                    target = Some(expr);
                }
                (Some("all"), AstProfileArgumentValue::Expr(expr)) => {
                    all = self.resolve_bool_profile_arg("all", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported {name} argument on set operation"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "set operation requires one table root binding",
                        &self.options,
                    ));
                }
            }
        }
        let kind = match name {
            "union" => SetOperationKind::Union,
            "intersect" => SetOperationKind::Intersect,
            "except" => SetOperationKind::Except,
            _ => unreachable!("guarded by caller"),
        };
        let right = self.resolve_table_binding_arg(
            target.ok_or_else(|| {
                profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "set operation requires a right-hand table root binding",
                    &self.options,
                )
            })?,
            name,
        )?;
        Ok(ResolvedSetOperation { kind, right, all })
    }

    fn resolve_window_spec(
        &self,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedWindowSpec, BuildResolvedQueryError> {
        let mut partition_by = Vec::new();
        let mut order_by = None;
        let mut frame = WindowFrameKind::Rows;
        let mut start = "unbounded_preceding".to_string();
        let mut end = "current_row".to_string();
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("partitionBy"), AstProfileArgumentValue::Expr(expr)) => {
                    partition_by.push(self.resolve_expr(expr, root)?);
                }
                (Some("orderBy"), AstProfileArgumentValue::Expr(expr)) => {
                    order_by = Some(ResolvedOrderClause {
                        expr: self.resolve_expr(expr, root)?,
                        direction: AstOrderDirection::Asc,
                        nulls: AstNullOrdering::Default,
                        uses_default_ordering: true,
                    });
                }
                (Some("frame"), AstProfileArgumentValue::Expr(expr)) => {
                    frame = match self.profile_enum_literal("frame", expr)?.as_str() {
                        "rows" => WindowFrameKind::Rows,
                        "range" => WindowFrameKind::Range,
                        value => {
                            return Err(profile_rejection(
                                "E_UNKNOWN_ENUM_LITERAL",
                                format!("unsupported window frame {value}"),
                                &self.options,
                            ))
                        }
                    };
                }
                (Some("start"), AstProfileArgumentValue::Expr(expr)) => {
                    start = self.profile_enum_literal("start", expr)?;
                }
                (Some("end"), AstProfileArgumentValue::Expr(expr)) => {
                    end = self.profile_enum_literal("end", expr)?;
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported window argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "window accepts named partitionBy/orderBy/frame/start/end arguments",
                        &self.options,
                    ));
                }
            }
        }
        Ok(ResolvedWindowSpec {
            partition_by,
            order_by,
            frame,
            start,
            end,
        })
    }

    fn resolve_lookup_cardinality(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupCardinality, BuildResolvedQueryError> {
        match self.profile_enum_literal("cardinality", expr)?.as_str() {
            "one" => Ok(TableLookupCardinality::One),
            "many" => Ok(TableLookupCardinality::Many),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup cardinality enum literal {value}"),
                &self.options,
            )),
        }
    }

    fn resolve_lookup_unmatched_policy(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupUnmatchedPolicy, BuildResolvedQueryError> {
        match self.profile_enum_literal("unmatched", expr)?.as_str() {
            "nulls" => Ok(TableLookupUnmatchedPolicy::Nulls),
            "reject" => Ok(TableLookupUnmatchedPolicy::Reject),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup unmatched enum literal {value}"),
                &self.options,
            )),
        }
    }

    fn resolve_lookup_duplicate_policy(
        &self,
        expr: &Spanned<AstExpr>,
    ) -> Result<TableLookupDuplicatePolicy, BuildResolvedQueryError> {
        match self.profile_enum_literal("duplicate", expr)?.as_str() {
            "reject" => Ok(TableLookupDuplicatePolicy::Reject),
            "many" | "emit_all" => Ok(TableLookupDuplicatePolicy::EmitAll),
            value => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("unknown lookup duplicate enum literal {value}"),
                &self.options,
            )),
        }
    }

    fn resolve_bool_profile_arg(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<bool, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Literal(AstLiteral::Boolean(value)) => Ok(*value),
            _ => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("lookup argument {name} requires a boolean literal"),
                &self.options,
            )),
        }
    }

    fn profile_enum_literal(
        &self,
        name: &'static str,
        expr: &Spanned<AstExpr>,
    ) -> Result<String, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Path(path) if path.parts.len() == 1 => Ok(path.parts[0].name.clone()),
            AstExpr::Literal(AstLiteral::String(value)) => Ok(value.clone()),
            _ => Err(profile_rejection(
                "E_UNKNOWN_ENUM_LITERAL",
                format!("lookup argument {name} requires an enum literal"),
                &self.options,
            )),
        }
    }

    fn validate_table_lookup_predicate(
        &self,
        predicate: &ResolvedPredicate,
    ) -> Result<(), BuildResolvedQueryError> {
        match predicate {
            ResolvedPredicate::Compare { left, op, right }
                if *op == AstCompareOp::Eq
                    && matches!(left, ResolvedExpr::Path(_))
                    && matches!(right, ResolvedExpr::Path(_)) =>
            {
                Ok(())
            }
            _ => Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "lookup requires an equality on: predicate between table paths",
                &self.options,
            )),
        }
    }

    fn resolve_graph_traversal(
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

    fn resolve_ai_operation(
        &self,
        name: &str,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedAiOperation, BuildResolvedQueryError> {
        let operation = ai_operation_for_method(name).expect("guarded by caller");
        let mut resolved_args = Vec::with_capacity(args.len());
        for arg in args {
            let value = match &arg.value {
                AstProfileArgumentValue::Expr(expr) => {
                    ResolvedAiArgumentValue::Expr(self.resolve_expr(expr, root)?)
                }
                AstProfileArgumentValue::Predicate(predicate) => {
                    ResolvedAiArgumentValue::Predicate(self.resolve_predicate(predicate, root)?)
                }
            };
            resolved_args.push(ResolvedAiArgument {
                name: arg.name.as_ref().map(|name| name.name.clone()),
                value,
            });
        }
        Ok(ResolvedAiOperation {
            operation,
            method_name: name.into(),
            args: resolved_args,
            sidecar_required: true,
            authority: ai_authority(operation).into(),
            policy_scope: ai_policy_scope(root).into(),
        })
    }

    fn resolve_graph_algorithm(
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
        let kind = graph_algorithm_kind(name).expect("guarded by caller");
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

    fn graph_traversal_contract_for(
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

    fn validate_graph_algorithm_variant(
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

    fn graph_algorithm_contract_for(
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

    fn validate_graph_algorithm_contract(
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

    fn resolve_usize_profile_arg(
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

    fn validate_graph_traversal_contract(
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

    fn resolve_graph_traversal_expr(
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

    fn resolve_traversal_depth_arg(
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

    fn resolve_predicate(
        &self,
        predicate: &Spanned<AstPredicate>,
        root: &ResolvedRoot,
    ) -> Result<ResolvedPredicate, BuildResolvedQueryError> {
        match &predicate.node {
            AstPredicate::Compare { left, op, right } => {
                let mut left = self.resolve_expr(left, root)?;
                let mut right = self.resolve_expr(right, root)?;
                self.coerce_compare_literals(&mut left, &mut right)?;
                Ok(ResolvedPredicate::Compare {
                    left,
                    op: *op,
                    right,
                })
            }
            AstPredicate::InList { expr, values } => {
                let expr = self.resolve_expr(expr, root)?;
                let target_type = expr_logical_type(&expr).map(str::to_owned);
                let values = values
                    .iter()
                    .map(|literal| {
                        let literal = self.resolve_literal(&literal.node)?;
                        if let Some(target_type) = target_type.as_deref() {
                            self.coerce_literal_to_type(literal, target_type)
                        } else {
                            Ok(literal)
                        }
                    })
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?;
                Ok(ResolvedPredicate::InList { expr, values })
            }
            AstPredicate::NullCheck { expr, negated } => Ok(ResolvedPredicate::NullCheck {
                expr: self.resolve_expr(expr, root)?,
                negated: *negated,
            }),
            AstPredicate::Exists { target, args } => {
                if !args.is_empty() || matches!(target.node, AstExpr::RootBinding(_)) {
                    let expr = self.resolve_profile_exists(target, args, root)?;
                    self.validate_exists_disclosure(&expr, false)?;
                    return Ok(ResolvedPredicate::Exists(expr));
                }
                if let AstExpr::Relationship(relationship) = &target.node {
                    let expr = ResolvedExpr::Association(
                        self.resolve_relationship_association(relationship, root)?,
                    );
                    self.validate_exists_disclosure(&expr, false)?;
                    return Ok(ResolvedPredicate::Exists(expr));
                }
                let expr = self.resolve_expr(target, root)?;
                self.validate_exists_disclosure(&expr, false)?;
                Ok(ResolvedPredicate::Exists(expr))
            }
            AstPredicate::BoolExpr(expr) => {
                let expr = self.resolve_expr(expr, root)?;
                Ok(canonicalize_null_check_predicate(expr))
            }
            AstPredicate::Not(inner) => {
                let inner = self.resolve_predicate(inner, root)?;
                self.validate_predicate_disclosure(&inner, true)?;
                Ok(ResolvedPredicate::Not(Box::new(inner)))
            }
            AstPredicate::And(parts) => Ok(ResolvedPredicate::And(
                parts
                    .iter()
                    .map(|part| self.resolve_predicate(part, root))
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
            )),
            AstPredicate::Or(parts) => Ok(ResolvedPredicate::Or(
                parts
                    .iter()
                    .map(|part| self.resolve_predicate(part, root))
                    .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?,
            )),
        }
    }

    fn resolve_profile_exists(
        &self,
        target: &Spanned<AstExpr>,
        args: &[AstProfileArgument],
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Table(_)) {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "profile-aware exists(root, on: ...) requires a declared bridge from the current root grain to CoveQL/Table",
                &self.options,
            ));
        }
        let AstExpr::RootBinding(binding) = &target.node else {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "profile-aware exists requires a table(...) root binding target",
                &self.options,
            ));
        };
        let mut right_root = self.resolve_root(&binding.root.node)?;
        apply_root_alias(&mut right_root, binding.alias.as_ref());
        let ResolvedRoot::Table(right) = right_root else {
            return Err(profile_rejection(
                "E_UNKNOWN_BRIDGE",
                "profile-aware exists target requires a declared bridge to a CoveQL/Table surface",
                &self.options,
            ));
        };
        let mut on = None;
        for arg in args {
            match (arg.name.as_ref().map(|name| name.name.as_str()), &arg.value) {
                (Some("on"), AstProfileArgumentValue::Predicate(predicate)) if on.is_none() => {
                    on = Some(predicate);
                }
                (Some(name), _) => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        format!("unsupported exists argument {name}"),
                        &self.options,
                    ));
                }
                _ => {
                    return Err(profile_rejection(
                        "E_UNSUPPORTED_PROFILE_METHOD",
                        "profile-aware exists requires one named on: predicate",
                        &self.options,
                    ));
                }
            }
        }
        let on = on.ok_or_else(|| {
            profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "profile-aware exists requires an on: predicate",
                &self.options,
            )
        })?;
        let scoped = self.clone_for_lookup_scope(right.clone());
        let on = scoped.resolve_predicate(on, root)?;
        scoped.validate_table_lookup_predicate(&on)?;
        Ok(ResolvedExpr::TableExists(ResolvedTableExists {
            right,
            on: Box::new(on),
        }))
    }

    fn clone_for_lookup_scope(&self, right: ResolvedTableRoot) -> Self {
        let mut resolver = Self {
            surface: self.surface.clone(),
            options: self.options.clone(),
            diagnostics: Vec::new(),
            lookup_scope: self.lookup_scope.clone(),
            cte_table_scope: self.cte_table_scope.clone(),
            graph_node_scope: self.graph_node_scope.clone(),
            graph_edge_scope: self.graph_edge_scope.clone(),
            window_function_scope: self.window_function_scope,
        };
        resolver.lookup_scope.push(right);
        resolver
    }

    fn resolve_expr(
        &self,
        expr: &Spanned<AstExpr>,
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        match &expr.node {
            AstExpr::Path(path) => Ok(ResolvedExpr::Path(self.resolve_path(path, root)?)),
            AstExpr::Literal(literal) => Ok(ResolvedExpr::Literal(self.resolve_literal(literal)?)),
            AstExpr::FunctionCall { name, args } => self.resolve_function(name, args, root),
            AstExpr::AggregateCall { name, arg, star } => {
                if self.window_function_scope {
                    let function_id = window_aggregate_function_name(*name).ok_or_else(|| {
                        self.function_contract_error(
                            "aggregate is not a supported CoveQL window function",
                            aggregate_function_name(*name),
                        )
                    })?;
                    let mut resolved_args = Vec::new();
                    if let Some(arg) = arg.as_deref() {
                        resolved_args.push(self.resolve_expr(arg, root)?);
                    } else if !star && *name != AstAggregateName::Count {
                        return Err(self.function_contract_error(
                            "window aggregate requires an argument",
                            function_id,
                        ));
                    }
                    let contract = self.resolve_function_contract(function_id, &resolved_args)?;
                    return Ok(ResolvedExpr::FunctionCall {
                        function_id: function_id.into(),
                        deterministic: contract.deterministic,
                        logical_type: function_logical_type(function_id, &resolved_args),
                        physical_kind: function_physical_kind(function_id, &resolved_args).into(),
                        contract,
                        args: resolved_args,
                    });
                }
                let arg = arg
                    .as_deref()
                    .map(|arg| self.resolve_expr(arg, root).map(Box::new))
                    .transpose()?;
                if let Some(arg) = arg.as_deref() {
                    self.validate_aggregate_disclosure(*name, arg)?;
                }
                Ok(ResolvedExpr::AggregateCall {
                    name: *name,
                    arg,
                    star: *star,
                    logical_type: aggregate_logical_type(*name).into(),
                    aggregate_disclosure: format!(
                        "{:?}",
                        self.options.security.aggregate_disclosure_policy
                    )
                    .to_ascii_lowercase(),
                })
            }
            AstExpr::Association(association) => {
                Ok(ResolvedExpr::Association(self.resolve_association_root(
                    association,
                    Some(root),
                    AssociationResolveUsage::ObjectRelative,
                )?))
            }
            AstExpr::Evidence(evidence) => Ok(ResolvedExpr::Evidence(
                self.resolve_evidence_root(evidence, Some(root))?,
            )),
            AstExpr::Relationship(relationship) => Ok(ResolvedExpr::Association(
                self.resolve_relationship_association(relationship, root)?,
            )),
            AstExpr::RootBinding(_) => Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE",
                "profile root expressions are valid only where a profile method, evidence target, or exists expression declares root-binding semantics",
                &self.options,
            )),
            AstExpr::Conditional {
                predicate,
                then_expr,
                else_expr,
            } => {
                let predicate = self.resolve_predicate(predicate, root)?;
                let then_expr = self.resolve_expr(then_expr, root)?;
                let else_expr = self.resolve_expr(else_expr, root)?;
                let logical_type = common_output_logical_type(&then_expr, &else_expr).into();
                Ok(ResolvedExpr::Conditional {
                    predicate: Box::new(predicate),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                    logical_type,
                })
            }
        }
    }

    fn coerce_compare_literals(
        &self,
        left: &mut ResolvedExpr,
        right: &mut ResolvedExpr,
    ) -> Result<(), BuildResolvedQueryError> {
        let left_type = expr_logical_type(left).map(str::to_owned);
        let right_type = expr_logical_type(right).map(str::to_owned);
        if let (ResolvedExpr::Literal(literal), Some(target_type)) = (left, right_type.as_deref()) {
            *literal = self.coerce_literal_to_type(literal.clone(), target_type)?;
        }
        if let (Some(target_type), ResolvedExpr::Literal(literal)) = (left_type.as_deref(), right) {
            *literal = self.coerce_literal_to_type(literal.clone(), target_type)?;
        }
        Ok(())
    }

    fn coerce_literal_to_type(
        &self,
        literal: ResolvedLiteral,
        target_type: &str,
    ) -> Result<ResolvedLiteral, BuildResolvedQueryError> {
        match target_type {
            "timestamp_micros" => {
                let AstLiteral::String(value) = &literal.literal else {
                    return Ok(literal);
                };
                let (micros, canonical_rfc3339) = timestamp_micros(value, &self.options.security)?;
                Ok(ResolvedLiteral {
                    literal: AstLiteral::Timestamp(value.clone()),
                    logical_type: "timestamp_micros".into(),
                    canonical: format!("{canonical_rfc3339}:{micros}"),
                    typed_value: ResolvedLiteralValue::TimestampMicros {
                        micros,
                        canonical_rfc3339,
                    },
                    precision: None,
                    scale: None,
                })
            }
            "uuid" => match &literal.literal {
                AstLiteral::String(value) | AstLiteral::Uuid(value) => {
                    let Some((bytes, canonical_hex)) = parse_uuid_literal(value) else {
                        return Err(BuildResolvedQueryError::single(diagnostic(
                            "E_LITERAL",
                            "malformed UUID literal",
                            "resolve",
                            &self.options.security,
                        )));
                    };
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Uuid(value.clone()),
                        logical_type: "uuid".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Uuid {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                _ => Ok(literal),
            },
            "binary" => match &literal.literal {
                AstLiteral::String(value) => {
                    let bytes = value.as_bytes().to_vec();
                    let canonical_hex = crate::hex_lower(&bytes);
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Binary(canonical_hex.clone()),
                        logical_type: "binary".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Binary {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                AstLiteral::Binary(value) => {
                    let Some(bytes) = decode_hex_bytes(value) else {
                        return Err(BuildResolvedQueryError::single(diagnostic(
                            "E_LITERAL",
                            "malformed binary literal",
                            "resolve",
                            &self.options.security,
                        )));
                    };
                    let canonical_hex = value.to_ascii_lowercase();
                    Ok(ResolvedLiteral {
                        literal: AstLiteral::Binary(canonical_hex.clone()),
                        logical_type: "binary".into(),
                        canonical: canonical_hex.clone(),
                        typed_value: ResolvedLiteralValue::Binary {
                            canonical_hex,
                            bytes,
                        },
                        precision: None,
                        scale: None,
                    })
                }
                _ => Ok(literal),
            },
            _ => Ok(literal),
        }
    }

    fn resolve_function(
        &self,
        name: &AstIdentifier,
        args: &[Spanned<AstExpr>],
        root: &ResolvedRoot,
    ) -> Result<ResolvedExpr, BuildResolvedQueryError> {
        let resolved_args = args
            .iter()
            .map(|arg| self.resolve_expr(arg, root))
            .collect::<Result<Vec<_>, BuildResolvedQueryError>>()?;
        let contract = self.resolve_function_contract(&name.name, &resolved_args)?;
        Ok(ResolvedExpr::FunctionCall {
            function_id: name.name.clone(),
            deterministic: contract.deterministic,
            logical_type: function_logical_type(&name.name, &resolved_args),
            physical_kind: function_physical_kind(&name.name, &resolved_args).into(),
            contract,
            args: resolved_args,
        })
    }

    fn resolve_function_contract(
        &self,
        name: &str,
        args: &[ResolvedExpr],
    ) -> Result<ResolvedFunctionContract, BuildResolvedQueryError> {
        self.validate_builtin_function_signature(name, args)?;
        match name {
            "coalesce" | "isNull" | "isNotNull" => {
                let execution_class = builtin_function_execution_class(name, args);
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-0.1".into(),
                    deterministic: true,
                    dependency: "pure".into(),
                    execution_class,
                    unicode_or_collation_contract: None,
                })
            }
            "startsWith" | "length" => {
                if let Some(function) = self.deterministic_function_entry(name) {
                    if declares_exact_string_function_dependency(name, &function.dependency) {
                        return Ok(ResolvedFunctionContract {
                            function_id: function.function_id.clone(),
                            version: function.version.clone(),
                            deterministic: true,
                            dependency: function.dependency.clone(),
                            execution_class: FunctionExecutionClass::CodedSafe,
                            unicode_or_collation_contract: Some(function.dependency.clone()),
                        });
                    }
                }
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-0.1".into(),
                    deterministic: true,
                    dependency: "materialized-string-built-in".into(),
                    execution_class: FunctionExecutionClass::MaterializedOnly,
                    unicode_or_collation_contract: None,
                })
            }
            "cast" => {
                let execution_class = builtin_function_execution_class(name, args);
                if execution_class == FunctionExecutionClass::CodedSafe {
                    return Ok(ResolvedFunctionContract {
                        function_id: name.into(),
                        version: "coveql-0.1".into(),
                        deterministic: true,
                        dependency: "cove-map-safe-cast:coded-identity".into(),
                        execution_class,
                        unicode_or_collation_contract: None,
                    });
                }
                let Some(function) = self.deterministic_function_entry(name) else {
                    return Err(self.function_contract_error(
                        "non-identity cast requires deterministic COVE-MAP safe-cast metadata",
                        name,
                    ));
                };
                if !declares_safe_cast_dependency(&function.dependency) {
                    return Err(self.function_contract_error(
                        "non-identity cast requires COVE-MAP safe-cast dependency metadata",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: function.function_id.clone(),
                    version: function.version.clone(),
                    deterministic: true,
                    dependency: function.dependency.clone(),
                    execution_class,
                    unicode_or_collation_contract: None,
                })
            }
            "identity" | "trim" | "lower" | "lowercase" | "upper" | "uppercase" => {
                let Some(function) = self.deterministic_function_entry(name) else {
                    return Err(self.function_contract_error(
                        "function requires deterministic COVE-MAP metadata",
                        name,
                    ));
                };
                if matches!(name, "trim" | "lower" | "lowercase" | "upper" | "uppercase")
                    && !declares_unicode_or_collation_dependency(&function.dependency)
                {
                    return Err(self.function_contract_error(
                        "function requires Unicode/collation version metadata in the function dependency",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: function.function_id.clone(),
                    version: function.version.clone(),
                    deterministic: true,
                    dependency: function.dependency.clone(),
                    execution_class: FunctionExecutionClass::CodedSafe,
                    unicode_or_collation_contract: Some(function.dependency.clone()),
                })
            }
            "exists" => Ok(ResolvedFunctionContract {
                function_id: name.into(),
                version: "coveql-0.1".into(),
                deterministic: true,
                dependency: "pure".into(),
                execution_class: FunctionExecutionClass::MaterializedOnly,
                unicode_or_collation_contract: None,
            }),
            name if is_window_function_name(name) => {
                if !self.window_function_scope {
                    return Err(self.function_contract_error(
                        "window function requires a window(...) method",
                        name,
                    ));
                }
                Ok(ResolvedFunctionContract {
                    function_id: name.into(),
                    version: "coveql-relational-window-1".into(),
                    deterministic: true,
                    dependency: "materialized-window-frame".into(),
                    execution_class: FunctionExecutionClass::MaterializedOnly,
                    unicode_or_collation_contract: None,
                })
            }
            _ if self.deterministic_function_entry(name).is_some() => Err(self
                .function_contract_error(
                    "registered deterministic function has no CoveQL executable body",
                    name,
                )),
            _ => Err(self.unknown(
                "E_UNSUPPORTED_FUNCTION",
                "function is not registered as deterministic COVE-MAP metadata",
                Some(name),
            )),
        }
    }

    fn validate_builtin_function_signature(
        &self,
        name: &str,
        args: &[ResolvedExpr],
    ) -> Result<(), BuildResolvedQueryError> {
        let arity_ok = match name {
            "coalesce" => !args.is_empty(),
            "identity" | "trim" | "lower" | "lowercase" | "upper" | "uppercase" | "length"
            | "exists" | "isNull" | "isNotNull" => args.len() == 1,
            "row_number" | "rank" | "dense_rank" => args.is_empty(),
            "lag" | "lead" => args.len() == 1 || args.len() == 2,
            "first_value" | "last_value" | "sum" | "avg" | "min" | "max" => args.len() == 1,
            "count" => args.len() <= 1,
            "startsWith" | "cast" => args.len() == 2,
            _ => true,
        };
        if !arity_ok {
            return Err(self.function_contract_error("invalid function arity", name));
        }
        let string_arg = |arg: &ResolvedExpr| {
            matches!(
                expr_logical_type(arg),
                Some("utf8" | "string" | "json" | "unknown")
            )
        };
        let types_ok = match name {
            "coalesce" => coalesce_common_logical_type(args).is_some(),
            "trim" | "lower" | "lowercase" | "upper" | "uppercase" => {
                args.first().is_some_and(string_arg)
            }
            "startsWith" => args.iter().all(string_arg),
            "cast" => cast_target_logical_type(args).is_some(),
            _ => true,
        };
        if !types_ok {
            return Err(self.function_contract_error(
                "function argument types do not satisfy the deterministic function contract",
                name,
            ));
        }
        Ok(())
    }

    fn deterministic_function_entry(
        &self,
        name: &str,
    ) -> Option<&cove_core::profile::cove_map::MapFunctionEntry> {
        self.surface
            .embedded_map_sections
            .iter()
            .find_map(|section| match section {
                EmbeddedMapSection::FunctionRegistry(registry) => registry
                    .functions
                    .iter()
                    .find(|function| function.function_id == name && function.deterministic),
                _ => None,
            })
    }

    fn function_contract_error(
        &self,
        message: &'static str,
        name: &str,
    ) -> BuildResolvedQueryError {
        let mut diagnostic = diagnostic(
            "E_FUNCTION_CONTRACT",
            message,
            "resolve",
            &self.options.security,
        );
        if self.options.security.metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected
        {
            diagnostic.safe_details = json!({ "function_id": name });
            diagnostic.redacted = false;
        }
        BuildResolvedQueryError {
            diagnostics: vec![diagnostic],
            rejections: vec![RejectionReport {
                kind: RejectionKind::FeatureValidation,
                reason: message.into(),
            }],
            source: None,
        }
    }

    fn validate_exists_disclosure(
        &self,
        expr: &ResolvedExpr,
        negated: bool,
    ) -> Result<(), BuildResolvedQueryError> {
        match expr {
            ResolvedExpr::Association(_) if negated && !self.protected_metadata_allowed() => {
                Err(self.disclosure_error(
                    "E_PROTECTED_ASSOCIATION_EXISTENCE",
                    "association non-existence disclosure requires protected metadata disclosure permission",
                ))
            }
            ResolvedExpr::Evidence(_) if !self.protected_metadata_allowed() => {
                Err(self.disclosure_error(
                    "E_PROTECTED_EVIDENCE_EXISTENCE",
                    "evidence existence disclosure requires protected metadata disclosure permission",
                ))
            }
            _ => Ok(()),
        }
    }

    fn validate_predicate_disclosure(
        &self,
        predicate: &ResolvedPredicate,
        negated: bool,
    ) -> Result<(), BuildResolvedQueryError> {
        match predicate {
            ResolvedPredicate::Exists(expr) => self.validate_exists_disclosure(expr, negated),
            ResolvedPredicate::Not(inner) => self.validate_predicate_disclosure(inner, !negated),
            ResolvedPredicate::And(parts) | ResolvedPredicate::Or(parts) => {
                for part in parts {
                    self.validate_predicate_disclosure(part, negated)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn validate_aggregate_disclosure(
        &self,
        name: AstAggregateName,
        arg: &ResolvedExpr,
    ) -> Result<(), BuildResolvedQueryError> {
        if !matches!(
            name,
            AstAggregateName::Count | AstAggregateName::Exists | AstAggregateName::DistinctCount
        ) {
            return Ok(());
        }
        match self.options.security.aggregate_disclosure_policy {
            crate::AggregateDisclosurePolicy::AllowExact
            | crate::AggregateDisclosurePolicy::AllowMaterializedOnly
            | crate::AggregateDisclosurePolicy::AllowThresholded
            | crate::AggregateDisclosurePolicy::AllowRedacted => {}
            crate::AggregateDisclosurePolicy::Reject => {
                return Err(self.disclosure_error(
                    "E_AGGREGATE_DISCLOSURE_FORBIDDEN",
                    "association/evidence aggregate disclosure is forbidden by the active policy",
                ))
            }
        }
        if !self.protected_metadata_allowed() {
            if matches!(arg, ResolvedExpr::Association(_)) {
                return Err(self.disclosure_error(
                    "E_PROTECTED_ASSOCIATION_EXISTENCE",
                    "association count disclosure requires protected metadata disclosure permission",
                ));
            }
            if matches!(arg, ResolvedExpr::Evidence(_)) {
                return Err(self.disclosure_error(
                    "E_PROTECTED_EVIDENCE_EXISTENCE",
                    "evidence count disclosure requires protected metadata disclosure permission",
                ));
            }
        }
        Ok(())
    }

    fn protected_metadata_allowed(&self) -> bool {
        self.options.security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected
    }

    fn disclosure_error(
        &self,
        code: &'static str,
        message: &'static str,
    ) -> BuildResolvedQueryError {
        BuildResolvedQueryError {
            diagnostics: vec![diagnostic(code, message, "resolve", &self.options.security)],
            rejections: vec![RejectionReport {
                kind: RejectionKind::SecurityPolicy,
                reason: message.into(),
            }],
            source: None,
        }
    }

    fn resolve_path(
        &self,
        path: &AstPath,
        root: &ResolvedRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        match root {
            ResolvedRoot::Object(object) => self.resolve_object_path(path, object),
            ResolvedRoot::Association(association) => {
                self.resolve_association_path(path, association)
            }
            ResolvedRoot::Node(node) => self.resolve_graph_path_with_scope(path, node),
            ResolvedRoot::Edge(edge) => self.resolve_graph_edge_path(path, edge),
            ResolvedRoot::Table(table) => self.resolve_table_path_with_scope(path, table),
            ResolvedRoot::Projection(projection) => self.resolve_projection_path(path, projection),
            ResolvedRoot::Evidence(evidence) => self.resolve_evidence_path(path, evidence),
        }
    }

    fn resolve_object_path(
        &self,
        path: &AstPath,
        object: &ResolvedObjectRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 2 && parts[0] == object.type_name {
            parts[1].as_str()
        } else if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "path is ambiguous for selected object root",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = object_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Object,
                Some(object.object_type_id),
                system,
            ));
        }
        let object_type = self.resolve_object_type(&object.type_name)?;
        let property = find_property(object_type, field).ok_or_else(|| {
            self.unknown(
                "E_UNKNOWN_PROPERTY",
                "unknown object property for selected root",
                Some(field),
            )
        })?;
        Ok(property_resolved_path(
            field,
            ResolvedPathRootKind::Object,
            Some(object.object_type_id),
            None,
            property,
        ))
    }

    fn resolve_association_path(
        &self,
        path: &AstPath,
        association: &ResolvedAssociationRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "association path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = association_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Association,
                Some(association.object_type_id),
                system,
            ));
        }
        let object_type = self.resolve_object_type(&association.type_name)?;
        let property = find_property(object_type, field).ok_or_else(|| {
            self.unknown(
                "E_UNKNOWN_PROPERTY",
                "unknown association property for selected root",
                Some(field),
            )
        })?;
        Ok(property_resolved_path(
            field,
            ResolvedPathRootKind::Association,
            Some(association.object_type_id),
            Some(association.object_type_id),
            property,
        ))
    }

    fn resolve_graph_node_path(
        &self,
        path: &AstPath,
        node: &ResolvedGraphNodeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [field] = parts.as_slice() {
            if let Some(logical_type) = graph_algorithm_output_logical_type(field) {
                return Ok(ResolvedPath {
                    display_name: field.to_string(),
                    root_kind: ResolvedPathRootKind::Node,
                    object_type_id: None,
                    property_id: None,
                    association_type_id: None,
                    evidence_field_id: None,
                    projection_id: None,
                    projection_column: Some(field.to_string()),
                    system_field: None,
                    logical_type: logical_type.into(),
                    physical_kind: physical_kind_for_logical_name(logical_type).into(),
                    collation_id: None,
                    nullable: true,
                    null_policy: "generated_algorithm_field".into(),
                    temporal_role: None,
                    code_domain_id: CodeDomainId::Placeholder {
                        root: "graph_algorithm".into(),
                        object_type_id: None,
                        property_id: None,
                        projection_id: None,
                        field: Some(field.to_string()),
                    },
                });
            }
        }
        let path = path_without_graph_binding(path, &node.label, node.binding_name.as_deref())?;
        let mut resolved = self.resolve_object_path(&path, &node.object)?;
        resolved.root_kind = ResolvedPathRootKind::Node;
        if let [binding, field] = parts.as_slice() {
            resolved.projection_column = Some(format!("{binding}.{field}"));
        }
        let CodeDomainId::Placeholder { root, .. } = &mut resolved.code_domain_id;
        *root = "node".into();
        Ok(resolved)
    }

    fn resolve_graph_edge_path(
        &self,
        path: &AstPath,
        edge: &ResolvedGraphEdgeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let path = path_without_graph_binding(path, &edge.label, edge.binding_name.as_deref())?;
        let mut resolved = self.resolve_association_path(&path, &edge.association)?;
        resolved.root_kind = ResolvedPathRootKind::Edge;
        if let [binding, field] = parts.as_slice() {
            resolved.projection_column = Some(format!("{binding}.{field}"));
        }
        let CodeDomainId::Placeholder { root, .. } = &mut resolved.code_domain_id;
        *root = "edge".into();
        Ok(resolved)
    }

    fn resolve_graph_path_with_scope(
        &self,
        path: &AstPath,
        node: &ResolvedGraphNodeRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [binding, ..] = parts.as_slice() {
            if binding == &node.label
                || node
                    .binding_name
                    .as_ref()
                    .is_some_and(|alias| binding == alias)
            {
                return self.resolve_graph_node_path(path, node);
            }
            if let Some(scoped_node) = self.graph_node_scope.iter().find(|scoped| {
                binding == &scoped.label
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_graph_node_path(path, scoped_node);
            }
            if let Some(scoped_edge) = self.graph_edge_scope.iter().find(|scoped| {
                binding == &scoped.label
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_graph_edge_path(path, scoped_edge);
            }
        }
        self.resolve_graph_node_path(path, node)
    }

    fn resolve_projection_path(
        &self,
        path: &AstPath,
        projection: &ResolvedProjectionRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "projection path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        let catalog = self.projection_catalog()?;
        let entry = catalog
            .projections
            .iter()
            .find(|entry| entry.projection_id == projection.projection_id)
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_PROJECTION",
                    "unknown projection",
                    Some(&projection.projection_id),
                )
            })?;
        let column = entry
            .columns
            .iter()
            .find(|column| column.name == field)
            .ok_or_else(|| {
                self.unknown("E_UNKNOWN_PATH", "unknown projection column", Some(field))
            })?;
        let logical = column.logical_type.as_deref().unwrap_or("utf8");
        Ok(ResolvedPath {
            display_name: field.into(),
            root_kind: ResolvedPathRootKind::Projection,
            object_type_id: None,
            property_id: None,
            association_type_id: None,
            evidence_field_id: None,
            projection_id: Some(projection.projection_id.clone()),
            projection_column: Some(field.into()),
            system_field: None,
            logical_type: logical.into(),
            physical_kind: physical_kind_for_logical_name(logical).into(),
            collation_id: None,
            nullable: column.missing_policy != "reject",
            null_policy: column.missing_policy.clone(),
            temporal_role: None,
            code_domain_id: CodeDomainId::Placeholder {
                root: "projection".into(),
                object_type_id: None,
                property_id: None,
                projection_id: Some(projection.projection_id.clone()),
                field: Some(field.into()),
            },
        })
    }

    fn resolve_table_path_with_scope(
        &self,
        path: &AstPath,
        table: &ResolvedTableRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        if let [binding, ..] = parts.as_slice() {
            if binding == &table.table_name
                || table
                    .binding_name
                    .as_ref()
                    .is_some_and(|alias| binding == alias)
            {
                return self.resolve_table_path_for_table(path, table);
            }
            if let Some(scoped) = self.lookup_scope.iter().find(|scoped| {
                binding == &scoped.table_name
                    || scoped
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias)
            }) {
                return self.resolve_table_path_for_table(path, scoped);
            }
        }
        self.resolve_table_path_for_table(path, table)
    }

    fn resolve_table_path_for_table(
        &self,
        path: &AstPath,
        table: &ResolvedTableRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let (field, qualified_binding) = match parts.as_slice() {
            [field] => (field.as_str(), None),
            [binding, field]
                if binding == &table.table_name
                    || table
                        .binding_name
                        .as_ref()
                        .is_some_and(|alias| binding == alias) =>
            {
                (field.as_str(), Some(binding.as_str()))
            }
            _ => {
                return Err(self.unknown(
                    "E_AMBIGUOUS_PATH",
                    "table path is ambiguous",
                    Some(&parts.join(".")),
                ));
            }
        };
        let column = table
            .projection
            .columns
            .iter()
            .find(|column| column.name == field)
            .ok_or_else(|| self.unknown("E_UNKNOWN_PATH", "unknown table column", Some(field)))?;
        let logical = column.logical_type.as_deref().unwrap_or("utf8");
        let value_key = qualified_binding
            .map(|binding| format!("{binding}.{field}"))
            .unwrap_or_else(|| field.to_string());
        Ok(ResolvedPath {
            display_name: field.into(),
            root_kind: ResolvedPathRootKind::Table,
            object_type_id: None,
            property_id: None,
            association_type_id: None,
            evidence_field_id: None,
            projection_id: Some(table.projection.projection_id.clone()),
            projection_column: Some(value_key.clone()),
            system_field: None,
            logical_type: logical.into(),
            physical_kind: physical_kind_for_logical_name(logical).into(),
            collation_id: None,
            nullable: column.missing_policy != "reject",
            null_policy: column.missing_policy.clone(),
            temporal_role: None,
            code_domain_id: CodeDomainId::Placeholder {
                root: "table".into(),
                object_type_id: None,
                property_id: None,
                projection_id: Some(table.projection.projection_id.clone()),
                field: Some(value_key),
            },
        })
    }

    fn resolve_evidence_path(
        &self,
        path: &AstPath,
        _evidence: &ResolvedEvidenceRoot,
    ) -> Result<ResolvedPath, BuildResolvedQueryError> {
        let parts = path_names(path);
        let field = if parts.len() == 1 {
            parts[0].as_str()
        } else {
            return Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "evidence path is ambiguous",
                Some(&parts.join(".")),
            ));
        };
        if let Some(system) = evidence_system_field(field) {
            return Ok(system_resolved_path(
                field,
                ResolvedPathRootKind::Evidence,
                None,
                system,
            ));
        }
        if self.evidence_metadata_key_exists(field) {
            return Ok(ResolvedPath {
                display_name: field.into(),
                root_kind: ResolvedPathRootKind::Evidence,
                object_type_id: None,
                property_id: None,
                association_type_id: None,
                evidence_field_id: Some(field.into()),
                projection_id: None,
                projection_column: None,
                system_field: None,
                logical_type: "json".into(),
                physical_kind: "var_bytes".into(),
                collation_id: None,
                nullable: true,
                null_policy: "missing_is_null".into(),
                temporal_role: None,
                code_domain_id: CodeDomainId::Placeholder {
                    root: "evidence".into(),
                    object_type_id: None,
                    property_id: None,
                    projection_id: None,
                    field: Some(field.into()),
                },
            });
        }
        Err(self.unknown("E_UNKNOWN_PATH", "unknown evidence field", Some(field)))
    }

    fn resolve_literal(
        &self,
        literal: &AstLiteral,
    ) -> Result<ResolvedLiteral, BuildResolvedQueryError> {
        match literal {
            AstLiteral::Null => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "null".into(),
                canonical: "null".into(),
                typed_value: ResolvedLiteralValue::Null,
                precision: None,
                scale: None,
            }),
            AstLiteral::Boolean(value) => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "bool".into(),
                canonical: value.to_string(),
                typed_value: ResolvedLiteralValue::Boolean(*value),
                precision: None,
                scale: None,
            }),
            AstLiteral::String(value) => Ok(ResolvedLiteral {
                literal: literal.clone(),
                logical_type: "utf8".into(),
                canonical: value.clone(),
                typed_value: ResolvedLiteralValue::String(value.clone()),
                precision: None,
                scale: None,
            }),
            AstLiteral::Integer(value) => {
                let parsed = value.parse::<i128>().map_err(|_| {
                    BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "integer literal is out of range",
                        "resolve",
                        &self.options.security,
                    ))
                })?;
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: if parsed < 0 { "int64" } else { "uint64" }.into(),
                    canonical: parsed.to_string(),
                    typed_value: integer_literal_value(parsed),
                    precision: Some(value.trim_start_matches('-').len() as u32),
                    scale: Some(0),
                })
            }
            AstLiteral::Decimal(value) => {
                let unsigned = value.trim_start_matches('-');
                let mut split = unsigned.split('.');
                let whole = split.next().unwrap_or("");
                let fractional = split.next().unwrap_or("");
                if split.next().is_some() {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed decimal literal",
                        "resolve",
                        &self.options.security,
                    )));
                }
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "decimal128".into(),
                    canonical: value.clone(),
                    typed_value: ResolvedLiteralValue::Decimal {
                        canonical: value.clone(),
                        precision: (whole.len() + fractional.len()) as u32,
                        scale: fractional.len() as u32,
                    },
                    precision: Some((whole.len() + fractional.len()) as u32),
                    scale: Some(fractional.len() as u32),
                })
            }
            AstLiteral::Timestamp(value) => {
                let (micros, canonical) = timestamp_micros(value, &self.options.security)?;
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "timestamp_micros".into(),
                    canonical: format!("{canonical}:{micros}"),
                    typed_value: ResolvedLiteralValue::TimestampMicros {
                        micros,
                        canonical_rfc3339: canonical,
                    },
                    precision: None,
                    scale: None,
                })
            }
            AstLiteral::Uuid(value) => {
                let Some((bytes, canonical_hex)) = parse_uuid_literal(value) else {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed UUID literal",
                        "resolve",
                        &self.options.security,
                    )));
                };
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "uuid".into(),
                    canonical: canonical_hex.clone(),
                    typed_value: ResolvedLiteralValue::Uuid {
                        canonical_hex,
                        bytes,
                    },
                    precision: None,
                    scale: None,
                })
            }
            AstLiteral::Binary(value) => {
                let Some(bytes) = decode_hex_bytes(value) else {
                    return Err(BuildResolvedQueryError::single(diagnostic(
                        "E_LITERAL",
                        "malformed binary literal",
                        "resolve",
                        &self.options.security,
                    )));
                };
                let canonical_hex = value.to_ascii_lowercase();
                Ok(ResolvedLiteral {
                    literal: literal.clone(),
                    logical_type: "binary".into(),
                    canonical: canonical_hex.clone(),
                    typed_value: ResolvedLiteralValue::Binary {
                        canonical_hex,
                        bytes,
                    },
                    precision: None,
                    scale: None,
                })
            }
        }
    }

    fn resolve_change_bound(
        &self,
        bound: &AstChangeBound,
    ) -> Result<ResolvedTimeBound, BuildResolvedQueryError> {
        match bound {
            AstChangeBound::Csn(value) => Ok(ResolvedTimeBound::Csn(*value)),
            AstChangeBound::Timestamp { role, timestamp } => {
                let (role, binding) = temporal_role(*role, &self.options)?;
                let (timestamp_micros, canonical_rfc3339) =
                    timestamp_micros(timestamp, &self.options.security)?;
                Ok(ResolvedTimeBound::TimestampMicros {
                    role,
                    binding,
                    timestamp_micros,
                    canonical_rfc3339,
                })
            }
        }
    }

    fn resolve_object_type(
        &self,
        name: &str,
    ) -> Result<&ObjectTypeEntryV1, BuildResolvedQueryError> {
        let matches = self
            .surface
            .object_types
            .iter()
            .filter(|object_type| object_type.type_name == name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [object_type] => Ok(*object_type),
            [] => Err(self.unknown("E_UNKNOWN_OBJECT_TYPE", "unknown object type", Some(name))),
            _ => Err(self.unknown(
                "E_AMBIGUOUS_PATH",
                "object type name is ambiguous",
                Some(name),
            )),
        }
    }

    fn resolve_association_root(
        &self,
        association: &AstAssociationExpr,
        contextual_root: Option<&ResolvedRoot>,
        usage: AssociationResolveUsage,
    ) -> Result<ResolvedAssociationRoot, BuildResolvedQueryError> {
        let object_type = self.resolve_object_type(&association.type_name.name)?;
        if !object_type_is_association_like(object_type) {
            return Err(self.unknown(
                "E_UNKNOWN_ASSOCIATION_TYPE",
                "object type is not declared as an association/link type",
                Some(&association.type_name.name),
            ));
        }
        let source_property_id =
            property_id_by_flag(&object_type.properties, PROPERTY_FLAG_ASSOCIATION_FROM_GOID);
        let target_property_id =
            property_id_by_flag(&object_type.properties, PROPERTY_FLAG_ASSOCIATION_TO_GOID);
        let object_relative =
            usage == AssociationResolveUsage::ObjectRelative && contextual_root.is_some();
        let endpoint_role = self.resolve_association_endpoint_role(
            association,
            source_property_id,
            target_property_id,
            object_relative,
        )?;
        Ok(ResolvedAssociationRoot {
            object_type_id: object_type.object_type_id,
            type_name: object_type.type_name.clone(),
            flags: object_type.flags,
            source_property_id,
            target_property_id,
            association_type_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_TYPE,
            ),
            valid_from_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
            ),
            valid_to_property_id: property_id_by_flag(
                &object_type.properties,
                PROPERTY_FLAG_ASSOCIATION_VALID_TO,
            ),
            direction: association.direction,
            role: association.role_name.as_ref().map(|role| role.name.clone()),
            endpoint_role,
            disclosure_outcome: association_disclosure_outcome(
                endpoint_role,
                &self.options.security,
            ),
            object_relative,
            target_node_object_type_id: None,
            target_node_label: None,
        })
    }

    fn resolve_relationship_association(
        &self,
        relationship: &AstRelationshipExpr,
        root: &ResolvedRoot,
    ) -> Result<ResolvedAssociationRoot, BuildResolvedQueryError> {
        if !matches!(root, ResolvedRoot::Node(_)) {
            return Err(profile_rejection(
                "E_UNSUPPORTED_PROFILE_METHOD",
                "relationship expressions require a CoveQL/Graph node or path root grain",
                &self.options,
            ));
        }
        let association = AstAssociationExpr {
            type_name: relationship.edge.type_name.clone(),
            direction: Some(relationship.direction),
            role: relationship.edge.role,
            role_name: relationship.edge.role_name.clone(),
        };
        let mut resolved = self.resolve_association_root(
            &association,
            Some(root),
            AssociationResolveUsage::ObjectRelative,
        )?;
        if let Some(target) = &relationship.target {
            let mut target_root = self.resolve_root(&target.root.node)?;
            apply_root_alias(&mut target_root, target.alias.as_ref());
            let ResolvedRoot::Node(node) = target_root else {
                return Err(profile_rejection(
                    "E_UNSUPPORTED_PROFILE_METHOD",
                    "relationship targets require node(...) bindings",
                    &self.options,
                ));
            };
            resolved.target_node_object_type_id = Some(node.object.object_type_id);
            resolved.target_node_label = Some(node.label);
        }
        Ok(resolved)
    }

    fn resolve_graph_node_root(
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

    fn resolve_graph_edge_root(
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

    fn resolve_association_endpoint_role(
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

    fn resolve_projection_root(
        &self,
        projection_id: &str,
    ) -> Result<ResolvedProjectionRoot, BuildResolvedQueryError> {
        let catalog = self.projection_catalog()?;
        let entry = catalog
            .projections
            .iter()
            .find(|entry| entry.projection_id == projection_id)
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_PROJECTION",
                    "unknown projection root",
                    Some(projection_id),
                )
            })?;
        let anchor_object = entry
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.object_type.as_deref())
            .and_then(|object_type| self.resolve_object_type(object_type).ok());
        Ok(ResolvedProjectionRoot {
            projection_id: entry.projection_id.clone(),
            mapping_id: catalog.mapping_id.clone(),
            mapping_version: catalog.mapping_version.clone(),
            output_table: entry.output_table.clone(),
            row_grain: entry.row_grain.clone(),
            anchor: entry
                .anchor
                .as_ref()
                .map(|anchor| ResolvedProjectionAnchor {
                    object_type: anchor.object_type.clone(),
                    association_type: anchor.association_type.clone(),
                }),
            temporal_mode: entry.temporal_mode.clone(),
            columns: entry
                .columns
                .iter()
                .map(|column| ResolvedProjectionColumn {
                    name: column.name.clone(),
                    value: column.value.clone(),
                    logical_type: column.logical_type.clone(),
                    nested_shape: column.nested_shape.clone(),
                    conflict_policy: column.conflict_policy.clone(),
                    missing_policy: column.missing_policy.clone(),
                    source_property_id: anchor_object.and_then(|object_type| {
                        projection_column_source_property_id(column.value.as_str(), object_type)
                    }),
                })
                .collect(),
            assertion_ids: entry.assertion_ids.clone(),
            multi_value_policy: entry.multi_value_policy.clone(),
            missing_policy: entry.missing_policy.clone(),
            ordering: entry.ordering.clone(),
            evidence_policy: entry.evidence_policy.clone(),
            output_modes: entry.output_modes.clone(),
            column_count: entry.columns.len(),
        })
    }

    fn resolve_table_root(
        &self,
        table_name: &str,
    ) -> Result<ResolvedTableRoot, BuildResolvedQueryError> {
        if let Some(table) = self.cte_table_scope.get(table_name) {
            return Ok(table.clone());
        }
        if let Some(authority) = self.table_authority_for_name(table_name) {
            self.validate_registered_table_authority(authority)?;
            let projection = synthetic_projection_for_table_contract(&authority.contract);
            return Ok(ResolvedTableRoot {
                table_name: authority.contract.table_name.clone(),
                binding_name: None,
                table_id: authority.contract.table_id.clone(),
                authority_kind: authority.contract.authority_kind,
                row_grain: authority.contract.row_grain.clone(),
                row_identity: authority.contract.row_identity.clone(),
                canonical_order: authority.contract.canonical_order.clone(),
                temporal_authority: authority.contract.temporal_authority,
                evidence_capabilities: authority.contract.evidence_capabilities.clone(),
                table_surface_contract: authority.contract.clone(),
                execution_authority: authority.execution_authority.clone(),
                projection,
            });
        }
        let catalog = self.projection_catalog().map_err(|_| {
            self.unknown(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surfaces require COVE-MAP projection metadata",
                Some(table_name),
            )
        })?;
        let projection_id = catalog
            .projections
            .iter()
            .find(|entry| {
                entry.output_table.as_deref() == Some(table_name)
                    || entry.projection_id == table_name
            })
            .map(|entry| entry.projection_id.clone())
            .ok_or_else(|| {
                self.unknown(
                    "E_UNKNOWN_TABLE_SURFACE",
                    "unknown CoveQL/Table surface",
                    Some(table_name),
                )
            })?;
        let projection = self.resolve_projection_root(&projection_id)?;
        let mut row_identity = projection.ordering.clone();
        if row_identity.is_empty() {
            row_identity.push("projection_row_identity".into());
        }
        let row_grain = projection
            .row_grain
            .clone()
            .unwrap_or_else(|| "visible_table_row".into());
        let mut canonical_order = projection.ordering.clone();
        if canonical_order.is_empty() {
            canonical_order = row_identity.clone();
        }
        let temporal_authority = if matches!(
            projection.temporal_mode.as_deref(),
            Some("as_of" | "recomputable" | "recomputable_projection_at_temporal_cut")
        ) {
            TableTemporalAuthority::RecomputableProjectionAtTemporalCut
        } else {
            TableTemporalAuthority::MaterializedSnapshotOnly
        };
        let auto_contract = self.projection_table_surface_contract(
            table_name,
            &projection,
            &row_grain,
            &row_identity,
            &canonical_order,
            temporal_authority,
        );
        let table_surface_contract = self
            .table_surface_contract_override(table_name, &projection, &auto_contract)?
            .unwrap_or(auto_contract);
        Ok(ResolvedTableRoot {
            table_name: table_name.into(),
            binding_name: None,
            table_id: format!("projection:{}", projection.projection_id),
            authority_kind: table_surface_contract.authority_kind,
            row_grain: table_surface_contract.row_grain.clone(),
            row_identity: table_surface_contract.row_identity.clone(),
            canonical_order: table_surface_contract.canonical_order.clone(),
            temporal_authority: table_surface_contract.temporal_authority,
            evidence_capabilities: table_surface_contract.evidence_capabilities.clone(),
            table_surface_contract,
            execution_authority: TableExecutionAuthority::DeterministicProjection {
                projection_id: projection.projection_id.clone(),
            },
            projection,
        })
    }

    fn table_authority_for_name(&self, table_name: &str) -> Option<&TableSurfaceAuthority> {
        self.options
            .table_authorities
            .get(table_name)
            .or_else(|| {
                self.options
                    .table_authorities
                    .values()
                    .find(|authority| authority.contract.table_name == table_name)
            })
            .or_else(|| {
                self.options
                    .table_authorities
                    .values()
                    .find(|authority| authority.contract.table_id == table_name)
            })
    }

    fn validate_registered_table_authority(
        &self,
        authority: &TableSurfaceAuthority,
    ) -> Result<(), BuildResolvedQueryError> {
        self.validate_table_surface_contract_fields(&authority.contract)?;
        match (
            authority.contract.authority_kind,
            &authority.execution_authority,
        ) {
            (
                TableSurfaceAuthorityKind::DeterministicProjection,
                TableExecutionAuthority::DeterministicProjection { .. },
            )
            | (
                TableSurfaceAuthorityKind::MaterializedTable,
                TableExecutionAuthority::MaterializedRows { .. },
            )
            | (TableSurfaceAuthorityKind::RawTable, TableExecutionAuthority::RawRows { .. })
            | (
                TableSurfaceAuthorityKind::ExternalRegisteredTable,
                TableExecutionAuthority::ExternalRows { .. },
            ) => Ok(()),
            _ => Err(self.unknown(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "table authority kind and execution authority do not match",
                Some(&authority.contract.table_name),
            )),
        }
    }

    fn projection_table_surface_contract(
        &self,
        table_name: &str,
        projection: &ResolvedProjectionRoot,
        row_grain: &str,
        row_identity: &[String],
        canonical_order: &[String],
        temporal_authority: TableTemporalAuthority,
    ) -> TableSurfaceContract {
        let table_id = format!("projection:{}", projection.projection_id);
        let logical_column_map = projection
            .columns
            .iter()
            .map(|column| TableSurfaceColumnContract {
                name: column.name.clone(),
                logical_type: column.logical_type.clone(),
                nullable: column.missing_policy != "reject",
                source_path: Some(column.value.clone()),
                code_domain: None,
                collation: None,
            })
            .collect::<Vec<_>>();
        TableSurfaceContract {
            table_id,
            table_name: table_name.into(),
            contract_version: crate::COVEQL_PROFILE_CONTRACT_VERSION.into(),
            authority_kind: TableSurfaceAuthorityKind::DeterministicProjection,
            authority_fingerprint: projection.projection_id.clone(),
            schema_fingerprint: projection.projection_id.clone(),
            logical_column_map,
            row_grain: row_grain.into(),
            row_identity: row_identity.to_vec(),
            canonical_order: canonical_order.to_vec(),
            visibility_authority: "cove_o_visibility".into(),
            redaction_authority: "cove_o_redaction".into(),
            temporal_authority,
            evidence_capabilities: vec![
                AstEvidenceGrain::Row,
                AstEvidenceGrain::Column,
                AstEvidenceGrain::Projection,
                AstEvidenceGrain::Source,
            ],
            null_missing_nan_policy: "projection_contract".into(),
            collation_policy: "projection_contract".into(),
            code_domain_contexts: Vec::new(),
            code_domain_bridges: Vec::new(),
            projection_dependency_contract_id: Some(projection.projection_id.clone()),
            datafusion_interop_contract: Some("coveql_table_projection_provider".into()),
        }
    }

    fn table_surface_contract_override(
        &self,
        table_name: &str,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<Option<TableSurfaceContract>, BuildResolvedQueryError> {
        let supplied = self
            .options
            .table_surface_contracts
            .get(table_name)
            .or_else(|| {
                self.options
                    .table_surface_contracts
                    .get(&projection.projection_id)
            })
            .or_else(|| {
                self.options
                    .table_surface_contracts
                    .get(&auto_contract.table_id)
            });
        let Some(contract) = supplied else {
            return Ok(None);
        };
        self.validate_table_surface_contract(contract, projection, auto_contract)?;
        Ok(Some(contract.clone()))
    }

    fn validate_table_surface_contract(
        &self,
        contract: &TableSurfaceContract,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        self.validate_table_surface_contract_fields(contract)?;
        if contract.authority_kind != TableSurfaceAuthorityKind::DeterministicProjection
            && !self
                .options
                .table_authorities
                .values()
                .any(|authority| authority.contract.table_id == contract.table_id)
        {
            return Err(self.unknown(
                "E_TABLE_AUTHORITY_UNSUPPORTED",
                "non-projection table surface contracts require a registered materialized, raw, or external table authority",
                Some(&contract.table_name),
            ));
        }
        if contract.authority_kind != TableSurfaceAuthorityKind::DeterministicProjection {
            return Ok(());
        }
        self.validate_projection_backed_table_contract(contract, projection, auto_contract)
    }

    fn validate_table_surface_contract_fields(
        &self,
        contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        let required_string_fields = [
            ("contract_version", contract.contract_version.as_str()),
            (
                "authority_fingerprint",
                contract.authority_fingerprint.as_str(),
            ),
            ("schema_fingerprint", contract.schema_fingerprint.as_str()),
            ("row_grain", contract.row_grain.as_str()),
            (
                "visibility_authority",
                contract.visibility_authority.as_str(),
            ),
            ("redaction_authority", contract.redaction_authority.as_str()),
            (
                "null_missing_nan_policy",
                contract.null_missing_nan_policy.as_str(),
            ),
            ("collation_policy", contract.collation_policy.as_str()),
        ];
        for (field, value) in required_string_fields {
            if value.trim().is_empty() {
                return Err(profile_rejection(
                    "E_UNKNOWN_TABLE_SURFACE",
                    format!("table surface contract is missing required field {field}"),
                    &self.options,
                ));
            }
        }
        if contract.row_identity.is_empty() {
            return Err(profile_rejection(
                "E_TABLE_ROW_IDENTITY_MISSING",
                "table surface contract requires a non-empty row_identity",
                &self.options,
            ));
        }
        if contract.canonical_order.is_empty() {
            return Err(profile_rejection(
                "E_TABLE_ORDER_MISSING",
                "table surface contract requires a non-empty canonical_order",
                &self.options,
            ));
        }
        if contract.logical_column_map.is_empty() {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract requires a non-empty logical_column_map",
                &self.options,
            ));
        }
        Ok(())
    }

    fn validate_projection_backed_table_contract(
        &self,
        contract: &TableSurfaceContract,
        projection: &ResolvedProjectionRoot,
        auto_contract: &TableSurfaceContract,
    ) -> Result<(), BuildResolvedQueryError> {
        if contract.table_id != auto_contract.table_id {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract table_id must match the deterministic projection authority",
                &self.options,
            ));
        }
        if contract.row_grain != auto_contract.row_grain {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract row_grain must match the deterministic projection authority",
                &self.options,
            ));
        }
        if contract.temporal_authority != auto_contract.temporal_authority {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract temporal authority does not match the projection authority",
                &self.options,
            ));
        }
        if contract.projection_dependency_contract_id.as_deref()
            != Some(projection.projection_id.as_str())
        {
            return Err(profile_rejection(
                "E_UNKNOWN_TABLE_SURFACE",
                "table surface contract must reference the projection dependency contract it executes",
                &self.options,
            ));
        }
        for column in &projection.columns {
            if !contract
                .logical_column_map
                .iter()
                .any(|contract_column| contract_column.name == column.name)
            {
                return Err(profile_rejection(
                    "E_UNKNOWN_TABLE_SURFACE",
                    format!(
                        "table surface contract is missing logical column {}",
                        column.name
                    ),
                    &self.options,
                ));
            }
        }
        Ok(())
    }

    fn resolve_evidence_root(
        &self,
        evidence: &AstEvidenceExpr,
        contextual_root: Option<&ResolvedRoot>,
    ) -> Result<ResolvedEvidenceRoot, BuildResolvedQueryError> {
        let target = match &evidence.target {
            Some(target) => Some(self.resolve_evidence_target(target, contextual_root)?),
            None => {
                if let Some(root) = contextual_root {
                    Some(contextual_evidence_target(root))
                } else {
                    None
                }
            }
        };
        let grain = evidence.grain.unwrap_or_else(|| match target {
            Some(ResolvedEvidenceTarget::AssociationType { .. }) => AstEvidenceGrain::Association,
            Some(ResolvedEvidenceTarget::Projection { .. }) => AstEvidenceGrain::Row,
            Some(ResolvedEvidenceTarget::TableRow { .. }) => AstEvidenceGrain::Row,
            Some(ResolvedEvidenceTarget::TableColumn { .. }) => AstEvidenceGrain::Column,
            Some(ResolvedEvidenceTarget::GraphNode { .. }) => AstEvidenceGrain::Node,
            Some(ResolvedEvidenceTarget::GraphEdge { .. }) => AstEvidenceGrain::Edge,
            Some(ResolvedEvidenceTarget::GraphPath { .. }) => AstEvidenceGrain::Path,
            Some(ResolvedEvidenceTarget::Property { .. }) => AstEvidenceGrain::Property,
            Some(ResolvedEvidenceTarget::CurrentRoot) => match contextual_root {
                Some(ResolvedRoot::Association(_)) => AstEvidenceGrain::Association,
                Some(ResolvedRoot::Node(_)) => AstEvidenceGrain::Node,
                Some(ResolvedRoot::Edge(_)) => AstEvidenceGrain::Edge,
                Some(ResolvedRoot::Table(_)) => AstEvidenceGrain::Row,
                Some(ResolvedRoot::Projection(_)) => AstEvidenceGrain::Row,
                _ => AstEvidenceGrain::Object,
            },
            _ => AstEvidenceGrain::Object,
        });
        if self.surface.evidence_index.is_none() {
            return Err(self.unknown(
                "E_MISSING_METADATA",
                "evidence roots and helpers require COVE-MAP evidence metadata",
                None,
            ));
        }
        let (mapping_id, mapping_version) = self
            .surface
            .evidence_index
            .as_ref()
            .map(|index| {
                (
                    Some(index.mapping_id.clone()),
                    Some(index.mapping_version.clone()),
                )
            })
            .unwrap_or((None, None));
        Ok(ResolvedEvidenceRoot {
            target,
            grain,
            mapping_id,
            mapping_version,
        })
    }

    fn resolve_evidence_target(
        &self,
        target: &AstEvidenceTarget,
        contextual_root: Option<&ResolvedRoot>,
    ) -> Result<ResolvedEvidenceTarget, BuildResolvedQueryError> {
        match target {
            AstEvidenceTarget::SelfTarget => Ok(contextual_root
                .map(contextual_evidence_target)
                .unwrap_or(ResolvedEvidenceTarget::CurrentRoot)),
            AstEvidenceTarget::Path(path) => {
                if path.parts.len() == 1 {
                    let name = &path.parts[0].name;
                    if let Ok(object_type) = self.resolve_object_type(name) {
                        return Ok(ResolvedEvidenceTarget::ObjectType {
                            object_type_id: object_type.object_type_id,
                            type_name: object_type.type_name.clone(),
                        });
                    }
                }
                if path.parts.len() == 2 {
                    if let Ok(object_type) = self.resolve_object_type(&path.parts[0].name) {
                        let property_name = &path.parts[1].name;
                        let property =
                            find_property(object_type, property_name).ok_or_else(|| {
                                self.unknown(
                                    "E_UNKNOWN_PROPERTY",
                                    "unknown object property for evidence target",
                                    Some(property_name),
                                )
                            })?;
                        return Ok(ResolvedEvidenceTarget::Property {
                            object_type_id: Some(object_type.object_type_id),
                            property_id: property.property_id,
                            property_name: property.property_name.clone(),
                        });
                    }
                }
                let root = contextual_root.ok_or_else(|| {
                    self.unknown(
                        "E_UNSUPPORTED_CONSTRUCT",
                        "property evidence target requires a contextual root",
                        None,
                    )
                })?;
                let resolved = self.resolve_path(path, root)?;
                if let ResolvedRoot::Table(table) = root {
                    return Ok(ResolvedEvidenceTarget::TableColumn {
                        table_id: table.table_id.clone(),
                        table_name: table.table_name.clone(),
                        projection_id: table.projection.projection_id.clone(),
                        column_name: resolved.display_name,
                    });
                }
                Ok(ResolvedEvidenceTarget::Property {
                    object_type_id: resolved.object_type_id,
                    property_id: resolved.property_id.unwrap_or_default(),
                    property_name: resolved.display_name,
                })
            }
            AstEvidenceTarget::RootBinding(binding) => {
                if let AstRoot::Path(path) = &binding.root.node {
                    let mut root = self.resolve_path_root_start(path)?;
                    apply_root_alias(&mut root, binding.alias.as_ref());
                    if let ResolvedRoot::Node(node) = root {
                        return Ok(ResolvedEvidenceTarget::GraphPath {
                            start_object_type_id: node.object.object_type_id,
                            start_label: node.binding_name.unwrap_or(node.label),
                        });
                    }
                }
                let mut root = self.resolve_root(&binding.root.node)?;
                apply_root_alias(&mut root, binding.alias.as_ref());
                Ok(contextual_evidence_target(&root))
            }
            AstEvidenceTarget::Association(association) => {
                let resolved = self.resolve_association_root(
                    association,
                    contextual_root,
                    AssociationResolveUsage::EvidenceTarget,
                )?;
                Ok(ResolvedEvidenceTarget::AssociationType {
                    object_type_id: resolved.object_type_id,
                    type_name: resolved.type_name,
                })
            }
            AstEvidenceTarget::Projection(projection) => {
                let resolved = self.resolve_projection_root(&projection.name)?;
                Ok(ResolvedEvidenceTarget::Projection {
                    projection_id: resolved.projection_id,
                })
            }
        }
    }

    fn projection_catalog(&self) -> Result<&MapProjectionCatalog, BuildResolvedQueryError> {
        self.surface.projection_catalog.as_ref().ok_or_else(|| {
            self.unknown(
                "E_MISSING_METADATA",
                "projection roots require COVE-MAP projection metadata",
                None,
            )
        })
    }

    fn evidence_metadata_key_exists(&self, key: &str) -> bool {
        self.surface
            .evidence_index
            .as_ref()
            .map(|index| evidence_key_exists(index, key))
            .unwrap_or(false)
    }

    fn unknown(
        &self,
        code: &'static str,
        public_message: &'static str,
        protected_name: Option<&str>,
    ) -> BuildResolvedQueryError {
        let mut diag = diagnostic(code, public_message, "resolve", &self.options.security);
        if self.options.security.metadata_disclosure_policy
            == MetadataDisclosurePolicy::AllowProtected
        {
            if let Some(name) = protected_name {
                diag.message = format!("{public_message}: {name}");
                diag.safe_details = json!({ "name": name });
                diag.redacted = false;
            }
        }
        BuildResolvedQueryError::single(diag)
    }
}

fn resolved_object_root(object_type: &ObjectTypeEntryV1) -> ResolvedObjectRoot {
    ResolvedObjectRoot {
        object_type_id: object_type.object_type_id,
        type_name: object_type.type_name.clone(),
        flags: object_type.flags,
    }
}

fn contextual_evidence_target(root: &ResolvedRoot) -> ResolvedEvidenceTarget {
    match root {
        ResolvedRoot::Object(object) => ResolvedEvidenceTarget::ObjectType {
            object_type_id: object.object_type_id,
            type_name: object.type_name.clone(),
        },
        ResolvedRoot::Association(association) => ResolvedEvidenceTarget::AssociationType {
            object_type_id: association.object_type_id,
            type_name: association.type_name.clone(),
        },
        ResolvedRoot::Node(node) => ResolvedEvidenceTarget::GraphNode {
            object_type_id: node.object.object_type_id,
            type_name: node.object.type_name.clone(),
            label: node
                .binding_name
                .clone()
                .unwrap_or_else(|| node.label.clone()),
        },
        ResolvedRoot::Edge(edge) => ResolvedEvidenceTarget::GraphEdge {
            object_type_id: edge.association.object_type_id,
            type_name: edge.association.type_name.clone(),
            label: edge
                .binding_name
                .clone()
                .unwrap_or_else(|| edge.label.clone()),
        },
        ResolvedRoot::Table(table) => ResolvedEvidenceTarget::TableRow {
            table_id: table.table_id.clone(),
            table_name: table.table_name.clone(),
            projection_id: table.projection.projection_id.clone(),
        },
        ResolvedRoot::Projection(projection) => ResolvedEvidenceTarget::Projection {
            projection_id: projection.projection_id.clone(),
        },
        ResolvedRoot::Evidence(_) => ResolvedEvidenceTarget::CurrentRoot,
    }
}

fn association_disclosure_outcome(
    endpoint_role: AssociationEndpointRole,
    security: &SecurityContext,
) -> AssociationDisclosureOutcome {
    if endpoint_role == AssociationEndpointRole::Unknown {
        return AssociationDisclosureOutcome::ProtectedEndpoint;
    }
    if security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected {
        AssociationDisclosureOutcome::Public
    } else {
        AssociationDisclosureOutcome::ProtectedEndpoint
    }
}

fn find_property<'a>(
    object_type: &'a ObjectTypeEntryV1,
    name: &str,
) -> Option<&'a PropertyEntryV1> {
    object_type
        .properties
        .iter()
        .find(|property| property.property_name == name)
}

fn projection_column_source_property_id(
    value: &str,
    object_type: &ObjectTypeEntryV1,
) -> Option<u32> {
    let property_name = value.strip_prefix("property.")?;
    (!property_name.is_empty() && !property_name.contains('.'))
        .then_some(property_name)
        .and_then(|property_name| find_property(object_type, property_name))
        .map(|property| property.property_id)
}

fn property_id_by_flag(properties: &[PropertyEntryV1], flag: u32) -> Option<u32> {
    properties
        .iter()
        .find(|property| property.flags & flag != 0)
        .map(|property| property.property_id)
}

fn temporal_role_inference_names(role: TemporalRole) -> &'static [&'static str] {
    match role {
        TemporalRole::ValidTime => &["valid_time"],
        TemporalRole::ObservedTime => &["observed_time"],
        TemporalRole::SourceEventTime => &["source_event_time", "event_time"],
        TemporalRole::CommitTime | TemporalRole::AssociationValidTime => &[],
    }
}

fn object_type_is_association_like(object_type: &ObjectTypeEntryV1) -> bool {
    object_type.flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT) != 0
}

fn object_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "goid" => Some(ResolvedSystemField::Goid),
        "object_type" => Some(ResolvedSystemField::ObjectType),
        "branch_key" => Some(ResolvedSystemField::BranchKey),
        "timestamp_us" => Some(ResolvedSystemField::TimestampUs),
        "csn" => Some(ResolvedSystemField::Csn),
        "record_kind" => Some(ResolvedSystemField::RecordKind),
        _ => None,
    }
}

fn association_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "source_goid" => Some(ResolvedSystemField::SourceGoid),
        "target_goid" => Some(ResolvedSystemField::TargetGoid),
        "association_type" => Some(ResolvedSystemField::AssociationType),
        "valid_from" => Some(ResolvedSystemField::ValidFrom),
        "valid_to" => Some(ResolvedSystemField::ValidTo),
        other => object_system_field(other),
    }
}

fn evidence_system_field(field: &str) -> Option<ResolvedSystemField> {
    match field {
        "source_id" => Some(ResolvedSystemField::SourceId),
        "source_row_identity" => Some(ResolvedSystemField::SourceRowIdentity),
        "rule_id" => Some(ResolvedSystemField::RuleId),
        "assertion_id" => Some(ResolvedSystemField::AssertionId),
        "output_object_id" => Some(ResolvedSystemField::OutputObjectId),
        "observed_schema_fingerprint" => Some(ResolvedSystemField::ObservedSchemaFingerprint),
        "observed_snapshot_digest" => Some(ResolvedSystemField::ObservedSnapshotDigest),
        _ => None,
    }
}

fn system_resolved_path(
    field: &str,
    root_kind: ResolvedPathRootKind,
    object_type_id: Option<u32>,
    system: ResolvedSystemField,
) -> ResolvedPath {
    let (logical_type, physical_kind, nullable, temporal_role) = match system {
        ResolvedSystemField::Goid
        | ResolvedSystemField::SourceGoid
        | ResolvedSystemField::TargetGoid => ("uuid", "fixed_bytes", false, None),
        ResolvedSystemField::BranchKey | ResolvedSystemField::Csn => {
            ("uint64", "num_code", false, None)
        }
        ResolvedSystemField::TimestampUs
        | ResolvedSystemField::ValidFrom
        | ResolvedSystemField::ValidTo => (
            "timestamp_micros",
            "num_code",
            false,
            Some(match system {
                ResolvedSystemField::ValidFrom | ResolvedSystemField::ValidTo => {
                    TemporalRole::AssociationValidTime
                }
                _ => TemporalRole::CommitTime,
            }),
        ),
        ResolvedSystemField::ObservedSchemaFingerprint
        | ResolvedSystemField::ObservedSnapshotDigest => ("utf8", "var_bytes", true, None),
        _ => ("utf8", "var_bytes", false, None),
    };
    ResolvedPath {
        display_name: field.into(),
        root_kind,
        object_type_id,
        property_id: None,
        association_type_id: if root_kind == ResolvedPathRootKind::Association {
            object_type_id
        } else {
            None
        },
        evidence_field_id: if root_kind == ResolvedPathRootKind::Evidence {
            Some(field.into())
        } else {
            None
        },
        projection_id: None,
        projection_column: None,
        system_field: Some(system),
        logical_type: logical_type.into(),
        physical_kind: physical_kind.into(),
        collation_id: None,
        nullable,
        null_policy: if nullable { "nullable" } else { "not_null" }.into(),
        temporal_role,
        code_domain_id: CodeDomainId::Placeholder {
            root: format!("{root_kind:?}").to_ascii_lowercase(),
            object_type_id,
            property_id: None,
            projection_id: None,
            field: Some(field.into()),
        },
    }
}

fn property_resolved_path(
    field: &str,
    root_kind: ResolvedPathRootKind,
    object_type_id: Option<u32>,
    association_type_id: Option<u32>,
    property: &PropertyEntryV1,
) -> ResolvedPath {
    ResolvedPath {
        display_name: field.into(),
        root_kind,
        object_type_id,
        property_id: Some(property.property_id),
        association_type_id,
        evidence_field_id: None,
        projection_id: None,
        projection_column: None,
        system_field: None,
        logical_type: logical_type_name(property.logical_type).into(),
        physical_kind: physical_kind_name(property.physical_kind).into(),
        collation_id: Some(property.collation_id),
        nullable: property.nullable,
        null_policy: if property.nullable {
            "nullable"
        } else {
            "not_null"
        }
        .into(),
        temporal_role: None,
        code_domain_id: CodeDomainId::Placeholder {
            root: format!("{root_kind:?}").to_ascii_lowercase(),
            object_type_id,
            property_id: Some(property.property_id),
            projection_id: None,
            field: Some(field.into()),
        },
    }
}

fn path_names(path: &AstPath) -> Vec<String> {
    path.parts.iter().map(|part| part.name.clone()).collect()
}

fn path_without_graph_binding(
    path: &AstPath,
    label: &str,
    alias: Option<&str>,
) -> Result<AstPath, BuildResolvedQueryError> {
    let parts = path_names(path);
    if parts.len() <= 1 {
        return Ok(path.clone());
    }
    let binding = &parts[0];
    if binding == label || alias.is_some_and(|alias| binding == alias) {
        return Ok(AstPath {
            parts: path.parts[1..].to_vec(),
        });
    }
    Err(BuildResolvedQueryError::single(diagnostic(
        "E_BINDING_OUT_OF_SCOPE",
        "qualified graph path references a binding that is not in scope",
        "resolve",
        &SecurityContext::default(),
    )))
}

fn evidence_key_exists(index: &MapEvidenceIndex, key: &str) -> bool {
    index
        .entries
        .iter()
        .any(|entry| entry.operation_metadata.contains_key(key))
}

fn physical_kind_name(physical: CovePhysicalKind) -> &'static str {
    match physical {
        CovePhysicalKind::FileCode => "file_code",
        CovePhysicalKind::NumCode => "num_code",
        CovePhysicalKind::Boolean => "boolean",
        CovePhysicalKind::FixedBytes => "fixed_bytes",
        CovePhysicalKind::VarBytes => "var_bytes",
        CovePhysicalKind::List => "list",
        CovePhysicalKind::Struct => "struct",
        CovePhysicalKind::Map => "map",
        _ => "unknown",
    }
}

fn physical_kind_for_logical_name(logical: &str) -> &'static str {
    match logical {
        "bool" | "boolean" => "boolean",
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64"
        | "float32" | "float64" | "decimal64" | "date_days" | "timestamp_micros"
        | "timestamp_nanos" => "num_code",
        "uuid" | "decimal128" => "fixed_bytes",
        "list" => "list",
        "struct" => "struct",
        "map" => "map",
        _ => "var_bytes",
    }
}

fn aggregate_logical_type(name: AstAggregateName) -> &'static str {
    match name {
        AstAggregateName::Count | AstAggregateName::DistinctCount => "uint64",
        AstAggregateName::Exists => "bool",
        AstAggregateName::Min
        | AstAggregateName::Max
        | AstAggregateName::Sum
        | AstAggregateName::Avg => "unknown",
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

fn builtin_function_execution_class(name: &str, args: &[ResolvedExpr]) -> FunctionExecutionClass {
    match name {
        "isNull" | "isNotNull" => FunctionExecutionClass::CodedSafe,
        "coalesce" if coalesce_bool_args_are_coded_safe(args) => FunctionExecutionClass::CodedSafe,
        "cast" if cast_args_are_identity(args) => FunctionExecutionClass::CodedSafe,
        _ => FunctionExecutionClass::MaterializedOnly,
    }
}

fn coalesce_bool_args_are_coded_safe(args: &[ResolvedExpr]) -> bool {
    !args.is_empty()
        && args.iter().all(|arg| {
            matches!(
                expr_logical_type(arg).map(normalized_function_type),
                Some("bool" | "null")
            )
        })
}

fn cast_args_are_identity(args: &[ResolvedExpr]) -> bool {
    let Some(value_type) = args.first().and_then(expr_logical_type) else {
        return false;
    };
    let Some(target_type) = cast_target_logical_type(args) else {
        return false;
    };
    normalized_function_type(value_type) == normalized_function_type(target_type)
}

fn normalized_function_type(value: &str) -> &str {
    match value {
        "bool" | "boolean" => "bool",
        "string" | "utf8" => "utf8",
        "null" => "null",
        other => other,
    }
}

fn is_window_function_name(name: &str) -> bool {
    matches!(
        name,
        "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
            | "first_value"
            | "last_value"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "count"
    )
}

fn window_aggregate_function_name(name: AstAggregateName) -> Option<&'static str> {
    match name {
        AstAggregateName::Count => Some("count"),
        AstAggregateName::Sum => Some("sum"),
        AstAggregateName::Avg => Some("avg"),
        AstAggregateName::Min => Some("min"),
        AstAggregateName::Max => Some("max"),
        AstAggregateName::Exists | AstAggregateName::DistinctCount => None,
    }
}

fn function_logical_type(name: &str, args: &[ResolvedExpr]) -> String {
    match name {
        "startsWith" | "exists" | "isNull" | "isNotNull" => "bool".into(),
        "length" | "row_number" | "rank" | "dense_rank" | "count" => "uint64".into(),
        "sum" | "avg" => "float64".into(),
        "lag" | "lead" | "first_value" | "last_value" | "min" | "max" => args
            .first()
            .and_then(expr_logical_type)
            .unwrap_or("unknown")
            .into(),
        "cast" => cast_target_logical_type(args).unwrap_or("unknown").into(),
        "coalesce" => coalesce_common_logical_type(args)
            .unwrap_or("unknown")
            .into(),
        "identity" => args
            .first()
            .and_then(expr_logical_type)
            .unwrap_or("unknown")
            .into(),
        "trim" | "lower" | "lowercase" | "upper" | "uppercase" => "utf8".into(),
        _ => "unknown".into(),
    }
}

fn function_physical_kind(name: &str, args: &[ResolvedExpr]) -> &'static str {
    match name {
        "startsWith" | "isNull" | "isNotNull" => "boolean",
        "length" | "row_number" | "rank" | "dense_rank" | "count" | "sum" | "avg" => "num_code",
        "lag" | "lead" | "first_value" | "last_value" | "min" | "max" => args
            .first()
            .and_then(expr_logical_type)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        "coalesce" => coalesce_common_logical_type(args)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        "cast" => cast_target_logical_type(args)
            .map(physical_kind_for_logical_name)
            .unwrap_or("var_bytes"),
        _ => "var_bytes",
    }
}

fn cast_target_logical_type(args: &[ResolvedExpr]) -> Option<&'static str> {
    let Some(ResolvedExpr::Literal(literal)) = args.get(1) else {
        return None;
    };
    let ResolvedLiteralValue::String(target) = &literal.typed_value else {
        return None;
    };
    match target.trim().to_ascii_lowercase().as_str() {
        "bool" | "boolean" => Some("bool"),
        "utf8" | "string" => Some("utf8"),
        "json" => Some("json"),
        "int" | "int64" => Some("int64"),
        "uint" | "uint64" => Some("uint64"),
        "decimal" | "decimal128" => Some("decimal128"),
        "timestamp" | "timestamp_micros" => Some("timestamp_micros"),
        "uuid" => Some("uuid"),
        "binary" => Some("binary"),
        _ => None,
    }
}

fn canonicalize_null_check_predicate(expr: ResolvedExpr) -> ResolvedPredicate {
    match expr {
        ResolvedExpr::FunctionCall {
            function_id,
            mut args,
            ..
        } if matches!(function_id.as_str(), "isNull" | "isNotNull") && args.len() == 1 => {
            ResolvedPredicate::NullCheck {
                expr: args.remove(0),
                negated: function_id == "isNotNull",
            }
        }
        expr => ResolvedPredicate::BoolExpr(expr),
    }
}

fn declares_unicode_or_collation_dependency(dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    dependency.contains("unicode") || dependency.contains("collation")
}

fn declares_exact_string_function_dependency(name: &str, dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    if !declares_unicode_or_collation_dependency(&dependency) {
        return false;
    }
    match name {
        "startsWith" => {
            dependency.contains("startswith")
                && dependency.contains("exact")
                && (dependency.contains("accelerator") || dependency.contains("prefix"))
        }
        "length" => {
            dependency.contains("length")
                && dependency.contains("exact")
                && (dependency.contains("logical") || dependency.contains("encoded"))
        }
        _ => false,
    }
}

fn declares_safe_cast_dependency(dependency: &str) -> bool {
    let dependency = dependency.to_ascii_lowercase();
    dependency.contains("safe-cast") || dependency.contains("safe_cast")
}

fn timestamp_micros(
    value: &str,
    security: &SecurityContext,
) -> Result<(i64, String), BuildResolvedQueryError> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "timestamp literals must be RFC3339 strings with an explicit offset",
            "resolve",
            security,
        ))
    })?;
    let nanos = timestamp.unix_timestamp_nanos();
    let micros = i64::try_from(nanos / 1_000).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "timestamp literal is outside supported microsecond range",
            "resolve",
            security,
        ))
    })?;
    let canonical = timestamp.format(&Rfc3339).map_err(|_| {
        BuildResolvedQueryError::single(diagnostic(
            "E_LITERAL",
            "failed to canonicalize timestamp literal",
            "resolve",
            security,
        ))
    })?;
    Ok((micros, canonical))
}

fn synthetic_projection_for_table_contract(
    contract: &TableSurfaceContract,
) -> ResolvedProjectionRoot {
    ResolvedProjectionRoot {
        projection_id: contract.table_id.clone(),
        mapping_id: format!("table-surface:{}", contract.table_id),
        mapping_version: contract.contract_version.clone(),
        output_table: Some(contract.table_name.clone()),
        row_grain: Some(contract.row_grain.clone()),
        anchor: None,
        temporal_mode: Some(format!("{:?}", contract.temporal_authority).to_ascii_lowercase()),
        columns: contract
            .logical_column_map
            .iter()
            .map(|column| ResolvedProjectionColumn {
                name: column.name.clone(),
                value: column
                    .source_path
                    .clone()
                    .unwrap_or_else(|| column.name.clone()),
                logical_type: column.logical_type.clone(),
                nested_shape: None,
                conflict_policy: "table_surface_contract".into(),
                missing_policy: if column.nullable {
                    "missing_is_null".into()
                } else {
                    "reject".into()
                },
                source_property_id: None,
            })
            .collect(),
        assertion_ids: Vec::new(),
        multi_value_policy: Some("reject".into()),
        missing_policy: contract.null_missing_nan_policy.clone(),
        ordering: contract.canonical_order.clone(),
        evidence_policy: "table_surface_contract".into(),
        output_modes: vec!["json_rows".into(), "arrow_record_batch".into()],
        column_count: contract.logical_column_map.len(),
    }
}

fn resolved_fingerprint(
    root: &ResolvedRoot,
    method_chain: &ResolvedMethodChain,
    output_mode: &CoveQlOutputMode,
    temporal: &TemporalContext,
    branch: &BranchContext,
    tombstone: &TombstoneContext,
    profiles: &[CoveQlProfileId],
) -> String {
    let profile_contracts = profiles
        .iter()
        .map(|profile| {
            let contract = crate::coveql_profile_contract(*profile);
            json!({
                "profile_id": profile,
                "profile_version": contract.profile_version,
                "implemented": contract.implemented,
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "core_contract": {
            "language_version": crate::COVEQL_LANGUAGE_VERSION,
            "core_version": crate::COVEQL_CORE_VERSION,
            "profile_contract_version": crate::COVEQL_PROFILE_CONTRACT_VERSION,
        },
        "profiles": profiles,
        "profile_contracts": profile_contracts,
        "root": root,
        "method_chain": method_chain,
        "output_mode": output_mode,
        "temporal": temporal,
        "branch": branch,
        "tombstone": tombstone,
    });
    sha256_hex(
        serde_json::to_string(&value)
            .expect("canonical resolved query serializes")
            .as_bytes(),
    )
}

fn registered_table_rows_for_recursive_cte(
    table: &ResolvedTableRoot,
) -> Option<Vec<TableSurfaceRow>> {
    match &table.execution_authority {
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows }
        | TableExecutionAuthority::ExternalRows { rows, .. } => Some(rows.clone()),
        TableExecutionAuthority::DeterministicProjection { .. } => None,
    }
}

fn recursive_cte_key_for_row(
    row: &TableSurfaceRow,
    key: &ResolvedExpr,
    options: &ResolveOptions,
) -> Result<String, BuildResolvedQueryError> {
    let ResolvedExpr::Path(path) = key else {
        return Err(profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            "withRecursive key must resolve to a table column path for materialized fixpoint execution",
            options,
        ));
    };
    let field = path
        .projection_column
        .as_deref()
        .unwrap_or(path.display_name.as_str());
    let value = row
        .get(field)
        .or_else(|| {
            field
                .rsplit_once('.')
                .and_then(|(_, unqualified)| row.get(unqualified))
        })
        .unwrap_or(&serde_json::Value::Null);
    serde_json::to_string(value).map_err(|error| {
        profile_rejection(
            "E_UNSUPPORTED_PROFILE_METHOD",
            format!("withRecursive key could not be serialized: {error}"),
            options,
        )
    })
}

fn warning(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> CoveQlDiagnostic {
    let mut diagnostic = diagnostic(code, message, phase, security);
    diagnostic.severity = DiagnosticSeverity::Warning;
    diagnostic
}

fn diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> CoveQlDiagnostic {
    CoveQlDiagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        phase: phase.into(),
        safe_details: json!({}),
        redacted: security.metadata_disclosure_policy != MetadataDisclosurePolicy::AllowProtected,
    }
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
