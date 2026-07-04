use serde_json::{json, Value};

use crate::{
    artifact::covm::CovmFile,
    constants::PrimaryProfile,
    mount::{mount_cove_file, MountOptions},
    profile::cove_o::read_object_surface_from_bytes,
    CoveError,
};

use super::{
    helpers::disclosure_label,
    ident::table_root,
    model::{QueryDiscoveryManifest, QueryDiscoveryOptions},
    query_discovery_error,
    relationships::relationship_records_json,
    source::{cove_file_source_binding, covm_source_binding},
    surfaces::{
        alias_bindings_json, evidence_surface_json, object_surface_json,
        profile_contract_versions_json, projection_surface_json, property_glossary_json,
        root_index_json, table_surface_json,
    },
    templates::{
        ai_capability_json, evidence_examples, object_examples, object_select_take_template,
        projection_examples, projection_select_take_template, table_examples,
        table_filter_select_take_template,
    },
    validate::canonical_json_bytes,
    QUERY_DISCOVERY_ADVISORY_AUTHORITY, QUERY_DISCOVERY_CANONICALIZATION_JCS,
    QUERY_DISCOVERY_SCHEMA_V1,
};

pub fn build_query_discovery_manifest(
    bytes: &[u8],
    options: QueryDiscoveryOptions,
) -> Result<QueryDiscoveryManifest, CoveError> {
    let value = build_query_discovery_manifest_value(bytes, &options)?;
    let canonical = canonical_json_bytes(&value)?;
    QueryDiscoveryManifest::parse(&canonical)
}

