use super::*;
use std::{fs, sync::Arc};

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_ipc::writer::FileWriter as IpcFileWriter;
use cove_core::{
    artifact::covedelta::{
        CoveDeltaFile, DELTA_FEATURE_INLINE_DICTIONARY, DELTA_FEATURE_SPARSE_PATCH_ROWS,
        DELTA_PROPERTY_OP_SET_VALUE,
    },
    artifact::covemap::{
        CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1, CovemapSection,
        CovemapSectionEntryV1,
    },
    artifact::covm::CovmDeltaArtifactRefV1,
    compression,
    constants::{DigestAlgorithm, FEATURE_CODEC_ZSTD, FEATURE_SEMANTIC_MAP},
    digest::compute_digest,
    profile::cove_map::{is_compact_evidence_index_bytes, MapEvidenceIndex},
    profile::cove_o::{
        read_object_surface_from_base_and_delta_files, read_object_surface_from_bytes,
        reconstruct_object_states, reconstruct_object_states_from_base_and_delta_files,
        CoveObjectState, TemporalSegmentData, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
        PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
        PROPERTY_FLAG_EVIDENCE_REF,
    },
    reader::validate_bytes,
};
use orc_rust::ArrowWriterBuilder as OrcWriterBuilder;
use parquet::arrow::ArrowWriter;

fn test_section(kind: SectionKind, value: Value) -> CovemapSection {
    let payload = serde_json::to_vec_pretty(&covemap_payload_value(kind, value))
        .expect("serializing serde_json::Value cannot fail");
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: 0,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    }
}

fn mutate_section_payload(file: &mut CovemapFile, index: usize, edit: impl FnOnce(&mut Value)) {
    let mut payload: Value = serde_json::from_slice(&file.sections[index].payload).unwrap();
    edit(&mut payload);
    let bytes =
        serde_json::to_vec_pretty(&payload).expect("serializing serde_json::Value cannot fail");
    file.sections[index].entry.length = bytes.len() as u64;
    file.sections[index].entry.uncompressed_length = bytes.len() as u64;
    file.sections[index].payload = bytes;
}

fn covemap_payload_value(kind: SectionKind, mut value: Value) -> Value {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".to_string(),
            Value::String("org.coveformat.covemap.v2".to_string()),
        );
        object.insert(
            "section_id".to_string(),
            Value::Number((kind as u16).into()),
        );
    }
    value
}

fn test_covemap(sections: Vec<CovemapSection>) -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0x42; 16], 0),
        mapping_version: "test/v1".into(),
        sections,
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn resolution_catalog_section(mapping_id: &str, mapping_version: &str) -> CovemapSection {
    company_resolution_catalog_section_with_policy(
        mapping_id,
        mapping_version,
        "candidate_only",
        None,
        "retain_raw",
    )
}

fn redacted_resolution_catalog_section(mapping_id: &str, mapping_version: &str) -> CovemapSection {
    company_resolution_catalog_section_with_policy(
        mapping_id,
        mapping_version,
        "candidate_only",
        None,
        "redact_raw",
    )
}

fn company_resolution_catalog_section_with_miss_policy(
    mapping_id: &str,
    mapping_version: &str,
    on_miss: &str,
    miss_confidence_class: Option<&str>,
) -> CovemapSection {
    company_resolution_catalog_section_with_policy(
        mapping_id,
        mapping_version,
        on_miss,
        miss_confidence_class,
        "retain_raw",
    )
}

fn normalized_miss_resolution_catalog_section(
    mapping_id: &str,
    mapping_version: &str,
) -> CovemapSection {
    company_resolution_catalog_section_with_miss_policy(
        mapping_id,
        mapping_version,
        "normalized_value",
        Some("weak_deterministic"),
    )
}

