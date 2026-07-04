use super::*;
use crate::{
    constants::{
        CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind,
        FEATURE_QUERY_DISCOVERY_METADATA, FEATURE_TABLE_PROFILE,
    },
    header::CoveHeaderV1,
    profile::{
        cove_map::{
            EmbeddedMapSection, MapAssociationBinding, MapIdentityRule, MapIdentityRuleCatalog,
            MapJoinKeyComponent, MapProjectionColumn, MapProjectionEntry, MapRowSemanticRule,
            MapRowSemanticsCatalog, SourceOperationKind,
        },
        cove_o::{
            ObjectTypeEntryV1, PropertyEntryV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            PROPERTY_FLAG_ASSOCIATION_TYPE,
        },
    },
    reader::validate_bytes,
    table::{ColumnEntry, TableCatalog, TableEntry},
    utility::{build_covm_artifact_from_bytes, CovmInputArtifact},
    writer::{ScanProfileCoveWriter, SectionPayload},
    CoveError,
};
use serde_json::{json, Value};

use super::{
    ident::projection_root,
    relationships::{map_relationship_records_json, object_catalog_relationship_json},
    surfaces::{evidence_surface_json, projection_surface_json},
    templates::{evidence_examples, projection_select_take_template},
};

const MINIMAL: &[u8] = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"policy":{},"schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{}}"#;

#[test]
fn accepts_minimal_valid_manifest() {
    let manifest = QueryDiscoveryManifest::parse(MINIMAL).unwrap();
    assert_eq!(manifest.schema(), QUERY_DISCOVERY_SCHEMA_V1);
    assert_eq!(manifest.authority(), QUERY_DISCOVERY_ADVISORY_AUTHORITY);
    assert_eq!(manifest.raw_canonical_json(), MINIMAL);
}

#[test]
fn rejects_duplicate_object_keys_before_value_parsing() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"policy":{},"schema":"cove.query_discovery.v1","schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{}}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("duplicate object key")
    ));
}

#[test]
fn rejects_noncanonical_json_in_strict_mode() {
    let bytes = br#"{"schema":"cove.query_discovery.v1","canonicalization":"rfc8785-jcs","authority":"advisory_discovery_not_archive_truth","source_binding":{},"coveql":{},"surfaces":{},"policy":{}}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("not canonical")
    ));
}

#[test]
fn rejects_unsafe_json_numbers() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"policy":{},"schema":"cove.query_discovery.v1","source_binding":{"as_of_csn":9007199254740992},"surfaces":{}}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("safe integer")
    ));
}

#[test]
fn rejects_self_asserted_validation_status() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"policy":{},"schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{},"validation_status":"valid"}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("external validation report")
    ));
}

#[test]
fn rejects_forbidden_payload_keys() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"policy":{},"schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{"embeddings":[]}}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("forbidden")
    ));
}

#[test]
fn rejects_unknown_manifest_features_for_automation() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"manifest_features":["future_block"],"policy":{},"schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{}}"#;
    assert!(matches!(
        QueryDiscoveryManifest::parse(bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("unsupported manifest_feature")
    ));
}

#[test]
fn allows_unknown_manifest_features_for_human_inspection() {
    let bytes = br#"{"authority":"advisory_discovery_not_archive_truth","canonicalization":"rfc8785-jcs","coveql":{},"manifest_features":["future_block"],"policy":{},"schema":"cove.query_discovery.v1","source_binding":{},"surfaces":{}}"#;
    let options = QueryDiscoveryParseOptions {
        allow_unknown_manifest_features_for_human_inspection: true,
        ..QueryDiscoveryParseOptions::default()
    };
    let manifest = QueryDiscoveryManifest::parse_with_options(bytes, &options).unwrap();
    assert!(!manifest.automation_allowed());
}

