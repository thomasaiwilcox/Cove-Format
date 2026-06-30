use super::*;

pub(super) fn is_coveql_profile_method(name: &str) -> bool {
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

pub(super) fn profile_method_expands_relationships(name: &str) -> bool {
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

pub(super) fn validate_profile_shell(
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

pub(super) fn is_host_profile(profile: CoveQlProfileId) -> bool {
    matches!(
        profile,
        CoveQlProfileId::Object | CoveQlProfileId::Table | CoveQlProfileId::Graph
    )
}

pub(super) fn parsed_requires_ai_profile(parsed: &ParsedQuery) -> bool {
    parsed.methods.iter().any(|method| match &method.node {
        AstMethod::Explain(ExplainMode::Ai) => true,
        AstMethod::ProfileCall { name, .. } => ai_operation_for_method(&name.name).is_some(),
        _ => false,
    })
}

pub(super) fn first_unsupported_profile_method(parsed: &ParsedQuery) -> Option<&str> {
    parsed.methods.iter().find_map(|method| match &method.node {
        AstMethod::ProfileCall { name, .. } if !is_coveql_profile_method(&name.name) => {
            Some(name.name.as_str())
        }
        _ => None,
    })
}

pub(super) fn profile_for_root(root: &AstRoot) -> CoveQlProfileId {
    match root {
        AstRoot::Object(_)
        | AstRoot::Association(_)
        | AstRoot::Projection(_)
        | AstRoot::Evidence(_) => CoveQlProfileId::Object,
        AstRoot::Table(_) => CoveQlProfileId::Table,
        AstRoot::Node(_) | AstRoot::Edge(_) | AstRoot::Path(_) => CoveQlProfileId::Graph,
    }
}

pub(super) fn profile_rejection(
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

pub(super) fn apply_root_alias(root: &mut ResolvedRoot, alias: Option<&AstIdentifier>) {
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