fn company_resolution_catalog_section_with_policy(
    mapping_id: &str,
    mapping_version: &str,
    on_miss: &str,
    miss_confidence_class: Option<&str>,
    evidence_policy: &str,
) -> CovemapSection {
    let pipeline_digest_input = json!({
        "pipeline_id": "company_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
    let alias_catalog_digest_input = json!({
        "alias_catalog_id": "company_aliases",
        "entries": [{
            "alias_entry_id": "company:tesco",
            "canonical_key": "uk-company:tesco",
            "canonical_label": "Tesco",
            "aliases": ["Tesco", "Tesco PLC", "tesco supermarket"]
        }]
    });
    let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
    let resolver_digest_input = json!({
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": on_miss,
        "miss_confidence_class": miss_confidence_class,
        "ambiguous_policy": "reject_auto_merge",
        "catalog_digest": catalog_digest,
        "evidence_policy": evidence_policy,
    });
    let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
    let mut payload = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "normalization_pipelines": [{
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        }],
        "resolvers": [{
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "on_hit": "canonical_key",
            "on_miss": on_miss,
            "miss_confidence_class": miss_confidence_class,
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "pipeline_digest": pipeline_digest,
            "resolver_digest": resolver_digest,
            "evidence_policy": evidence_policy,
            "alias_catalog": {
                "alias_catalog_id": "company_aliases",
                "entries": [{
                    "alias_entry_id": "company:tesco",
                    "canonical_key": "uk-company:tesco",
                    "canonical_label": "Tesco",
                    "aliases": ["tesco supermarket", "Tesco PLC", "Tesco"]
                }]
            }
        }],
        "match_rules": [],
        "reviewed_decisions": []
    });
    if miss_confidence_class.is_none() {
        payload["resolvers"][0]
            .as_object_mut()
            .unwrap()
            .remove("miss_confidence_class");
    }
    test_section(SectionKind::MapResolutionCatalog, payload)
}