#[test]
fn best_effort_human_inspection_does_not_enable_template_expansion() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {
            "profiles": ["table"],
            "roots": ["table"],
            "capabilities": ["select", "limit"]
        },
        "surfaces": {},
        "policy": {},
        "manifest_features": ["future_block"],
        "templates": [{
            "id": "future_safe",
            "profiles": ["table"],
            "binding_mode": "typed_ast_fragments",
            "operator_chain": [
                {"method": "root", "kind": "table", "arg": "{table}"},
                {"method": "take", "arg": "{limit}"}
            ],
            "template_validation": "operator_chain_validated",
            "parameters": [
                {
                    "name": "table",
                    "kind": "root_name",
                    "allowed_query_identifiers": ["people"]
                },
                {
                    "name": "limit",
                    "kind": "uint",
                    "default": 50,
                    "max": 500
                }
            ]
        }]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();
    let options = QueryDiscoveryParseOptions {
        allow_unknown_manifest_features_for_human_inspection: true,
        ..QueryDiscoveryParseOptions::default()
    };
    let manifest = QueryDiscoveryManifest::parse_with_options(&bytes, &options).unwrap();

    assert!(matches!(
        render_query_discovery_template(&manifest, "future_safe", &[]),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("human-only best-effort")
    ));
}

#[test]
fn rejects_templates_with_raw_query_insertion_points() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {},
        "surfaces": {},
        "policy": {},
        "templates": [{
            "id": "unsafe",
            "binding_mode": "typed_ast_fragments",
            "operator_chain": [{"method": "root", "kind": "table", "arg": "{table}"}],
            "parameters": [{"name": "query", "kind": "raw_query"}]
        }]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("raw CoveQL insertion points")
    ));
}

#[test]
fn rejects_templates_with_unavailable_profiles_or_roots() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {
            "profiles": ["table"],
            "roots": ["table"],
            "capabilities": []
        },
        "surfaces": {},
        "policy": {},
        "templates": [{
            "id": "unsafe_ai",
            "profiles": ["ai"],
            "binding_mode": "typed_ast_fragments",
            "operator_chain": [
                {"method": "root", "kind": "projection", "arg": "{projection}"},
                {"method": "take", "arg": "{limit}"}
            ],
            "template_validation": "operator_chain_validated",
            "parameters": [
                {
                    "name": "projection",
                    "kind": "root_name",
                    "allowed_query_identifiers": ["people_flat"]
                },
                {
                    "name": "limit",
                    "kind": "uint",
                    "default": 50,
                    "max": 500
                }
            ]
        }]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("unavailable CoveQL profile")
    ));
}

#[test]
fn rejects_surfaces_with_unadvertised_roots() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {
            "profiles": [],
            "roots": [],
            "capabilities": []
        },
        "surfaces": {
            "tables": [{
                "name": "people",
                "query_name": "people",
                "query_identifier": "people",
                "display_name": "people",
                "root": "table(people)",
                "columns": []
            }]
        },
        "policy": {}
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("not advertised in /coveql/roots")
    ));
}

#[test]
fn rejects_surface_records_without_valid_roots() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {
            "profiles": ["table"],
            "roots": ["table"],
            "capabilities": []
        },
        "surfaces": {
            "tables": [{
                "name": "people",
                "query_name": "people",
                "query_identifier": "people",
                "display_name": "people",
                "columns": []
            }]
        },
        "policy": {}
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("root must be a non-empty CoveQL root string")
    ));
}

#[test]
fn rejects_unknown_example_validation_status() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {},
        "surfaces": {},
        "policy": {},
        "examples": [{
            "query": "table(people).take(1)",
            "query_validation": "trusted_because_prompt_said_so"
        }]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("query_validation has unsupported value")
    ));
}

