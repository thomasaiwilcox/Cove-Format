use super::*;

#[test]
fn join_key_is_deterministic() {
    let components = [
        JoinKeyComponent {
            role_id: "email",
            logical_type_id: "utf8",
            value: Some(b"a@example.com"),
        },
        JoinKeyComponent {
            role_id: "tenant",
            logical_type_id: "utf8",
            value: Some(b"t1"),
        },
    ];
    assert_eq!(
        join_key_tuple(1, "person_by_email", &components),
        join_key_tuple(1, "person_by_email", &components)
    );
}

#[test]
fn join_key_distinguishes_null_from_empty_value() {
    let null_component = [JoinKeyComponent {
        role_id: "email",
        logical_type_id: "utf8",
        value: None,
    }];
    let empty_component = [JoinKeyComponent {
        role_id: "email",
        logical_type_id: "utf8",
        value: Some(b""),
    }];
    assert_ne!(
        join_key_tuple(1, "person_by_email", &null_component),
        join_key_tuple(1, "person_by_email", &empty_component)
    );
}

#[test]
fn unicode_casefold_uses_full_unicode_mapping() {
    let folded = apply_canonicalization(
        &json!("Straße"),
        "unicode_casefold",
        &["unicode_casefold".to_string()],
    )
    .unwrap();
    assert_eq!(folded, json!("strasse"));
}

#[test]
fn goid_is_sha256_truncated_to_16_bytes() {
    let goid = goid16_parts(&[b"map", b"v1", b"person", b"rule", b"key"]);
    assert_eq!(goid.len(), 16);
    assert_eq!(
        goid,
        goid16_parts(&[b"map", b"v1", b"person", b"rule", b"key"])
    );
}

#[test]
fn csv_reader_is_deterministic_for_simple_rows() {
    let dir = std::env::temp_dir().join(format!("cove-map-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("people.csv");
    fs::write(&path, "id,name\n1,Ada\n2,Linus\n").unwrap();
    let rows = read_csv(&path, "people").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].values["id"], json!("1"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cross_source_authoritative_identity_merges_to_one_goid() {
    let file = two_source_identity_map(Vec::new());
    let rows = vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
    ];
    let planned = plan_identities(&file, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);
    let index = identity_equivalence_index("people-map", "test/v1", &planned.canonical);
    assert_eq!(index["equivalences"].as_array().unwrap().len(), 0);
    assert_eq!(index["components"].as_array().unwrap().len(), 1);
    assert_eq!(
        index["components"][0]["members"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn alias_catalog_resolver_merges_alias_hits_and_emits_evidence() {
    let file = company_resolution_map();
    file.validate_map_sections().unwrap();
    let rows = ["Tesco", "Tesco PLC", "tesco supermarket"]
        .into_iter()
        .enumerate()
        .map(|(row_index, company_name)| SourceRow {
            source_id: "suppliers".into(),
            row_index,
            values: BTreeMap::from([("company_name".into(), json!(company_name))]),
        })
        .collect::<Vec<_>>();

    let planned = plan_identities(&file, &rows).unwrap();
    assert_eq!(planned.candidates.len(), 0);
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);
    assert!(planned.canonical.iter().all(|identity| {
        identity.resolution_metadata[0].canonical_key.as_deref() == Some("uk-company:tesco")
            && identity.resolution_metadata[0].alias_hit
    }));

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(materialized.rows.len(), 3);
    assert_eq!(
        materialized.conversion_report["candidate_match_count"],
        json!(0)
    );
    assert_eq!(
        materialized.conversion_report["resolver_hit_count"],
        json!(3)
    );
    assert_eq!(
        materialized.conversion_report["resolver_miss_count"],
        json!(0)
    );
    let impact = materialized.conversion_report["resolver_goid_impact"]
        .as_array()
        .unwrap();
    assert_eq!(impact.len(), 1);
    assert_eq!(
        impact[0]["normalization_pipeline_id"],
        json!("company_name.v1")
    );
    assert_eq!(impact[0]["affected_goid_count"], json!(1));
    assert_eq!(impact[0]["affected_goids"].as_array().unwrap().len(), 1);
    assert!(materialized.evidence_entries.iter().all(|entry| {
        entry["resolver_id"] == json!("uk_company_name_resolver")
            && entry["canonical_key"] == json!("uk-company:tesco")
            && entry["canonical_label"] == json!("Tesco")
            && entry["alias_hit"] == json!(true)
    }));
}

#[test]
fn resolver_backed_identity_rule_rejects_object_type_mismatch_at_runtime() {
    let mut file = company_resolution_map();
    mutate_section_payload(&mut file, 2, |payload| {
        payload["identity_rules"][0]["object_type"] = json!("Person");
    });
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
    }];

    let err = plan_identities(&file, &rows).unwrap_err();
    assert!(err.contains("resolver 'uk_company_name_resolver' targets object type 'Company'"));
}

#[test]
fn replay_verify_accepts_current_report_and_rejects_stale_resolver() {
    let file = company_resolution_map();
    let rows = ["Tesco", "Tesco PLC"]
        .into_iter()
        .enumerate()
        .map(|(row_index, company_name)| SourceRow {
            source_id: "suppliers".into(),
            row_index,
            values: BTreeMap::from([("company_name".into(), json!(company_name))]),
        })
        .collect::<Vec<_>>();
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();

    let report = verify_replay_report(&file, &materialized.conversion_report).unwrap();
    assert_eq!(report["ok"], json!(true));
    assert_eq!(report["resolver_catalog_digest_count"], json!(1));

    let mut stale = materialized.conversion_report.clone();
    stale["resolver_catalog_digests"][0]["resolver_digest"] =
        json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let err = verify_replay_report(&file, &stale).unwrap_err();
    assert!(err.contains("MAP_REPLAY_STALE_RESOLVER"));
}

#[test]
fn replay_verify_rejects_stale_source_binding() {
    let state = ObservedSourceState {
        source_id: "crm".into(),
        source_kind: "csv".into(),
        schema_fingerprint:
            "cove-map-schema-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
        snapshot_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
    };
    let mut file = two_source_identity_map(Vec::new());
    file.sections[0] = test_section(
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "sources": [
                {
                    "source_id": "crm",
                    "row_identity_rules": ["person_by_id"],
                    "schema_fingerprint": state.schema_fingerprint.clone(),
                    "snapshot_digest": state.snapshot_digest.clone(),
                    "replay_claimed": true
                },
                {"source_id": "support", "row_identity_rules": ["person_by_id"]}
            ]
        }),
    );
    let rows = vec![SourceRow {
        source_id: "crm".into(),
        row_index: 0,
        values: BTreeMap::from([("id".into(), json!("1"))]),
    }];
    let materialized =
        materialize_with_source_states(&file, &rows, std::slice::from_ref(&state)).unwrap();
    verify_replay_report(&file, &materialized.conversion_report).unwrap();

    let mut stale = materialized.conversion_report.clone();
    stale["sources"][0]["snapshot_digest"] =
        json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let err = verify_replay_report(&file, &stale).unwrap_err();
    assert!(err.contains("MAP_REPLAY_SOURCE_STALE"));
}

