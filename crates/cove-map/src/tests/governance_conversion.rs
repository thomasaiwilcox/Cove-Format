use super::*;

#[test]
fn governance_metadata_emits_effective_policy_by_default() {
    let file = governance_map("emit_effective_policy");
    let rows = vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("2"))]),
        },
    ];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let governance = &materialized.conversion_report["governance"];
    assert_eq!(governance["effective_sensitivity_rank"], json!(5));
    assert_eq!(
        governance["effective_sensitivity_labels"],
        json!(["restricted"])
    );
    assert_eq!(
        governance["access_policy_ids"],
        json!(["hipaa", "internal"])
    );
}

#[test]
fn governance_policy_rejects_mixed_sensitivity_when_requested() {
    let file = governance_map("reject_on_mixed_sensitivity");
    let rows = vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("2"))]),
        },
    ];
    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("mixed source sensitivity"));
}

#[test]
fn replay_claimed_source_validates_fingerprints() {
    let dir = std::env::temp_dir().join(format!("cove-map-replay-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("crm.csv");
    fs::write(&path, "id\n1\n").unwrap();
    let inputs = read_source_inputs(&[path]).unwrap();
    let state = &inputs.states[0];
    let mut file = two_source_identity_map(Vec::new());
    file.sections[0] = test_section(
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "sources": [{
                "source_id": "crm",
                "row_identity_rules": ["person_by_id"],
                "schema_fingerprint": state.schema_fingerprint,
                "snapshot_digest": state.snapshot_digest,
                "replay_claimed": true
            }]
        }),
    );
    validate_source_inputs(&file, &inputs.states).unwrap();
    file.sections[0] = test_section(
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "sources": [{
                "source_id": "crm",
                "row_identity_rules": ["person_by_id"],
                "schema_fingerprint": state.schema_fingerprint,
                "snapshot_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "replay_claimed": true
            }]
        }),
    );
    assert!(validate_source_inputs(&file, &inputs.states).is_err());
    assert!(validate_source_inputs(&file, &[]).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn build_cove_o_emits_valid_object_temporal_file() {
    fn section(kind: SectionKind, value: Value) -> CovemapSection {
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
    let file = CovemapFile {
        header: CovemapHeaderV1::new([0x42; 16], 0),
        mapping_version: "test/v1".into(),
        sections: vec![
            section(
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
            section(
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
            section(
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
            section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [{
                        "rule_id": "upsert_person",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "name_assertion",
                            "property_id": "name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8"
                        }]
                    }]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    let rows = vec![
        SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), json!("Ada"))]),
        },
        SourceRow {
            source_id: "people".into(),
            row_index: 1,
            values: BTreeMap::from([("id".into(), json!("2")), ("name".into(), json!("Linus"))]),
        },
    ];
    let bytes = build_cove_o(&file, &rows).unwrap();
    let report = validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        report.validated.header.required_features & FEATURE_SEMANTIC_MAP,
        0
    );
    assert_ne!(
        report.validated.header.optional_features & FEATURE_SEMANTIC_MAP,
        0
    );
    assert!(report
        .validated
        .footer
        .sections
        .iter()
        .filter(|entry| {
            matches!(
                SectionKind::from_u16(entry.section_kind),
                Some(
                    SectionKind::MapSourceCatalog
                        | SectionKind::MapFunctionRegistry
                        | SectionKind::MapIdentityRuleCatalog
                        | SectionKind::MapRowSemanticsCatalog
                        | SectionKind::MapAssertionLog
                        | SectionKind::MapIdentityEquivalenceIndex
                        | SectionKind::MapEvidenceIndex
                        | SectionKind::MapConversionReport
                )
            )
        })
        .all(|entry| entry.required_features & FEATURE_SEMANTIC_MAP == 0
            && entry.optional_features & FEATURE_SEMANTIC_MAP != 0));
    let kinds = report
        .validated
        .footer
        .sections
        .iter()
        .map(|entry| SectionKind::from_u16(entry.section_kind).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            SectionKind::MapSourceCatalog,
            SectionKind::MapFunctionRegistry,
            SectionKind::MapIdentityRuleCatalog,
            SectionKind::MapRowSemanticsCatalog,
            SectionKind::ObjectTypeCatalog,
            SectionKind::TemporalSegmentIndex,
            SectionKind::TemporalSegmentData,
            SectionKind::TrustManifest,
            SectionKind::MapAssertionLog,
            SectionKind::MapIdentityEquivalenceIndex,
            SectionKind::MapEvidenceIndex,
            SectionKind::MapConversionReport,
        ]
    );
    let segment_entry = report
        .validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::TemporalSegmentData as u16)
        .unwrap();
    let segment_bytes = compression::section_payload(&bytes, segment_entry).unwrap();
    let segment = TemporalSegmentData::parse(&segment_bytes).unwrap();
    assert_eq!(segment.header.column_count, 1);
    assert_eq!(segment.property_columns.len(), 1);
    assert_eq!(segment.property_columns[0].page_index.entries.len(), 1);

    let mut projected_file = file.clone();
    projected_file.sections.push(section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "people_names.v1",
                "output_table": "people_names",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "person_goid", "value": "object.goid"},
                    {"name": "name", "value": "Person.name"}
                ],
                "output_modes": ["json"]
            }]
        }),
    ));
    let projected = project_rows(&projected_file, &rows).unwrap();
    assert_eq!(projected["rows"].as_array().unwrap().len(), 2);
    assert_eq!(projected["rows"][0]["name"], json!("Ada"));
}

#[test]
fn cove_o_conversion_accepts_parquet_orc_and_arrow_ipc_sources() {
    let dir = std::env::temp_dir().join(format!(
        "cove-map-multi-source-ingest-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let map_path = dir.join("mapping.covemap");
    fs::write(&map_path, association_readback_map().serialize().unwrap()).unwrap();
    let batch = people_batch();
    let cases = [
        ("people.parquet", write_parquet(&batch)),
        ("people.orc", write_orc(&batch)),
        ("people.arrow", write_arrow_ipc(&batch)),
    ];
    for (file_name, bytes) in cases {
        let source_path = dir.join(file_name);
        fs::write(&source_path, bytes).unwrap();
        let cove_bytes = cove_o_from_paths(&map_path, std::slice::from_ref(&source_path)).unwrap();
        let surface = read_object_surface_from_bytes(&cove_bytes).unwrap();
        assert_eq!(surface.records.len(), 3, "{file_name}");
        assert_eq!(
            surface
                .records
                .iter()
                .filter(|record| record.association.is_some())
                .count(),
            1,
            "{file_name}"
        );
        fs::remove_file(&source_path).unwrap();
    }
    fs::remove_dir_all(&dir).unwrap();
}