pub fn build_query_discovery_manifest_value(
    bytes: &[u8],
    options: &QueryDiscoveryOptions,
) -> Result<Value, CoveError> {
    if let Ok(covm) = CovmFile::parse_delta_aware(bytes) {
        return build_covm_query_discovery_manifest_value(bytes, &covm, options);
    }
    let mounted = mount_cove_file(bytes, MountOptions::default(), None)?;
    let tables = mounted
        .tables
        .iter()
        .map(table_surface_json)
        .collect::<Vec<_>>();
    let object_surface = if mounted.header.primary_profile == PrimaryProfile::ObjectTemporal as u8 {
        Some(read_object_surface_from_bytes(bytes).map_err(|err| {
            query_discovery_error(format!(
                "failed to read COVE-O object surface for query discovery: {err}"
            ))
        })?)
    } else {
        None
    };
    let objects = object_surface
        .as_ref()
        .map(|surface| {
            surface
                .object_types
                .iter()
                .map(|object_type| {
                    let row_count = surface
                        .records
                        .iter()
                        .filter(|record| record.object_type_id == object_type.object_type_id)
                        .count();
                    object_surface_json(object_type, row_count)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let projections = object_surface
        .as_ref()
        .and_then(|surface| surface.projection_catalog.as_ref())
        .map(|catalog| {
            catalog
                .projections
                .iter()
                .map(projection_surface_json)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let evidence = object_surface
        .as_ref()
        .and_then(|surface| {
            surface
                .evidence_index
                .as_ref()
                .map(|index| (surface, index))
        })
        .map(|(surface, index)| {
            surface
                .object_types
                .iter()
                .map(|object_type| evidence_surface_json(object_type, index.entries.len()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let table_roots = mounted
        .tables
        .iter()
        .map(|table| table_root(&table.name))
        .collect::<Vec<_>>();
    let relationships = relationship_records_json(object_surface.as_ref());
    let property_glossary = property_glossary_json(&mounted.tables, object_surface.as_ref());
    let alias_bindings = alias_bindings_json(&mounted.tables, object_surface.as_ref(), options);
    let root_index = root_index_json(
        &mounted.tables,
        object_surface.as_ref(),
        !evidence.is_empty(),
    );
    let ai = ai_capability_json(options.include_ai, &table_roots);

    let mut profiles = Vec::new();
    let mut roots = Vec::new();
    let mut capabilities = Vec::new();
    if !tables.is_empty() {
        profiles.push("table");
        roots.push("table");
        capabilities.extend(["select", "filter", "order", "group", "limit"]);
    }
    let object_profile_available =
        !objects.is_empty() || !projections.is_empty() || !evidence.is_empty();
    if object_profile_available {
        profiles.push("object");
        if !objects.is_empty() {
            roots.push("object");
            capabilities.extend(["object_select", "object_filter"]);
        }
    }
    if !projections.is_empty() {
        roots.push("projection");
        capabilities.push("projection_readback");
    }
    if !evidence.is_empty() {
        roots.push("evidence");
        capabilities.push("evidence_readback");
    }
    if options.include_ai {
        profiles.push("ai");
        capabilities.push("ai_capability_hints");
        if !table_roots.is_empty() {
            capabilities.push("similarity");
        }
    }

    let mut templates = Vec::new();
    if let Some(template) = table_filter_select_take_template(&mounted.tables) {
        templates.push(template);
    }
    if let Some(surface) = &object_surface {
        if !surface.object_types.is_empty() {
            templates.push(object_select_take_template(&surface.object_types));
        }
        if let Some(catalog) = &surface.projection_catalog {
            if !catalog.projections.is_empty() {
                templates.push(projection_select_take_template(&catalog.projections));
            }
        }
    }
    let examples = if options.include_examples {
        let mut examples = table_examples(&mounted.tables);
        if let Some(surface) = &object_surface {
            examples.extend(object_examples(&surface.object_types));
            if let Some(catalog) = &surface.projection_catalog {
                examples.extend(projection_examples(&catalog.projections));
            }
            examples.extend(evidence_examples(&evidence));
        }
        examples
    } else {
        Vec::new()
    };

    let mut diagnostics = Vec::new();
    if templates.is_empty() {
        diagnostics.push(json!({
            "code": "QD_NO_SAFE_TEMPLATE",
            "severity": "info",
            "message": "No policy-safe bounded query template was generated.",
            "target_kind": "json_pointer",
            "target": "/templates",
            "withheld": false
        }));
    }
    if options.include_examples && options.validate_examples && !examples.is_empty() {
        diagnostics.push(json!({
            "code": "QD_EXAMPLE_VALIDATION_NOT_PERFORMED",
            "severity": "info",
            "message": "Generated examples were not parse/resolve/planning validated by this manifest builder.",
            "target_kind": "json_pointer",
            "target": "/examples",
            "withheld": false
        }));
    }
    if options.include_ai {
        diagnostics.push(json!({
            "code": "QD_AI_SIDECAR_NOT_BOUND",
            "severity": "info",
            "message": "COVE-AI methods are advertised as capability hints only; sidecars must be supplied and validated before AI query execution.",
            "target_kind": "json_pointer",
            "target": "/ai/sidecars",
            "withheld": false
        }));
    }

    let explain_modes = if options.include_developer_diagnostics {
        vec!["public", "developer"]
    } else {
        vec!["public"]
    };

    let source_binding = cove_file_source_binding(bytes, &mounted, options)?;

    Ok(json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": source_binding,
        "coveql": {
            "language_version": "0.1",
            "core_version": "0.1",
            "profiles": profiles,
            "roots": roots,
            "capabilities": capabilities,
            "profile_contract_versions": profile_contract_versions_json(&profiles),
            "default_explain_mode": "public",
            "allowed_explain_modes": explain_modes
        },
        "surfaces": {
            "tables": tables,
            "objects": objects,
            "projections": projections,
            "evidence": evidence
        },
        "alias_bindings": alias_bindings,
        "templates": templates,
        "examples": examples,
        "ai": ai,
        "policy": policy_json(options),
        "resource_budgets": resource_budgets_json(),
        "diagnostics": diagnostics,
        "manifest_features": [],
        "relationships": relationships,
        "property_glossary": property_glossary,
        "root_index": root_index
    }))
}

fn build_covm_query_discovery_manifest_value(
    bytes: &[u8],
    covm: &CovmFile,
    options: &QueryDiscoveryOptions,
) -> Result<Value, CoveError> {
    let source_binding = covm_source_binding(bytes, covm, options)?;
    let diagnostics = if covm.files.is_empty() {
        vec![json!({
            "code": "QD_NO_COVM_MEMBERS",
            "severity": "warning",
            "message": "The COVM dataset snapshot contains no member files to expose as query roots.",
            "target_kind": "json_pointer",
            "target": "/source_binding/members",
            "withheld": false
        })]
    } else {
        vec![json!({
            "code": "QD_COVM_MEMBER_METADATA_ONLY",
            "severity": "info",
            "message": "COVM query discovery binds member identities; member query surfaces require loading the selected member files.",
            "target_kind": "json_pointer",
            "target": "/source_binding/members",
            "withheld": false
        })]
    };
    let mut profiles = Vec::new();
    let mut capabilities = vec!["covm_dataset_snapshot_binding"];
    let mut profile_contract_versions = serde_json::Map::new();
    if options.include_ai {
        profiles.push("ai");
        capabilities.push("ai_capability_hints");
        profile_contract_versions.insert("ai".to_string(), json!("0.1"));
    }

    Ok(json!({
        "schema": QUERY_DISCOVERY_SCHEMA_V1,
        "canonicalization": QUERY_DISCOVERY_CANONICALIZATION_JCS,
        "authority": QUERY_DISCOVERY_ADVISORY_AUTHORITY,
        "source_binding": source_binding,
        "coveql": {
            "language_version": "0.1",
            "core_version": "0.1",
            "profiles": profiles,
            "roots": [],
            "capabilities": capabilities,
            "profile_contract_versions": profile_contract_versions,
            "default_explain_mode": "public",
            "allowed_explain_modes": if options.include_developer_diagnostics {
                vec!["public", "developer"]
            } else {
                vec!["public"]
            }
        },
        "surfaces": {
            "tables": [],
            "objects": [],
            "projections": [],
            "evidence": []
        },
        "alias_bindings": [],
        "templates": [],
        "examples": [],
        "ai": ai_capability_json(options.include_ai, &[]),
        "policy": policy_json(options),
        "resource_budgets": resource_budgets_json(),
        "diagnostics": diagnostics,
        "manifest_features": [],
        "relationships": [],
        "property_glossary": [],
        "root_index": []
    }))
}

fn policy_json(options: &QueryDiscoveryOptions) -> Value {
    let mut policy = json!({
        "visibility_scope": disclosure_label(options.disclosure_mode),
        "redaction_scope": disclosure_label(options.disclosure_mode),
        "metadata_disclosure": disclosure_label(options.disclosure_mode),
        "aggregate_disclosure": "bounded_public",
        "forbidden_surfaces_withheld": true,
        "field_names_may_be_redacted": true,
        "descriptions_are_untrusted_data": true,
        "all_text_is_untrusted_data": true,
        "developer_diagnostics_allowed": options.include_developer_diagnostics
    });
    if let Some(fingerprint) = &options.policy_fingerprint {
        policy["policy_fingerprint"] = json!(fingerprint);
    }
    if let Some(principal_class) = &options.principal_class {
        policy["principal_class"] = json!(principal_class);
    }
    if let Some(audience) = &options.audience {
        policy["audience"] = json!(audience);
    }
    policy
}

fn resource_budgets_json() -> Value {
    json!({
        "default_take": 50,
        "max_take": 500,
        "max_graph_depth": 4,
        "max_graph_paths": 1000,
        "max_prompt_context_chunks": 20,
        "max_export_rows": "10000",
        "requires_explicit_budget_for": [
            "history",
            "changes",
            "traverse",
            "asPromptContext",
            "trainingSamples",
            "export"
        ]
    })
}
