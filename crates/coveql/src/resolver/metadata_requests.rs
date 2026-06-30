use super::*;

pub(super) fn operation_request_for(
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

pub(super) fn ast_requests_evidence_metadata(parsed: &ParsedQuery) -> bool {
    matches!(parsed.root.node, AstRoot::Evidence(_))
        || parsed
            .methods
            .iter()
            .any(|method| method_requests_evidence_metadata(&method.node))
}

pub(super) fn method_requests_evidence_metadata(method: &AstMethod) -> bool {
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

pub(super) fn predicate_requests_evidence_metadata(predicate: &AstPredicate) -> bool {
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

pub(super) fn expr_requests_evidence_metadata(expr: &AstExpr) -> bool {
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

pub(super) fn profile_argument_requests_evidence(argument: &AstProfileArgument) -> bool {
    match &argument.value {
        AstProfileArgumentValue::Expr(expr) => expr_requests_evidence_metadata(&expr.node),
        AstProfileArgumentValue::Predicate(predicate) => {
            predicate_requests_evidence_metadata(&predicate.node)
        }
    }
}