#[test]
fn aliases_import_updates_catalog_digests_and_runtime_lookup() {
    let file = company_resolution_map();
    let csv = br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:acme,Acme,Acme Ltd,curated,authoritative,{"source":"manual"}
uk-company:acme,Acme,ACME LIMITED,curated,authoritative,
"#;
    let options = alias_import::AliasImportOptions {
        catalog_id: "company_aliases".into(),
        resolver_id: "uk_company_name_resolver".into(),
    };
    let (updated, report) =
        alias_import::import_aliases_from_csv_bytes(&file, csv, &options).unwrap();
    assert_eq!(report["alias_entry_count"], json!(1));
    assert_eq!(report["alias_count"], json!(2));
    assert!(report["catalog_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(report["resolver_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    updated.validate_map_sections().unwrap();
    let serialized = updated.serialize().unwrap();
    CovemapFile::parse_validated(&serialized).unwrap();

    let rows = ["Acme Ltd", "ACME LIMITED"]
        .into_iter()
        .enumerate()
        .map(|(row_index, company_name)| SourceRow {
            source_id: "suppliers".into(),
            row_index,
            values: BTreeMap::from([("company_name".into(), json!(company_name))]),
        })
        .collect::<Vec<_>>();
    let planned = plan_identities(&updated, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);
    assert!(planned.canonical.iter().all(|identity| {
        identity.resolution_metadata[0].canonical_key.as_deref() == Some("uk-company:acme")
            && identity.resolution_metadata[0].alias_hit
    }));
}

#[test]
fn redacted_resolver_evidence_omits_raw_alias_but_preserves_hit_proof() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() =
        redacted_resolution_catalog_section("company-map", "test/v1");
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
    }];

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    let entry = materialized
        .evidence_entries
        .iter()
        .find(|entry| entry["alias_hit"] == json!(true))
        .unwrap();
    assert_eq!(entry["evidence_policy"], json!("redact_raw"));
    assert_eq!(entry["redacted_resolution_evidence"], json!(true));
    assert_eq!(entry["redacted"], json!(true));
    assert_eq!(entry["redaction_scope"], json!("resolver_evidence"));
    assert!(entry.get("raw_observed_value").is_none());
    assert!(entry.get("normalized_value").is_none());
    assert_eq!(entry["canonical_key"], json!("uk-company:tesco"));
    assert!(entry["resolver_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(entry["catalog_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(entry["pipeline_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn alias_catalog_candidate_only_miss_emits_candidate_without_goid() {
    let file = company_resolution_map();
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Unknown Stores"))]),
    }];

    let planned = plan_identities(&file, &rows).unwrap();
    assert!(planned.canonical.is_empty());
    assert_eq!(planned.candidates.len(), 1);
    assert_eq!(
        planned.candidates[0].resolution_metadata[0].normalized_value,
        "unknown stores"
    );
    assert!(planned.candidates[0].resolution_metadata[0].alias_miss);

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert!(materialized.rows.is_empty());
    assert_eq!(
        materialized.conversion_report["resolver_hit_count"],
        json!(0)
    );
    assert_eq!(
        materialized.conversion_report["resolver_miss_count"],
        json!(1)
    );
    assert_eq!(
        materialized.conversion_report["candidate_match_count"],
        json!(1)
    );
    assert_eq!(materialized.evidence_entries.len(), 1);
    assert_eq!(materialized.evidence_entries[0]["candidate"], json!(true));
    assert_eq!(materialized.evidence_entries[0]["alias_miss"], json!(true));
    assert_eq!(
        materialized.evidence_entries[0]["miss_policy"],
        json!("candidate_only")
    );
}

#[test]
fn redacted_alias_miss_error_omits_raw_alias_value() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() = company_resolution_catalog_section_with_policy(
        "company-map",
        "test/v1",
        "reject",
        None,
        "redact_raw",
    );
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Protected Unknown Stores"))]),
    }];

    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("MAP_ALIAS_MISS"));
    assert!(err.contains("<redacted>"));
    assert!(!err.contains("Protected Unknown Stores"));
    assert!(!err.contains("protected unknown stores"));
}

#[test]
fn alias_catalog_ambiguous_hit_rejects_auto_merge_by_default() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() =
        ambiguous_company_resolution_catalog_section("company-map", "test/v1", "reject_auto_merge");
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
    }];

    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("MAP_ALIAS_AMBIGUOUS"));
}

#[test]
fn redacted_ambiguous_alias_error_omits_normalized_alias_value() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() = ambiguous_company_resolution_catalog_section_with_policy(
        "company-map",
        "test/v1",
        "reject_auto_merge",
        "redact_raw",
    );
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
    }];

    let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
    assert!(err.contains("MAP_ALIAS_AMBIGUOUS"));
    assert!(err.contains("<redacted>"));
    assert!(!err.contains("Tesco"));
    assert!(!err.contains("tesco"));
}

#[test]
fn alias_catalog_ambiguous_hit_can_route_to_candidate_only() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() =
        ambiguous_company_resolution_catalog_section("company-map", "test/v1", "candidate_only");
    file.validate_map_sections().unwrap();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
    }];

    let planned = plan_identities(&file, &rows).unwrap();
    assert!(planned.canonical.is_empty());
    assert_eq!(planned.candidates.len(), 1);
    assert!(planned.candidates[0].resolution_metadata[0].alias_ambiguous);

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert!(materialized.rows.is_empty());
    assert_eq!(
        materialized.conversion_report["ambiguous_alias_count"],
        json!(1)
    );
    assert_eq!(materialized.evidence_entries[0]["candidate"], json!(true));
    assert_eq!(
        materialized.evidence_entries[0]["alias_ambiguous"],
        json!(true)
    );
}

