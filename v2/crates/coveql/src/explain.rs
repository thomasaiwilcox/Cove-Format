use serde_json::{json, Map, Value};

use crate::{
    diagnostic_severity_name, plan_printer, CoveQlDiagnostic, CoveQlExecutionResult,
    CoveQlOutputMode, CoveQlSelectedOperation, DiagnosticSeverity, EvidenceAuthority,
    ExecutedQuery, ExecutionDiagnostic, ExplainDisclosurePolicy, ExplainMode, FallbackReport,
    KernelFallbackReason, LogicalPlanDiagnostic, MetadataDisclosurePolicy, OperationContext,
    OptionalMetadataStatus, PhysicalPlanDiagnostic, PhysicalPlannedQuery, PlannedQuery,
    PredicatePlacement, RejectionReport, ResolvedQuery, ResolvedRoot, TemporalMode,
};

pub fn render_explain_text(value: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "schema_version: {}",
        string_at(value, "schema_version").unwrap_or("unknown")
    ));
    lines.push(format!(
        "mode: {}",
        string_at(value, "mode").unwrap_or("public")
    ));

    if let Some(operation) = value
        .get("operation_context")
        .and_then(|context| context.get("operation"))
        .and_then(Value::as_str)
    {
        lines.push(format!("operation: {operation}"));
    }

    if let Some(fingerprints) = value.get("fingerprints").and_then(Value::as_object) {
        for key in [
            "query_text",
            "parsed_ast",
            "resolved_query",
            "logical_plan",
            "physical_plan",
        ] {
            let rendered = fingerprints
                .get(key)
                .map(render_compact)
                .unwrap_or_else(|| "null".into());
            lines.push(format!("fingerprint.{key}: {rendered}"));
        }
    }

    lines.push(format!(
        "logical_plan.nodes: {}",
        array_len(value, "logical_plan")
    ));
    lines.push(format!(
        "physical_plan.nodes: {}",
        array_len(value, "physical_plan")
    ));
    lines.push(format!(
        "predicate_forms: {}",
        array_len(value, "predicate_forms")
    ));
    lines.push(format!(
        "decode_boundaries: {}",
        array_len(value, "decode_boundaries")
    ));
    lines.push(format!(
        "residual_predicates: {}",
        array_len(value, "residual_predicates")
    ));
    lines.push(format!(
        "trusted_metadata: {}",
        array_len(value, "trusted_metadata")
    ));
    lines.push(format!(
        "ignored_metadata: {}",
        array_len(value, "ignored_metadata")
    ));
    lines.push(format!("fallbacks: {}", array_len(value, "fallbacks")));
    lines.push(format!("rejections: {}", array_len(value, "rejections")));

    if let Some(execution) = value.get("execution") {
        let completed = execution
            .get("completed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let kind = execution
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        lines.push(format!("execution: completed={completed} kind={kind}"));
        if let Some(outcome) = execution
            .get("pushdown_report")
            .and_then(|report| report.get("outcome"))
            .and_then(Value::as_str)
        {
            lines.push(format!("pushdown.outcome: {outcome}"));
        }
        if let Some(kind) = execution
            .get("kernel_report")
            .and_then(|report| report.get("decision"))
            .and_then(|decision| decision.get("kind"))
            .and_then(Value::as_str)
        {
            lines.push(format!("kernel.decision: {kind}"));
        }
    }

    lines.push(format!("warnings: {}", array_len(value, "warnings")));
    lines.push(format!("redactions: {}", array_len(value, "redactions")));
    lines.push(format!("diagnostics: {}", array_len(value, "diagnostics")));
    lines.join("\n")
}

pub(crate) fn operation_context_explain_json(context: &OperationContext) -> Value {
    let requested_mode = requested_mode(context);
    let effective_mode = effective_mode(requested_mode, context.security.explain_policy);
    let allow_protected = allow_protected(context);
    let mut diagnostics = context
        .diagnostics
        .iter()
        .map(|diagnostic| coveql_diagnostic_json(diagnostic, allow_protected))
        .collect::<Vec<_>>();
    diagnostics.extend(mode_policy_diagnostics(
        requested_mode,
        effective_mode,
        allow_protected,
    ));

    let mut value = explain_document(ExplainDocumentParts {
        schema_version: context.explain_json_schema_version.into(),
        mode: explain_mode_name(effective_mode).into(),
        primary_profile: Value::String("object".into()),
        profiles: json!(["object"]),
        profile_contracts: default_profile_contracts_json(),
        root: Value::Null,
        grain: Value::Null,
        canonical_order: json!([]),
        fingerprints: fingerprints_json(context, None, None, None, None, None, None),
        operation_context: operation_context_json(context),
        logical_plan: json!([]),
        physical_plan: json!([]),
        resolved_dependencies: json!([]),
        predicate_forms: json!([]),
        trusted_metadata: trusted_metadata_json(context),
        ignored_metadata: ignored_metadata_json(context),
        fallbacks: context
            .fallbacks
            .iter()
            .map(FallbackReport::to_json)
            .collect(),
        rejections: context
            .rejections
            .iter()
            .map(RejectionReport::to_json)
            .collect(),
        decode_boundaries: json!([]),
        residual_predicates: json!([]),
        visibility: visibility_json(context),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: plan_only_execution_json(None, allow_protected),
    });
    finalize_explain_document(&mut value, allow_protected);
    value
}

pub(crate) fn resolved_query_explain_json(resolved: &ResolvedQuery) -> Value {
    let context = &resolved.operation_context;
    let requested_mode = requested_mode(context);
    let effective_mode = effective_mode(requested_mode, context.security.explain_policy);
    let allow_protected = allow_protected(context);
    let mut diagnostics = resolved
        .diagnostics
        .iter()
        .map(|diagnostic| coveql_diagnostic_json(diagnostic, allow_protected))
        .collect::<Vec<_>>();
    diagnostics.extend(mode_policy_diagnostics(
        requested_mode,
        effective_mode,
        allow_protected,
    ));

    let mut value = explain_document(ExplainDocumentParts {
        schema_version: context.explain_json_schema_version.into(),
        mode: explain_mode_name(effective_mode).into(),
        primary_profile: primary_profile_json(resolved),
        profiles: profiles_json(resolved),
        profile_contracts: profile_contracts_json(resolved),
        root: root_json(&resolved.root),
        grain: resolved_grain_json(&resolved.root),
        canonical_order: json!([]),
        fingerprints: fingerprints_json(
            context,
            Some(&resolved.resolved_query_fingerprint),
            None,
            None,
            None,
            None,
            None,
        ),
        operation_context: operation_context_json(context),
        logical_plan: json!([]),
        physical_plan: json!([]),
        resolved_dependencies: json!([]),
        predicate_forms: json!([]),
        trusted_metadata: trusted_metadata_json(context),
        ignored_metadata: ignored_metadata_json(context),
        fallbacks: context
            .fallbacks
            .iter()
            .map(FallbackReport::to_json)
            .collect(),
        rejections: context
            .rejections
            .iter()
            .map(RejectionReport::to_json)
            .collect(),
        decode_boundaries: json!([]),
        residual_predicates: json!([]),
        visibility: visibility_json(context),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: plan_only_execution_json(None, allow_protected),
    });
    finalize_explain_document(&mut value, allow_protected);
    value
}