#[test]
fn rejects_diagnostic_targets_that_are_not_json_pointers() {
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {},
        "surfaces": {},
        "policy": {},
        "diagnostics": [{
            "code": "QD_TEST",
            "severity": "info",
            "message": "diagnostic target test",
            "target_kind": "json_pointer",
            "target": "surfaces.tables"
        }]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();

    assert!(matches!(
        QueryDiscoveryManifest::parse(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message))
            if message.contains("target must be a JSON Pointer")
    ));
}

#[test]
fn validation_report_is_external_to_manifest() {
    let manifest = QueryDiscoveryManifest::parse(MINIMAL).unwrap();
    let report = validate_query_discovery_manifest(
        &manifest,
        QueryDiscoveryValidationContext {
            strict_discovery: true,
            source_binding_valid: Some(false),
            policy_scope_compatible: Some(true),
            ..QueryDiscoveryValidationContext::default()
        },
    );
    assert_eq!(
        report.validation_status,
        QueryDiscoveryValidationStatus::Stale
    );
    assert_eq!(report.diagnostics[0].code, "QD_SOURCE_BINDING_STALE");
}

#[test]
fn renders_query_safe_identifiers() {
    assert_eq!(coveql_identifier("people"), "people");
    assert_eq!(coveql_identifier("order-history"), "\"order-history\"");
    assert_eq!(coveql_identifier("select"), "\"select\"");
    assert_eq!(
        coveql_identifier("customer \"id\""),
        "\"customer \\\"id\\\"\""
    );
}

