use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

use crate::CoveError;

use super::{
    model::{
        QueryDiscoveryDiagnostic, QueryDiscoveryDiagnosticSeverity, QueryDiscoveryManifest,
        QueryDiscoveryParseOptions, QueryDiscoveryValidationContext,
        QueryDiscoveryValidationReport, QueryDiscoveryValidationStatus,
    },
    query_discovery_error, QUERY_DISCOVERY_ADVISORY_AUTHORITY,
    QUERY_DISCOVERY_CANONICALIZATION_JCS, QUERY_DISCOVERY_SCHEMA_V1,
};

const JSON_SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
const JSON_SAFE_INTEGER_MIN: i64 = -9_007_199_254_740_991;

const REQUIRED_TOP_LEVEL_FIELDS: &[&str] = &[
    "schema",
    "canonicalization",
    "authority",
    "source_binding",
    "coveql",
    "surfaces",
    "policy",
];

const REQUIRED_OBJECT_FIELDS: &[&str] = &["source_binding", "coveql", "surfaces", "policy"];

const FORBIDDEN_PAYLOAD_KEYS: &[&str] = &[
    "protected_values",
    "copied_payloads",
    "copied_source_payloads",
    "token_ids",
    "vectors",
    "embeddings",
    "prompt_text",
    "prompt_context",
    "hidden_sidecar_payloads",
    "hidden_metadata",
    "training_samples",
];

pub fn validate_query_discovery_manifest(
    manifest: &QueryDiscoveryManifest,
    context: QueryDiscoveryValidationContext,
) -> QueryDiscoveryValidationReport {
    let mut diagnostics = Vec::new();
    let mut validation_status = QueryDiscoveryValidationStatus::Valid;
    if context.source_binding_valid == Some(false) {
        push_validation_diagnostic(
            &mut diagnostics,
            "QD_SOURCE_BINDING_STALE",
            QueryDiscoveryDiagnosticSeverity::Error,
            "query-discovery source binding is stale",
            "/source_binding",
        );
        validation_status = QueryDiscoveryValidationStatus::Stale;
    }
    if context.policy_scope_compatible == Some(false) {
        push_validation_diagnostic(
            &mut diagnostics,
            "QD_POLICY_SCOPE_INCOMPATIBLE",
            QueryDiscoveryDiagnosticSeverity::Error,
            "query-discovery policy scope is incompatible",
            "/policy",
        );
        validation_status = QueryDiscoveryValidationStatus::Invalid;
    }
    validate_source_binding_fields(
        manifest.value(),
        &context,
        &mut diagnostics,
        &mut validation_status,
    );
    validate_policy_binding_fields(
        manifest.value(),
        &context,
        &mut diagnostics,
        &mut validation_status,
    );

    QueryDiscoveryValidationReport {
        validation_status,
        validation_flags: context.validation_flags,
        diagnostics,
    }
}