pub(crate) fn planned_query_explain_json(planned: &PlannedQuery) -> Value {
    let context = &planned.resolved.operation_context;
    let requested_mode = requested_mode(context);
    let effective_mode = effective_mode(requested_mode, context.security.explain_policy);
    let allow_protected = allow_protected(context);
    let disclosure = disclosure_policy(allow_protected);
    let mut diagnostics = planned
        .diagnostics
        .iter()
        .map(|diagnostic| logical_diagnostic_json(diagnostic, allow_protected))
        .collect::<Vec<_>>();
    diagnostics.extend(mode_policy_diagnostics(
        requested_mode,
        effective_mode,
        allow_protected,
    ));

    let mut value = explain_document(ExplainDocumentParts {
        schema_version: context.explain_json_schema_version.into(),
        mode: explain_mode_name(effective_mode).into(),
        primary_profile: primary_profile_json(&planned.resolved),
        profiles: profiles_json(&planned.resolved),
        profile_contracts: profile_contracts_json(&planned.resolved),
        root: serde_json::to_value(planned.logical_plan.context.root_kind).unwrap_or(Value::Null),
        grain: serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap_or(Value::Null),
        canonical_order: json!(planned.logical_plan.canonical_order.clone()),
        fingerprints: fingerprints_json(
            context,
            Some(&planned.resolved.resolved_query_fingerprint),
            Some(&planned.logical_plan_fingerprint),
            None,
            Some(&planned.predicate_ast_fingerprint),
            Some(&planned.predicate_cnf_fingerprint),
            Some(&planned.projection_dependency_fingerprint),
        ),
        operation_context: operation_context_json(context),
        logical_plan: planned.logical_plan.to_json(disclosure),
        physical_plan: json!([]),
        resolved_dependencies: plan_printer::dependencies_json(&planned.dependencies, disclosure),
        predicate_forms: plan_printer::predicate_forms_json(
            &planned.logical_plan.predicate_forms,
            disclosure,
        ),
        trusted_metadata: trusted_metadata_json(context),
        ignored_metadata: ignored_metadata_json(context),
        fallbacks: context
            .fallbacks
            .iter()
            .map(FallbackReport::to_json)
            .collect(),
        rejections: context
            .rejections
            .iter()
            .map(RejectionReport::to_json)
            .collect(),
        decode_boundaries: json!(planned.logical_plan.decode_boundaries),
        residual_predicates: plan_printer::predicate_forms_json(
            &planned.logical_plan.residual_predicates,
            disclosure,
        ),
        visibility: visibility_json(context),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: plan_only_execution_json(Some(planned), allow_protected),
    });
    finalize_explain_document(&mut value, allow_protected);
    value
}

pub(crate) fn executed_query_explain_json(executed: &ExecutedQuery) -> Value {
    if let CoveQlExecutionResult::ExplainJson(value) = &executed.result {
        return value.clone();
    }

    let planned = &executed.planned;
    let context = &planned.resolved.operation_context;
    let requested_mode = requested_mode(context);
    let effective_mode = effective_mode(requested_mode, context.security.explain_policy);
    let allow_protected = allow_protected(context);
    let disclosure = disclosure_policy(allow_protected);
    let mut diagnostics = executed
        .diagnostics
        .iter()
        .map(|diagnostic| execution_diagnostic_json(diagnostic, allow_protected))
        .collect::<Vec<_>>();
    diagnostics.extend(mode_policy_diagnostics(
        requested_mode,
        effective_mode,
        allow_protected,
    ));

    let mut value = explain_document(ExplainDocumentParts {
        schema_version: context.explain_json_schema_version.into(),
        mode: explain_mode_name(effective_mode).into(),
        primary_profile: primary_profile_json(&planned.resolved),
        profiles: profiles_json(&planned.resolved),
        profile_contracts: profile_contracts_json(&planned.resolved),
        root: serde_json::to_value(planned.logical_plan.context.root_kind).unwrap_or(Value::Null),
        grain: serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap_or(Value::Null),
        canonical_order: json!(planned.logical_plan.canonical_order.clone()),
        fingerprints: fingerprints_json(
            context,
            Some(&planned.resolved.resolved_query_fingerprint),
            Some(&planned.logical_plan_fingerprint),
            None,
            Some(&planned.predicate_ast_fingerprint),
            Some(&planned.predicate_cnf_fingerprint),
            Some(&planned.projection_dependency_fingerprint),
        ),
        operation_context: operation_context_json(context),
        logical_plan: planned.logical_plan.to_json(disclosure),
        physical_plan: json!([]),
        resolved_dependencies: plan_printer::dependencies_json(&planned.dependencies, disclosure),
        predicate_forms: plan_printer::predicate_forms_json(
            &planned.logical_plan.predicate_forms,
            disclosure,
        ),
        trusted_metadata: trusted_metadata_json(context),
        ignored_metadata: ignored_metadata_json(context),
        fallbacks: context
            .fallbacks
            .iter()
            .map(FallbackReport::to_json)
            .collect(),
        rejections: context
            .rejections
            .iter()
            .map(RejectionReport::to_json)
            .collect(),
        decode_boundaries: json!(planned.logical_plan.decode_boundaries),
        residual_predicates: plan_printer::predicate_forms_json(
            &planned.logical_plan.residual_predicates,
            disclosure,
        ),
        visibility: visibility_json(context),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: completed_execution_json(executed, allow_protected),
    });
    finalize_explain_document(&mut value, allow_protected);
    value
}