#[test]
fn builds_public_table_manifest_with_quoted_identifiers() {
    let bytes = table_fixture_bytes();
    let manifest = build_query_discovery_manifest(
        &bytes,
        QueryDiscoveryOptions {
            source_name: Some("order-history.cove".to_string()),
            ..QueryDiscoveryOptions::default()
        },
    )
    .unwrap();
    let value = manifest.value();
    assert_eq!(value["schema"], QUERY_DISCOVERY_SCHEMA_V1);
    assert_eq!(
        value["source_binding"]["source_uri_hint"],
        "order-history.cove"
    );
    assert_eq!(
        value["surfaces"]["tables"][0]["query_identifier"],
        "\"order-history\""
    );
    assert_eq!(
        value["surfaces"]["tables"][0]["columns"][0]["query_identifier"],
        "\"select\""
    );
    assert_eq!(
        value["templates"][0]["parameters"][1]["allowed_operators"][0],
        "eq"
    );
    assert_eq!(
        value["templates"][0]["parameters"][2]["allowed_query_identifiers_by_root"]
            ["table(\"order-history\")"][0],
        "\"select\""
    );
    assert!(value["source_binding"]["file_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(
        value["source_binding"]["file_length"],
        bytes.len().to_string()
    );
    assert!(value["source_binding"]["footer_crc32c"].as_str().is_some());
    assert!(value["source_binding"]["schema_fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn validation_context_detects_stale_source_binding_fields() {
    let bytes = table_fixture_bytes();
    let options = QueryDiscoveryOptions::default();
    let manifest = build_query_discovery_manifest(&bytes, options.clone()).unwrap();
    let mut context = query_discovery_validation_context_for_source(&bytes, &options).unwrap();
    context.expected_schema_fingerprint = Some("sha256:stale-schema".to_string());

    let report = validate_query_discovery_manifest(&manifest, context);

    assert_eq!(
        report.validation_status,
        QueryDiscoveryValidationStatus::Stale
    );
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "QD_SOURCE_BINDING_STALE"));
}

#[test]
fn typed_manifest_accessors_expose_stable_records() {
    let manifest =
        build_query_discovery_manifest(&table_fixture_bytes(), QueryDiscoveryOptions::default())
            .unwrap();

    let source = manifest.source_binding_record();
    assert_eq!(source.source_kind, "cove_file");
    assert!(source.file_digest.unwrap().starts_with("sha256:"));

    let contract = manifest.coveql_contract();
    assert_eq!(contract.profiles, vec!["table"]);
    assert!(contract.roots.contains(&"table".to_string()));

    let surfaces = manifest.surface_records();
    assert_eq!(surfaces[0].surface_kind, "table");
    assert_eq!(
        surfaces[0].query_identifier.as_deref(),
        Some("\"order-history\"")
    );
    assert_eq!(
        surfaces[0].fields[0].query_identifier.as_deref(),
        Some("\"select\"")
    );

    let templates = manifest.template_records();
    assert_eq!(templates[0].id, "table_filter_select_take");
    assert_eq!(
        templates[0].template_validation,
        QueryTemplateValidationStatus::OperatorChainValidated
    );
    assert_eq!(templates[0].operator_chain[0].method, "root");

    let examples = manifest.example_records();
    assert_eq!(
        examples[0].query_validation,
        QueryExampleValidationStatus::NotValidated
    );
    assert!(manifest
        .value()
        .get("diagnostics")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.get("code").and_then(Value::as_str)
            == Some("QD_EXAMPLE_VALIDATION_NOT_PERFORMED")));
    assert_eq!(
        manifest.value()["property_glossary"][0]["query_identifier_path"],
        r#"table("order-history")."select""#
    );
    assert_eq!(
        manifest.value()["alias_bindings"][0]["alias"],
        "order-history"
    );
    assert!(manifest.value()["root_index"]
        .as_array()
        .unwrap()
        .contains(&json!(r#"table("order-history")"#)));
    assert_eq!(
        manifest.policy_record().metadata_disclosure.as_deref(),
        Some("public")
    );
    assert_eq!(manifest.budget_record().default_take, Some(50));
}

#[test]
fn builds_covm_dataset_binding_and_detects_stale_members() {
    let member = table_fixture_bytes();
    let (covm_bytes, _) = build_covm_artifact_from_bytes(
        "dataset.covm",
        &[CovmInputArtifact {
            uri: "order-history.cove".to_string(),
            bytes: &member,
        }],
    )
    .unwrap();
    let options = QueryDiscoveryOptions {
        source_name: Some("dataset.covm".to_string()),
        ..QueryDiscoveryOptions::default()
    };
    let manifest = build_query_discovery_manifest(&covm_bytes, options.clone()).unwrap();
    let value = manifest.value();

    assert_eq!(
        value["source_binding"]["source_kind"],
        "covm_dataset_snapshot"
    );
    assert!(value["source_binding"]["covm_snapshot_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(
        value["source_binding"]["members"][0]["member_id"],
        "order-history.cove"
    );
    assert!(value["source_binding"]["members"][0]["file_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));

    let mut context = query_discovery_validation_context_for_source(&covm_bytes, &options).unwrap();
    context.expected_member_file_digests.insert(
        "order-history.cove".to_string(),
        "sha256:stale-member".to_string(),
    );
    let report = validate_query_discovery_manifest(&manifest, context);

    assert_eq!(
        report.validation_status,
        QueryDiscoveryValidationStatus::Stale
    );
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "QD_COVM_MEMBER_STALE"));
}

#[test]
fn covm_ai_contract_does_not_invent_member_table_methods() {
    let member = table_fixture_bytes();
    let (covm_bytes, _) = build_covm_artifact_from_bytes(
        "dataset.covm",
        &[CovmInputArtifact {
            uri: "order-history.cove".to_string(),
            bytes: &member,
        }],
    )
    .unwrap();

    let manifest = build_query_discovery_manifest(
        &covm_bytes,
        QueryDiscoveryOptions {
            include_ai: true,
            ..QueryDiscoveryOptions::default()
        },
    )
    .unwrap();
    let value = manifest.value();

    assert!(value["coveql"]["profiles"]
        .as_array()
        .unwrap()
        .contains(&json!("ai")));
    assert!(value["coveql"]["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("ai_capability_hints")));
    assert!(!value["coveql"]["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("similarity")));
    assert_eq!(value["ai"]["profile_available"], true);
    assert!(value["ai"]["methods"].as_array().unwrap().is_empty());
}

#[test]
fn renders_template_from_operator_chain_not_display_string() {
    let manifest =
        build_query_discovery_manifest(&table_fixture_bytes(), QueryDiscoveryOptions::default())
            .unwrap();
    let mut value = manifest.value().clone();
    value["templates"][0]["template_display"] =
        json!("table(fake).where(hidden == true).select(hidden).take(999)");
    let canonical = canonical_query_discovery_json(&value).unwrap();
    let manifest = QueryDiscoveryManifest::parse(&canonical).unwrap();

    let query = render_query_discovery_template(
        &manifest,
        "table_filter_select_take",
        &[
            ("table".to_string(), "order-history".to_string()),
            ("select".to_string(), "active".to_string()),
            ("columns".to_string(), "select,score".to_string()),
            ("limit".to_string(), "10".to_string()),
        ],
    )
    .unwrap();

    assert_eq!(
        query,
        r#"table("order-history").where("select" == "active").select("select", score).take(10)"#
    );
}

#[test]
fn renderer_rejects_identifiers_absent_from_manifest() {
    let manifest =
        build_query_discovery_manifest(&table_fixture_bytes(), QueryDiscoveryOptions::default())
            .unwrap();

    assert!(matches!(
        render_query_discovery_template(
            &manifest,
            "table_filter_select_take",
            &[
                ("table".to_string(), "order-history".to_string()),
                ("hidden".to_string(), "true".to_string())
            ],
        ),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("not selectable/filterable")
    ));
}

#[test]
fn renderer_enforces_field_literal_types() {
    let manifest =
        build_query_discovery_manifest(&table_fixture_bytes(), QueryDiscoveryOptions::default())
            .unwrap();

    let query = render_query_discovery_template(
        &manifest,
        "table_filter_select_take",
        &[
            ("table".to_string(), "order-history".to_string()),
            ("score".to_string(), "20".to_string()),
            ("columns".to_string(), "score".to_string()),
        ],
    )
    .unwrap();
    assert_eq!(
        query,
        r#"table("order-history").where(score == 20).select(score).take(50)"#
    );

    assert!(matches!(
        render_query_discovery_template(
            &manifest,
            "table_filter_select_take",
            &[
                ("table".to_string(), "order-history".to_string()),
                ("score".to_string(), "active".to_string())
            ],
        ),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("expects an integer literal")
    ));
}

#[test]
fn renders_projection_template_from_root_scoped_columns() {
    let projection = MapProjectionEntry {
        projection_id: "people-flat".to_string(),
        assertion_ids: Vec::new(),
        output_table: Some("people".to_string()),
        row_grain: Some("object".to_string()),
        anchor: None,
        temporal_mode: None,
        columns: vec![MapProjectionColumn {
            name: "select".to_string(),
            value: "status".to_string(),
            logical_type: Some("utf8".to_string()),
            nested_shape: None,
            conflict_policy: "first".to_string(),
            missing_policy: "null".to_string(),
            lineage: None,
        }],
        multi_value_policy: None,
        missing_policy: "null".to_string(),
        ordering: Vec::new(),
        evidence_policy: "metadata".to_string(),
        output_modes: Vec::new(),
    };
    let root = projection_root(&projection.projection_id);
    let value = json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": {},
        "coveql": {
            "profiles": ["object"],
            "roots": ["projection"],
            "capabilities": ["projection_readback"]
        },
        "surfaces": {
            "projections": [projection_surface_json(&projection)]
        },
        "policy": {},
        "templates": [projection_select_take_template(&[projection])]
    });
    let bytes = canonical_query_discovery_json(&value).unwrap();
    let manifest = QueryDiscoveryManifest::parse(&bytes).unwrap();

    let query = render_query_discovery_template(
        &manifest,
        "projection_select_take",
        &[
            ("projection".to_string(), "people-flat".to_string()),
            ("columns".to_string(), "select".to_string()),
            ("limit".to_string(), "25".to_string()),
        ],
    )
    .unwrap();

    assert_eq!(query, format!("{root}.select(\"select\").take(25)"));
}

#[test]
fn generated_contract_versions_follow_enabled_profiles() {
    let manifest =
        build_query_discovery_manifest(&table_fixture_bytes(), QueryDiscoveryOptions::default())
            .unwrap();
    assert_eq!(
        manifest.value()["coveql"]["profile_contract_versions"]["table"],
        "0.1"
    );
    assert!(manifest.value()["coveql"]["profile_contract_versions"]
        .get("object")
        .is_none());

    let ai_manifest = build_query_discovery_manifest(
        &table_fixture_bytes(),
        QueryDiscoveryOptions {
            include_ai: true,
            ..QueryDiscoveryOptions::default()
        },
    )
    .unwrap();
    assert!(ai_manifest.value()["coveql"]["profiles"]
        .as_array()
        .unwrap()
        .contains(&json!("ai")));
    assert_eq!(
        ai_manifest.value()["coveql"]["profile_contract_versions"]["ai"],
        "0.1"
    );
    assert_eq!(ai_manifest.ai_capability().methods.len(), 1);
    assert_eq!(ai_manifest.value()["ai"]["methods"][0]["name"], "similar");
    assert!(ai_manifest.value()["coveql"]["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("similarity")));
    assert!(ai_manifest.value()["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic.get("code").and_then(Value::as_str)
            == Some("QD_AI_SIDECAR_NOT_BOUND")));
}

#[test]
fn builds_relationship_from_object_association_flags() {
    let association = ObjectTypeEntryV1 {
        object_type_id: 42,
        type_name: "Customer-Order".to_string(),
        flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
        properties: vec![
            object_property_fixture(1, "from goid", PROPERTY_FLAG_ASSOCIATION_FROM_GOID),
            object_property_fixture(2, "to_goid", PROPERTY_FLAG_ASSOCIATION_TO_GOID),
            object_property_fixture(3, "select", PROPERTY_FLAG_ASSOCIATION_TYPE),
        ],
    };

    let (_, relationship) = object_catalog_relationship_json(&association).unwrap();

    assert_eq!(
        relationship["association_root"],
        r#"association("Customer-Order")"#
    );
    assert_eq!(
        relationship["endpoint_properties"]["from"],
        r#""from goid""#
    );
    assert_eq!(relationship["endpoint_properties"]["type"], r#""select""#);
    assert_eq!(relationship["authority"], "object_catalog_flags");
}

#[test]
fn evidence_surfaces_expose_system_fields_and_bounded_examples() {
    let object_type = ObjectTypeEntryV1 {
        object_type_id: 1,
        type_name: "Person".to_string(),
        flags: 0,
        properties: Vec::new(),
    };
    let surface = evidence_surface_json(&object_type, 12);

    assert_eq!(surface["root"], "evidence(Person, grain: object)");
    assert_eq!(surface["columns"][0]["query_identifier"], "source_id");
    assert_eq!(
        surface["columns"][1]["query_identifier"],
        "source_row_identity"
    );

    let examples = evidence_examples(&[surface]);
    assert_eq!(
        examples[0]["query"],
        "evidence(Person, grain: object).select(source_id, source_row_identity).take(20)"
    );
    assert_eq!(examples[0]["profiles"][0], "object");
}

#[test]
fn builds_relationship_from_map_row_semantics() {
    let sections = vec![
        EmbeddedMapSection::IdentityRuleCatalog(MapIdentityRuleCatalog {
            mapping_id: "orders".to_string(),
            mapping_version: "v1".to_string(),
            identity_rules: vec![
                identity_rule_fixture("customer_identity", "Customer"),
                identity_rule_fixture("order_identity", "Order"),
            ],
            do_not_merge: Vec::new(),
        }),
        EmbeddedMapSection::RowSemanticsCatalog(MapRowSemanticsCatalog {
            mapping_id: "orders".to_string(),
            mapping_version: "v1".to_string(),
            rules: vec![MapRowSemanticRule {
                rule_id: "orders_rule".to_string(),
                source_id: "orders_csv".to_string(),
                identity_rule_id: "customer_identity".to_string(),
                row_semantics_kind: "Object".to_string(),
                source_operation_kind: SourceOperationKind::Fact,
                assertion_kinds: vec!["Object".to_string()],
                tombstone_target: None,
                record_kind: "Baseline".to_string(),
                temporal_policy: "latest_committed".to_string(),
                conflict_policy: "reject_conflict".to_string(),
                function_ids: Vec::new(),
                output_assertion_ids: Vec::new(),
                association_endpoints: Vec::new(),
                property_bindings: Vec::new(),
                association_bindings: vec![MapAssociationBinding {
                    assertion_id: "placed_order_assertion".to_string(),
                    association_type: "Placed-Order".to_string(),
                    target_identity_rule_id: "order_identity".to_string(),
                    source_identity_rule_id: String::new(),
                    source_role: "customer".to_string(),
                    target_role: "order".to_string(),
                    source_endpoint_expression: "source.customer_id".to_string(),
                    target_endpoint_expression: "source.order_id".to_string(),
                    valid_from_expression: None,
                    valid_to_expression: None,
                    cardinality_policy: "one_to_many".to_string(),
                    missing_policy: "reject".to_string(),
                    link_object_materialization: "required".to_string(),
                }],
            }],
        }),
    ];

    let relationships = map_relationship_records_json(&sections);

    assert_eq!(relationships.len(), 1);
    let relationship = &relationships[0].1;
    assert_eq!(relationship["from_root"], "object(Customer)");
    assert_eq!(relationship["to_root"], "object(Order)");
    assert_eq!(
        relationship["association_root"],
        r#"association("Placed-Order")"#
    );
    assert_eq!(relationship["cardinality_hint"], "one_to_many");
    assert_eq!(
        relationship["example"],
        r#"object(Customer).where(exists(association("Placed-Order"))).take(20)"#
    );
}

#[test]
fn embedded_section_payload_marks_query_discovery_optional() {
    let manifest = QueryDiscoveryManifest::parse(MINIMAL).unwrap();
    let section = query_discovery_section_payload(&manifest);
    assert_eq!(
        section.section_kind,
        SectionKind::QueryDiscoveryManifest as u16
    );
    assert_eq!(section.profile, PrimaryProfile::QueryDiscovery as u8);
    assert_eq!(section.required_features, 0);
    assert_eq!(
        section.optional_features & FEATURE_QUERY_DISCOVERY_METADATA,
        FEATURE_QUERY_DISCOVERY_METADATA
    );
    assert_eq!(section.data, MINIMAL);
}

#[test]
fn scan_writer_embeds_query_discovery_without_required_feature_bit() {
    let base = table_fixture_bytes();
    let manifest = build_query_discovery_manifest(&base, QueryDiscoveryOptions::default()).unwrap();
    let mut writer = ScanProfileCoveWriter::new(table_catalog_fixture());
    writer.push_extra_section(query_discovery_section_payload(&manifest));

    let bytes = writer.write().unwrap();
    let header = CoveHeaderV1::parse(&bytes).unwrap();
    assert_eq!(
        header.required_features & FEATURE_QUERY_DISCOVERY_METADATA,
        0
    );
    assert_eq!(
        header.optional_features & FEATURE_QUERY_DISCOVERY_METADATA,
        FEATURE_QUERY_DISCOVERY_METADATA
    );

    let validated = validate_bytes(&bytes).unwrap();
    let entry = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::QueryDiscoveryManifest as u16)
        .unwrap();
    assert_eq!(entry.required_features, 0);
    assert_eq!(
        entry.optional_features & FEATURE_QUERY_DISCOVERY_METADATA,
        FEATURE_QUERY_DISCOVERY_METADATA
    );

    let embedded = embedded_query_discovery_manifests(&bytes).unwrap();
    assert_eq!(embedded.len(), 1);
    assert_eq!(
        embedded[0].raw_canonical_json(),
        manifest.raw_canonical_json()
    );

    let embedded_context = query_discovery_validation_context_for_embedded_source(
        &bytes,
        &QueryDiscoveryOptions::default(),
    )
    .unwrap();
    let embedded_report = validate_query_discovery_manifest(&embedded[0], embedded_context);
    assert_eq!(
        embedded_report.validation_status,
        QueryDiscoveryValidationStatus::Valid
    );
}

#[test]
fn embedded_extraction_rejects_noncanonical_manifest_payload() {
    let mut writer = ScanProfileCoveWriter::new(table_catalog_fixture());
    writer.push_extra_section(SectionPayload {
        section_kind: SectionKind::QueryDiscoveryManifest as u16,
        profile: PrimaryProfile::QueryDiscovery as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_QUERY_DISCOVERY_METADATA,
        data: br#"{"schema":"cove.query_discovery.v1","canonicalization":"rfc8785-jcs","authority":"advisory_discovery_not_archive_truth","source_binding":{},"coveql":{},"surfaces":{},"policy":{}}"#.to_vec(),
    });
    let bytes = writer.write().unwrap();
    validate_bytes(&bytes).unwrap();
    assert!(matches!(
        embedded_query_discovery_manifests(&bytes),
        Err(CoveError::QueryDiscoveryInvalid(message)) if message.contains("not canonical")
    ));
}

fn table_fixture_bytes() -> Vec<u8> {
    let mut writer = ScanProfileCoveWriter::new(table_catalog_fixture());
    writer.file_id = [7; 16];
    let bytes = writer.write().unwrap();
    let header = crate::header::CoveHeaderV1::parse(&bytes).unwrap();
    assert_eq!(header.primary_profile, PrimaryProfile::TableScan as u8);
    assert_ne!(header.required_features & FEATURE_TABLE_PROFILE, 0);
    bytes
}

fn table_catalog_fixture() -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: String::new(),
            name: "order-history".to_string(),
            row_count: 0,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                ColumnEntry {
                    column_id: 1,
                    name: "select".to_string(),
                    logical: CoveLogicalType::Utf8,
                    physical: CovePhysicalKind::VarBytes,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
                ColumnEntry {
                    column_id: 2,
                    name: "score".to_string(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: true,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
            ],
        }],
    }
}

fn object_property_fixture(property_id: u32, name: &str, flags: u32) -> PropertyEntryV1 {
    PropertyEntryV1 {
        property_id,
        property_name: name.to_string(),
        logical_type: CoveLogicalType::Utf8,
        physical_kind: CovePhysicalKind::VarBytes,
        nullable: true,
        collation_id: 0,
        flags,
    }
}

fn identity_rule_fixture(rule_id: &str, object_type: &str) -> MapIdentityRule {
    MapIdentityRule {
        rule_id: rule_id.to_string(),
        object_type: object_type.to_string(),
        semantic_role: "entity".to_string(),
        confidence_class: "strong".to_string(),
        auto_merge: Some(true),
        candidate_only: false,
        property_conflicts_declared: false,
        allow_reviewed_equivalence: true,
        function_ids: Vec::new(),
        join_keys: vec![MapJoinKeyComponent {
            role_id: "id".to_string(),
            source_column: "id".to_string(),
            logical_type: "utf8".to_string(),
            canonicalization: "identity".to_string(),
            null_policy: "reject".to_string(),
            ordering: "ascending".to_string(),
            resolution: None,
        }],
    }
}