#[test]
fn resolution_property_expressions_project_from_identity_evidence() {
    let mut file = company_resolution_map();
    file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"},
                        {"name": "canonical_label", "value": "identity(company_by_resolved_name).resolution(company).canonical_label"},
                        {"name": "normalized_value", "value": "identity(company_by_resolved_name).resolution(company).normalized_value"},
                        {"name": "raw_observed_value", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
    }];

    let projected = project_rows(&file, &rows).unwrap();
    let projected_row = &projected["rows"][0];
    assert_eq!(projected_row["canonical_key"], json!("uk-company:tesco"));
    assert_eq!(projected_row["canonical_label"], json!("Tesco"));
    assert_eq!(projected_row["normalized_value"], json!("tesco plc"));
    assert_eq!(projected_row["raw_observed_value"], json!("Tesco PLC"));

    let bytes = build_cove_o(&file, &rows).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "cove-map-resolution-projection-test-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let object_path = dir.join("company.cove");
    fs::write(&object_path, bytes).unwrap();
    let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
    assert_eq!(persisted_projected["rows"], projected["rows"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resolution_property_expressions_select_declared_role() {
    let mut file = company_resolution_map();
    mutate_section_payload(&mut file, 2, |payload| {
        payload["identity_rules"][0]["join_keys"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "role_id": "parent_company",
                "source_column": "parent_company_name",
                "logical_type": "utf8",
                "canonicalization": "identity",
                "null_policy": "reject",
                "ordering": "declared",
                "resolution": {
                    "resolver_id": "uk_company_name_resolver"
                }
            }));
    });
    file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "company_raw", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"},
                        {"name": "parent_raw", "value": "identity(company_by_resolved_name).resolution(parent_company).raw_observed_value"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("company_name".into(), json!("Tesco")),
            ("parent_company_name".into(), json!("Tesco PLC")),
        ]),
    }];

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(
        materialized.evidence_entries[0]["resolution_metadata"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let projected = project_rows(&file, &rows).unwrap();
    let projected_row = &projected["rows"][0];
    assert_eq!(projected_row["company_raw"], json!("Tesco"));
    assert_eq!(projected_row["parent_raw"], json!("Tesco PLC"));
}

#[test]
fn resolution_property_expressions_fail_closed_without_resolver_hit() {
    let mut file = company_resolution_map();
    *file.sections.last_mut().unwrap() =
        normalized_miss_resolution_catalog_section("company-map", "test/v1");
    file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Unknown Stores"))]),
    }];

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(materialized.rows.len(), 1);
    assert_eq!(materialized.evidence_entries[0]["alias_miss"], json!(true));
    assert!(materialized.evidence_entries[0]["alias_hit"].is_null());

    let err = project_rows(&file, &rows).unwrap_err();
    assert!(err.contains("found no resolver hit"));
}

fn add_company_candidate_match_rule(file: &mut CovemapFile, max_pairs_per_block: u64) {
    mutate_section_payload(file, 4, |payload| {
        payload["match_rules"].as_array_mut().unwrap().push(json!({
            "match_rule_id": "company_name_similarity",
            "object_type": "Company",
            "inputs": [{
                "source_id": "suppliers",
                "column": "company_name"
            }],
            "blocking": {
                "kind": "normalized_prefix",
                "length": 4
            },
            "normalization_pipeline_id": "company_name.v1",
            "scoring": {
                "kind": "token_jaccard",
                "candidate_threshold": 0.3,
                "merge_behavior": "never",
                "score_scale": 1000000,
                "rounding": "floor"
            },
            "limits": {
                "max_pairs_per_block": max_pairs_per_block,
                "max_pairs_total": 100,
                "on_limit": "fail_closed"
            },
            "output": {
                "assertion_kinds": ["candidate_match", "evidence"]
            }
        }));
    });
}

#[test]
fn candidate_match_rule_emits_stable_token_jaccard_json() {
    let mut file = company_resolution_map();
    add_company_candidate_match_rule(&mut file, 10);
    file.validate_map_sections().unwrap();
    let rows = vec![
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        },
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 1,
            values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
        },
    ];

    let candidates = candidate_matches(&file, &rows).unwrap();
    let matches = candidates["candidate_matches"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0]["match_rule_id"],
        json!("company_name_similarity")
    );
    assert_eq!(matches[0]["candidate_score"], json!(333333));
    assert_eq!(matches[0]["score_scale"], json!(1000000));
    assert_eq!(matches[0]["blocking_key"], json!("tesc"));
    assert_eq!(matches[0]["merge_behavior"], json!("never"));
    assert_eq!(matches[0]["left"]["normalized_value"], json!("tesco plc"));
    assert_eq!(
        matches[0]["right"]["normalized_value"],
        json!("tesco supermarket")
    );

    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(materialized.rows.len(), 2);
    assert_eq!(
        materialized.conversion_report["candidate_match_count"],
        json!(1)
    );
    assert_eq!(
        materialized.conversion_report["candidate_matches"][0]["match_rule_id"],
        json!("company_name_similarity")
    );
    assert!(materialized.evidence_entries.iter().any(|entry| {
        entry["candidate_match_id"] == matches[0]["candidate_match_id"]
            && entry["match_rule_id"] == json!("company_name_similarity")
            && entry["candidate_score"] == json!(333333)
            && entry["left_normalized_value"] == json!("tesco plc")
            && entry["right_normalized_value"] == json!("tesco supermarket")
    }));
}

#[test]
fn review_worklist_from_candidate_matches_emits_decision_templates() {
    let mut file = company_resolution_map();
    add_company_candidate_match_rule(&mut file, 10);
    let rows = vec![
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        },
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 1,
            values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
        },
    ];

    let candidates = candidate_matches(&file, &rows).unwrap();
    let review = review_worklist_from_candidate_matches(&candidates).unwrap();
    assert_eq!(
        review["schema_id"],
        json!("org.coveformat.covemap.review-worklist.v1")
    );
    assert_eq!(review["candidate_match_count"], json!(1));
    assert_eq!(
        review["review_items"][0]["same_object_decision_template"]["left"]["kind"],
        json!("row_digest")
    );
    assert_eq!(
        review["review_items"][0]["same_object_decision_template"]["left"]["source_id"],
        json!("suppliers")
    );
    assert_eq!(
        review["review_items"][0]["same_object_decision_template"]["left"]["source_row_identity"],
        json!("suppliers:0")
    );
    assert_eq!(
        review["review_items"][0]["do_not_merge_decision_template"]["decision"],
        json!("do_not_merge")
    );
    assert_eq!(
        review["review_items"][0]["left"]["normalized_value"],
        json!("tesco plc")
    );
}