fn team_resolution_catalog_section(mapping_id: &str, mapping_version: &str) -> CovemapSection {
    let pipeline_digest_input = json!({
        "pipeline_id": "team_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
    let alias_catalog_digest_input = json!({
        "alias_catalog_id": "team_aliases",
        "entries": [{
            "alias_entry_id": "team:alpha",
            "canonical_key": "team:alpha",
            "canonical_label": "Alpha Team",
            "aliases": ["Alpha Team Ltd", "Team Alpha", "alpha team"]
        }]
    });
    let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
    let resolver_digest_input = json!({
        "resolver_id": "team_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Team",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "team_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "reject",
        "miss_confidence_class": null,
        "ambiguous_policy": "reject_auto_merge",
        "catalog_digest": catalog_digest,
        "evidence_policy": "retain_raw",
    });
    let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
    test_section(
        SectionKind::MapResolutionCatalog,
        json!({
            "mapping_id": mapping_id,
            "mapping_version": mapping_version,
            "normalization_pipelines": [{
                "pipeline_id": "team_name.v1",
                "functions": [
                    {"function_id": "unicode_nfkc", "version": "1"},
                    {"function_id": "unicode_casefold", "version": "1"},
                    {"function_id": "trim", "version": "1"},
                    {"function_id": "collapse_whitespace", "version": "1"}
                ],
                "tables": []
            }],
            "resolvers": [{
                "resolver_id": "team_name_resolver",
                "kind": "alias_catalog",
                "object_type": "Team",
                "authority": "curated",
                "confidence_class": "authoritative",
                "normalization_pipeline_id": "team_name.v1",
                "on_hit": "canonical_key",
                "on_miss": "reject",
                "ambiguous_policy": "reject_auto_merge",
                "catalog_digest": catalog_digest,
                "pipeline_digest": pipeline_digest,
                "resolver_digest": resolver_digest,
                "alias_catalog": {
                    "alias_catalog_id": "team_aliases",
                    "entries": [{
                        "alias_entry_id": "team:alpha",
                        "canonical_key": "team:alpha",
                        "canonical_label": "Alpha Team",
                        "aliases": ["Team Alpha", "Alpha Team Ltd", "alpha team"]
                    }]
                }
            }],
            "match_rules": [],
            "reviewed_decisions": []
        }),
    )
}

fn ambiguous_company_resolution_catalog_section(
    mapping_id: &str,
    mapping_version: &str,
    ambiguous_policy: &str,
) -> CovemapSection {
    ambiguous_company_resolution_catalog_section_with_policy(
        mapping_id,
        mapping_version,
        ambiguous_policy,
        "retain_raw",
    )
}

fn ambiguous_company_resolution_catalog_section_with_policy(
    mapping_id: &str,
    mapping_version: &str,
    ambiguous_policy: &str,
    evidence_policy: &str,
) -> CovemapSection {
    let pipeline_digest_input = json!({
        "pipeline_id": "company_name.v1",
        "functions": [
            {"function_id": "unicode_nfkc", "version": "1"},
            {"function_id": "unicode_casefold", "version": "1"},
            {"function_id": "trim", "version": "1"},
            {"function_id": "collapse_whitespace", "version": "1"}
        ],
        "tables": []
    });
    let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
    let alias_catalog_digest_input = json!({
        "alias_catalog_id": "company_aliases",
        "entries": [{
            "alias_entry_id": "company:tesco",
            "canonical_key": "uk-company:tesco",
            "canonical_label": "Tesco",
            "aliases": ["Tesco", "Tesco PLC", "tesco supermarket"],
            "ambiguous": true
        }]
    });
    let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
    let resolver_digest_input = json!({
        "resolver_id": "uk_company_name_resolver",
        "kind": "alias_catalog",
        "object_type": "Company",
        "authority": "curated",
        "confidence_class": "authoritative",
        "normalization_pipeline_id": "company_name.v1",
        "pipeline_digest": pipeline_digest,
        "on_hit": "canonical_key",
        "on_miss": "candidate_only",
        "miss_confidence_class": null,
        "ambiguous_policy": ambiguous_policy,
        "catalog_digest": catalog_digest,
        "evidence_policy": evidence_policy,
    });
    let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
    test_section(
        SectionKind::MapResolutionCatalog,
        json!({
            "mapping_id": mapping_id,
            "mapping_version": mapping_version,
            "normalization_pipelines": [{
                "pipeline_id": "company_name.v1",
                "functions": [
                    {"function_id": "unicode_nfkc", "version": "1"},
                    {"function_id": "unicode_casefold", "version": "1"},
                    {"function_id": "trim", "version": "1"},
                    {"function_id": "collapse_whitespace", "version": "1"}
                ],
                "tables": []
            }],
            "resolvers": [{
                "resolver_id": "uk_company_name_resolver",
                "kind": "alias_catalog",
                "object_type": "Company",
                "authority": "curated",
                "confidence_class": "authoritative",
                "normalization_pipeline_id": "company_name.v1",
                "on_hit": "canonical_key",
                "on_miss": "candidate_only",
                "ambiguous_policy": ambiguous_policy,
                "catalog_digest": catalog_digest,
                "pipeline_digest": pipeline_digest,
                "resolver_digest": resolver_digest,
                "evidence_policy": evidence_policy,
                "alias_catalog": {
                    "alias_catalog_id": "company_aliases",
                    "entries": [{
                        "alias_entry_id": "company:tesco",
                        "canonical_key": "uk-company:tesco",
                        "canonical_label": "Tesco",
                        "aliases": ["tesco supermarket", "Tesco PLC", "Tesco"],
                        "ambiguous": true
                    }]
                }
            }],
            "match_rules": [],
            "reviewed_decisions": []
        }),
    )
}

fn company_resolution_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "suppliers",
                    "row_identity_rules": ["company_by_resolved_name"]
                }]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "identity_rules": [{
                    "rule_id": "company_by_resolved_name",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "auto_merge": true,
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "company",
                        "source_column": "company_name",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared",
                        "resolution": {
                            "resolver_id": "uk_company_name_resolver"
                        }
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "rules": [{
                    "rule_id": "supplier_company",
                    "source_id": "suppliers",
                    "identity_rule_id": "company_by_resolved_name",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": []
                }]
            }),
        ),
        resolution_catalog_section("company-map", "test/v1"),
    ])
}

fn test_sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

fn test_canonical_json(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_test_canonical_json(value, &mut out);
    out
}

fn write_test_canonical_json(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(value) => {
            out.extend_from_slice(Value::String(value.clone()).to_string().as_bytes());
        }
        Value::Array(values) => {
            out.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_test_canonical_json(value, out);
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            let mut keys = object
                .keys()
                .filter(|key| key.as_str() != "non_semantic_metadata")
                .collect::<Vec<_>>();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(Value::String((*key).clone()).to_string().as_bytes());
                out.push(b':');
                write_test_canonical_json(object.get(*key).unwrap(), out);
            }
            out.push(b'}');
        }
    }
}

fn people_batch() -> RecordBatch {
    RecordBatch::try_from_iter(vec![
        (
            "person_id",
            Arc::new(StringArray::from(vec!["p1"])) as arrow_array::ArrayRef,
        ),
        (
            "team_id",
            Arc::new(StringArray::from(vec!["t1"])) as arrow_array::ArrayRef,
        ),
        (
            "valid_from",
            Arc::new(StringArray::from(vec!["2026-01-01"])) as arrow_array::ArrayRef,
        ),
        (
            "valid_to",
            Arc::new(StringArray::from(vec!["2026-12-31"])) as arrow_array::ArrayRef,
        ),
    ])
    .unwrap()
}