pub(crate) fn physical_planned_query_explain_json(physical: &PhysicalPlannedQuery) -> Value {
    let planned = &physical.planned;
    let context = &planned.resolved.operation_context;
    let requested_mode = requested_mode(context);
    let effective_mode = effective_mode(requested_mode, context.security.explain_policy);
    let allow_protected = allow_protected(context);
    let disclosure = disclosure_policy(allow_protected);
    let mut diagnostics = physical
        .diagnostics
        .iter()
        .map(|diagnostic| physical_diagnostic_json(diagnostic, allow_protected))
        .collect::<Vec<_>>();
    diagnostics.extend(mode_policy_diagnostics(
        requested_mode,
        effective_mode,
        allow_protected,
    ));

    let mut value = explain_document(ExplainDocumentParts {
        schema_version: context.explain_json_schema_version.into(),
        mode: explain_mode_name(effective_mode).into(),
        primary_profile: primary_profile_json(&planned.resolved),
        profiles: profiles_json(&planned.resolved),
        profile_contracts: profile_contracts_json(&planned.resolved),
        root: serde_json::to_value(planned.logical_plan.context.root_kind).unwrap_or(Value::Null),
        grain: serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap_or(Value::Null),
        canonical_order: json!(planned.logical_plan.canonical_order.clone()),
        fingerprints: fingerprints_json(
            context,
            Some(&planned.resolved.resolved_query_fingerprint),
            Some(&planned.logical_plan_fingerprint),
            Some(&physical.physical_plan_fingerprint),
            Some(&planned.predicate_ast_fingerprint),
            Some(&planned.predicate_cnf_fingerprint),
            Some(&planned.projection_dependency_fingerprint),
        ),
        operation_context: operation_context_json(context),
        logical_plan: planned.logical_plan.to_json(disclosure),
        physical_plan: physical.physical_plan.to_json(disclosure),
        resolved_dependencies: plan_printer::dependencies_json(&planned.dependencies, disclosure),
        predicate_forms: plan_printer::predicate_forms_json(
            &planned.logical_plan.predicate_forms,
            disclosure,
        ),
        trusted_metadata: trusted_metadata_json(context),
        ignored_metadata: ignored_metadata_json(context),
        fallbacks: context
            .fallbacks
            .iter()
            .map(FallbackReport::to_json)
            .chain(
                physical
                    .physical_plan
                    .fallbacks
                    .iter()
                    .map(|fallback| serde_json::to_value(fallback).unwrap_or(Value::Null)),
            )
            .collect(),
        rejections: context
            .rejections
            .iter()
            .map(RejectionReport::to_json)
            .collect(),
        decode_boundaries: json!(planned.logical_plan.decode_boundaries),
        residual_predicates: plan_printer::predicate_forms_json(
            &planned.logical_plan.residual_predicates,
            disclosure,
        ),
        visibility: visibility_json(context),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: physical_plan_only_execution_json(physical, allow_protected),
    });
    finalize_explain_document(&mut value, allow_protected);
    value
}

pub(crate) fn error_explain_json(
    phase: &str,
    diagnostics: Vec<Value>,
    rejections: Vec<Value>,
) -> Value {
    let mut value = explain_document(ExplainDocumentParts {
        schema_version: crate::EXPLAIN_JSON_SCHEMA_VERSION.into(),
        mode: "public".into(),
        primary_profile: Value::Null,
        profiles: json!([]),
        profile_contracts: json!([]),
        root: Value::Null,
        grain: Value::Null,
        canonical_order: json!([]),
        fingerprints: object([
            ("query_text", Value::Null),
            ("parsed_ast", Value::Null),
            ("resolved_query", Value::Null),
            ("predicate_ast", Value::Null),
            ("predicate_cnf", Value::Null),
            ("projection_dependency", Value::Null),
            ("logical_plan", Value::Null),
            ("physical_plan", Value::Null),
        ]),
        operation_context: object([
            ("operation", Value::Null),
            ("phase", Value::String(phase.into())),
            ("visibility_applied", Value::Bool(false)),
            ("redaction_applied", Value::Bool(false)),
        ]),
        logical_plan: json!([]),
        physical_plan: json!([]),
        resolved_dependencies: json!([]),
        predicate_forms: json!([]),
        trusted_metadata: Vec::new(),
        ignored_metadata: Vec::new(),
        fallbacks: Vec::new(),
        rejections,
        decode_boundaries: json!([]),
        residual_predicates: json!([]),
        visibility: object([
            ("applied", Value::Bool(false)),
            ("policy_id_redacted", Value::Bool(true)),
        ]),
        redactions: Vec::new(),
        warnings: Vec::new(),
        diagnostics,
        execution: object([
            ("completed", Value::Bool(false)),
            ("kind", Value::String("rejected".into())),
            ("diagnostics_phase", Value::String(phase.into())),
        ]),
    });
    finalize_explain_document(&mut value, false);
    value
}

pub(crate) fn redact_explain_value(value: &mut Value) -> bool {
    match value {
        Value::Object(object) => redact_object(object),
        Value::Array(values) => {
            let mut redacted = false;
            for value in values {
                redacted = redact_explain_value(value) || redacted;
            }
            redacted
        }
        _ => false,
    }
}

struct ExplainDocumentParts {
    schema_version: Value,
    mode: Value,
    primary_profile: Value,
    profiles: Value,
    profile_contracts: Value,
    root: Value,
    grain: Value,
    canonical_order: Value,
    fingerprints: Value,
    operation_context: Value,
    logical_plan: Value,
    physical_plan: Value,
    resolved_dependencies: Value,
    predicate_forms: Value,
    trusted_metadata: Vec<Value>,
    ignored_metadata: Vec<Value>,
    fallbacks: Vec<Value>,
    rejections: Vec<Value>,
    decode_boundaries: Value,
    residual_predicates: Value,
    visibility: Value,
    redactions: Vec<Value>,
    warnings: Vec<Value>,
    diagnostics: Vec<Value>,
    execution: Value,
}

fn explain_document(parts: ExplainDocumentParts) -> Value {
    let operation = parts
        .operation_context
        .get("operation")
        .cloned()
        .unwrap_or(Value::Null);
    let temporal_mode = parts
        .operation_context
        .get("temporal_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let visibility_applied = parts
        .operation_context
        .get("visibility_applied")
        .cloned()
        .unwrap_or(Value::Bool(false));
    let redaction_applied = parts
        .operation_context
        .get("redaction_applied")
        .cloned()
        .unwrap_or(Value::Bool(false));
    object([
        ("schema_version", parts.schema_version),
        ("mode", parts.mode),
        (
            "coveql_version",
            Value::String(crate::COVEQL_LANGUAGE_VERSION.into()),
        ),
        (
            "core_version",
            Value::String(crate::COVEQL_CORE_VERSION.into()),
        ),
        ("primary_profile", parts.primary_profile),
        ("profiles", parts.profiles),
        ("profile_contracts", parts.profile_contracts),
        ("root", parts.root),
        ("grain", parts.grain),
        ("operation", operation),
        ("temporal_mode", temporal_mode),
        ("canonical_order", parts.canonical_order),
        ("visibility_applied", visibility_applied),
        ("redaction_applied", redaction_applied),
        ("fingerprints", parts.fingerprints),
        ("operation_context", parts.operation_context),
        ("logical_plan", parts.logical_plan),
        ("physical_plan", parts.physical_plan),
        ("resolved_dependencies", parts.resolved_dependencies),
        ("predicate_forms", parts.predicate_forms),
        ("trusted_metadata", Value::Array(parts.trusted_metadata)),
        ("ignored_metadata", Value::Array(parts.ignored_metadata)),
        ("fallbacks", Value::Array(parts.fallbacks)),
        ("rejections", Value::Array(parts.rejections)),
        ("decode_boundaries", parts.decode_boundaries),
        ("residual_predicates", parts.residual_predicates),
        ("visibility", parts.visibility),
        ("redactions", Value::Array(parts.redactions)),
        ("warnings", Value::Array(parts.warnings)),
        ("diagnostics", Value::Array(parts.diagnostics)),
        ("execution", parts.execution),
    ])
}

fn finalize_explain_document(value: &mut Value, allow_protected: bool) {
    let diagnostics = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let redactions = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .get("redacted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .get("severity")
                .and_then(Value::as_str)
                .is_some_and(|severity| severity == "warning")
        })
        .cloned()
        .collect::<Vec<_>>();

    if let Some(object) = value.as_object_mut() {
        object.insert("redactions".into(), Value::Array(redactions));
        object.insert("warnings".into(), Value::Array(warnings));
    }
    if !allow_protected {
        redact_explain_value(value);
    }
}

