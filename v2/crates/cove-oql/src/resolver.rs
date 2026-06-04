use crate::{
    ast::*, build_operation_context, parse_query, BranchContext, BranchSelector,
    CoveOqlExplainTarget, CoveOqlOperationRequest, CoveOqlOutputMode, CoveOqlSelectedOperation,
    DiagnosticSeverity, ExplainMode, MetadataDisclosurePolicy, OqlDiagnostic, RedactionReference,
    RejectionKind, RejectionReport, ResourceUseEstimate, SecurityContext, TemporalContext,
    TemporalMode, TemporalRole, TombstoneContext, VisibilityReference,
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
use std::{error::Error, fmt};
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
    let mut settings = collect_chain_settings(&parsed, &resolve_options)?;
    let request = operation_request_for(&parsed, &settings, &resolve_options);
    let operation_context = build_operation_context(bytes, request, validation_options)
        .map_err(BuildResolvedQueryError::from_operation_context)?;

    let surface = read_object_surface_from_bytes_with_options(
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
    )
    .map_err(|error| {
        BuildResolvedQueryError::single(diagnostic(
            "E_RESOLVE",
            format!("failed to read resolver metadata: {error}"),
            "resolve",
            &resolve_options.security,
        ))
    })?;

    let mut resolver = Resolver::new(surface, resolve_options.clone());
    let root = resolver.resolve_root(&parsed.root.node)?;
    settings.temporal = resolver.resolve_temporal_context(settings.temporal.clone(), &root)?;
    let method_chain = resolver.resolve_methods(&parsed, &root, &settings)?;
    let resolved_query_fingerprint = resolved_fingerprint(
        &root,
        &method_chain,
        &settings.output_mode,
        &settings.temporal,
        &settings.branch,
        &settings.tombstone,
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

#[derive(Debug, Clone)]
pub struct BuildResolvedQueryError {
    pub diagnostics: Vec<OqlDiagnostic>,
    pub rejections: Vec<RejectionReport>,
    pub source: Option<String>,
}

impl BuildResolvedQueryError {
    fn single(diagnostic: OqlDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            rejections: Vec::new(),
            source: None,
        }
    }

    fn from_parse(diagnostics: Vec<OqlDiagnostic>) -> Self {
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
                .map(crate::OqlDiagnostic::to_json)
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
            write!(f, "Cove-OQL query resolution failed")
        }
    }
}

impl Error for BuildResolvedQueryError {}

#[derive(Debug, Clone)]
struct ChainSettings {
    selected_operation: CoveOqlSelectedOperation,
    output_mode: CoveOqlOutputMode,
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
    let mut explain = None;