#[test]
fn candidate_match_rule_limits_fail_closed() {
    let mut file = company_resolution_map();
    add_company_candidate_match_rule(&mut file, 0);
    let rows = vec![
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        },
        SourceRow {
            source_id: "suppliers".into(),
            row_index: 1,
            values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
        },
    ];

    let err = candidate_matches(&file, &rows).unwrap_err();
    assert!(err.contains("max_pairs_per_block"));
}

#[test]
fn explain_includes_resolution_metadata_from_evidence_index() {
    let mut file = company_resolution_map();
    let rows = vec![SourceRow {
        source_id: "suppliers".into(),
        row_index: 0,
        values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
    }];
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    file.sections.push(test_section(
        SectionKind::MapEvidenceIndex,
        materialized.evidence_index.clone(),
    ));

    let goid = hex_encode(&materialized.rows[0].goid);
    let explained = explain(&file, &goid).unwrap();
    assert_eq!(
        explained["operation_metadata"]["identity_rule_id"],
        json!("company_by_resolved_name")
    );
    assert_eq!(
        explained["resolution"]["resolver_id"],
        json!("uk_company_name_resolver")
    );
    assert_eq!(
        explained["resolution"]["resolution_role_id"],
        json!("company")
    );
    assert_eq!(
        explained["resolution"]["raw_observed_value"],
        json!("Tesco PLC")
    );
    assert_eq!(
        explained["resolution"]["canonical_key"],
        json!("uk-company:tesco")
    );
}

#[test]
fn candidate_identity_rules_emit_evidence_without_goids() {
    let mut file = two_source_identity_map(Vec::new());
    file.sections[2] = test_section(
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "identity_rules": [{
                "rule_id": "person_by_id",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "candidate",
                "candidate_only": true,
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
    );
    file.sections[3] = test_section(
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "rules": [
                {
                    "rule_id": "crm_candidate_person",
                    "source_id": "crm",
                    "identity_rule_id": "person_by_id",
                    "row_semantics_kind": "EvidenceOnly",
                    "assertion_kinds": ["candidate_match", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": []
                },
                {
                    "rule_id": "support_candidate_person",
                    "source_id": "support",
                    "identity_rule_id": "person_by_id",
                    "row_semantics_kind": "EvidenceOnly",
                    "assertion_kinds": ["candidate_match", "evidence"],
                    "function_ids": ["identity"],
                    "output_assertion_ids": [],
                    "association_endpoints": []
                }
            ]
        }),
    );
    let rows = vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
    ];
    let plan = plan_identities(&file, &rows).unwrap();
    assert!(plan.canonical.is_empty());
    assert_eq!(plan.candidates.len(), 2);
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert!(materialized.rows.is_empty());
    assert_eq!(
        materialized.conversion_report["candidate_match_count"],
        json!(2)
    );
    assert_eq!(
        materialized.identity_equivalence_index["equivalences"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(materialized
        .evidence_entries
        .iter()
        .all(|entry| entry["candidate"] == json!(true)));
}

#[test]
fn do_not_merge_conflict_rejects_identity_resolution() {
    let file = two_source_identity_map(vec![json!({
        "left_identity": "crm:0",
        "right_identity": "support:0"
    })]);
    let rows = vec![
        SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
        SourceRow {
            source_id: "support".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        },
    ];
    assert!(plan_identities(&file, &rows).is_err());
}

#[test]
fn reviewed_same_object_merges_only_when_identity_rule_allows_it() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    add_reviewed_decisions(
        &mut file,
        vec![reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        )],
    );

    let planned = plan_identities(&file, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);

    let mut disallowed = two_source_identity_map(Vec::new());
    add_reviewed_decisions(
        &mut disallowed,
        vec![reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        )],
    );
    let err = plan_identities(&disallowed, &rows).unwrap_err();
    assert!(err.contains("does not allow reviewed equivalence"));
}

#[test]
fn reviewed_row_digest_reference_rejects_ambiguous_matches() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "1")]);
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    let reference = json!({
        "kind": "row_digest",
        "object_type": "Person",
        "row_digest": row_digest(&rows[0])
    });
    add_reviewed_decisions(
        &mut file,
        vec![reviewed_same_object_decision(
            reference.clone(),
            reference,
            None,
        )],
    );

    let err = plan_identities(&file, &rows).unwrap_err();
    assert!(err.contains("row_digest reference matched"));
}

#[test]
fn review_import_creates_resolution_catalog_and_merges_decisions() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    let decision = reviewed_same_object_decision(
        identity_alias_ref("Person", "crm:0"),
        identity_alias_ref("Person", "support:0"),
        None,
    );
    let review = serde_json::to_vec(&json!({
        "schema_id": "org.coveformat.covemap.review-worklist.v1",
        "mapping_id": "people-map",
        "mapping_version": "test/v1",
        "reviewed_decisions": [decision.clone()]
    }))
    .unwrap();

    let (updated, report) = review::import_reviewed_decisions_from_bytes(
        &file,
        &review,
        &review::ReviewImportOptions { replace: false },
    )
    .unwrap();
    assert_eq!(report["existing_reviewed_decision_count"], json!(0));
    assert_eq!(report["imported_reviewed_decision_count"], json!(1));
    assert_eq!(report["reviewed_decision_count"], json!(1));
    updated.validate_map_sections().unwrap();
    let serialized = updated.serialize().unwrap();
    CovemapFile::parse_validated(&serialized).unwrap();
    let exported = review::export_reviewed_decisions(&updated).unwrap();
    assert_eq!(
        exported["schema_id"],
        json!("org.coveformat.covemap.review-worklist.v1")
    );
    assert_eq!(exported["mapping_id"], json!("people-map"));
    assert_eq!(exported["mapping_version"], json!("test/v1"));
    assert_eq!(exported["reviewed_decision_count"], json!(1));
    assert_eq!(exported["reviewed_decisions"], json!([decision]));

    let planned = plan_identities(&updated, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);
}

#[test]
fn reviewed_decision_catalog_digest_binds_conversion_report() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let decision = reviewed_same_object_decision(
        identity_alias_ref("Person", "crm:0"),
        identity_alias_ref("Person", "support:0"),
        None,
    );

    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    add_reviewed_decisions(&mut file, vec![decision.clone()]);
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    assert_eq!(
        materialized.conversion_report["reviewed_decision_count"],
        json!(1)
    );
    let original_digest = materialized.conversion_report["reviewed_decision_catalog_digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(original_digest.starts_with("sha256:"));

    let mut changed_decision = decision;
    changed_decision["reason"] = json!("manual adjudication update");
    let mut changed = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut changed, true);
    add_reviewed_decisions(&mut changed, vec![changed_decision]);
    let changed_materialized = materialize_with_source_states(&changed, &rows, &[]).unwrap();
    assert_eq!(
        changed_materialized.conversion_report["reviewed_decision_count"],
        json!(1)
    );
    assert_ne!(
        original_digest,
        changed_materialized.conversion_report["reviewed_decision_catalog_digest"]
            .as_str()
            .unwrap()
    );
}