fn write_arrow_ipc(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = IpcFileWriter::try_new(&mut bytes, &batch.schema()).unwrap();
        writer.write(batch).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn write_parquet(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    bytes
}

fn write_orc(batch: &RecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = OrcWriterBuilder::new(&mut bytes, batch.schema())
            .try_build()
            .unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    bytes
}

fn two_source_identity_map(do_not_merge: Vec<Value>) -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [
                    {"source_id": "crm", "row_identity_rules": ["person_by_id"]},
                    {"source_id": "support", "row_identity_rules": ["person_by_id"]}
                ]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": do_not_merge
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "crm_person",
                        "source_id": "crm",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "support_person",
                        "source_id": "support",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
    ])
}

fn reviewed_decisions_section(
    mapping_id: &str,
    mapping_version: &str,
    decisions: Vec<Value>,
) -> CovemapSection {
    test_section(
        SectionKind::MapResolutionCatalog,
        json!({
            "mapping_id": mapping_id,
            "mapping_version": mapping_version,
            "normalization_pipelines": [],
            "resolvers": [],
            "match_rules": [],
            "reviewed_decisions": decisions
        }),
    )
}

fn add_reviewed_decisions(file: &mut CovemapFile, decisions: Vec<Value>) {
    file.sections.push(reviewed_decisions_section(
        "people-map",
        "test/v1",
        decisions,
    ));
}

fn set_person_reviewed_equivalence(file: &mut CovemapFile, allowed: bool) {
    mutate_section_payload(file, 2, |payload| {
        payload["identity_rules"][0]["allow_reviewed_equivalence"] = json!(allowed);
    });
}

fn identity_alias_ref(object_type: &str, identity_alias: &str) -> Value {
    json!({
        "kind": "identity_alias",
        "object_type": object_type,
        "identity_alias": identity_alias
    })
}

fn reviewed_same_object_decision(left: Value, right: Value, anchor: Option<Value>) -> Value {
    let mut decision = json!({
        "decision_id": "review:same-object",
        "decision": "same_object",
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "mapping-author",
        "reviewed_at": "2026-06-25T00:00:00Z",
        "left": left,
        "right": right
    });
    if let Some(anchor) = anchor {
        decision["canonical_anchor"] = anchor;
    }
    decision
}

fn reviewed_do_not_merge_decision(left: Value, right: Value) -> Value {
    json!({
        "decision_id": "review:do-not-merge",
        "decision": "do_not_merge",
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "mapping-author",
        "reviewed_at": "2026-06-25T00:00:00Z",
        "left": left,
        "right": right
    })
}

fn reviewed_rows(ids: &[(&str, &str)]) -> Vec<SourceRow> {
    ids.iter()
        .map(|(source_id, id)| SourceRow {
            source_id: (*source_id).into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!(*id))]),
        })
        .collect()
}

fn three_source_identity_map() -> CovemapFile {
    let mut file = two_source_identity_map(Vec::new());
    mutate_section_payload(&mut file, 0, |payload| {
        payload["sources"].as_array_mut().unwrap().push(json!({
            "source_id": "ops",
            "row_identity_rules": ["person_by_id"]
        }));
    });
    mutate_section_payload(&mut file, 3, |payload| {
        payload["rules"].as_array_mut().unwrap().push(json!({
            "rule_id": "ops_person",
            "source_id": "ops",
            "identity_rule_id": "person_by_id",
            "row_semantics_kind": "Object",
            "assertion_kinds": ["object", "evidence"],
            "function_ids": ["identity"],
            "output_assertion_ids": [],
            "association_endpoints": []
        }));
    });
    file
}

fn cross_rule_reviewed_identity_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "people",
                    "row_identity_rules": ["person_by_id", "person_by_email"]
                }]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "allow_reviewed_equivalence": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "person_by_email",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "allow_reviewed_equivalence": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "email",
                            "source_column": "email",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "person_id_row",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "person_email_row",
                        "source_id": "people",
                        "identity_rule_id": "person_by_email",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
    ])
}

fn add_optional_i64(object: &mut Value, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        object
            .as_object_mut()
            .unwrap()
            .insert(key.into(), json!(value));
    }
}