fn fingerprints_json(
    context: &OperationContext,
    resolved_query: Option<&str>,
    logical_plan: Option<&str>,
    physical_plan: Option<&str>,
    predicate_ast: Option<&str>,
    predicate_cnf: Option<&str>,
    projection_dependency: Option<&str>,
) -> Value {
    object([
        (
            "query_text",
            context
                .request
                .query_text_fingerprint
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "parsed_ast",
            context
                .request
                .parsed_ast_fingerprint
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "resolved_query",
            resolved_query
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "predicate_ast",
            predicate_ast
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "predicate_cnf",
            predicate_cnf
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "projection_dependency",
            projection_dependency
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "logical_plan",
            logical_plan
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "physical_plan",
            physical_plan
                .map(|fingerprint| Value::String(fingerprint.into()))
                .unwrap_or(Value::Null),
        ),
    ])
}

fn operation_context_json(context: &OperationContext) -> Value {
    object([
        (
            "language_version",
            Value::String(context.language_version.into()),
        ),
        ("core_version", Value::String(context.core_version.into())),
        (
            "grammar_version",
            Value::String(context.grammar_version.into()),
        ),
        (
            "resolved_ast_version",
            Value::String(context.resolved_ast_version.into()),
        ),
        (
            "logical_plan_version",
            Value::String(context.logical_plan_version.into()),
        ),
        (
            "physical_plan_version",
            Value::String(context.physical_plan_version.into()),
        ),
        (
            "projection_dependency_contract_version",
            Value::String(context.projection_dependency_contract_version.into()),
        ),
        (
            "predicate_normal_form_version",
            Value::String(context.predicate_normal_form_version.into()),
        ),
        (
            "explain_json_schema_version",
            Value::String(context.explain_json_schema_version.into()),
        ),
        (
            "profile_contract_version",
            Value::String(context.profile_contract_version.into()),
        ),
        (
            "bridge_contract_version",
            Value::String(context.bridge_contract_version.into()),
        ),
        (
            "operation",
            Value::String(selected_operation_name(&context.request.selected_operation).into()),
        ),
        ("file_len", json!(context.file.file_len)),
        (
            "file_id",
            Value::String(crate::hex_lower(&context.file.file_id)),
        ),
        ("footer_crc32c", json!(context.file.footer_crc32c)),
        ("primary_profile", json!(context.file.primary_profile)),
        (
            "dataset_id",
            context
                .snapshot
                .dataset_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "snapshot_id",
            context
                .snapshot
                .snapshot_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "selected_snapshot_ref",
            context
                .snapshot
                .selected_snapshot_ref
                .map(|value| json!(value))
                .unwrap_or(Value::Null),
        ),
        (
            "schema_fingerprint",
            context
                .snapshot
                .schema_fingerprint
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "semantic_map_fingerprint",
            context
                .snapshot
                .semantic_map_fingerprint
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "projection_version",
            context
                .semantic_map
                .projection_version
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "file_digest",
            context
                .snapshot
                .file_digest
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "authority",
            context
                .snapshot
                .authority
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "dataset",
            serde_json::to_value(&context.dataset).unwrap_or(Value::Null),
        ),
        (
            "temporal_mode",
            Value::String(temporal_mode_name(&context.temporal.mode).into()),
        ),
        (
            "temporal",
            serde_json::to_value(&context.temporal).unwrap_or(Value::Null),
        ),
        (
            "branch",
            serde_json::to_value(&context.branch).unwrap_or(Value::Null),
        ),
        (
            "tombstone",
            serde_json::to_value(&context.tombstone).unwrap_or(Value::Null),
        ),
        ("visibility_applied", Value::Bool(true)),
        ("redaction_applied", Value::Bool(true)),
        (
            "cache",
            Value::String(cache_state_name(context.semantic_map.cache_state).into()),
        ),
        (
            "security",
            serde_json::to_value(&context.security).unwrap_or(Value::Null),
        ),
    ])
}

fn visibility_json(context: &OperationContext) -> Value {
    object([
        ("applied", Value::Bool(true)),
        (
            "policy_id_redacted",
            Value::Bool(
                context.security.metadata_disclosure_policy
                    != MetadataDisclosurePolicy::AllowProtected,
            ),
        ),
        (
            "visibility_policy",
            serde_json::to_value(&context.security.visibility_policy).unwrap_or(Value::Null),
        ),
        (
            "redaction_policy",
            serde_json::to_value(&context.security.redaction_policy).unwrap_or(Value::Null),
        ),
    ])
}

fn trusted_metadata_json(context: &OperationContext) -> Vec<Value> {
    context
        .optional_metadata
        .iter()
        .filter(|outcome| outcome.status == OptionalMetadataStatus::Trusted)
        .map(crate::OptionalMetadataOutcome::to_json)
        .collect()
}

fn ignored_metadata_json(context: &OperationContext) -> Vec<Value> {
    context
        .optional_metadata
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.status,
                OptionalMetadataStatus::Ignored
                    | OptionalMetadataStatus::Disabled
                    | OptionalMetadataStatus::NotRequested
            )
        })
        .map(crate::OptionalMetadataOutcome::to_json)
        .collect()
}

fn plan_only_execution_json(planned: Option<&PlannedQuery>, allow_protected: bool) -> Value {
    object([
        ("completed", Value::Bool(false)),
        ("kind", Value::String("plan_explanation".into())),
        (
            "output_mode",
            planned
                .map(|planned| output_mode_json(&planned.resolved.output_mode))
                .unwrap_or(Value::Null),
        ),
        (
            "output_grain",
            planned.map(output_grain_json).unwrap_or(Value::Null),
        ),
        (
            "fallback_authority",
            planned
                .map(|_| Value::String("materialized_baseline_available".into()))
                .unwrap_or(Value::Null),
        ),
        ("optimized_execution_authority", Value::Null),
        ("row_counts", Value::Null),
        ("output_fingerprint", Value::Null),
        ("materialization_boundaries", json!([])),
        ("pushdown_report", plan_only_pushdown_report_json()),
        (
            "storage",
            planned
                .map(|planned| storage_diagnostics_json(planned, allow_protected))
                .unwrap_or_else(|| json!({})),
        ),
        (
            "coded_execution",
            planned
                .map(planned_coded_execution_json)
                .unwrap_or(Value::Null),
        ),
        ("diagnostics", json!([])),
    ])
}