#[test]
fn semantic_delta_reviewed_decision_changes_fingerprint_and_matches_rebuild() {
    let base = empty_cove_o_parent_bytes();
    let parent = delta_parent_from_base_bytes(&base);
    let sources = [("crm.csv", "id\n1\n"), ("support.csv", "id\n2\n")];
    let plain = two_source_identity_map(Vec::new());
    let mut reviewed = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut reviewed, true);
    add_reviewed_decisions(
        &mut reviewed,
        vec![reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        )],
    );

    let (plain_report, plain_delta, plain_dir) = build_semantic_delta_fixture(
        "semantic-delta-reviewed-plain",
        &plain,
        &sources,
        parent.clone(),
    );
    let (reviewed_report, reviewed_delta, reviewed_dir) = build_semantic_delta_fixture(
        "semantic-delta-reviewed-merged",
        &reviewed,
        &sources,
        parent,
    );

    assert_ne!(
        plain_report["fingerprints"]["semantic_map_sha256"],
        reviewed_report["fingerprints"]["semantic_map_sha256"]
    );
    assert_eq!(
        reviewed_report["object_delta_validation"]["evidence_patches"],
        json!(1)
    );
    assert!(
        reviewed_report["object_delta_validation"]["touched_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={reviewed_report}"
    );
    assert_ne!(
        composed_delta_state_keys(&base, &plain_delta),
        composed_delta_state_keys(&base, &reviewed_delta)
    );
    assert_eq!(
        composed_delta_state_keys(&base, &reviewed_delta),
        full_rebuild_state_keys("semantic-delta-reviewed-full", &reviewed, &sources)
    );

    fs::remove_dir_all(plain_dir).unwrap();
    fs::remove_dir_all(reviewed_dir).unwrap();
}

#[test]
fn semantic_delta_existing_parent_reviewed_identity_remap_matches_rebuild() {
    let sources = [("crm.csv", "id\n1\n"), ("support.csv", "id\n2\n")];
    let plain = two_source_identity_map(Vec::new());
    let (_plain_map, plain_source_paths, plain_dir) =
        write_map_and_sources("semantic-delta-reviewed-parent", &plain, &sources);
    let inputs = read_source_inputs(&plain_source_paths).unwrap();
    validate_source_inputs(&plain, &inputs.states).unwrap();
    let parent_bytes =
        build_cove_o_with_source_states(&plain, &inputs.rows, &inputs.states).unwrap();
    let parent_surface = read_object_surface_from_bytes(&parent_bytes).unwrap();
    let parent_states = reconstruct_object_states(&parent_surface, &Default::default()).unwrap();
    assert_eq!(state_keys(&parent_states).len(), 2);

    let mut reviewed = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut reviewed, true);
    add_reviewed_decisions(
        &mut reviewed,
        vec![reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        )],
    );
    validate_source_inputs(&reviewed, &inputs.states).unwrap();
    let rebuilt_bytes =
        build_cove_o_with_source_states(&reviewed, &inputs.rows, &inputs.states).unwrap();
    let rebuilt_surface = read_object_surface_from_bytes(&rebuilt_bytes).unwrap();
    let rebuilt_states = reconstruct_object_states(&rebuilt_surface, &Default::default()).unwrap();
    assert_eq!(state_keys(&rebuilt_states).len(), 1);

    let (reviewed_map, reviewed_source_paths, reviewed_dir) =
        write_map_and_sources("semantic-delta-reviewed-remap", &reviewed, &sources);
    let out = reviewed_dir.join("semantic.covedelta");
    let result = build_semantic_delta_from_paths(
        &reviewed_map,
        &reviewed_source_paths,
        semantic_delta_options_from_parent_bytes(out, &parent_bytes),
    )
    .unwrap();
    assert!(
        result.report["object_delta_validation"]["tombstone_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={}",
        result.report
    );
    let delta_bytes = fs::read(reviewed_dir.join("semantic.covedelta")).unwrap();
    assert_eq!(
        composed_delta_state_keys(&parent_bytes, &delta_bytes),
        state_keys(&rebuilt_states)
    );
    assert_eq!(
        composed_delta_evidence_keys(&parent_bytes, &delta_bytes),
        evidence_keys_from_cove_o_bytes(&rebuilt_bytes)
    );

    fs::remove_dir_all(plain_dir).unwrap();
    fs::remove_dir_all(reviewed_dir).unwrap();
}

