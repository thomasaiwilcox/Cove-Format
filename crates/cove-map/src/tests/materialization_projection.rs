use super::*;

#[test]
fn property_conflict_rejects_unequal_cross_source_values() {
    let file = two_source_property_map("reject_conflict", None, None);
    let rows = conflict_rows(json!("Ada"), json!("Ada Lovelace"));
    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("unresolved property conflict"));
}

#[test]
fn property_conflict_accepts_equal_duplicate_values() {
    let file = two_source_property_map("reject_conflict", None, None);
    let rows = conflict_rows(json!("Ada"), json!("Ada"));
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let name_values = materialized
        .rows
        .iter()
        .flat_map(|row| row.properties.values())
        .filter(|property| property.entry.property_name == "name")
        .map(|property| property.value.clone())
        .collect::<Vec<_>>();
    assert_eq!(name_values, vec![json!("Ada"), json!("Ada")]);
}

#[test]
fn null_property_candidate_does_not_overwrite_non_null_value() {
    let file = two_source_property_map("reject_conflict", None, None);
    let rows = conflict_rows(Value::Null, json!("Ada"));
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let name_values = materialized
        .rows
        .iter()
        .flat_map(|row| row.properties.values())
        .filter(|property| property.entry.property_name == "name")
        .map(|property| property.value.clone())
        .collect::<Vec<_>>();
    assert_eq!(name_values, vec![json!("Ada")]);
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry.get("suppressed_reason").and_then(Value::as_str)
            == Some("null_does_not_overwrite_non_null")
    }));
}

#[test]
fn source_priority_wins_suppresses_losing_property_values() {
    let file = two_source_property_map("source_priority_wins", Some(10), Some(1));
    let rows = conflict_rows(json!("CRM"), json!("Support"));
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let name_values = materialized
        .rows
        .iter()
        .flat_map(|row| row.properties.values())
        .filter(|property| property.entry.property_name == "name")
        .map(|property| property.value.clone())
        .collect::<Vec<_>>();
    assert_eq!(name_values, vec![json!("Support")]);
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry.get("suppressed_reason").and_then(Value::as_str) == Some("source_priority_wins")
            && entry.get("suppressed_value") == Some(&json!("CRM"))
    }));
}

#[test]
fn patch_operation_sets_delta_metadata_and_round_trips_evidence() {
    let mut file = two_source_property_map("reject_conflict", None, None);
    mutate_section_payload(&mut file, 3, |payload| {
        let rule = payload["rules"].as_array_mut().unwrap()[0]
            .as_object_mut()
            .unwrap();
        rule.insert("source_operation_kind".into(), json!("PatchProperty"));
    });
    let rows = vec![SourceRow {
        source_id: "crm".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("id".into(), json!("1")),
            ("name".into(), json!("Ada")),
            ("correction_of".into(), json!("crm:previous")),
            ("replacement_of".into(), json!("goid:previous")),
        ]),
    }];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(materialized.rows[0].record_kind, RecordKind::Delta);
    let evidence = materialized
        .evidence_entries
        .iter()
        .find(|entry| entry["rule_id"] == json!("crm_person"))
        .unwrap();
    assert_eq!(evidence["source_operation_kind"], json!("PatchProperty"));
    assert_eq!(evidence["operation_effect"], json!("patch_property"));
    assert_eq!(evidence["operation_target"], json!("property"));
    assert_eq!(evidence["correction_of"], json!("crm:previous"));
    assert_eq!(evidence["replacement_of"], json!("goid:previous"));
    assert_eq!(
        materialized.conversion_report["operation_counts"]["PatchProperty"],
        json!(1)
    );

    let bytes = build_cove_o(&file, &rows).unwrap();
    let surface = read_object_surface_from_bytes(&bytes).unwrap();
    let persisted = surface
        .evidence_index
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.rule_id == "crm_person")
        .unwrap();
    assert_eq!(
        persisted.operation_metadata["source_operation_kind"],
        json!("PatchProperty")
    );
    assert_eq!(
        persisted.operation_metadata["correction_of"],
        json!("crm:previous")
    );
}

#[test]
fn close_association_operation_marks_association_delta_and_policy_metadata() {
    let mut file = association_readback_map();
    mutate_section_payload(&mut file, 3, |payload| {
        let rule = payload["rules"].as_array_mut().unwrap()[0]
            .as_object_mut()
            .unwrap();
        rule.insert("source_operation_kind".into(), json!("CloseAssociation"));
    });
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
            ("closes_association".into(), json!("member_of:p1:t1")),
        ]),
    }];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let association = materialized
        .rows
        .iter()
        .find(|row| row.object_type == "Association:member_of")
        .unwrap();
    assert_eq!(association.record_kind, RecordKind::Delta);
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry["source_operation_kind"] == json!("CloseAssociation")
            && entry["operation_effect"] == json!("close_association")
            && entry["operation_target"] == json!("association")
            && entry["closes_association"] == json!("member_of:p1:t1")
    }));
}

#[test]
fn evidence_only_operation_emits_evidence_without_object_rows() {
    let mut file = two_source_identity_map(Vec::new());
    mutate_section_payload(&mut file, 3, |payload| {
        let rule = payload["rules"].as_array_mut().unwrap()[0]
            .as_object_mut()
            .unwrap();
        rule.insert("row_semantics_kind".into(), json!("EvidenceOnly"));
        rule.insert("source_operation_kind".into(), json!("RedactEvidence"));
        rule.insert("assertion_kinds".into(), json!(["evidence"]));
    });
    let rows = vec![SourceRow {
        source_id: "crm".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("id".into(), json!("1")),
            ("redaction_scope".into(), json!("source_evidence")),
        ]),
    }];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert!(materialized.rows.is_empty());
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry["source_operation_kind"] == json!("RedactEvidence")
            && entry["operation_effect"] == json!("redact_evidence")
            && entry["operation_target"] == json!("evidence")
            && entry["redaction_scope"] == json!("source_evidence")
    }));
}