fn physical_plan_only_execution_json(
    physical: &PhysicalPlannedQuery,
    allow_protected: bool,
) -> Value {
    object([
        ("completed", Value::Bool(false)),
        ("kind", Value::String("physical_plan_explanation".into())),
        (
            "output_mode",
            output_mode_json(&physical.planned.resolved.output_mode),
        ),
        ("output_grain", output_grain_json(&physical.planned)),
        (
            "fallback_authority",
            Value::String("materialized_baseline_available".into()),
        ),
        ("optimized_execution_authority", Value::Null),
        ("row_counts", Value::Null),
        ("output_fingerprint", Value::Null),
        ("materialization_boundaries", json!([])),
        ("pushdown_report", plan_only_pushdown_report_json()),
        (
            "storage",
            physical_storage_diagnostics_json(physical, allow_protected),
        ),
        ("coded_execution", physical_coded_execution_json(physical)),
        ("diagnostics", json!([])),
    ])
}

fn completed_execution_json(executed: &ExecutedQuery, allow_protected: bool) -> Value {
    object([
        ("completed", Value::Bool(true)),
        ("kind", Value::String("materialized_execution".into())),
        (
            "output_mode",
            output_mode_json(&executed.planned.resolved.output_mode),
        ),
        ("output_grain", output_grain_json(&executed.planned)),
        (
            "fallback_authority",
            Value::String(
                if executed.authority.materialized_fallback {
                    "materialized_fallback"
                } else {
                    "materialized_baseline_available"
                }
                .into(),
            ),
        ),
        (
            "optimized_execution_authority",
            serde_json::to_value(&executed.authority).unwrap_or(Value::Null),
        ),
        (
            "row_counts",
            serde_json::to_value(&executed.row_counts).unwrap_or(Value::Null),
        ),
        (
            "output_fingerprint",
            Value::String(executed.output_fingerprint.clone()),
        ),
        (
            "materialization_boundaries",
            materialization_boundaries_json(&executed.planned),
        ),
        (
            "pushdown_report",
            serde_json::to_value(&executed.pushdown_report).unwrap_or(Value::Null),
        ),
        (
            "evidence_authority",
            executed
                .evidence_authority
                .map(evidence_authority_name)
                .map(|name| Value::String(name.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "storage",
            storage_diagnostics_json(&executed.planned, allow_protected),
        ),
        (
            "coded_execution",
            planned_coded_execution_json(&executed.planned),
        ),
        (
            "diagnostics",
            Value::Array(
                executed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| execution_diagnostic_json(diagnostic, allow_protected))
                    .collect(),
            ),
        ),
    ])
}

fn planned_coded_execution_json(planned: &PlannedQuery) -> Value {
    let shape = crate::kernel_plan::diagnostic_kernel_shape_for_plan(
        planned,
        KernelFallbackReason::PhysicalPlanRequired,
    );
    object([
        ("eligible", Value::Null),
        (
            "coded_suitability",
            Value::String("physical_plan_required".into()),
        ),
        ("stage", Value::String("logical_plan".into())),
        (
            "fallback_reason",
            Value::String("physical_plan_required".into()),
        ),
        ("fallback_reasons", json!(["physical_plan_required"])),
        (
            "residual_verification",
            Value::Bool(shape.residual_verification_required),
        ),
        (
            "residual_verification_required",
            Value::Bool(shape.residual_verification_required),
        ),
        ("pushed_filters", pushed_filter_forms_json(planned)),
        ("pushed_columns", pushed_columns_json(planned)),
        (
            "operator_contracts",
            serde_json::to_value(&shape.operator_contracts).unwrap_or(Value::Null),
        ),
        (
            "decode_boundaries",
            serde_json::to_value(&shape.decode_boundaries).unwrap_or(Value::Null),
        ),
        (
            "bridge_decisions",
            serde_json::to_value(&shape.bridge_decisions).unwrap_or(Value::Null),
        ),
        (
            "kernel_shape",
            serde_json::to_value(shape).unwrap_or(Value::Null),
        ),
    ])
}

fn physical_coded_execution_json(physical: &PhysicalPlannedQuery) -> Value {
    match crate::kernel_plan::compile_kernel_shape(physical) {
        Ok(shape) => object([
            ("eligible", Value::Bool(true)),
            ("coded_suitability", Value::String("eligible".into())),
            ("stage", Value::String("physical_plan".into())),
            ("fallback_reason", Value::Null),
            ("fallback_reasons", json!([])),
            (
                "residual_verification",
                Value::Bool(shape.public.residual_verification_required),
            ),
            (
                "residual_verification_required",
                Value::Bool(shape.public.residual_verification_required),
            ),
            (
                "pushed_filters",
                pushed_filter_forms_json(&physical.planned),
            ),
            ("pushed_columns", pushed_columns_json(&physical.planned)),
            (
                "operator_contracts",
                serde_json::to_value(&shape.public.operator_contracts).unwrap_or(Value::Null),
            ),
            (
                "decode_boundaries",
                serde_json::to_value(&shape.public.decode_boundaries).unwrap_or(Value::Null),
            ),
            (
                "bridge_decisions",
                serde_json::to_value(&shape.public.bridge_decisions).unwrap_or(Value::Null),
            ),
            (
                "kernel_shape",
                serde_json::to_value(shape.public).unwrap_or(Value::Null),
            ),
        ]),
        Err(reason) => {
            let shape =
                crate::kernel_plan::diagnostic_kernel_shape_for_plan(&physical.planned, reason);
            object([
                ("eligible", Value::Bool(false)),
                ("coded_suitability", Value::String("fallback".into())),
                ("stage", Value::String("physical_plan".into())),
                ("fallback_reason", Value::String(format!("{reason:?}"))),
                ("fallback_reasons", json!([format!("{reason:?}")])),
                (
                    "residual_verification",
                    Value::Bool(shape.residual_verification_required),
                ),
                (
                    "residual_verification_required",
                    Value::Bool(shape.residual_verification_required),
                ),
                (
                    "pushed_filters",
                    pushed_filter_forms_json(&physical.planned),
                ),
                ("pushed_columns", pushed_columns_json(&physical.planned)),
                (
                    "operator_contracts",
                    serde_json::to_value(&shape.operator_contracts).unwrap_or(Value::Null),
                ),
                (
                    "decode_boundaries",
                    serde_json::to_value(&shape.decode_boundaries).unwrap_or(Value::Null),
                ),
                (
                    "bridge_decisions",
                    serde_json::to_value(&shape.bridge_decisions).unwrap_or(Value::Null),
                ),
                (
                    "kernel_shape",
                    serde_json::to_value(shape).unwrap_or(Value::Null),
                ),
            ])
        }
    }
}

fn pushed_filter_forms_json(planned: &PlannedQuery) -> Value {
    let forms = planned
        .logical_plan
        .predicate_forms
        .iter()
        .filter(|form| {
            matches!(
                form.placement,
                PredicatePlacement::PreReconstruction
                    | PredicatePlacement::Association
                    | PredicatePlacement::Evidence
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut filters = match plan_printer::predicate_forms_json(
        &forms,
        planned
            .resolved
            .operation_context
            .security
            .metadata_disclosure_policy,
    ) {
        Value::Array(filters) => filters,
        other => vec![other],
    };
    for contract in &planned.dependencies.projection_contracts {
        for predicate in &contract.pushed_predicates {
            filters.push(json!({
                "kind": "projection_filter",
                "placement": "projection_readback",
                "projection_id": contract.projection_id.as_str(),
                "predicate": predicate,
                "residual_required": false,
            }));
        }
    }
    Value::Array(filters)
}

fn pushed_columns_json(planned: &PlannedQuery) -> Value {
    let mut columns = planned
        .dependencies
        .projection_contracts
        .iter()
        .flat_map(|contract| contract.pushed_columns.iter().cloned())
        .collect::<Vec<_>>();
    columns.sort();
    columns.dedup();
    json!(columns)
}

fn evidence_authority_name(authority: EvidenceAuthority) -> &'static str {
    match authority {
        EvidenceAuthority::CoveMapMetadata => "cove_map_metadata",
        EvidenceAuthority::MaterializedEvidenceObjects => "materialized_evidence_objects",
        EvidenceAuthority::Missing => "missing",
    }
}

fn plan_only_pushdown_report_json() -> Value {
    object([
        ("enabled", Value::Null),
        ("outcome", Value::String("not_executed".into())),
        ("decisions", json!([])),
        ("counters", json!({})),
        ("residual_predicates", json!([])),
        ("decode_boundaries", json!([])),
    ])
}

fn storage_diagnostics_json(planned: &PlannedQuery, allow_protected: bool) -> Value {
    let dependencies = &planned.dependencies;
    if allow_protected {
        object([
            (
                "selected_object_type_ids",
                serde_json::to_value(&dependencies.object_type_ids).unwrap_or(Value::Null),
            ),
            (
                "selected_object_type_names",
                serde_json::to_value(&dependencies.object_type_names).unwrap_or(Value::Null),
            ),
            (
                "requested_property_ids",
                serde_json::to_value(&dependencies.property_ids).unwrap_or(Value::Null),
            ),
            (
                "requested_evidence_metadata_keys",
                serde_json::to_value(&dependencies.evidence_fields).unwrap_or(Value::Null),
            ),
            (
                "loaded_system_columns",
                serde_json::to_value(&dependencies.system_fields).unwrap_or(Value::Null),
            ),
            (
                "loaded_property_columns",
                serde_json::to_value(&dependencies.property_ids).unwrap_or(Value::Null),
            ),
            (
                "temporal_cut",
                serde_json::to_value(&planned.resolved.temporal).unwrap_or(Value::Null),
            ),
            (
                "branch",
                serde_json::to_value(&planned.resolved.branch).unwrap_or(Value::Null),
            ),
            (
                "include_tombstones",
                Value::Bool(planned.resolved.tombstone.include_tombstones),
            ),
            ("segment_filters", json!([])),
        ])
    } else {
        object([
            (
                "selected_object_type_count",
                json!(dependencies.object_type_ids.len()),
            ),
            (
                "requested_property_count",
                json!(dependencies.property_ids.len()),
            ),
            (
                "requested_evidence_metadata_key_count",
                json!(dependencies.evidence_fields.len()),
            ),
            (
                "loaded_system_columns",
                serde_json::to_value(&dependencies.system_fields).unwrap_or(Value::Null),
            ),
            (
                "temporal_mode",
                Value::String(temporal_mode_name(&planned.resolved.temporal.mode).into()),
            ),
            (
                "include_tombstones",
                Value::Bool(planned.resolved.tombstone.include_tombstones),
            ),
            ("segment_filters", json!([])),
            ("redacted", Value::Bool(true)),
        ])
    }
}

fn physical_storage_diagnostics_json(
    physical: &PhysicalPlannedQuery,
    allow_protected: bool,
) -> Value {
    let mut base = storage_diagnostics_json(&physical.planned, allow_protected);
    let physical_value = if allow_protected {
        json!({
            "proof_validation_report": physical.proof_validation_report,
            "index_capability_report": physical.index_capability_report,
            "layout_range_plan": physical.layout_range_plan,
            "zero_copy_eligibility": physical.zero_copy_eligibility,
            "sidecar_validations": physical.sidecar_validations,
        })
    } else {
        json!({
            "coverage_candidate_count": physical.proof_validation_report.accepted_proof_count,
            "index_lookup_candidate_count": physical.index_capability_report.lookup_candidates,
            "layout_range_count": physical.layout_range_plan.range_count,
            "zero_copy_candidate": physical.zero_copy_eligibility.candidate,
            "sidecar_validation_count": physical.sidecar_validations.len(),
            "redacted": true,
        })
    };
    if let Some(object) = base.as_object_mut() {
        object.insert("physical_planning".into(), physical_value);
    }
    base
}

fn materialization_boundaries_json(planned: &PlannedQuery) -> Value {
    let readback = match &planned.resolved.root {
        crate::ResolvedRoot::Table(_) => "coveql_table_projection_readback",
        crate::ResolvedRoot::Projection(_) => "cove_map_projection_readback",
        crate::ResolvedRoot::Node(_) => "coveql_graph_node_object_readback",
        crate::ResolvedRoot::Edge(_) => "coveql_graph_edge_association_readback",
        crate::ResolvedRoot::Object(_)
        | crate::ResolvedRoot::Association(_)
        | crate::ResolvedRoot::Evidence(_) => "cove_o_materialized_readback",
    };
    json!([
        readback,
        "materialized_predicate_evaluation",
        "materialized_sort_and_pagination",
        "output_boundary"
    ])
}

fn output_mode_json(mode: &CoveQlOutputMode) -> Value {
    serde_json::to_value(mode).unwrap_or(Value::Null)
}

fn output_grain_json(planned: &PlannedQuery) -> Value {
    serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap_or(Value::Null)
}

fn coveql_diagnostic_json(diagnostic: &CoveQlDiagnostic, allow_protected: bool) -> Value {
    diagnostic_json(
        &diagnostic.code,
        diagnostic.severity,
        &diagnostic.message,
        &diagnostic.phase,
        &diagnostic.safe_details,
        diagnostic.redacted,
        allow_protected,
    )
}

fn logical_diagnostic_json(diagnostic: &LogicalPlanDiagnostic, allow_protected: bool) -> Value {
    diagnostic_json(
        &diagnostic.code,
        diagnostic.severity,
        &diagnostic.message,
        &diagnostic.phase,
        &diagnostic.safe_details,
        diagnostic.redacted,
        allow_protected,
    )
}

fn physical_diagnostic_json(diagnostic: &PhysicalPlanDiagnostic, allow_protected: bool) -> Value {
    diagnostic_json(
        &diagnostic.code,
        diagnostic.severity,
        &diagnostic.message,
        &diagnostic.phase,
        &diagnostic.safe_details,
        diagnostic.redacted,
        allow_protected,
    )
}

fn execution_diagnostic_json(diagnostic: &ExecutionDiagnostic, allow_protected: bool) -> Value {
    diagnostic_json(
        &diagnostic.code,
        diagnostic.severity,
        &diagnostic.message,
        &diagnostic.phase,
        &diagnostic.safe_details,
        diagnostic.redacted,
        allow_protected,
    )
}

fn diagnostic_json(
    code: &str,
    severity: DiagnosticSeverity,
    message: &str,
    phase: &str,
    safe_details: &Value,
    redacted: bool,
    allow_protected: bool,
) -> Value {
    let span = safe_details.get("span").cloned().unwrap_or(Value::Null);
    let mut safe_details = safe_details.clone();
    let policy_redacted = if allow_protected {
        false
    } else {
        redact_explain_value(&mut safe_details)
    };
    object([
        ("code", Value::String(code.into())),
        (
            "severity",
            Value::String(diagnostic_severity_name(severity).into()),
        ),
        ("message", Value::String(message.into())),
        ("span", span),
        ("phase", Value::String(phase.into())),
        ("safe_details", safe_details),
        ("redacted", Value::Bool(redacted || policy_redacted)),
    ])
}

fn mode_policy_diagnostics(
    requested: ExplainMode,
    effective: ExplainMode,
    allow_protected: bool,
) -> Vec<Value> {
    if explain_mode_policy_rank(requested) <= explain_mode_policy_rank(effective) {
        return Vec::new();
    }
    let safe_details = object([
        (
            "requested_mode",
            Value::String(explain_mode_name(requested).into()),
        ),
        (
            "effective_mode",
            Value::String(explain_mode_name(effective).into()),
        ),
    ]);
    vec![diagnostic_json(
        "E_SECURITY_DISCLOSURE_FORBIDDEN",
        DiagnosticSeverity::Warning,
        "requested explain mode exceeds the active explain disclosure policy; output was clamped",
        "security",
        &safe_details,
        true,
        allow_protected,
    )]
}

fn requested_mode(context: &OperationContext) -> ExplainMode {
    match &context.request.selected_operation {
        CoveQlSelectedOperation::Explain { mode, .. } => *mode,
        _ => ExplainMode::Public,
    }
}

fn effective_mode(requested: ExplainMode, policy: ExplainDisclosurePolicy) -> ExplainMode {
    let maximum = match policy {
        ExplainDisclosurePolicy::PublicOnly => ExplainMode::Public,
        ExplainDisclosurePolicy::Developer => ExplainMode::Developer,
        ExplainDisclosurePolicy::Proof => ExplainMode::Proof,
        ExplainDisclosurePolicy::Forensic => ExplainMode::Forensic,
    };
    if explain_mode_policy_rank(requested) <= explain_mode_policy_rank(maximum) {
        requested
    } else {
        maximum
    }
}

fn allow_protected(context: &OperationContext) -> bool {
    context.security.metadata_disclosure_policy == MetadataDisclosurePolicy::AllowProtected
}

fn disclosure_policy(allow_protected: bool) -> MetadataDisclosurePolicy {
    if allow_protected {
        MetadataDisclosurePolicy::AllowProtected
    } else {
        MetadataDisclosurePolicy::DenyProtected
    }
}

fn profiles_json(resolved: &ResolvedQuery) -> Value {
    json!(resolved
        .parsed
        .profiles
        .iter()
        .map(|profile| profile.as_str())
        .collect::<Vec<_>>())
}

fn primary_profile_json(resolved: &ResolvedQuery) -> Value {
    resolved
        .parsed
        .profiles
        .first()
        .map(|profile| Value::String(profile.as_str().into()))
        .unwrap_or(Value::Null)
}

fn profile_contracts_json(resolved: &ResolvedQuery) -> Value {
    json!(resolved
        .parsed
        .profiles
        .iter()
        .map(|profile| {
            let contract = crate::coveql_profile_contract(*profile);
            let mut contract_json = json!({
                "profile_id": profile.as_str(),
                "profile_version": contract.profile_version,
                "implemented": contract.implemented,
                "supported_roots": contract.supported_roots,
                "input_grains": contract.input_grains,
                "root_authority": contract.root_authority,
                "identity_model": contract.identity_model,
                "canonical_order": contract.canonical_order,
                "temporal_capabilities": contract.temporal_capabilities,
                "evidence_targets": contract.evidence_targets,
                "relationship_capabilities": contract.relationship_capabilities,
                "profile_methods": contract.profile_methods,
                "bridge_requirements": contract.bridge_requirements,
                "aggregate_rules": contract.aggregate_rules,
                "null_missing_nan_rules": contract.null_missing_nan_rules,
                "security_barriers": contract.security_barriers,
                "materialization_boundaries": contract.materialization_boundaries,
                "output_modes": contract.output_modes,
            });
            if let Some(object) = contract_json.as_object_mut() {
                if let Some(details) = query_specific_profile_contract_json(resolved, *profile) {
                    object.insert("query_contract".into(), details);
                }
            }
            contract_json
        })
        .collect::<Vec<_>>())
}

fn query_specific_profile_contract_json(
    resolved: &ResolvedQuery,
    profile: crate::CoveQlProfileId,
) -> Option<Value> {
    match profile {
        crate::CoveQlProfileId::Table => {
            let crate::ResolvedRoot::Table(table) = &resolved.root else {
                return None;
            };
            Some(json!({
                "table_id": table.table_surface_contract.table_id,
                "table_name": table.table_surface_contract.table_name,
                "contract_version": table.table_surface_contract.contract_version,
                "authority_kind": table.table_surface_contract.authority_kind,
                "row_grain": table.table_surface_contract.row_grain,
                "row_identity": table.table_surface_contract.row_identity,
                "canonical_order": table.table_surface_contract.canonical_order,
                "temporal_authority": table.table_surface_contract.temporal_authority,
                "evidence_capabilities": table.table_surface_contract.evidence_capabilities,
                "projection_dependency_contract_id": table.table_surface_contract.projection_dependency_contract_id,
                "datafusion_interop_contract": table.table_surface_contract.datafusion_interop_contract,
            }))
        }
        crate::CoveQlProfileId::Graph => {
            if resolved.method_chain.traversals.is_empty() {
                return None;
            }
            Some(json!({
                "traversals": resolved.method_chain.traversals.iter().map(|traversal| {
                    json!({
                        "direction": traversal.direction,
                        "edge_type": traversal.edge.association.type_name,
                        "target_label": traversal.target.as_ref().map(|target| target.label.clone()),
                        "min_depth": traversal.min_depth,
                        "max_depth": traversal.max_depth,
                        "mode": traversal.mode.as_str(),
                        "distinct": traversal.distinct.as_str(),
                        "contract_present": traversal.contract.is_some(),
                        "contract": traversal.contract.as_ref().map(|contract| {
                            json!({
                                "contract_version": contract.contract_version,
                                "allow_variable_length": contract.allow_variable_length,
                                "supported_modes": contract.supported_modes,
                                "supported_distinct_policies": contract.supported_distinct_policies,
                                "max_depth": contract.max_depth,
                                "max_fanout_per_node": contract.max_fanout_per_node,
                                "max_paths": contract.max_paths,
                                "max_frontier": contract.max_frontier,
                                "path_identity": contract.path_identity,
                                "hidden_endpoint_policy": contract.hidden_endpoint_policy,
                                "ordering_policy": contract.ordering_policy,
                                "execution_authority": contract.execution_authority,
                            })
                        }),
                    })
                }).collect::<Vec<_>>()
            }))
        }
        crate::CoveQlProfileId::Object => None,
    }
}

fn default_profile_contracts_json() -> Value {
    json!([{
        "profile_id": crate::CoveQlProfileId::Object.as_str(),
        "profile_version": crate::COVEQL_OBJECT_PROFILE_VERSION,
        "implemented": true,
    }])
}

fn root_json(root: &ResolvedRoot) -> Value {
    Value::String(
        match root {
            ResolvedRoot::Object(_) => "object",
            ResolvedRoot::Association(_) => "association",
            ResolvedRoot::Node(_) => "node",
            ResolvedRoot::Edge(_) => "edge",
            ResolvedRoot::Table(_) => "table",
            ResolvedRoot::Projection(_) => "projection",
            ResolvedRoot::Evidence(_) => "evidence",
        }
        .into(),
    )
}

fn resolved_grain_json(root: &ResolvedRoot) -> Value {
    Value::String(
        match root {
            ResolvedRoot::Object(_) => "object_state",
            ResolvedRoot::Association(_) => "association_state",
            ResolvedRoot::Node(_) => "graph_node_state",
            ResolvedRoot::Edge(_) => "graph_edge_state",
            ResolvedRoot::Table(_) => "visible_table_row",
            ResolvedRoot::Projection(_) => "projection_row",
            ResolvedRoot::Evidence(_) => "evidence_row",
        }
        .into(),
    )
}

fn selected_operation_name(operation: &CoveQlSelectedOperation) -> &'static str {
    match operation {
        CoveQlSelectedOperation::Object => "object_reconstruction",
        CoveQlSelectedOperation::Association => "association_read",
        CoveQlSelectedOperation::GraphNode => "graph_node_read",
        CoveQlSelectedOperation::GraphEdge => "graph_edge_read",
        CoveQlSelectedOperation::Table => "table_scan",
        CoveQlSelectedOperation::Projection => "projection_read",
        CoveQlSelectedOperation::Evidence => "evidence_read",
        CoveQlSelectedOperation::IndexOnlyAnswer => "index_only_answer",
        CoveQlSelectedOperation::ArrowExport { .. } => "arrow_export",
        CoveQlSelectedOperation::Explain { target, .. } => match target {
            crate::CoveQlExplainTarget::Object => "explain_object",
            crate::CoveQlExplainTarget::Association => "explain_association",
            crate::CoveQlExplainTarget::GraphNode => "explain_graph_node",
            crate::CoveQlExplainTarget::GraphEdge => "explain_graph_edge",
            crate::CoveQlExplainTarget::Table => "explain_table",
            crate::CoveQlExplainTarget::Projection => "explain_projection",
            crate::CoveQlExplainTarget::Evidence => "explain_evidence",
            crate::CoveQlExplainTarget::IndexOnlyAnswer => "explain_index_only_answer",
            crate::CoveQlExplainTarget::ArrowExport { .. } => "explain_arrow_export",
        },
    }
}

fn temporal_mode_name(mode: &TemporalMode) -> &'static str {
    match mode {
        TemporalMode::Latest => "latest",
        TemporalMode::AsOfCsn(_) => "as_of_csn",
        TemporalMode::AsOfTimestampMicros(_) => "as_of_timestamp_micros",
        TemporalMode::HistoryRecords => "history_records",
        TemporalMode::HistoryStates => "history_states",
        TemporalMode::HistoryRecordsAndStates => "history_records_and_states",
        TemporalMode::ChangesRecords => "changes_records",
        TemporalMode::ChangesStateTransitions => "changes_state_transitions",
        TemporalMode::ChangesPropertyDiffs => "changes_property_diffs",
        TemporalMode::ChangesFinalObjects => "changes_final_rows",
    }
}

fn cache_state_name(state: crate::CacheState) -> &'static str {
    match state {
        crate::CacheState::Disabled => "disabled",
        crate::CacheState::HookRegistered => "hook_registered",
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

fn explain_mode_policy_rank(mode: ExplainMode) -> u8 {
    match mode {
        ExplainMode::Public => 0,
        ExplainMode::Developer => 1,
        ExplainMode::Proof | ExplainMode::Coded => 2,
        ExplainMode::Forensic => 3,
    }
}

fn redact_object(object: &mut Map<String, Value>) -> bool {
    let mut redacted = false;
    for (key, value) in object.iter_mut() {
        if key_should_redact(key) && !value.is_null() {
            *value = Value::String("<redacted>".into());
            redacted = true;
        } else {
            redacted = redact_explain_value(value) || redacted;
        }
    }
    redacted
}

fn key_should_redact(key: &str) -> bool {
    matches!(
        key,
        "type_name"
            | "type_names"
            | "display_name"
            | "projection_id"
            | "projection_ids"
            | "projection_column"
            | "projection_columns"
            | "function_id"
            | "deterministic_function_ids"
            | "field"
            | "evidence_field_id"
            | "evidence_fields"
            | "object_type_id"
            | "object_type_ids"
            | "object_type_names"
            | "property_id"
            | "property_ids"
            | "selected_object_type_ids"
            | "selected_object_type_names"
            | "requested_property_ids"
            | "requested_evidence_metadata_keys"
            | "loaded_property_columns"
            | "association_type"
            | "association_type_ids"
            | "canonical"
            | "literal"
            | "left"
            | "right"
            | "expr"
            | "items"
            | "safe_details"
            | "security"
            | "security_scope"
            | "code_domain_id"
            | "code_domains"
            | "code_domain"
            | "execution_code_domains"
            | "physical_planning"
            | "principal_or_session"
            | "visibility_policy"
            | "redaction_policy"
            | "policy_id"
            | "cache_id"
            | "index_artifact_id"
            | "coverage_set_checksum"
            | "proof_id"
            | "proof_record"
            | "proof_validation_report"
            | "file_id"
            | "footer_crc32c"
            | "snapshot_id"
            | "row_counts"
            | "segments_seen"
            | "segments_skipped"
            | "rows_seen"
            | "rows_candidates"
            | "rows_after_candidate_retain"
            | "candidate_goids"
            | "branch_key"
            | "segment_id"
            | "segment_ids"
            | "input_rows"
            | "filtered_rows"
            | "output_rows"
            | "coverage_degree"
            | "sidecar_id"
            | "source_goid"
            | "target_goid"
            | "fields"
    )
}

fn object<const N: usize>(items: [(&'static str, Value); N]) -> Value {
    let mut map = Map::new();
    for (key, value) in items {
        map.insert(key.into(), value);
    }
    Value::Object(map)
}

fn string_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn array_len(value: &Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn render_compact(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}