    for method in &parsed.methods {
        match &method.node {
            AstMethod::Where(_) => {}
            AstMethod::Select(_) => reject_duplicate(&mut seen_select, "select", resolve_options)?,
            AstMethod::AsOf(bound) => {
                reject_duplicate(&mut seen_as_of, "asOf", resolve_options)?;
                if seen_history || seen_changes {
                    return Err(conflict(
                        "asOf cannot be mixed with history or changes",
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
                    AstChangeMode::FinalObjects => TemporalMode::ChangesFinalObjects,
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
        }
    }

    let target = explain_target_for_root(&parsed.root.node);
    let default_output = output_mode_for_root(&parsed.root.node);
    let output_mode = if explain.is_some() {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(CoveOqlOutputMode::ExplainJson)
    } else {
        resolve_options
            .output_mode
            .clone()
            .unwrap_or(default_output)
    };
    let selected_operation = if let Some(mode) = explain {
        CoveOqlSelectedOperation::Explain { target, mode }
    } else if let CoveOqlOutputMode::ArrowRecordBatch {
        zero_copy_requested,
    } = &output_mode
    {
        CoveOqlSelectedOperation::ArrowExport {
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

fn operation_request_for(
    parsed: &ParsedQuery,
    settings: &ChainSettings,
    resolve_options: &ResolveOptions,
) -> CoveOqlOperationRequest {
    CoveOqlOperationRequest {
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
        | AstPredicate::Exists(expr)
        | AstPredicate::BoolExpr(expr) => expr_requests_evidence_metadata(&expr.node),
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
        AstExpr::Association(_) | AstExpr::Literal(_) | AstExpr::Path(_) => false,
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

fn selected_operation_for_root(root: &AstRoot) -> CoveOqlSelectedOperation {
    match root {
        AstRoot::Object(_) => CoveOqlSelectedOperation::Object,
        AstRoot::Association(_) => CoveOqlSelectedOperation::Association,
        AstRoot::Projection(_) => CoveOqlSelectedOperation::Projection,
        AstRoot::Evidence(_) => CoveOqlSelectedOperation::Evidence,
    }
}

fn explain_target_for_root(root: &AstRoot) -> CoveOqlExplainTarget {
    match root {
        AstRoot::Object(_) => CoveOqlExplainTarget::Object,
        AstRoot::Association(_) => CoveOqlExplainTarget::Association,
        AstRoot::Projection(_) => CoveOqlExplainTarget::Projection,
        AstRoot::Evidence(_) => CoveOqlExplainTarget::Evidence,
    }
}

fn output_mode_for_root(root: &AstRoot) -> CoveOqlOutputMode {
    match root {
        AstRoot::Object(_) => CoveOqlOutputMode::ObjectRows,
        AstRoot::Association(_) => CoveOqlOutputMode::AssociationRows,
        AstRoot::Projection(_) => CoveOqlOutputMode::ProjectionRows,
        AstRoot::Evidence(_) => CoveOqlOutputMode::EvidenceRows,
    }
}

fn expr_logical_type(expr: &ResolvedExpr) -> Option<&str> {
    match expr {
        ResolvedExpr::Path(path) => Some(path.logical_type.as_str()),
        ResolvedExpr::Literal(literal) => Some(literal.logical_type.as_str()),
        ResolvedExpr::FunctionCall { logical_type, .. }
        | ResolvedExpr::AggregateCall { logical_type, .. }
        | ResolvedExpr::Conditional { logical_type, .. } => Some(logical_type.as_str()),
        ResolvedExpr::Association(_) | ResolvedExpr::Evidence(_) => None,
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
    diagnostics: Vec<OqlDiagnostic>,
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
            AstRoot::Projection(identifier) => self
                .resolve_projection_root(&identifier.name)
                .map(ResolvedRoot::Projection),
            AstRoot::Evidence(evidence) => self
                .resolve_evidence_root(evidence, None)
                .map(ResolvedRoot::Evidence),
        }
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
        if matches!(settings.output_mode, CoveOqlOutputMode::ExplainJson) {
            chain.explain.get_or_insert(ExplainMode::Public);
        }
        Ok(chain)
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
            AstPredicate::Exists(expr) => {
                let expr = self.resolve_expr(expr, root)?;
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
                    version: "cove-oql-0.1".into(),
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
                    version: "cove-oql-0.1".into(),
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
                        version: "cove-oql-0.1".into(),
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
                version: "cove-oql-0.1".into(),
                deterministic: true,
                dependency: "pure".into(),
                execution_class: FunctionExecutionClass::MaterializedOnly,
                unicode_or_collation_contract: None,
            }),
            _ if self.deterministic_function_entry(name).is_some() => Err(self
                .function_contract_error(
                    "registered deterministic function has no Cove-OQL executable body",
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
            Some(ResolvedEvidenceTarget::Property { .. }) => AstEvidenceGrain::Property,
            Some(ResolvedEvidenceTarget::CurrentRoot) => match contextual_root {
                Some(ResolvedRoot::Association(_)) => AstEvidenceGrain::Association,
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
                    let object_type = self.resolve_object_type(&path.parts[0].name)?;
                    let property_name = &path.parts[1].name;
                    let property = find_property(object_type, property_name).ok_or_else(|| {
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
                let root = contextual_root.ok_or_else(|| {
                    self.unknown(
                        "E_UNSUPPORTED_CONSTRUCT",
                        "property evidence target requires a contextual root",
                        None,
                    )
                })?;
                let resolved = self.resolve_path(path, root)?;
                Ok(ResolvedEvidenceTarget::Property {
                    object_type_id: resolved.object_type_id,
                    property_id: resolved.property_id.unwrap_or_default(),
                    property_name: resolved.display_name,
                })
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

fn function_logical_type(name: &str, args: &[ResolvedExpr]) -> String {
    match name {
        "startsWith" | "exists" | "isNull" | "isNotNull" => "bool".into(),
        "length" => "uint64".into(),
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
        "length" => "num_code",
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

fn resolved_fingerprint(
    root: &ResolvedRoot,
    method_chain: &ResolvedMethodChain,
    output_mode: &CoveOqlOutputMode,
    temporal: &TemporalContext,
    branch: &BranchContext,
    tombstone: &TombstoneContext,
) -> String {
    let value = json!({
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

fn warning(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> OqlDiagnostic {
    let mut diagnostic = diagnostic(code, message, phase, security);
    diagnostic.severity = DiagnosticSeverity::Warning;
    diagnostic
}

fn diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    phase: impl Into<String>,
    security: &SecurityContext,
) -> OqlDiagnostic {
    OqlDiagnostic {
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