#[test]
fn association_readback_preserves_roles_validity_and_cardinality() {
    let file = association_readback_map();
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let association = materialized
        .rows
        .iter()
        .find(|row| row.object_type == "Association:member_of")
        .unwrap();
    assert_eq!(
        property_by_name(association, "source_role"),
        json!("member")
    );
    assert_eq!(property_by_name(association, "target_role"), json!("team"));
    assert_eq!(
        property_by_name(association, "valid_from"),
        json!("2026-01-01")
    );
    assert_eq!(
        property_by_name(association, "valid_to"),
        json!("2026-12-31")
    );
    assert_eq!(
        property_by_name(association, "cardinality_policy"),
        json!("many_to_one")
    );
}

#[test]
fn association_endpoint_resolution_uses_alias_backed_target_identity() {
    let file = alias_backed_association_map();
    let rows = vec![
        SourceRow {
            source_id: "memberships".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_name".into(), json!("Alpha Team Ltd")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        },
        SourceRow {
            source_id: "teams".into(),
            row_index: 0,
            values: BTreeMap::from([("team_name".into(), json!("Team Alpha"))]),
        },
    ];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let team = materialized
        .rows
        .iter()
        .find(|row| row.object_type == "Team")
        .unwrap();
    let association = materialized
        .rows
        .iter()
        .find(|row| row.object_type == "Association:member_of")
        .unwrap();

    assert_eq!(
        property_by_name(association, "target_goid"),
        json!(hex_encode(&team.goid))
    );
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry["source_id"] == json!("teams")
            && entry["rule_id"] == json!("team_row")
            && entry["alias_hit"] == json!(true)
            && entry["canonical_key"] == json!("team:alpha")
    }));
}

#[test]
fn association_endpoint_rejects_source_scoped_join_key_ambiguity() {
    let file = source_scoped_ambiguous_association_map();
    let rows = vec![
        SourceRow {
            source_id: "memberships".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("team-1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        },
        SourceRow {
            source_id: "teams_a".into(),
            row_index: 0,
            values: BTreeMap::from([("team_id".into(), json!("team-1"))]),
        },
        SourceRow {
            source_id: "teams_b".into(),
            row_index: 0,
            values: BTreeMap::from([("team_id".into(), json!("team-1"))]),
        },
    ];

    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("ambiguous across 2 GOIDs"));
}

#[test]
fn cove_o_readback_decodes_association_surface_from_persisted_bytes() {
    let file = association_readback_map();
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];
    let bytes = build_cove_o(&file, &rows).unwrap();
    let surface = read_object_surface_from_bytes(&bytes).unwrap();
    let association_records = surface
        .records
        .iter()
        .filter(|record| record.association.is_some())
        .collect::<Vec<_>>();
    assert_eq!(surface.records.len(), 3);
    assert_eq!(association_records.len(), 1);

    let association = association_records[0];
    let metadata = association.association.as_ref().unwrap();
    assert_eq!(metadata.association_type.as_deref(), Some("member_of"));
    let source = association
        .properties
        .iter()
        .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_FROM_GOID != 0)
        .unwrap();
    let target = association
        .properties
        .iter()
        .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_TO_GOID != 0)
        .unwrap();
    let association_type = association
        .properties
        .iter()
        .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_TYPE != 0)
        .unwrap();
    let evidence = association
        .properties
        .iter()
        .find(|property| property.flags & PROPERTY_FLAG_EVIDENCE_REF != 0)
        .unwrap();
    assert_eq!(source.value.as_str().unwrap().len(), 32);
    assert_eq!(target.value.as_str().unwrap().len(), 32);
    assert_eq!(association_type.value, json!("member_of"));
    assert_eq!(evidence.value, json!("people:0"));
    assert_eq!(
        metadata.source_goid,
        source.value.as_str().map(str::to_string)
    );
    assert_eq!(
        metadata.target_goid,
        target.value.as_str().map(str::to_string)
    );
    assert_eq!(metadata.evidence_ref.as_deref(), Some("people:0"));
}