#[test]
fn semantic_delta_alias_catalog_change_updates_fingerprint_and_matches_rebuild() {
    let base = empty_cove_o_parent_bytes();
    let parent = delta_parent_from_base_bytes(&base);
    let base_map = company_resolution_map();
    let alias_csv =
        br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:tesco,Tesco,Tesco PLC,curated,authoritative,{}
uk-company:tesco,Tesco,Tesco Holdings,curated,authoritative,{}
"#;
    let (updated_map, alias_report) = alias_import::import_aliases_from_csv_bytes(
        &base_map,
        alias_csv,
        &alias_import::AliasImportOptions {
            catalog_id: "company_aliases".into(),
            resolver_id: "uk_company_name_resolver".into(),
        },
    )
    .unwrap();
    assert_eq!(alias_report["alias_count"], json!(2));
    let sources = [("suppliers.csv", "company_name\nTesco PLC\n")];

    let (base_report, base_delta, base_dir) = build_semantic_delta_fixture(
        "semantic-delta-alias-base",
        &base_map,
        &sources,
        parent.clone(),
    );
    let (updated_report, updated_delta, updated_dir) = build_semantic_delta_fixture(
        "semantic-delta-alias-updated",
        &updated_map,
        &sources,
        parent,
    );

    assert_eq!(updated_report["counts"]["evidence_entries"], json!(1));
    assert!(
        updated_report["object_delta_validation"]["touched_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={updated_report}"
    );
    assert_ne!(
        base_report["fingerprints"]["semantic_map_sha256"],
        updated_report["fingerprints"]["semantic_map_sha256"]
    );
    assert_eq!(
        composed_delta_state_keys(&base, &base_delta),
        composed_delta_state_keys(&base, &updated_delta)
    );
    assert_eq!(
        composed_delta_state_keys(&base, &updated_delta),
        full_rebuild_state_keys("semantic-delta-alias-full", &updated_map, &sources)
    );

    fs::remove_dir_all(base_dir).unwrap();
    fs::remove_dir_all(updated_dir).unwrap();
}

#[test]
fn semantic_delta_emits_inline_dictionary_overlay_for_filecode_properties() {
    let base = empty_cove_o_parent_bytes();
    let parent = delta_parent_from_base_bytes(&base);
    let mut map_file = two_source_property_map("reject_conflict", None, None);
    mutate_section_payload(&mut map_file, 3, |payload| {
        for rule in payload["rules"].as_array_mut().unwrap() {
            rule["property_bindings"][0]["physical_kind"] = json!("filecode");
        }
    });
    let sources = [
        ("crm.csv", "id,name\n1,Ada\n"),
        ("support.csv", "id,name\n2,Grace\n"),
    ];
    let (map, source_paths, dir) =
        write_map_and_sources("semantic-delta-filecode-overlay", &map_file, &sources);
    let out = dir.join("semantic.covedelta");
    let result =
        build_semantic_delta_from_paths(&map, &source_paths, semantic_delta_options(out, parent))
            .unwrap();
    let delta_bytes = fs::read(dir.join("semantic.covedelta")).unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let validation = delta.validate_object_delta().unwrap();
    assert_eq!(validation.dictionary_overlay_entries.len(), 2);
    assert_eq!(validation.inline_values.len(), 2);
    assert!(
        delta.header.required_delta_features & DELTA_FEATURE_INLINE_DICTIONARY != 0,
        "report={}",
        result.report
    );
    let states = reconstruct_object_states_from_base_and_delta_files(
        &base,
        &[delta],
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let names = states
        .iter()
        .flat_map(|state| state.properties.iter())
        .filter(|property| property.property_name == "name")
        .map(|property| property.value.clone())
        .collect::<Vec<_>>();
    assert!(names.contains(&json!("Ada")));
    assert!(names.contains(&json!("Grace")));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn semantic_delta_emits_sparse_property_ops_for_null_only_delta_rows() {
    let map_file = two_source_property_map("reject_conflict", None, None);
    let parent_sources = [
        ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada\"}\n"),
        ("support.jsonl", "{\"id\":\"2\",\"name\":\"Grace\"}\n"),
    ];
    let (_parent_map, parent_source_paths, parent_dir) =
        write_map_and_sources("semantic-delta-sparse-parent", &map_file, &parent_sources);
    let parent_inputs = read_source_inputs(&parent_source_paths).unwrap();
    validate_source_inputs(&map_file, &parent_inputs.states).unwrap();
    let parent_bytes =
        build_cove_o_with_source_states(&map_file, &parent_inputs.rows, &parent_inputs.states)
            .unwrap();

    let delta_sources = [
        ("crm.jsonl", "{\"id\":\"1\",\"name\":null}\n"),
        ("support.jsonl", "{\"id\":\"3\",\"name\":\"Hedy\"}\n"),
    ];
    let (map, source_paths, delta_dir) =
        write_map_and_sources("semantic-delta-sparse-null", &map_file, &delta_sources);
    let out = delta_dir.join("semantic.covedelta");
    let result = build_semantic_delta_from_paths(
        &map,
        &source_paths,
        semantic_delta_options_from_parent_bytes(out.clone(), &parent_bytes),
    )
    .unwrap();
    assert_eq!(
        result.report["object_delta_validation"]["sparse_patch_rows"],
        json!(1),
        "report={}",
        result.report
    );

    let delta_bytes = fs::read(out).unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let validation = delta.validate_object_delta().unwrap();
    assert_eq!(validation.sparse_patch_records.len(), 1);
    assert_ne!(
        delta.header.required_delta_features & DELTA_FEATURE_SPARSE_PATCH_ROWS,
        0
    );
    let states = reconstruct_object_states_from_base_and_delta_files(
        &parent_bytes,
        &[delta],
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(states.iter().any(|state| {
        state
            .properties
            .iter()
            .any(|property| property.property_name == "name" && property.value == Value::Null)
    }));

    fs::remove_dir_all(parent_dir).unwrap();
    fs::remove_dir_all(delta_dir).unwrap();
}

#[test]
fn semantic_delta_emits_sparse_set_value_property_ops() {
    let map_file = two_source_property_map("reject_conflict", None, None);
    let parent_sources = [
        ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada\"}\n"),
        ("support.jsonl", "{\"id\":\"2\",\"name\":\"Grace\"}\n"),
    ];
    let (_parent_map, parent_source_paths, parent_dir) = write_map_and_sources(
        "semantic-delta-sparse-set-parent",
        &map_file,
        &parent_sources,
    );
    let parent_inputs = read_source_inputs(&parent_source_paths).unwrap();
    validate_source_inputs(&map_file, &parent_inputs.states).unwrap();
    let parent_bytes =
        build_cove_o_with_source_states(&map_file, &parent_inputs.rows, &parent_inputs.states)
            .unwrap();

    let delta_sources = [
        ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada Lovelace\"}\n"),
        ("support.jsonl", "{\"id\":\"3\",\"name\":\"Hedy\"}\n"),
    ];
    let (map, source_paths, delta_dir) =
        write_map_and_sources("semantic-delta-sparse-set", &map_file, &delta_sources);
    let out = delta_dir.join("semantic.covedelta");
    let _ = build_semantic_delta_from_paths(
        &map,
        &source_paths,
        semantic_delta_options_from_parent_bytes(out.clone(), &parent_bytes),
    )
    .unwrap();

    let delta_bytes = fs::read(out).unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let validation = delta.validate_object_delta().unwrap();
    assert_eq!(validation.sparse_patch_records.len(), 1);
    assert_eq!(
        validation.sparse_patch_records[0].changed_properties[0].property_op,
        DELTA_PROPERTY_OP_SET_VALUE
    );
    assert!(!validation.inline_values.is_empty());
    let states = reconstruct_object_states_from_base_and_delta_files(
        &parent_bytes,
        &[delta],
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(states.iter().any(|state| {
        state.properties.iter().any(|property| {
            property.property_name == "name" && property.value == json!("Ada Lovelace")
        })
    }));

    fs::remove_dir_all(parent_dir).unwrap();
    fs::remove_dir_all(delta_dir).unwrap();
}

#[test]
fn semantic_delta_existing_parent_alias_identity_remap_matches_rebuild() {
    let sources = [(
        "suppliers.csv",
        "company_name\nAcme Trading\nAcme Holdings\n",
    )];
    let mut base_map = company_resolution_map();
    base_map.sections[4] = normalized_miss_resolution_catalog_section("company-map", "test/v1");
    let (_base_map_path, base_source_paths, base_dir) =
        write_map_and_sources("semantic-delta-alias-parent", &base_map, &sources);
    let inputs = read_source_inputs(&base_source_paths).unwrap();
    validate_source_inputs(&base_map, &inputs.states).unwrap();
    let parent_bytes =
        build_cove_o_with_source_states(&base_map, &inputs.rows, &inputs.states).unwrap();
    let parent_surface = read_object_surface_from_bytes(&parent_bytes).unwrap();
    let parent_states = reconstruct_object_states(&parent_surface, &Default::default()).unwrap();
    assert_eq!(state_keys(&parent_states).len(), 2);

    let alias_csv =
        br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:acme,Acme,Acme Trading,curated,authoritative,{}
uk-company:acme,Acme,Acme Holdings,curated,authoritative,{}
"#;
    let (updated_map, alias_report) = alias_import::import_aliases_from_csv_bytes(
        &base_map,
        alias_csv,
        &alias_import::AliasImportOptions {
            catalog_id: "company_aliases".into(),
            resolver_id: "uk_company_name_resolver".into(),
        },
    )
    .unwrap();
    assert_eq!(alias_report["alias_count"], json!(2));
    validate_source_inputs(&updated_map, &inputs.states).unwrap();
    let rebuilt_bytes =
        build_cove_o_with_source_states(&updated_map, &inputs.rows, &inputs.states).unwrap();
    let rebuilt_surface = read_object_surface_from_bytes(&rebuilt_bytes).unwrap();
    let rebuilt_states = reconstruct_object_states(&rebuilt_surface, &Default::default()).unwrap();
    assert_eq!(state_keys(&rebuilt_states).len(), 1);

    let (updated_map_path, updated_source_paths, updated_dir) =
        write_map_and_sources("semantic-delta-alias-remap", &updated_map, &sources);
    let out = updated_dir.join("semantic.covedelta");
    let result = build_semantic_delta_from_paths(
        &updated_map_path,
        &updated_source_paths,
        semantic_delta_options_from_parent_bytes(out, &parent_bytes),
    )
    .unwrap();
    assert!(
        result.report["object_delta_validation"]["tombstone_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={}",
        result.report
    );
    let delta_bytes = fs::read(updated_dir.join("semantic.covedelta")).unwrap();
    assert_eq!(
        composed_delta_state_keys(&parent_bytes, &delta_bytes),
        state_keys(&rebuilt_states)
    );
    assert_eq!(
        composed_delta_evidence_keys(&parent_bytes, &delta_bytes),
        evidence_keys_from_cove_o_bytes(&rebuilt_bytes)
    );

    fs::remove_dir_all(base_dir).unwrap();
    fs::remove_dir_all(updated_dir).unwrap();
}

#[test]
fn replay_verify_rejects_stale_reviewed_decision_digest() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let decision = reviewed_same_object_decision(
        identity_alias_ref("Person", "crm:0"),
        identity_alias_ref("Person", "support:0"),
        None,
    );
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    add_reviewed_decisions(&mut file, vec![decision.clone()]);
    let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
    verify_replay_report(&file, &materialized.conversion_report).unwrap();

    let mut changed_decision = decision;
    changed_decision["reason"] = json!("post-run adjudication changed");
    let mut changed = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut changed, true);
    add_reviewed_decisions(&mut changed, vec![changed_decision]);
    let err = verify_replay_report(&changed, &materialized.conversion_report).unwrap_err();
    assert!(err.contains("MAP_REPLAY_STALE_REVIEW"));
}

#[test]
fn reviewed_do_not_merge_rejects_conflicting_reviewed_merge() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    let left = identity_alias_ref("Person", "crm:0");
    let right = identity_alias_ref("Person", "support:0");
    add_reviewed_decisions(
        &mut file,
        vec![
            reviewed_same_object_decision(left.clone(), right.clone(), None),
            reviewed_do_not_merge_decision(left, right),
        ],
    );

    let err = plan_identities(&file, &rows).unwrap_err();
    assert!(err.contains("reviewed do-not-merge"));
}