fn two_source_property_map(
    conflict_policy: &str,
    crm_priority: Option<i64>,
    support_priority: Option<i64>,
) -> CovemapFile {
    let mut crm = json!({
        "source_id": "crm",
        "row_identity_rules": ["person_by_id"]
    });
    add_optional_i64(&mut crm, "source_priority", crm_priority);
    let mut support = json!({
        "source_id": "support",
        "row_identity_rules": ["person_by_id"]
    });
    add_optional_i64(&mut support, "source_priority", support_priority);

    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [crm, support]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "crm_person",
                        "source_id": "crm",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "crm_name",
                            "property_id": "name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy
                        }]
                    },
                    {
                        "rule_id": "support_person",
                        "source_id": "support",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "support_name",
                            "property_id": "name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": conflict_policy
                        }]
                    }
                ]
            }),
        ),
    ])
}

fn conflict_rows(crm_name: Value, support_name: Value) -> Vec<SourceRow> {
    vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), crm_name)]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), support_name)]),
        },
    ]
}

fn build_projection_map() -> CovemapFile {
    let mut file = two_source_property_map("source_priority_wins", Some(10), Some(1));
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "name", "value": "name", "logical_type": "utf8"}
                ],
                "output_modes": ["json", "cove-t"]
            }]
        }),
    ));
    file
}

fn temp_build_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cove-map-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_build_fixture(label: &str, file: &CovemapFile) -> (PathBuf, Vec<PathBuf>, PathBuf) {
    let dir = temp_build_dir(label);
    let map = dir.join("people.covemap");
    fs::write(&map, file.serialize().unwrap()).unwrap();
    let crm = dir.join("crm.csv");
    let support = dir.join("support.csv");
    fs::write(&crm, "id,name\n1,CRM Name\n").unwrap();
    fs::write(&support, "id,name\n1,Support Name\n").unwrap();
    (map, vec![crm, support], dir)
}

fn write_map_and_sources(
    label: &str,
    map_file: &CovemapFile,
    sources: &[(&str, &str)],
) -> (PathBuf, Vec<PathBuf>, PathBuf) {
    let dir = temp_build_dir(label);
    let map = dir.join("mapping.covemap");
    fs::write(&map, map_file.serialize().unwrap()).unwrap();
    let source_paths = sources
        .iter()
        .map(|(name, contents)| {
            let path = dir.join(name);
            fs::write(&path, contents).unwrap();
            path
        })
        .collect::<Vec<_>>();
    (map, source_paths, dir)
}

fn empty_cove_o_parent_bytes() -> Vec<u8> {
    compact_cove_o_from_object_states(Vec::new(), &[]).unwrap()
}

fn delta_parent_from_base_bytes(base: &[u8]) -> MapSemanticDeltaParent {
    let validation = validate_bytes(base).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, base).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    let file_id = validation.header.file_id;
    MapSemanticDeltaParent {
        dataset_id: [0xDD; 16],
        parent_snapshot_id: file_id,
        chain_ordinal: 1,
        chain_depth: 1,
        parent_ref: CovmDeltaArtifactRefV1 {
            chain_ordinal: 0,
            flags: 0,
            artifact_id: file_id,
            snapshot_id: file_id,
            parent_snapshot_id: [0; 16],
            file_len: base.len() as u64,
            footer_crc32c: validation.postscript.footer.crc32c,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest_len: 32,
            digest: digest_array,
            uri_ref: 0,
            checksum: 0,
        },
    }
}

fn semantic_delta_options(
    out: PathBuf,
    parent: MapSemanticDeltaParent,
) -> MapSemanticDeltaBuildOptions {
    MapSemanticDeltaBuildOptions {
        out,
        force: true,
        parent,
        parent_object_types: Vec::new(),
        parent_object_states: Vec::new(),
        parent_evidence_entries: Vec::new(),
        parent_projection_catalog: None,
        csn_start: 1,
        commit_time_start_us: 1_800_000_000_000_000,
        source_publish_range_us: None,
    }
}

fn semantic_delta_options_from_parent_bytes(
    out: PathBuf,
    parent_bytes: &[u8],
) -> MapSemanticDeltaBuildOptions {
    let surface = read_object_surface_from_bytes(parent_bytes).unwrap();
    let parent_object_states = reconstruct_object_states(&surface, &Default::default()).unwrap();
    let mut options = semantic_delta_options(out, delta_parent_from_base_bytes(parent_bytes));
    options.parent_object_types = surface.object_types;
    options.parent_object_states = parent_object_states;
    options.parent_evidence_entries = surface
        .evidence_index
        .map(|index| index.entries)
        .unwrap_or_default();
    options.parent_projection_catalog = surface.projection_catalog;
    options
}