#[test]
fn project_cove_o_matches_source_projection_for_objects_associations_and_evidence() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [
                {
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "goid", "value": "object.goid"},
                        {"name": "object_type", "value": "object.type"}
                    ],
                    "output_modes": ["json", "cove-o"]
                },
                {
                    "projection_id": "member_links.v1",
                    "output_table": "member_links",
                    "row_grain": "one_row_per_association",
                    "anchor": {"association_type": "member_of"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "explode",
                    "columns": [
                        {"name": "source_goid", "value": "association.source_goid"},
                        {"name": "target_goid", "value": "association.target_goid"},
                        {"name": "association_type", "value": "association.association_type"},
                        {"name": "evidence_id", "value": "association.source_evidence_id"}
                    ],
                    "output_modes": ["json"]
                },
                {
                    "projection_id": "evidence_rows.v1",
                    "output_table": "evidence_rows",
                    "row_grain": "one_row_per_evidence_assertion",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "source_id", "value": "evidence.source_id"},
                        {"name": "rule_id", "value": "evidence.rule_id"},
                        {"name": "assertion_id", "value": "evidence.assertion_id"},
                        {"name": "output_object_id", "value": "evidence.output_object_id"}
                    ],
                    "output_modes": ["json"]
                }
            ]
        }),
    ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];
    let source_projected = project_rows(&file, &rows).unwrap();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "cove-map-project-cove-o-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let object_path = dir.join("object.cove");
    fs::write(&object_path, bytes).unwrap();
    let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
    assert_eq!(persisted_projected["rows"], source_projected["rows"]);
    assert_eq!(
        persisted_projected["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["projection_id"] == json!("member_links.v1"))
            .count(),
        1
    );
    assert!(persisted_projected["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["projection_id"] == json!("evidence_rows.v1")));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn source_projection_matches_persisted_projection_after_conflict_resolution() {
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
                    {"name": "person_goid", "value": "object.goid"},
                    {"name": "name", "value": "name", "logical_type": "utf8"}
                ],
                "output_modes": ["json", "cove-o"]
            }]
        }),
    ));
    let rows = conflict_rows(json!("CRM Name"), json!("Support Name"));
    let source_projected = project_rows(&file, &rows).unwrap();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "cove-map-project-conflict-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let object_path = dir.join("object.cove");
    fs::write(&object_path, bytes).unwrap();
    let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
    assert_eq!(persisted_projected["rows"], source_projected["rows"]);
    let rows = source_projected["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["projection_id"], json!("person_projection"));
    assert_eq!(rows[0]["output_table"], json!("people_projection"));
    assert_eq!(rows[0]["name"], json!("Support Name"));
    assert!(rows[0]["person_goid"].as_str().is_some());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_writes_object_report_manifest_readme_and_projection() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-success", &file);
    let out_dir = dir.join("bundle");
    let result = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
    assert_eq!(result.manifest["mapping_id"], json!("people-map"));
    assert_eq!(result.manifest["counts"]["object_count"], json!(1));
    let expected_evidence_count = result.manifest["counts"]["evidence_entry_count"]
        .as_u64()
        .unwrap() as usize;
    let object_path = out_dir.join("people_map.cove");
    let report_path = out_dir.join("map-build-report.json");
    let manifest_path = out_dir.join("map-build-manifest.json");
    let readme_path = out_dir.join("README.md");
    let index_path = out_dir.join("indexes/object_properties.covi");
    let projection_path = out_dir.join("projections/people_projection.cove");
    assert!(object_path.exists());
    assert!(report_path.exists());
    assert!(manifest_path.exists());
    assert!(readme_path.exists());
    assert!(index_path.exists());
    assert!(projection_path.exists());
    validate_bytes_with_options(
        &fs::read(&object_path).unwrap(),
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    let object_bytes = fs::read(&object_path).unwrap();
    let object_report = validate_bytes_with_options(
        &object_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    let evidence_entry = object_report
        .validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::MapEvidenceIndex as u16)
        .unwrap();
    let evidence_bytes = compression::section_payload(&object_bytes, evidence_entry).unwrap();
    assert!(is_compact_evidence_index_bytes(&evidence_bytes));
    let evidence_index = MapEvidenceIndex::parse(&evidence_bytes).unwrap();
    assert_eq!(evidence_index.entries.len(), expected_evidence_count);
    validate_bytes_with_options(
        &fs::read(&projection_path).unwrap(),
        ValidationOptions {
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    let index = cove_index::CoviArtifactV2::parse(&fs::read(&index_path).unwrap()).unwrap();
    assert!(index.header.index_root_count > 0);
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["evidence_encoding"], json!("compact"));
    assert_eq!(
        manifest["evidence"]["logical_entry_count"],
        json!(expected_evidence_count)
    );
    assert!(
        manifest["evidence"]["compact_binary_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        manifest["artifacts"]["indexes"][0]["target"],
        json!("cove-o-object-properties")
    );
    assert_eq!(manifest["section_compression"], json!("zstd"));
    assert_eq!(
        manifest["compression_summary"]["format"],
        json!("cove-map-section-compression-summary-v1")
    );
    assert_eq!(
        manifest["cache"]["key_material"]["section_compression"],
        json!("zstd")
    );
    assert_eq!(
        manifest["artifacts"]["projections"][0]["projection_id"],
        json!("person_projection")
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_compresses_large_sections_and_none_disables_it() {
    let file = build_projection_map();
    let dir = temp_build_dir("build-section-compression");
    let map = dir.join("people.covemap");
    fs::write(&map, file.serialize().unwrap()).unwrap();
    let crm = dir.join("crm.csv");
    let support = dir.join("support.csv");
    let mut crm_csv = String::from("id,name\n");
    let mut support_csv = String::from("id,name\n");
    let repeated = "same-overlap-payload-".repeat(16);
    for index in 0..256 {
        crm_csv.push_str(&format!("{index},CRM {repeated}{index}\n"));
        support_csv.push_str(&format!("{index},Support {repeated}{index}\n"));
    }
    fs::write(&crm, crm_csv).unwrap();
    fs::write(&support, support_csv).unwrap();
    let sources = vec![crm, support];

    let compressed_out = dir.join("compressed");
    let compressed =
        build_from_paths(&map, &sources, MapBuildOptions::new(&compressed_out)).unwrap();
    assert_eq!(compressed.manifest["section_compression"], json!("zstd"));
    assert!(
        compressed.manifest["compression_summary"]["compressed_section_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        compressed.manifest["compression_summary"]["saved_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let compressed_object_path = compressed_out.join("people_map.cove");
    let compressed_bytes = fs::read(&compressed_object_path).unwrap();
    let compressed_report = validate_bytes_with_options(
        &compressed_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    assert_ne!(
        compressed_report.validated.header.required_features & FEATURE_CODEC_ZSTD,
        0
    );
    assert!(compressed_report
        .validated
        .footer
        .sections
        .iter()
        .any(|entry| entry.compression == CompressionCodec::Zstd as u8));

    let uncompressed_out = dir.join("uncompressed");
    let mut uncompressed_options = MapBuildOptions::new(&uncompressed_out);
    uncompressed_options.section_compression = MapBuildSectionCompression::None;
    let uncompressed = build_from_paths(&map, &sources, uncompressed_options).unwrap();
    assert_eq!(uncompressed.manifest["section_compression"], json!("none"));
    assert_eq!(
        uncompressed.manifest["compression_summary"]["compressed_section_count"],
        json!(0)
    );
    let uncompressed_object_path = uncompressed_out.join("people_map.cove");
    let uncompressed_bytes = fs::read(&uncompressed_object_path).unwrap();
    let uncompressed_report = validate_bytes_with_options(
        &uncompressed_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    assert!(uncompressed_report
        .validated
        .footer
        .sections
        .iter()
        .all(|entry| entry.compression == CompressionCodec::None as u8));
    assert!(compressed_bytes.len() < uncompressed_bytes.len());

    let compressed_projection = project_cove_o_path(&compressed_object_path, None).unwrap();
    let uncompressed_projection = project_cove_o_path(&uncompressed_object_path, None).unwrap();
    assert_eq!(
        compressed_projection["rows"],
        uncompressed_projection["rows"]
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_can_emit_expanded_evidence_index_for_compatibility() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-expanded-evidence", &file);
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.evidence_encoding = MapEvidenceEncoding::Expanded;
    let result = build_from_paths(&map, &sources, options).unwrap();
    assert_eq!(result.manifest["evidence_encoding"], json!("expanded"));
    assert!(result.manifest["evidence"]["compact_binary_bytes"].is_null());
    let expected_evidence_count = result.manifest["counts"]["evidence_entry_count"]
        .as_u64()
        .unwrap() as usize;

    let object_path = out_dir.join("people_map.cove");
    let object_bytes = fs::read(&object_path).unwrap();
    let object_report = validate_bytes_with_options(
        &object_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    let evidence_entry = object_report
        .validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::MapEvidenceIndex as u16)
        .unwrap();
    let evidence_bytes = compression::section_payload(&object_bytes, evidence_entry).unwrap();
    assert!(!is_compact_evidence_index_bytes(&evidence_bytes));
    let evidence_index = MapEvidenceIndex::parse(&evidence_bytes).unwrap();
    assert_eq!(evidence_index.entries.len(), expected_evidence_count);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_verify_runs_doctor_and_writes_projection_lineage() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-verify", &file);
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.verify = true;
    let result = build_from_paths(&map, &sources, options).unwrap();
    assert_eq!(
        result.report["verification"]["format"],
        json!("cove-map-doctor-report-v1")
    );
    assert_eq!(result.report["verification"]["status"], json!("ok"));
    assert!(!report_has_failures(&result.report["verification"], false));

    let doctor = verify_bundle_dir(&out_dir).unwrap();
    assert_eq!(doctor["status"], json!("ok"));
    assert!(!report_has_failures(&doctor, false));
    assert_eq!(
        doctor["acceleration"]["projection_covi"]["available"],
        json!(true)
    );

    let projection_path = out_dir.join("projections/people_projection.cove");
    let projection_bytes = fs::read(projection_path).unwrap();
    let report = validate_bytes_with_options(
        &projection_bytes,
        ValidationOptions {
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .unwrap();
    let lineage: Value = serde_json::from_slice(&report.validated.footer.metadata_json).unwrap();
    assert_eq!(lineage["format"], json!("cove-map-projection-lineage-v1"));
    assert_eq!(lineage["mapping_id"], json!("people-map"));
    assert_eq!(lineage["mapping_version"], json!("test/v1"));
    assert_eq!(lineage["projection_id"], json!("person_projection"));
    assert_eq!(lineage["projection_version"], json!("test/v1"));
    assert_eq!(lineage["source_cove_o"]["path"], json!("people_map.cove"));
    assert!(lineage["source_cove_o"]["digest"].as_str().is_some());
    assert!(lineage["mapping_artifact_digest"].as_str().is_some());
    assert!(lineage["covm_manifest"].is_null());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn doctor_reports_invalid_bundle_artifacts() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("doctor-invalid", &file);
    let out_dir = dir.join("bundle");
    let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
    fs::write(
        out_dir.join("projections/people_projection.cove"),
        b"not cove",
    )
    .unwrap();
    fs::write(out_dir.join("indexes/object_properties.covi"), b"not covi").unwrap();

    let doctor = verify_bundle_dir(&out_dir).unwrap();
    assert!(report_has_failures(&doctor, false));
    assert!(doctor["errors"].as_array().unwrap().iter().any(|error| {
        error["code"] == json!("invalid_cove_t_projection")
            && error["projection_id"] == json!("person_projection")
    }));
    assert!(doctor["errors"].as_array().unwrap().iter().any(|error| {
        error["code"] == json!("invalid_covi_index")
            && error["index_id"] == json!("object_properties")
    }));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn doctor_reports_projection_covi_missing_or_invalid_readiness() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("doctor-projection-covi-readiness", &file);
    let out_dir = dir.join("bundle");
    let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();

    match fs::remove_file(out_dir.join("indexes/projection_columns.covi")) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("cannot remove projection_columns.covi: {err}"),
    }
    let doctor = verify_bundle_dir(&out_dir).unwrap();
    assert_eq!(
        doctor["acceleration"]["projection_covi"]["sidecar_status"],
        json!("missing")
    );
    assert!(doctor["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == json!("missing_projection_covi_sidecar")));

    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    let _ = build_from_paths(&map, &sources, options).unwrap();
    fs::write(out_dir.join("indexes/projection_columns.covi"), b"not covi").unwrap();
    let doctor = verify_bundle_dir(&out_dir).unwrap();
    assert_eq!(
        doctor["acceleration"]["projection_covi"]["sidecar_status"],
        json!("invalid")
    );
    assert!(doctor["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == json!("missing_projection_covi_sidecar")));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn doctor_strict_treats_skipped_projection_warning_as_failure() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("doctor-skipped-projection", &file);
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.projection_output = MapBuildProjectionOutput::None;
    let _ = build_from_paths(&map, &sources, options).unwrap();

    let doctor = verify_bundle_dir(&out_dir).unwrap();
    assert!(!report_has_failures(&doctor, false));
    assert!(report_has_failures(&doctor, true));
    assert!(doctor["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| {
            warning["code"] == json!("skipped_projection")
                && warning["details"]["projection_id"] == json!("person_projection")
        }));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn suggest_outputs_non_authoritative_identity_and_join_candidates() {
    let dir = temp_build_dir("suggest");
    let crm = dir.join("crm.csv");
    let support = dir.join("support.csv");
    fs::write(
        &crm,
        "customer_id,email,name\n1,a@example.com,Ada\n2,b@example.com,Bo\n",
    )
    .unwrap();
    fs::write(
        &support,
        "customer_id,email,ticket_count\n1,a@example.com,3\n3,c@example.com,1\n",
    )
    .unwrap();

    let suggestions = suggest_from_paths(&[crm, support]).unwrap();
    assert_eq!(suggestions["format"], json!("cove-map-suggestions-v1"));
    assert_eq!(suggestions["non_authoritative"], json!(true));
    assert!(suggestions["identity_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| {
            source["source_id"] == json!("crm")
                && source["candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|candidate| {
                        candidate["column"] == json!("customer_id")
                            && candidate["non_authoritative"] == json!(true)
                    })
        }));
    assert!(suggestions["join_key_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| {
            candidate["left_column"] == json!("customer_id")
                && candidate["right_column"] == json!("customer_id")
        }));
    assert!(suggestions["starter_projections"]
        .as_array()
        .unwrap()
        .iter()
        .any(|projection| projection["projection_id"] == json!("crm_starter.v1")));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parity_reports_matches_and_keyed_differences() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("parity", &file);
    let expected = dir.join("expected.csv");
    fs::write(&expected, "name\nSupport Name\n").unwrap();

    let options = ParityOptions {
        projection_id: "person_projection".into(),
        expected: expected.clone(),
        expected_query: None,
        key: vec!["name".into()],
    };
    let report = parity_from_paths(&map, &sources, &options).unwrap();
    assert_eq!(report["status"], json!("ok"));
    assert!(!parity_has_failures(&report));

    fs::write(&expected, "name\nWrong Name\n").unwrap();
    let options = ParityOptions {
        projection_id: "person_projection".into(),
        expected,
        expected_query: None,
        key: vec!["name".into()],
    };
    let report = parity_from_paths(&map, &sources, &options).unwrap();
    assert_eq!(report["status"], json!("mismatch"));
    assert_eq!(report["diff"]["missing_count"], json!(1));
    assert_eq!(report["diff"]["extra_count"], json!(1));
    assert!(parity_has_failures(&report));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parity_cove_o_supports_expected_query_and_unordered_warning() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("parity-cove-o", &file);
    let object = dir.join("object.cove");
    let bytes = cove_o_from_paths(&map, &sources).unwrap();
    fs::write(&object, bytes).unwrap();
    let expected = dir.join("expected.csv");
    fs::write(&expected, "name\nIgnored Name\nSupport Name\n").unwrap();

    let report = parity_from_cove_o_path(
        &object,
        &ParityOptions {
            projection_id: "person_projection".into(),
            expected,
            expected_query: Some(r#"where(name == "Support Name")"#.into()),
            key: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(report["status"], json!("ok"));
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| { warning["code"] == json!("ordered_comparison_without_key") }));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_collision_requires_force() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-collision", &file);
    let out_dir = dir.join("bundle");
    let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
    let err = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap_err();
    assert!(err.to_string().contains("--force"));
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    let _ = build_from_paths(&map, &sources, options).unwrap();
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_rejects_duplicate_projection_output_names() {
    let mut file = build_projection_map();
    mutate_section_payload(&mut file, 4, |value| {
        value["projections"] = json!([
            {
                "projection_id": "person_projection",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [{"name": "name", "value": "name", "logical_type": "utf8"}],
                "output_modes": ["json", "cove-t"]
            },
            {
                "projection_id": "person_projection_copy",
                "output_table": "people_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [{"name": "name", "value": "name", "logical_type": "utf8"}],
                "output_modes": ["json", "cove-t"]
            }
        ]);
    });
    let (map, sources, dir) = write_build_fixture("build-duplicate-projection", &file);
    let err =
        build_from_paths(&map, &sources, MapBuildOptions::new(dir.join("bundle"))).unwrap_err();
    assert!(err.to_string().contains("both map to output file"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_rejects_unsupported_source_extension_and_missing_source() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-source-errors", &file);
    let unsupported = dir.join("crm.txt");
    fs::write(&unsupported, "id,name\n1,Ada\n").unwrap();
    let err = build_from_paths(
        &map,
        &[unsupported, sources[1].clone()],
        MapBuildOptions::new(dir.join("unsupported")),
    )
    .unwrap_err();
    assert!(err.to_string().contains("must be .jsonl, .csv"));

    let err = build_from_paths(
        &map,
        &[sources[0].clone()],
        MapBuildOptions::new(dir.join("missing-source")),
    )
    .unwrap_err();
    assert!(err.to_string().contains("source 'support' is required"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn map_build_surfaces_projection_generation_errors() {
    let mut file = build_projection_map();
    mutate_section_payload(&mut file, 4, |value| {
        value["projections"][0]["row_grain"] = json!("unsupported_row_grain");
    });
    let (map, sources, dir) = write_build_fixture("build-projection-error", &file);
    let err =
        build_from_paths(&map, &sources, MapBuildOptions::new(dir.join("bundle"))).unwrap_err();
    assert!(err.to_string().contains("projection"), "err={err}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn projected_record_batches_from_cove_o_bytes_chunks_arrow_output() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_objects.v1",
                "output_table": "person_objects",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                    {"name": "object_type", "value": "object.type", "logical_type": "utf8"}
                ],
                "output_modes": ["arrow"]
            }]
        }),
    ));
    let rows = vec![
        SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        },
        SourceRow {
            source_id: "people".into(),
            row_index: 1,
            values: BTreeMap::from([
                ("person_id".into(), json!("p2")),
                ("team_id".into(), json!("t2")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        },
    ];
    let bytes = build_cove_o(&file, &rows).unwrap();
    let batches = projected_record_batches_from_cove_o_bytes(
        &bytes,
        None,
        "person_objects.v1",
        &ProjectionBatchOptions {
            batch_size: Some(1),
            ..ProjectionBatchOptions::default()
        },
    )
    .unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].num_rows(), 1);
    assert_eq!(batches[1].num_rows(), 1);
}

#[test]
fn projection_catalog_readback_enriches_direct_property_lineage() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let catalog = projection_catalog_from_cove_o_bytes(&bytes, None).unwrap();
    let projection = catalog
        .projections
        .iter()
        .find(|projection| projection.projection_id == "people_primitives.v1")
        .unwrap();
    let goid = projection
        .columns
        .iter()
        .find(|column| column.name == "goid")
        .unwrap();
    assert!(goid.lineage.is_none());
    let score = projection
        .columns
        .iter()
        .find(|column| column.name == "score")
        .unwrap();
    let lineage = score.lineage.as_ref().unwrap();
    assert_eq!(lineage.source, "object_property");
    assert_eq!(lineage.object_type_name, "Person");
    assert_eq!(lineage.property_name, "score");
    assert_eq!(lineage.projection_table_id, 1);
    assert_eq!(lineage.projection_column_id, 3);
    assert_eq!(lineage.filter_pushdown, "projection_covi_prefilter");
}

#[test]
fn projection_covi_filter_plan_reports_stable_reason_codes() {
    let descriptor = ProjectionDescriptor {
        projection_id: "people_primitives.v1".into(),
        output_table: Some("people_primitives".into()),
        output_modes: vec!["arrow".into()],
        columns: vec![
            ProjectionColumnDescriptor {
                name: "score".into(),
                logical_type: "int64".into(),
                nested_shape: None,
                lineage: Some(ProjectionColumnLineageDescriptor {
                    source: "object_property".into(),
                    object_type_id: 1,
                    object_type_name: "Person".into(),
                    property_id: 3,
                    property_name: "score".into(),
                    projection_table_id: 1,
                    projection_column_id: 3,
                    expression: "score".into(),
                    transform: "identity".into(),
                    filter_pushdown: "projection_covi_prefilter".into(),
                }),
            },
            ProjectionColumnDescriptor {
                name: "computed".into(),
                logical_type: "utf8".into(),
                nested_shape: None,
                lineage: None,
            },
        ],
    };
    let filters = vec![
        ProjectionFilter::Compare {
            column: "score".into(),
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Int64(10),
        },
        ProjectionFilter::InList {
            column: "score".into(),
            literals: vec![
                ProjectionFilterLiteral::Int64(10),
                ProjectionFilterLiteral::Int64(20),
            ],
        },
        ProjectionFilter::Compare {
            column: "score".into(),
            op: ProjectionFilterOp::GtEq,
            literal: ProjectionFilterLiteral::Int64(10),
        },
        ProjectionFilter::Compare {
            column: "score".into(),
            op: ProjectionFilterOp::Ne,
            literal: ProjectionFilterLiteral::Int64(10),
        },
        ProjectionFilter::IsNull {
            column: "score".into(),
            negated: false,
        },
        ProjectionFilter::Compare {
            column: "score".into(),
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Null,
        },
        ProjectionFilter::Compare {
            column: "computed".into(),
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Utf8("x".into()),
        },
        ProjectionFilter::Compare {
            column: "missing".into(),
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Utf8("x".into()),
        },
    ];
    let plan = projection_covi_filter_plan(&descriptor, &filters);
    assert_eq!(plan.lookups.len(), 3);
    let reasons = plan
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.reason.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        reasons,
        vec![
            "eligible",
            "eligible",
            "eligible",
            "not_equal",
            "is_null",
            "null_literal",
            "missing_lineage",
            "column_not_found",
        ]
    );
    assert!(plan.diagnostics[0].eligible);
    assert_eq!(plan.diagnostics[0].op, "eq");
    assert_eq!(plan.diagnostics[0].lineage_status, "present");
    assert_eq!(plan.diagnostics[0].projection_table_id, Some(1));
    assert_eq!(plan.unsupported_filters.len(), 5);
}

#[test]
fn projection_candidate_rows_prefilter_before_residual_filters() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let batches = projected_record_batches_from_cove_o_bytes(
        &bytes,
        None,
        "people_primitives.v1",
        &ProjectionBatchOptions {
            max_rows: None,
            output_columns: Some(vec!["score".into()]),
            pushed_filters: vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }],
            batch_size: None,
            candidate_projection_rows: Some(ProjectionCandidateRows::from_ordinals([0, 2])),
        },
    )
    .unwrap();
    assert_eq!(int64_column_values(&batches, "score"), vec![10, 30]);
}

#[test]
fn map_build_emits_projection_column_covi_sidecar() {
    let file = build_projection_map();
    let (map, sources, dir) = write_build_fixture("build-projection-column-covi", &file);
    let out = dir.join("bundle");
    let result = build_from_paths(&map, &sources, MapBuildOptions::new(&out)).unwrap();
    assert!(out
        .join("indexes")
        .join("projection_columns.covi")
        .is_file());
    let indexes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .unwrap();
    assert!(indexes.iter().any(|artifact| {
        artifact.get("path").and_then(Value::as_str) == Some("indexes/projection_columns.covi")
    }));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn projected_record_batches_filter_primitives_without_leaking_filter_columns() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let batches = projected_record_batches_from_cove_o_bytes(
        &bytes,
        None,
        "people_primitives.v1",
        &ProjectionBatchOptions {
            output_columns: Some(vec!["score".into()]),
            pushed_filters: vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }],
            ..ProjectionBatchOptions::default()
        },
    )
    .unwrap();

    assert_eq!(int64_column_values(&batches, "score"), vec![10, 30, 40]);
    for batch in batches {
        assert_eq!(batch.schema().fields().len(), 1);
        assert_eq!(batch.schema().field(0).name(), "score");
    }
}

#[test]
fn projected_record_batches_filter_primitives_match_ordered_fallback() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let options = ProjectionBatchOptions {
        output_columns: Some(vec!["score".into()]),
        pushed_filters: vec![ProjectionFilter::Compare {
            column: "active".into(),
            op: ProjectionFilterOp::Eq,
            literal: ProjectionFilterLiteral::Boolean(true),
        }],
        ..ProjectionBatchOptions::default()
    };
    let fast =
        projected_record_batches_from_cove_o_bytes(&bytes, None, "people_primitives.v1", &options)
            .unwrap();
    let fallback = projected_record_batches_from_cove_o_bytes(
        &bytes,
        None,
        "people_primitives_ordered.v1",
        &options,
    )
    .unwrap();

    assert_eq!(
        int64_column_values(&fast, "score"),
        int64_column_values(&fallback, "score")
    );
}

#[test]
fn projected_record_batches_filter_primitives_honor_limit_after_filtering() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let batches = projected_record_batches_from_cove_o_bytes(
        &bytes,
        None,
        "people_primitives.v1",
        &ProjectionBatchOptions {
            max_rows: Some(2),
            output_columns: Some(vec!["score".into()]),
            pushed_filters: vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }],
            batch_size: Some(1),
            candidate_projection_rows: None,
        },
    )
    .unwrap();

    assert_eq!(batches.len(), 2);
    assert_eq!(int64_column_values(&batches, "score"), vec![10, 30]);
}

#[test]
fn projected_record_batches_filter_primitives_cover_exact_ops_and_nulls() {
    let file = primitive_projection_map();
    let rows = primitive_projection_rows();
    let bytes = build_cove_o(&file, &rows).unwrap();
    let cases = [
        (
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::GtEq,
                literal: ProjectionFilterLiteral::Int64(30),
            },
            vec![30, 40],
        ),
        (
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::Lt,
                literal: ProjectionFilterLiteral::Float64(30.0),
            },
            vec![10, 20],
        ),
        (
            ProjectionFilter::Compare {
                column: "status".into(),
                op: ProjectionFilterOp::Ne,
                literal: ProjectionFilterLiteral::Utf8("closed".into()),
            },
            vec![10, 30],
        ),
        (
            ProjectionFilter::InList {
                column: "status".into(),
                literals: vec![ProjectionFilterLiteral::Utf8("open".into())],
            },
            vec![10, 30],
        ),
        (
            ProjectionFilter::IsNull {
                column: "nickname".into(),
                negated: false,
            },
            vec![20, 30],
        ),
    ];

    for (filter, expected) in cases {
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives.v1",
            &ProjectionBatchOptions {
                output_columns: Some(vec!["score".into()]),
                pushed_filters: vec![filter],
                ..ProjectionBatchOptions::default()
            },
        )
        .unwrap();
        assert_eq!(int64_column_values(&batches, "score"), expected);
    }
}