#[test]
fn reviewed_same_object_transitive_closure_is_deterministic() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2"), ("ops", "3")]);
    let mut file = three_source_identity_map();
    set_person_reviewed_equivalence(&mut file, true);
    let mut crm_support = reviewed_same_object_decision(
        identity_alias_ref("Person", "crm:0"),
        identity_alias_ref("Person", "support:0"),
        None,
    );
    crm_support["decision_id"] = json!("review:crm-support");
    let mut support_ops = reviewed_same_object_decision(
        identity_alias_ref("Person", "support:0"),
        identity_alias_ref("Person", "ops:0"),
        None,
    );
    support_ops["decision_id"] = json!("review:support-ops");
    add_reviewed_decisions(&mut file, vec![crm_support, support_ops]);

    let first = plan_identities(&file, &rows).unwrap();
    let second = plan_identities(&file, &rows).unwrap();
    let first_goids = first
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    let second_goids = second
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(first_goids.len(), 1);
    assert_eq!(first_goids, second_goids);
}

#[test]
fn reviewed_source_row_references_bind_snapshot_and_schema() {
    let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
    let crm_snapshot = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let support_snapshot =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    let mut file = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut file, true);
    mutate_section_payload(&mut file, 0, |payload| {
        payload["sources"][0]["snapshot_digest"] = json!(crm_snapshot);
        payload["sources"][1]["snapshot_digest"] = json!(support_snapshot);
    });
    let left = json!({
        "kind": "source_row",
        "object_type": "Person",
        "identity_rule_id": "person_by_id",
        "source_id": "crm",
        "source_row_identity": "crm:0",
        "source_snapshot_digest": crm_snapshot,
        "schema_fingerprint": schema_fingerprint(&rows[0])
    });
    let right = json!({
        "kind": "source_row",
        "object_type": "Person",
        "identity_rule_id": "person_by_id",
        "source_id": "support",
        "source_row_identity": "support:0",
        "source_snapshot_digest": support_snapshot,
        "schema_fingerprint": schema_fingerprint(&rows[1])
    });
    add_reviewed_decisions(
        &mut file,
        vec![reviewed_same_object_decision(
            left.clone(),
            right.clone(),
            None,
        )],
    );
    let planned = plan_identities(&file, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);

    let mut wrong_digest = two_source_identity_map(Vec::new());
    set_person_reviewed_equivalence(&mut wrong_digest, true);
    mutate_section_payload(&mut wrong_digest, 0, |payload| {
        payload["sources"][0]["snapshot_digest"] = json!(crm_snapshot);
        payload["sources"][1]["snapshot_digest"] = json!(support_snapshot);
    });
    let mut wrong_right = right;
    wrong_right["source_snapshot_digest"] =
        json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    add_reviewed_decisions(
        &mut wrong_digest,
        vec![reviewed_same_object_decision(left, wrong_right, None)],
    );
    let err = plan_identities(&wrong_digest, &rows).unwrap_err();
    assert!(err.contains("did not match"));
}