fn validate_source_binding_fields(
    manifest: &Value,
    context: &QueryDiscoveryValidationContext,
    diagnostics: &mut Vec<QueryDiscoveryDiagnostic>,
    status: &mut QueryDiscoveryValidationStatus,
) {
    let binding = manifest.get("source_binding").unwrap_or(&Value::Null);
    for (field, expected, pointer) in [
        (
            "source_kind",
            context.expected_source_kind.as_deref(),
            "/source_binding/source_kind",
        ),
        (
            "file_digest",
            context.expected_file_digest.as_deref(),
            "/source_binding/file_digest",
        ),
        (
            "file_length",
            context.expected_file_length.as_deref(),
            "/source_binding/file_length",
        ),
        (
            "header_crc32c",
            context.expected_header_crc32c.as_deref(),
            "/source_binding/header_crc32c",
        ),
        (
            "footer_crc32c",
            context.expected_footer_crc32c.as_deref(),
            "/source_binding/footer_crc32c",
        ),
        (
            "schema_fingerprint",
            context.expected_schema_fingerprint.as_deref(),
            "/source_binding/schema_fingerprint",
        ),
        (
            "dictionary_crc32c",
            context.expected_dictionary_crc32c.as_deref(),
            "/source_binding/dictionary_crc32c",
        ),
        (
            "covm_snapshot_digest",
            context.expected_covm_snapshot_digest.as_deref(),
            "/source_binding/covm_snapshot_digest",
        ),
    ] {
        compare_binding_field(binding, field, expected, pointer, diagnostics, status);
    }
    if !context.expected_member_file_digests.is_empty() {
        let members = binding
            .get("members")
            .and_then(Value::as_array)
            .map(|members| {
                members
                    .iter()
                    .filter_map(|member| {
                        Some((
                            member.get("member_id")?.as_str()?.to_string(),
                            member.get("file_digest")?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        for (member_id, expected_digest) in &context.expected_member_file_digests {
            if members.get(member_id) != Some(expected_digest) {
                push_validation_diagnostic(
                    diagnostics,
                    "QD_COVM_MEMBER_STALE",
                    QueryDiscoveryDiagnosticSeverity::Error,
                    "query-discovery COVM member binding is stale",
                    "/source_binding/members",
                );
                if *status != QueryDiscoveryValidationStatus::Invalid {
                    *status = QueryDiscoveryValidationStatus::Stale;
                }
            }
        }
    }
}

fn validate_policy_binding_fields(
    manifest: &Value,
    context: &QueryDiscoveryValidationContext,
    diagnostics: &mut Vec<QueryDiscoveryDiagnostic>,
    status: &mut QueryDiscoveryValidationStatus,
) {
    let policy = manifest.get("policy").unwrap_or(&Value::Null);
    for (field, expected, pointer) in [
        (
            "policy_fingerprint",
            context.expected_policy_fingerprint.as_deref(),
            "/policy/policy_fingerprint",
        ),
        (
            "principal_class",
            context.expected_principal_class.as_deref(),
            "/policy/principal_class",
        ),
        (
            "audience",
            context.expected_audience.as_deref(),
            "/policy/audience",
        ),
    ] {
        if let Some(expected) = expected {
            if policy.get(field).and_then(Value::as_str) != Some(expected) {
                push_validation_diagnostic(
                    diagnostics,
                    "QD_POLICY_SCOPE_INCOMPATIBLE",
                    QueryDiscoveryDiagnosticSeverity::Error,
                    "query-discovery policy binding is incompatible",
                    pointer,
                );
                *status = QueryDiscoveryValidationStatus::Invalid;
            }
        }
    }
}

fn compare_binding_field(
    binding: &Value,
    field: &str,
    expected: Option<&str>,
    pointer: &str,
    diagnostics: &mut Vec<QueryDiscoveryDiagnostic>,
    status: &mut QueryDiscoveryValidationStatus,
) {
    let Some(expected) = expected else {
        return;
    };
    if binding.get(field).and_then(Value::as_str) != Some(expected) {
        push_validation_diagnostic(
            diagnostics,
            "QD_SOURCE_BINDING_STALE",
            QueryDiscoveryDiagnosticSeverity::Error,
            "query-discovery source binding is stale",
            pointer,
        );
        if *status != QueryDiscoveryValidationStatus::Invalid {
            *status = QueryDiscoveryValidationStatus::Stale;
        }
    }
}

fn push_validation_diagnostic(
    diagnostics: &mut Vec<QueryDiscoveryDiagnostic>,
    code: &str,
    severity: QueryDiscoveryDiagnosticSeverity,
    message: &str,
    target: &str,
) {
    diagnostics.push(QueryDiscoveryDiagnostic {
        code: code.to_string(),
        severity,
        message: message.to_string(),
        target_kind: Some("json_pointer".to_string()),
        target: Some(target.to_string()),
        withheld: false,
    });
}

pub(super) fn validate_manifest_value(
    bytes: &[u8],
    value: &Value,
    options: &QueryDiscoveryParseOptions,
) -> Result<(), CoveError> {
    if options.require_jcs {
        validate_jcs_canonical(bytes, value)?;
    }
    validate_safe_json_numbers(value, "")?;
    reject_forbidden_payload_keys(value, "")?;

    let object = value
        .as_object()
        .ok_or_else(|| query_discovery_error("manifest root must be a JSON object"))?;

    for field in REQUIRED_TOP_LEVEL_FIELDS {
        if !object.contains_key(*field) {
            return Err(query_discovery_error(format!(
                "manifest missing required field `{field}`"
            )));
        }
    }

    required_string_equals(object.get("schema"), "schema", QUERY_DISCOVERY_SCHEMA_V1)?;
    required_string_equals(
        object.get("canonicalization"),
        "canonicalization",
        QUERY_DISCOVERY_CANONICALIZATION_JCS,
    )?;
    required_string_equals(
        object.get("authority"),
        "authority",
        QUERY_DISCOVERY_ADVISORY_AUTHORITY,
    )?;

    for field in REQUIRED_OBJECT_FIELDS {
        if !object
            .get(*field)
            .ok_or_else(|| query_discovery_error(format!("missing `{field}`")))?
            .is_object()
        {
            return Err(query_discovery_error(format!(
                "`{field}` must be a JSON object"
            )));
        }
    }

    if object.contains_key("validation_status") || object.contains_key("validation_flags") {
        return Err(query_discovery_error(
            "validation_status and validation_flags belong in an external validation report, not in the manifest object",
        ));
    }

    validate_manifest_features(object.get("manifest_features"), options)?;
    validate_template_records(object.get("templates"))?;
    validate_example_records(object.get("examples"))?;
    validate_diagnostic_records(object.get("diagnostics"))?;
    validate_manifest_semantic_consistency(value)?;
    Ok(())
}

fn required_string_equals(
    value: Option<&Value>,
    field: &str,
    expected: &str,
) -> Result<(), CoveError> {
    match value.and_then(Value::as_str) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(query_discovery_error(format!(
            "`{field}` must be `{expected}`, got `{actual}`"
        ))),
        None => Err(query_discovery_error(format!(
            "`{field}` must be a string equal to `{expected}`"
        ))),
    }
}

fn validate_manifest_features(
    value: Option<&Value>,
    options: &QueryDiscoveryParseOptions,
) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let features = value
        .as_array()
        .ok_or_else(|| query_discovery_error("manifest_features must be an array"))?;
    for feature in features {
        let feature = feature
            .as_str()
            .ok_or_else(|| query_discovery_error("manifest_features entries must be strings"))?;
        if !options.known_manifest_features.contains(feature)
            && !options.allow_unknown_manifest_features_for_human_inspection
        {
            return Err(query_discovery_error(format!(
                "unsupported manifest_feature `{feature}` for automated query generation"
            )));
        }
    }
    Ok(())
}

pub(super) fn manifest_has_unknown_features_tolerated_for_human_inspection(
    value: &Value,
    options: &QueryDiscoveryParseOptions,
) -> Result<bool, CoveError> {
    if !options.allow_unknown_manifest_features_for_human_inspection {
        return Ok(false);
    }
    let Some(features) = value.get("manifest_features") else {
        return Ok(false);
    };
    let features = features
        .as_array()
        .ok_or_else(|| query_discovery_error("manifest_features must be an array"))?;
    Ok(features.iter().any(|feature| {
        feature
            .as_str()
            .is_some_and(|feature| !options.known_manifest_features.contains(feature))
    }))
}

fn validate_manifest_semantic_consistency(value: &Value) -> Result<(), CoveError> {
    let coveql = value.get("coveql").unwrap_or(&Value::Null);
    let profiles = contract_string_set(coveql.get("profiles"), "/coveql/profiles")?;
    let roots = contract_string_set(coveql.get("roots"), "/coveql/roots")?;
    let capabilities = contract_string_set(coveql.get("capabilities"), "/coveql/capabilities")?;

    validate_surface_roots_against_contract(value.get("surfaces"), &roots)?;
    validate_template_contract_refs(value.get("templates"), &profiles, &roots, &capabilities)?;
    validate_example_contract_refs(value.get("examples"), &profiles, &roots)?;
    validate_ai_contract_refs(value.get("ai"), &profiles, &capabilities)?;
    Ok(())
}

fn contract_string_set(value: Option<&Value>, path: &str) -> Result<BTreeSet<String>, CoveError> {
    let Some(value) = value else {
        return Ok(BTreeSet::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| query_discovery_error(format!("{path} must be an array")))?;
    values
        .iter()
        .map(|value| require_nonempty_string(Some(value), &format!("{path}/*")))
        .map(|result| result.map(str::to_string))
        .collect()
}

fn validate_surface_roots_against_contract(
    value: Option<&Value>,
    roots: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(surfaces) = value.and_then(Value::as_object) else {
        return Ok(());
    };
    for (surface_kind, expected_root_kind) in [
        ("tables", "table"),
        ("objects", "object"),
        ("projections", "projection"),
        ("evidence", "evidence"),
    ] {
        let Some(records) = surfaces.get(surface_kind).and_then(Value::as_array) else {
            continue;
        };
        for (index, record) in records.iter().enumerate() {
            let root = record
                .get("root")
                .and_then(Value::as_str)
                .filter(|root| !root.is_empty())
                .ok_or_else(|| {
                    query_discovery_error(format!(
                        "/surfaces/{surface_kind}/{index}/root must be a non-empty CoveQL root string"
                    ))
                })?;
            let root_kind = root_kind_from_root(root).ok_or_else(|| {
                query_discovery_error(format!(
                    "/surfaces/{surface_kind}/{index}/root is not a recognized CoveQL root string"
                ))
            })?;
            if root_kind != expected_root_kind {
                return Err(query_discovery_error(format!(
                    "/surfaces/{surface_kind}/{index}/root has kind `{root_kind}`, expected `{expected_root_kind}`"
                )));
            }
            if !roots.contains(root_kind) {
                return Err(query_discovery_error(format!(
                    "/surfaces/{surface_kind}/{index}/root uses root `{root_kind}` not advertised in /coveql/roots"
                )));
            }
        }
    }
    Ok(())
}

fn validate_template_contract_refs(
    value: Option<&Value>,
    profiles: &BTreeSet<String>,
    roots: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(templates) = value.and_then(Value::as_array) else {
        return Ok(());
    };
    for (template_index, template) in templates.iter().enumerate() {
        let path = format!("/templates/{template_index}");
        validate_declared_profiles(
            template.get("profiles"),
            &format!("{path}/profiles"),
            profiles,
        )?;
        if let Some(root_kind) = template
            .get("operator_chain")
            .and_then(Value::as_array)
            .and_then(|chain| chain.first())
            .and_then(|root| root.get("kind"))
            .and_then(Value::as_str)
        {
            if !roots.contains(root_kind) {
                return Err(query_discovery_error(format!(
                    "{path}/operator_chain/0/kind references unavailable root `{root_kind}`"
                )));
            }
        }
        if let Some(chain) = template.get("operator_chain").and_then(Value::as_array) {
            for (operator_index, operator) in chain.iter().enumerate() {
                let method = operator
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if method == "root" || is_core_operator_method(method) {
                    continue;
                }
                validate_declared_profiles(
                    operator.get("profiles"),
                    &format!("{path}/operator_chain/{operator_index}/profiles"),
                    profiles,
                )?;
                validate_declared_capabilities(
                    operator.get("capabilities"),
                    &format!("{path}/operator_chain/{operator_index}/capabilities"),
                    capabilities,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_example_contract_refs(
    value: Option<&Value>,
    profiles: &BTreeSet<String>,
    roots: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(examples) = value.and_then(Value::as_array) else {
        return Ok(());
    };
    for (example_index, example) in examples.iter().enumerate() {
        let path = format!("/examples/{example_index}");
        validate_declared_profiles(
            example.get("profiles"),
            &format!("{path}/profiles"),
            profiles,
        )?;
        if let Some(root_kind) = example
            .get("query")
            .and_then(Value::as_str)
            .and_then(query_root_kind_hint)
        {
            if !roots.contains(root_kind) {
                return Err(query_discovery_error(format!(
                    "{path}/query references unavailable root `{root_kind}`"
                )));
            }
        }
    }
    Ok(())
}

fn validate_ai_contract_refs(
    value: Option<&Value>,
    profiles: &BTreeSet<String>,
    capabilities: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(methods) = value
        .and_then(|ai| ai.get("methods"))
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for (method_index, method) in methods.iter().enumerate() {
        let path = format!("/ai/methods/{method_index}");
        validate_declared_profiles(
            method.get("profiles"),
            &format!("{path}/profiles"),
            profiles,
        )?;
        validate_declared_capabilities(
            method.get("capabilities"),
            &format!("{path}/capabilities"),
            capabilities,
        )?;
    }
    Ok(())
}

fn validate_declared_profiles(
    value: Option<&Value>,
    path: &str,
    profiles: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    for profile in string_array_at(Some(value), path)? {
        if !profiles.contains(profile) {
            return Err(query_discovery_error(format!(
                "{path} references unavailable CoveQL profile `{profile}`"
            )));
        }
    }
    Ok(())
}

fn validate_declared_capabilities(
    value: Option<&Value>,
    path: &str,
    capabilities: &BTreeSet<String>,
) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    for capability in string_array_at(Some(value), path)? {
        if !capabilities.contains(capability) {
            return Err(query_discovery_error(format!(
                "{path} references unavailable CoveQL capability `{capability}`"
            )));
        }
    }
    Ok(())
}

fn validate_template_records(value: Option<&Value>) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let templates = value
        .as_array()
        .ok_or_else(|| query_discovery_error("templates must be an array"))?;
    for (template_index, template) in templates.iter().enumerate() {
        let path = format!("/templates/{template_index}");
        let object = template
            .as_object()
            .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
        require_nonempty_string(object.get("id"), &format!("{path}/id"))?;
        required_string_equals(
            object.get("binding_mode"),
            &format!("{path}/binding_mode"),
            "typed_ast_fragments",
        )?;
        validate_template_validation_status(object.get("template_validation"), &path)?;
        validate_operator_chain(object.get("operator_chain"), &path)?;
        validate_template_parameters(object.get("parameters"), &path)?;
    }
    Ok(())
}

fn validate_operator_chain(value: Option<&Value>, template_path: &str) -> Result<(), CoveError> {
    let chain = value.and_then(Value::as_array).ok_or_else(|| {
        query_discovery_error(format!("{template_path}/operator_chain must be an array"))
    })?;
    if chain.is_empty() {
        return Err(query_discovery_error(format!(
            "{template_path}/operator_chain must not be empty"
        )));
    }
    for (index, entry) in chain.iter().enumerate() {
        let path = format!("{template_path}/operator_chain/{index}");
        let object = entry
            .as_object()
            .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
        let method = require_nonempty_string(object.get("method"), &format!("{path}/method"))?;
        if index == 0 && method != "root" {
            return Err(query_discovery_error(format!(
                "{template_path}/operator_chain must start with a root method"
            )));
        }
        if method == "root" {
            require_nonempty_string(object.get("kind"), &format!("{path}/kind"))?;
        } else if !is_core_operator_method(method) {
            let profiles = string_array_at(object.get("profiles"), &format!("{path}/profiles"))?;
            let capabilities =
                string_array_at(object.get("capabilities"), &format!("{path}/capabilities"))?;
            if profiles.is_empty() || capabilities.is_empty() {
                return Err(query_discovery_error(format!(
                    "{path} profile method `{method}` must declare required profiles and capabilities"
                )));
            }
        }
    }
    Ok(())
}

fn validate_template_parameters(
    value: Option<&Value>,
    template_path: &str,
) -> Result<(), CoveError> {
    let parameters = value.and_then(Value::as_array).ok_or_else(|| {
        query_discovery_error(format!("{template_path}/parameters must be an array"))
    })?;
    let mut names = BTreeSet::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let path = format!("{template_path}/parameters/{index}");
        let object = parameter
            .as_object()
            .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
        let name = require_nonempty_string(object.get("name"), &format!("{path}/name"))?;
        if !names.insert(name.to_string()) {
            return Err(query_discovery_error(format!(
                "{path}/name duplicates template parameter `{name}`"
            )));
        }
        let kind = require_nonempty_string(object.get("kind"), &format!("{path}/kind"))?;
        if is_raw_query_parameter_kind(kind) {
            return Err(query_discovery_error(format!(
                "{path}/kind `{kind}` is not allowed because COVE-QD templates must not accept raw CoveQL insertion points"
            )));
        }
        match kind {
            "root_name" => {
                let allowed = string_array_at(
                    object.get("allowed_query_identifiers"),
                    &format!("{path}/allowed_query_identifiers"),
                )?;
                if allowed.is_empty() {
                    return Err(query_discovery_error(format!(
                        "{path}/allowed_query_identifiers must not be empty"
                    )));
                }
            }
            "predicate" => {
                validate_root_scoped_identifier_map(
                    object.get("allowed_query_identifiers_by_root"),
                    &format!("{path}/allowed_query_identifiers_by_root"),
                )?;
                let operators = string_array_at(
                    object.get("allowed_operators"),
                    &format!("{path}/allowed_operators"),
                )?;
                if operators.is_empty() {
                    return Err(query_discovery_error(format!(
                        "{path}/allowed_operators must not be empty"
                    )));
                }
                for operator in operators {
                    if !is_semantic_operator_name(operator) {
                        return Err(query_discovery_error(format!(
                            "{path}/allowed_operators contains unsupported semantic operator `{operator}`"
                        )));
                    }
                }
                if string_array_at(
                    object.get("allowed_literal_types"),
                    &format!("{path}/allowed_literal_types"),
                )?
                .is_empty()
                {
                    return Err(query_discovery_error(format!(
                        "{path}/allowed_literal_types must not be empty"
                    )));
                }
            }
            "select_list" => {
                validate_root_scoped_identifier_map(
                    object.get("allowed_query_identifiers_by_root"),
                    &format!("{path}/allowed_query_identifiers_by_root"),
                )?;
            }
            "uint" => {
                if let (Some(default), Some(max)) = (
                    object.get("default").and_then(Value::as_u64),
                    object.get("max").and_then(Value::as_u64),
                ) {
                    if default > max {
                        return Err(query_discovery_error(format!(
                            "{path}/default must not exceed {path}/max"
                        )));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_root_scoped_identifier_map(value: Option<&Value>, path: &str) -> Result<(), CoveError> {
    let map = value
        .and_then(Value::as_object)
        .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
    if map.is_empty() {
        return Err(query_discovery_error(format!("{path} must not be empty")));
    }
    for (root, identifiers) in map {
        if !looks_like_coveql_root(root) {
            return Err(query_discovery_error(format!(
                "{path} key `{root}` is not a canonical CoveQL root string"
            )));
        }
        let identifiers = identifiers
            .as_array()
            .ok_or_else(|| query_discovery_error(format!("{path}/{root} must be an array")))?;
        if identifiers.is_empty() {
            return Err(query_discovery_error(format!(
                "{path}/{root} must not be empty"
            )));
        }
        for identifier in identifiers {
            require_nonempty_string(Some(identifier), &format!("{path}/{root}/*"))?;
        }
    }
    Ok(())
}

fn validate_example_records(value: Option<&Value>) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let examples = value
        .as_array()
        .ok_or_else(|| query_discovery_error("examples must be an array"))?;
    for (index, example) in examples.iter().enumerate() {
        let path = format!("/examples/{index}");
        let object = example
            .as_object()
            .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
        require_nonempty_string(object.get("query"), &format!("{path}/query"))?;
        validate_example_validation_status(object.get("query_validation"), &path)?;
    }
    Ok(())
}

fn validate_diagnostic_records(value: Option<&Value>) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let diagnostics = value
        .as_array()
        .ok_or_else(|| query_discovery_error("diagnostics must be an array"))?;
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let path = format!("/diagnostics/{index}");
        let object = diagnostic
            .as_object()
            .ok_or_else(|| query_discovery_error(format!("{path} must be an object")))?;
        require_nonempty_string(object.get("code"), &format!("{path}/code"))?;
        let severity =
            require_nonempty_string(object.get("severity"), &format!("{path}/severity"))?;
        if !matches!(severity, "info" | "warning" | "error") {
            return Err(query_discovery_error(format!(
                "{path}/severity must be info, warning, or error"
            )));
        }
        require_nonempty_string(object.get("message"), &format!("{path}/message"))?;
        if let Some(target_kind) = object.get("target_kind") {
            let target_kind =
                require_nonempty_string(Some(target_kind), &format!("{path}/target_kind"))?;
            if target_kind != "json_pointer" {
                return Err(query_discovery_error(format!(
                    "{path}/target_kind must be json_pointer"
                )));
            }
        }
        if let Some(target) = object.get("target") {
            let target = require_nonempty_string(Some(target), &format!("{path}/target"))?;
            if !target.starts_with('/') {
                return Err(query_discovery_error(format!(
                    "{path}/target must be a JSON Pointer"
                )));
            }
        }
    }
    Ok(())
}

fn require_nonempty_string<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str, CoveError> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| query_discovery_error(format!("{path} must be a string")))?;
    if text.is_empty() {
        return Err(query_discovery_error(format!("{path} must not be empty")));
    }
    Ok(text)
}

fn string_array_at<'a>(value: Option<&'a Value>, path: &str) -> Result<Vec<&'a str>, CoveError> {
    let array = value
        .and_then(Value::as_array)
        .ok_or_else(|| query_discovery_error(format!("{path} must be an array")))?;
    array
        .iter()
        .map(|item| require_nonempty_string(Some(item), &format!("{path}/*")))
        .collect()
}

fn validate_template_validation_status(value: Option<&Value>, path: &str) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let status = require_nonempty_string(Some(value), &format!("{path}/template_validation"))?;
    if matches!(
        status,
        "operator_chain_validated"
            | "parameters_validated"
            | "representative_planned_dry_run"
            | "planned_dry_run"
            | "not_validated_policy_limited"
            | "not_validated_budget_limited"
            | "not_validated"
    ) {
        Ok(())
    } else {
        Err(query_discovery_error(format!(
            "{path}/template_validation has unsupported value `{status}`"
        )))
    }
}

fn validate_example_validation_status(value: Option<&Value>, path: &str) -> Result<(), CoveError> {
    let Some(value) = value else {
        return Ok(());
    };
    let status = require_nonempty_string(Some(value), &format!("{path}/query_validation"))?;
    if matches!(
        status,
        "parsed_and_resolved"
            | "parsed_only"
            | "planned_dry_run"
            | "not_validated_policy_limited"
            | "not_validated_budget_limited"
            | "not_validated"
    ) {
        Ok(())
    } else {
        Err(query_discovery_error(format!(
            "{path}/query_validation has unsupported value `{status}`"
        )))
    }
}

fn is_core_operator_method(method: &str) -> bool {
    matches!(
        method,
        "root" | "where" | "select" | "orderBy" | "groupBy" | "take" | "skip" | "explain"
    )
}

fn is_raw_query_parameter_kind(kind: &str) -> bool {
    matches!(
        kind,
        "raw" | "raw_query" | "coveql" | "coveql_text" | "query_text" | "method_chain"
    )
}

fn is_semantic_operator_name(operator: &str) -> bool {
    matches!(
        operator,
        "eq" | "neq" | "lt" | "lte" | "gt" | "gte" | "and" | "or" | "not" | "in"
    )
}

fn looks_like_coveql_root(root: &str) -> bool {
    [
        "table(",
        "object(",
        "association(",
        "projection(",
        "evidence(",
        "node(",
        "edge(",
    ]
    .iter()
    .any(|prefix| root.starts_with(prefix) && root.ends_with(')'))
}

fn root_kind_from_root(root: &str) -> Option<&'static str> {
    [
        ("table(", "table"),
        ("object(", "object"),
        ("association(", "association"),
        ("projection(", "projection"),
        ("evidence(", "evidence"),
        ("node(", "node"),
        ("edge(", "edge"),
    ]
    .iter()
    .find_map(|(prefix, kind)| root.trim().starts_with(prefix).then_some(*kind))
}

fn query_root_kind_hint(query: &str) -> Option<&'static str> {
    query
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .and_then(root_kind_from_root)
}

/// Render a query-discovery manifest value as canonical JSON bytes.
pub fn canonical_query_discovery_json(value: &Value) -> Result<Vec<u8>, CoveError> {
    canonical_json_bytes(value)
}

pub(super) fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CoveError> {
    let mut out = Vec::new();
    write_canonical_json_value(value, &mut out)?;
    Ok(out)
}

fn validate_jcs_canonical(bytes: &[u8], value: &Value) -> Result<(), CoveError> {
    let canonical = canonical_json_bytes(value)?;
    if canonical != bytes {
        return Err(query_discovery_error(
            "manifest JSON is not canonical RFC 8785/JCS-compatible JSON",
        ));
    }
    Ok(())
}

fn write_canonical_json_value(value: &Value, out: &mut Vec<u8>) -> Result<(), CoveError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(string) => {
            serde_json::to_writer(out, string).map_err(|err| {
                query_discovery_error(format!("failed to render canonical JSON string: {err}"))
            })?;
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_json_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key).map_err(|err| {
                    query_discovery_error(format!(
                        "failed to render canonical JSON object key: {err}"
                    ))
                })?;
                out.push(b':');
                let Some(value) = object.get(key) else {
                    return Err(query_discovery_error(
                        "failed to render canonical JSON object: missing sorted key",
                    ));
                };
                write_canonical_json_value(value, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn validate_safe_json_numbers(value: &Value, path: &str) -> Result<(), CoveError> {
    match value {
        Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                if unsigned > JSON_SAFE_INTEGER_MAX {
                    return Err(query_discovery_error(format!(
                        "JSON number at {path_or_root} exceeds interoperable safe integer range",
                        path_or_root = path_or_root(path)
                    )));
                }
            } else if let Some(signed) = number.as_i64() {
                if signed < JSON_SAFE_INTEGER_MIN || signed > JSON_SAFE_INTEGER_MAX as i64 {
                    return Err(query_discovery_error(format!(
                        "JSON number at {path_or_root} exceeds interoperable safe integer range",
                        path_or_root = path_or_root(path)
                    )));
                }
            } else if let Some(float) = number.as_f64() {
                if !float.is_finite() || float.abs() > JSON_SAFE_INTEGER_MAX as f64 {
                    return Err(query_discovery_error(format!(
                        "JSON number at {path_or_root} exceeds interoperable safe numeric range",
                        path_or_root = path_or_root(path)
                    )));
                }
            }
        }
        Value::Array(values) => {
            for (index, item) in values.iter().enumerate() {
                validate_safe_json_numbers(item, &format!("{path}/{index}"))?;
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                validate_safe_json_numbers(item, &format!("{path}/{}", escape_json_pointer(key)))?;
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
    Ok(())
}

fn reject_forbidden_payload_keys(value: &Value, path: &str) -> Result<(), CoveError> {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                if FORBIDDEN_PAYLOAD_KEYS.contains(&key.as_str()) {
                    return Err(query_discovery_error(format!(
                        "manifest key `{key}` at {path_or_root} is forbidden because COVE-QD must not carry protected payloads",
                        path_or_root = path_or_root(path)
                    )));
                }
                reject_forbidden_payload_keys(
                    item,
                    &format!("{path}/{}", escape_json_pointer(key)),
                )?;
            }
        }
        Value::Array(values) => {
            for (index, item) in values.iter().enumerate() {
                reject_forbidden_payload_keys(item, &format!("{path}/{index}"))?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

pub(super) fn reject_duplicate_json_object_keys(bytes: &[u8]) -> Result<(), CoveError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    DuplicateKeySeed
        .deserialize(&mut deserializer)
        .map_err(|err| {
            query_discovery_error(format!("duplicate-key JSON validation failed: {err}"))
        })?;
    deserializer
        .end()
        .map_err(|err| query_discovery_error(format!("manifest has trailing JSON data: {err}")))?;
    Ok(())
}

#[derive(Clone, Copy)]
struct DuplicateKeySeed;

impl<'de> DeserializeSeed<'de> for DuplicateKeySeed {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for DuplicateKeySeed {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_borrowed_str<E>(self, _value: &'de str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element_seed(DuplicateKeySeed)?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate object key `{key}`")));
            }
            map.next_value_seed(DuplicateKeySeed)?;
        }
        Ok(())
    }
}

fn escape_json_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn path_or_root(path: &str) -> &str {
    if path.is_empty() {
        "/"
    } else {
        path
    }
}