#[test]
fn projection_rejects_undeclared_runtime_function() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_objects.v1",
                "output_table": "person_objects",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "normalized_type", "value": "lower(object.type)"}
                ],
                "output_modes": ["json"]
            }]
        }),
    ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];

    let err = project_rows(&file, &rows).unwrap_err();
    assert!(err.contains("undeclared projection function 'lower'"));
}

#[test]
fn projection_rejects_undeclared_function_inside_predicate_argument() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "label", "value": "if(unknown(object.type) == \"Person\", object.type, \"Other\")"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];

    let err = project_rows(&file, &rows).unwrap_err();
    assert!(err.contains("undeclared projection function 'unknown'"));
}

#[test]
fn projection_rejects_aggregate_without_aggregate_policy() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_memberships.v1",
                "output_table": "person_memberships",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "reject",
                "columns": [
                    {"name": "membership_count", "value": "count(association(member_of))"}
                ],
                "output_modes": ["json"]
            }]
        }),
    ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];

    let err = project_rows(&file, &rows).unwrap_err();
    assert!(err.contains(
            "projection 'person_memberships.v1' aggregate 'count' requires multi_value_policy='aggregate'"
        ));
}

#[test]
fn projection_cove_o_output_materializes_projected_objects() {
    let mut file = association_readback_map();
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_objects.v1",
                "output_table": "person_objects",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "list",
                "columns": [
                    {"name": "goid", "value": "object.goid"},
                    {"name": "object_type", "value": "object.type"}
                ],
                "output_modes": ["json", "cove-o"]
            }]
        }),
    ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
        ]),
    }];
    let bytes = crate::project::project_rows_with_source_states_output(
        &file,
        &rows,
        &[],
        crate::project::ProjectionFormat::CoveO,
        Some("person_objects.v1"),
    )
    .unwrap();
    let surface = read_object_surface_from_bytes(&bytes).unwrap();
    assert_eq!(
        surface.projection_catalog.as_ref().unwrap().projections[0].projection_id,
        "person_objects.v1"
    );
    let projected = surface
        .records
        .iter()
        .find(|record| record.object_type_name == "person_objects")
        .unwrap();
    assert!(projected.properties.iter().any(
        |property| property.property_name == "object_type" && property.value == json!("Person")
    ));
}