fn build_semantic_delta_fixture(
    label: &str,
    map_file: &CovemapFile,
    sources: &[(&str, &str)],
    parent: MapSemanticDeltaParent,
) -> (Value, Vec<u8>, PathBuf) {
    let (map, source_paths, dir) = write_map_and_sources(label, map_file, sources);
    let out = dir.join("semantic.covedelta");
    let result = build_semantic_delta_from_paths(
        &map,
        &source_paths,
        semantic_delta_options(out.clone(), parent),
    )
    .unwrap();
    let bytes = fs::read(&out).unwrap();
    assert_eq!(
        result.report["format"],
        json!("cove-map-semantic-delta-build-report-v1")
    );
    (result.report, bytes, dir)
}

fn full_rebuild_state_keys(
    label: &str,
    map_file: &CovemapFile,
    sources: &[(&str, &str)],
) -> Vec<StateKey> {
    let (_map, source_paths, _dir) = write_map_and_sources(label, map_file, sources);
    let inputs = read_source_inputs(&source_paths).unwrap();
    validate_source_inputs(map_file, &inputs.states).unwrap();
    let object_bytes =
        build_cove_o_with_source_states(map_file, &inputs.rows, &inputs.states).unwrap();
    let surface = read_object_surface_from_bytes(&object_bytes).unwrap();
    let states = reconstruct_object_states(&surface, &Default::default()).unwrap();
    state_keys(&states)
}

type StateKey = (u32, [u8; 16], Vec<(u32, Value)>);
type EvidenceKey = (String, String, String, String, String);

fn state_keys(states: &[CoveObjectState]) -> Vec<StateKey> {
    let mut keys = states
        .iter()
        .map(|state| {
            let mut properties = state
                .properties
                .iter()
                .map(|property| (property.property_id, property.value.clone()))
                .collect::<Vec<_>>();
            properties.sort_by_key(|(property_id, _)| *property_id);
            (state.object_type_id, state.goid, properties)
        })
        .collect::<Vec<_>>();
    keys.sort_by_key(|(object_type_id, goid, _)| (*object_type_id, *goid));
    keys
}

fn composed_delta_state_keys(base: &[u8], delta_bytes: &[u8]) -> Vec<StateKey> {
    let delta = CoveDeltaFile::parse(delta_bytes).unwrap();
    delta.validate_object_delta().unwrap();
    let states = reconstruct_object_states_from_base_and_delta_files(
        base,
        &[delta],
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    state_keys(&states)
}

fn evidence_keys_from_cove_o_bytes(bytes: &[u8]) -> BTreeSet<EvidenceKey> {
    let surface = read_object_surface_from_bytes(bytes).unwrap();
    evidence_keys_from_surface(&surface)
}

fn composed_delta_evidence_keys(base: &[u8], delta_bytes: &[u8]) -> BTreeSet<EvidenceKey> {
    let delta = CoveDeltaFile::parse(delta_bytes).unwrap();
    delta.validate_object_delta().unwrap();
    let surface = read_object_surface_from_base_and_delta_files(base, &[delta]).unwrap();
    evidence_keys_from_surface(&surface)
}

fn evidence_keys_from_surface(
    surface: &cove_core::profile::cove_o::CoveObjectSurface,
) -> BTreeSet<EvidenceKey> {
    surface
        .evidence_index
        .as_ref()
        .map(|index| {
            index
                .entries
                .iter()
                .map(|entry| {
                    (
                        entry.source_id.clone(),
                        entry.source_row_identity.clone(),
                        entry.rule_id.clone(),
                        entry.assertion_id.clone(),
                        entry.output_object_id.clone(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn primitive_projection_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "people",
                    "row_identity_rules": ["person_by_id"]
                }]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [{
                    "rule_id": "person_row",
                    "source_id": "people",
                    "identity_rule_id": "person_by_id",
                    "row_semantics_kind": "Object",
                    "assertion_kinds": ["object", "property", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": [],
                    "property_bindings": [
                        {
                            "assertion_id": "active",
                            "property_id": "active",
                            "property_name": "active",
                            "source_column": "active",
                            "logical_type": "bool",
                            "nullable": true,
                            "conflict_policy": "reject_conflict"
                        },
                        {
                            "assertion_id": "score",
                            "property_id": "score",
                            "property_name": "score",
                            "source_column": "score",
                            "logical_type": "int64",
                            "nullable": true,
                            "conflict_policy": "reject_conflict"
                        },
                        {
                            "assertion_id": "rating",
                            "property_id": "rating",
                            "property_name": "rating",
                            "source_column": "rating",
                            "logical_type": "int64",
                            "nullable": true,
                            "conflict_policy": "reject_conflict"
                        },
                        {
                            "assertion_id": "status",
                            "property_id": "status",
                            "property_name": "status",
                            "source_column": "status",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": "reject_conflict"
                        },
                        {
                            "assertion_id": "nickname",
                            "property_id": "nickname",
                            "property_name": "nickname",
                            "source_column": "nickname",
                            "logical_type": "utf8",
                            "nullable": true,
                            "conflict_policy": "reject_conflict"
                        }
                    ]
                }]
            }),
        ),
        test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [
                    {
                        "projection_id": "people_primitives.v1",
                        "output_table": "people_primitives",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                            {"name": "active", "value": "property.active", "logical_type": "bool"},
                            {"name": "score", "value": "score", "logical_type": "int64"},
                            {"name": "rating", "value": "rating", "logical_type": "int64"},
                            {"name": "status", "value": "status", "logical_type": "utf8"},
                            {"name": "nickname", "value": "nickname", "logical_type": "utf8"}
                        ],
                        "output_modes": ["arrow"]
                    },
                    {
                        "projection_id": "people_primitives_ordered.v1",
                        "output_table": "people_primitives",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "active", "value": "property.active", "logical_type": "bool"},
                            {"name": "score", "value": "score", "logical_type": "int64"},
                            {"name": "status", "value": "status", "logical_type": "utf8"}
                        ],
                        "ordering": ["score asc"],
                        "output_modes": ["arrow"]
                    }
                ]
            }),
        ),
    ])
}