#[test]
fn cross_rule_reviewed_same_object_requires_and_uses_canonical_anchor() {
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("id".into(), json!("1")),
            ("email".into(), json!("ada@example.test")),
        ]),
    }];
    let base = cross_rule_reviewed_identity_map();
    let planned = plan_identities(&base, &rows).unwrap();
    assert_eq!(planned.canonical.len(), 2);
    let person_by_id = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_id")
        .unwrap();
    let person_by_email = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_email")
        .unwrap();
    let left = json!({
        "kind": "identity_join_key",
        "object_type": "Person",
        "identity_rule_id": "person_by_id",
        "join_key_sha256": person_by_id.join_key_sha256
    });
    let right = json!({
        "kind": "identity_join_key",
        "object_type": "Person",
        "identity_rule_id": "person_by_email",
        "join_key_sha256": person_by_email.join_key_sha256
    });

    let mut missing_anchor = base.clone();
    add_reviewed_decisions(
        &mut missing_anchor,
        vec![reviewed_same_object_decision(
            left.clone(),
            right.clone(),
            None,
        )],
    );
    let err = plan_identities(&missing_anchor, &rows).unwrap_err();
    assert!(err.contains("requires canonical_anchor"));

    let mut wrong_shape = base.clone();
    add_reviewed_decisions(
        &mut wrong_shape,
        vec![reviewed_same_object_decision(
            left.clone(),
            right.clone(),
            Some(json!({
                "kind": "resolved_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_id",
                "components": [{
                    "role_id": "email",
                    "logical_type": "utf8",
                    "resolved_value": "ada@example.test"
                }]
            })),
        )],
    );
    let err = plan_identities(&wrong_shape, &rows).unwrap_err();
    assert!(err.contains("join key shape"));

    let mut anchored = base;
    add_reviewed_decisions(
        &mut anchored,
        vec![reviewed_same_object_decision(
            left,
            right,
            Some(json!({
                "kind": "resolved_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_id",
                "components": [{
                    "role_id": "person_id",
                    "logical_type": "utf8",
                    "resolved_value": "1"
                }]
            })),
        )],
    );
    let planned = plan_identities(&anchored, &rows).unwrap();
    let goids = planned
        .canonical
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    assert_eq!(goids.len(), 1);
    assert!(planned
        .canonical
        .iter()
        .all(|identity| identity.canonical_anchor.starts_with("person_by_id:")));
}

#[test]
fn reviewed_same_object_rejects_cross_object_type_components() {
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("id".into(), json!("1")),
            ("email".into(), json!("ada@example.test")),
        ]),
    }];
    let mut base = cross_rule_reviewed_identity_map();
    mutate_section_payload(&mut base, 2, |payload| {
        payload["identity_rules"][1]["object_type"] = json!("Company");
    });
    let planned = plan_identities(&base, &rows).unwrap();
    let person_by_id = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_id")
        .unwrap();
    let company_by_email = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_email")
        .unwrap();
    assert_eq!(person_by_id.object_type, "Person");
    assert_eq!(company_by_email.object_type, "Company");

    let mut reviewed = base;
    add_reviewed_decisions(
        &mut reviewed,
        vec![reviewed_same_object_decision(
            json!({
                "kind": "identity_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_id",
                "join_key_sha256": person_by_id.join_key_sha256
            }),
            json!({
                "kind": "identity_join_key",
                "object_type": "Company",
                "identity_rule_id": "person_by_email",
                "join_key_sha256": company_by_email.join_key_sha256
            }),
            Some(json!({
                "kind": "resolved_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_id",
                "components": [{
                    "role_id": "person_id",
                    "logical_type": "utf8",
                    "resolved_value": "1"
                }]
            })),
        )],
    );

    let err = plan_identities(&reviewed, &rows).unwrap_err();
    assert!(err.contains("crosses object types"));
}

#[test]
fn reviewed_same_object_rejects_canonical_anchor_object_type_mismatch() {
    let rows = vec![SourceRow {
        source_id: "people".into(),
        row_index: 0,
        values: BTreeMap::from([
            ("id".into(), json!("1")),
            ("email".into(), json!("ada@example.test")),
        ]),
    }];
    let mut base = cross_rule_reviewed_identity_map();
    mutate_section_payload(&mut base, 2, |payload| {
        payload["identity_rules"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "rule_id": "company_by_id",
                "object_type": "Company",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "allow_reviewed_equivalence": true,
                "function_ids": ["identity"],
                "join_keys": [{
                    "role_id": "company_id",
                    "source_column": "id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "declared"
                }]
            }));
    });
    let planned = plan_identities(&base, &rows).unwrap();
    let person_by_id = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_id")
        .unwrap();
    let person_by_email = planned
        .canonical
        .iter()
        .find(|identity| identity.identity_rule_id == "person_by_email")
        .unwrap();

    let mut reviewed = base;
    add_reviewed_decisions(
        &mut reviewed,
        vec![reviewed_same_object_decision(
            json!({
                "kind": "identity_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_id",
                "join_key_sha256": person_by_id.join_key_sha256
            }),
            json!({
                "kind": "identity_join_key",
                "object_type": "Person",
                "identity_rule_id": "person_by_email",
                "join_key_sha256": person_by_email.join_key_sha256
            }),
            Some(json!({
                "kind": "resolved_join_key",
                "object_type": "Company",
                "identity_rule_id": "company_by_id",
                "components": [{
                    "role_id": "company_id",
                    "logical_type": "utf8",
                    "resolved_value": "1"
                }]
            })),
        )],
    );

    let err = plan_identities(&reviewed, &rows).unwrap_err();
    assert!(err.contains("canonical anchor object type"));
}