#[test]
fn projection_cove_o_output_stores_nested_properties_as_filecodes() {
    let mut file = association_readback_map();
    mutate_section_payload(&mut file, 3, |payload| {
        let rule = payload["rules"].as_array_mut().unwrap()[0]
            .as_object_mut()
            .unwrap();
        rule.insert(
            "property_bindings".into(),
            json!([
                {
                    "assertion_id": "person_tags",
                    "property_id": "tags",
                    "property_name": "tags",
                    "source_column": "tags",
                    "logical_type": "list",
                    "physical_kind": "auto",
                    "nullable": true,
                    "missing_policy": "null",
                    "conflict_policy": "reject_conflict"
                },
                {
                    "assertion_id": "person_profile",
                    "property_id": "profile",
                    "property_name": "profile",
                    "source_column": "profile",
                    "logical_type": "struct",
                    "physical_kind": "auto",
                    "nullable": true,
                    "missing_policy": "null",
                    "conflict_policy": "reject_conflict"
                },
                {
                    "assertion_id": "person_scores",
                    "property_id": "scores",
                    "property_name": "scores",
                    "source_column": "scores",
                    "logical_type": "map",
                    "physical_kind": "auto",
                    "nullable": true,
                    "missing_policy": "null",
                    "conflict_policy": "reject_conflict"
                }
            ]),
        );
    });
    file.sections.push(test_section(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "projections": [{
                "projection_id": "person_nested.v1",
                "output_table": "person_nested",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Person"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "list",
                "columns": [
                    {
                        "name": "tags",
                        "value": "tags",
                        "logical_type": "list",
                        "nested_shape": {
                            "type": "list",
                            "item": {"logical_type": "utf8"}
                        }
                    },
                    {
                        "name": "profile",
                        "value": "profile",
                        "logical_type": "struct",
                        "nested_shape": {
                            "type": "struct",
                            "fields": [
                                {"name": "active", "logical_type": "bool"},
                                {"name": "level", "logical_type": "int64"}
                            ]
                        }
                    },
                    {
                        "name": "scores",
                        "value": "scores",
                        "logical_type": "map",
                        "nested_shape": {
                            "type": "map",
                            "key": {"logical_type": "utf8"},
                            "value": {"logical_type": "int64"}
                        }
                    }
                ],
                "output_modes": ["json", "cove-o"]
            }]
        }),
    ));
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("person_id".into(), json!("p1")),
            ("team_id".into(), json!("t1")),
            ("valid_from".into(), json!("2026-01-01")),
            ("valid_to".into(), json!("2026-12-31")),
            ("tags".into(), json!(["alpha", "beta"])),
            ("profile".into(), json!({"active": true, "level": 7})),
            ("scores".into(), json!({"logic": 100, "math": 99})),
        ]),
    }];
    let bytes = crate::project::project_rows_with_source_states_output(
        &file,
        &rows,
        &[],
        crate::project::ProjectionFormat::CoveO,
        Some("person_nested.v1"),
    )
    .unwrap();
    let report = validate_bytes_with_options(&bytes, ValidationOptions::default()).unwrap();
    assert!(report
        .validated
        .footer
        .sections
        .iter()
        .any(|entry| { entry.section_kind == SectionKind::FileDictionaryIndex as u16 }));
    let surface = read_object_surface_from_bytes(&bytes).unwrap();
    let object_type = surface
        .object_types
        .iter()
        .find(|object_type| object_type.type_name == "person_nested")
        .unwrap();
    for property_name in ["tags", "profile", "scores"] {
        let property = object_type
            .properties
            .iter()
            .find(|property| property.property_name == property_name)
            .unwrap();
        assert_eq!(property.physical_kind, CovePhysicalKind::FileCode);
    }
    assert_eq!(
        object_type
            .properties
            .iter()
            .find(|property| property.property_name == "tags")
            .unwrap()
            .logical_type,
        CoveLogicalType::List
    );
    assert_eq!(
        object_type
            .properties
            .iter()
            .find(|property| property.property_name == "profile")
            .unwrap()
            .logical_type,
        CoveLogicalType::Struct
    );
    assert_eq!(
        object_type
            .properties
            .iter()
            .find(|property| property.property_name == "scores")
            .unwrap()
            .logical_type,
        CoveLogicalType::Map
    );
    let projected = surface
        .records
        .iter()
        .find(|record| record.object_type_name == "person_nested")
        .unwrap();
    let projected_property = |name: &str| {
        projected
            .properties
            .iter()
            .find(|property| property.property_name == name)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(projected_property("tags"), json!(["alpha", "beta"]));
    assert_eq!(
        projected_property("profile"),
        json!({"active": true, "level": 7})
    );
    assert_eq!(
        projected_property("scores"),
        json!({"logic": 100, "math": 99})
    );
}