fn primitive_projection_rows() -> Vec<SourceRow> {
    vec![
        SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("p1")),
                ("active".into(), json!(true)),
                ("score".into(), json!(10)),
                ("rating".into(), json!(15)),
                ("status".into(), json!("open")),
                ("nickname".into(), json!("alpha")),
            ]),
        },
        SourceRow {
            source_id: "people".into(),
            row_index: 1,
            values: BTreeMap::from([
                ("id".into(), json!("p2")),
                ("active".into(), json!(false)),
                ("score".into(), json!(20)),
                ("rating".into(), json!(25)),
                ("status".into(), json!("closed")),
                ("nickname".into(), Value::Null),
            ]),
        },
        SourceRow {
            source_id: "people".into(),
            row_index: 2,
            values: BTreeMap::from([
                ("id".into(), json!("p3")),
                ("active".into(), json!(true)),
                ("score".into(), json!(30)),
                ("rating".into(), json!(35)),
                ("status".into(), json!("open")),
            ]),
        },
        SourceRow {
            source_id: "people".into(),
            row_index: 3,
            values: BTreeMap::from([
                ("id".into(), json!("p4")),
                ("active".into(), json!(true)),
                ("score".into(), json!(40)),
                ("rating".into(), json!(45)),
                ("status".into(), Value::Null),
                ("nickname".into(), json!("delta")),
            ]),
        },
    ]
}

fn int64_column_values(batches: &[RecordBatch], column_name: &str) -> Vec<i64> {
    let mut out = Vec::new();
    for batch in batches {
        let index = batch.schema().index_of(column_name).unwrap();
        let array = batch
            .column(index)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for row in 0..array.len() {
            if array.is_valid(row) {
                out.push(array.value(row));
            }
        }
    }
    out
}

fn association_readback_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "people",
                    "row_identity_rules": ["person_by_id", "team_by_id"]
                }]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "person_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "team_by_id",
                        "object_type": "Team",
                        "semantic_role": "organization",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "team_id",
                            "source_column": "team_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "person_row",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "association", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "association_bindings": [{
                            "assertion_id": "member_of_assertion",
                            "association_type": "member_of",
                            "source_identity_rule_id": "person_by_id",
                            "source_endpoint_expression": "source.goid",
                            "target_identity_rule_id": "team_by_id",
                            "target_endpoint_expression": "identity(team_by_id)",
                            "source_role": "member",
                            "target_role": "team",
                            "valid_from_expression": "source.valid_from",
                            "valid_to_expression": "source.valid_to",
                            "cardinality_policy": "many_to_one",
                            "missing_policy": "reject"
                        }]
                    },
                    {
                        "rule_id": "team_row",
                        "source_id": "people",
                        "identity_rule_id": "team_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
    ])
}

fn alias_backed_association_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [
                    {
                        "source_id": "memberships",
                        "row_identity_rules": ["person_by_id"]
                    },
                    {
                        "source_id": "teams",
                        "row_identity_rules": ["team_by_name"]
                    }
                ]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [
                    {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                    {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                ]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "person_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "team_by_name",
                        "object_type": "Team",
                        "semantic_role": "organization",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "team",
                            "source_column": "team_name",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared",
                            "resolution": {
                                "resolver_id": "team_name_resolver"
                            }
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "membership_row",
                        "source_id": "memberships",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "association", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "association_bindings": [{
                            "assertion_id": "member_of_assertion",
                            "association_type": "member_of",
                            "source_identity_rule_id": "person_by_id",
                            "source_endpoint_expression": "source.goid",
                            "target_identity_rule_id": "team_by_name",
                            "target_endpoint_expression": "identity(team_by_name)",
                            "source_role": "member",
                            "target_role": "team",
                            "valid_from_expression": "source.valid_from",
                            "valid_to_expression": "source.valid_to",
                            "cardinality_policy": "many_to_one",
                            "missing_policy": "reject"
                        }]
                    },
                    {
                        "rule_id": "team_row",
                        "source_id": "teams",
                        "identity_rule_id": "team_by_name",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
        team_resolution_catalog_section("people-map", "test/v1"),
    ])
}

fn source_scoped_ambiguous_association_map() -> CovemapFile {
    test_covemap(vec![
        test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [
                    {
                        "source_id": "memberships",
                        "row_identity_rules": ["person_by_id"]
                    },
                    {
                        "source_id": "teams_a",
                        "row_identity_rules": ["team_by_id"]
                    },
                    {
                        "source_id": "teams_b",
                        "row_identity_rules": ["team_by_id"]
                    }
                ]
            }),
        ),
        test_section(
            SectionKind::MapFunctionRegistry,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "functions": [{
                    "function_id": "identity",
                    "version": "1",
                    "deterministic": true,
                    "dependency": "pure"
                }]
            }),
        ),
        test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [
                    {
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "person_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    },
                    {
                        "rule_id": "team_by_id",
                        "object_type": "Team",
                        "semantic_role": "organization",
                        "confidence_class": "source_scoped",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "team_id",
                            "source_column": "team_id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }
                ],
                "do_not_merge": []
            }),
        ),
        test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "membership_row",
                        "source_id": "memberships",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "association", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "association_bindings": [{
                            "assertion_id": "member_of_assertion",
                            "association_type": "member_of",
                            "source_identity_rule_id": "person_by_id",
                            "source_endpoint_expression": "source.goid",
                            "target_identity_rule_id": "team_by_id",
                            "target_endpoint_expression": "identity(team_by_id)",
                            "source_role": "member",
                            "target_role": "team",
                            "valid_from_expression": "source.valid_from",
                            "valid_to_expression": "source.valid_to",
                            "cardinality_policy": "many_to_one",
                            "missing_policy": "reject"
                        }]
                    },
                    {
                        "rule_id": "team_a_row",
                        "source_id": "teams_a",
                        "identity_rule_id": "team_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "team_b_row",
                        "source_id": "teams_b",
                        "identity_rule_id": "team_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        ),
    ])
}

fn governance_map(policy: &str) -> CovemapFile {
    let mut file = two_source_identity_map(Vec::new());
    file.sections[0] = test_section(
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "governance_reconciliation_policy": policy,
            "sources": [
                {
                    "source_id": "crm",
                    "row_identity_rules": ["person_by_id"],
                    "sensitivity_label": "public",
                    "sensitivity_rank": 1,
                    "access_policy_ids": ["internal"]
                },
                {
                    "source_id": "support",
                    "row_identity_rules": ["person_by_id"],
                    "sensitivity_label": "restricted",
                    "sensitivity_rank": 5,
                    "access_policy_ids": ["hipaa"]
                }
            ]
        }),
    );
    file
}

mod cli;
mod governance_conversion;
mod identity_resolution;
mod materialization_projection;
