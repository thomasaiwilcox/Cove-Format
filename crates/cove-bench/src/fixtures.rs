use super::*;

pub(super) fn run_cove_map_identity_case(corpus: &Path) -> Result<Value, String> {
    let dir = corpus.join("cove-map-identity");
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create COVE-MAP dir: {err}"))?;
    let map_path = dir.join("people.covemap");
    let csv_path = dir.join("people.csv");
    durable::durable_replace(&map_path, &bench_covemap_bytes()?)
        .map_err(|err| format!("cannot publish COVE-MAP fixture: {err}"))?;
    let mut csv = String::from("id,name\n");
    for row in 0..512 {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(&csv_path, csv).map_err(|err| format!("cannot write COVE-MAP CSV: {err}"))?;
    let start = Instant::now();
    let summary = cove_map::conversion_summary_from_paths(&map_path, &[csv_path])
        .map_err(|err| format!("COVE-MAP identity benchmark failed: {err}"))?;
    let end_to_end_ns = start.elapsed().as_nanos();
    Ok(json!({
        "id": "cove_map_identity",
        "category": "COVE-MAP conversion and identity",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": end_to_end_ns,
            "end_to_end_ns": end_to_end_ns,
            "rows_materialized": summary["materialized_row_count"].as_u64().unwrap_or(0),
            "assertions": summary["assertion_count"].as_u64().unwrap_or(0),
            "evidence_entries": summary["evidence_entry_count"].as_u64().unwrap_or(0),
        },
        "optional_features": ["cove_map"],
    }))
}

pub(super) fn bench_covemap_bytes() -> Result<Vec<u8>, String> {
    let file = CovemapFile {
        header: CovemapHeaderV1::new([0x77; 16], 0),
        mapping_version: "bench/v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "sources": [{"source_id": "people", "row_identity_rules": ["person_by_id"]}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "functions": [{"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "rules": [{
                        "rule_id": "people_rows",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "record_kind": "Baseline",
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [{
                            "assertion_id": "person_name",
                            "property_id": "person_name",
                            "property_name": "name",
                            "source_column": "name",
                            "logical_type": "utf8",
                            "physical_kind": "varbytes",
                            "value_expression": "name",
                            "nullable": false
                        }]
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "bench-map",
                    "mapping_version": "bench/v1",
                    "projections": [{
                        "projection_id": "person_projection",
                        "output_table": "people_projection",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "person_goid", "logical_type": "uuid", "value": "object.goid"},
                            {"name": "name", "logical_type": "utf8", "value": "name"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    file.serialize().map_err(|err| err.to_string())
}

pub(super) fn projection_covi_covemap_bytes() -> Result<Vec<u8>, String> {
    let file = CovemapFile {
        header: CovemapHeaderV1::new([0x78; 16], 0),
        mapping_version: "bench/projection-covi.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "sources": [{"source_id": "people", "row_identity_rules": ["person_by_id"]}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "functions": [{"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"}]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "rules": [{
                        "rule_id": "people_projection_rows",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [
                            {
                                "assertion_id": "person_id",
                                "property_id": "id",
                                "property_name": "id",
                                "source_column": "id",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_name",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_status",
                                "property_id": "status",
                                "property_name": "status",
                                "source_column": "status",
                                "logical_type": "utf8",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "person_score",
                                "property_id": "score",
                                "property_name": "score",
                                "source_column": "score",
                                "logical_type": "int64",
                                "nullable": false,
                                "conflict_policy": "reject_conflict"
                            }
                        ]
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "projection-covi-bench-map",
                    "mapping_version": "bench/projection-covi.v1",
                    "projections": [{
                        "projection_id": "people_projection.v1",
                        "output_table": "people_projection",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "id", "logical_type": "utf8", "value": "id"},
                            {"name": "name", "logical_type": "utf8", "value": "name"},
                            {"name": "status", "logical_type": "utf8", "value": "status"},
                            {"name": "score", "logical_type": "int64", "value": "score"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    file.serialize().map_err(|err| err.to_string())
}

pub(super) fn showcase_multi_source_covemap() -> Result<CovemapFile, String> {
    Ok(CovemapFile {
        header: CovemapHeaderV1::new([0x53; 16], 0),
        mapping_version: "bench/showcase.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "sources": [
                        {"source_id": "crm", "row_identity_rules": ["person_by_id"], "source_priority": 10},
                        {"source_id": "directory", "row_identity_rules": ["person_by_id"], "source_priority": 20},
                        {"source_id": "subscription", "row_identity_rules": ["person_by_id"], "source_priority": 1}
                    ]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
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
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "rules": [
                        {
                            "rule_id": "upsert_person_name_crm",
                            "source_id": "crm",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_crm"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_crm",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_directory",
                            "source_id": "directory",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_directory"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_directory",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_subscription",
                            "source_id": "subscription",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_subscription"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_subscription",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        }
                    ]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "bench/showcase.v1",
                    "projections": [
                        {
                            "projection_id": "person_projection",
                            "output_table": "people_projection",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "person_goid", "logical_type": "uuid", "value": "object.goid"},
                                {"name": "name", "logical_type": "utf8", "value": "name"}
                            ]
                        },
                        {
                            "projection_id": "evidence_projection",
                            "output_table": "evidence_projection",
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "source_id", "logical_type": "utf8", "value": "evidence.source_id"},
                                {"name": "source_row_identity", "logical_type": "utf8", "value": "evidence.source_row_identity"},
                                {"name": "output_object_id", "logical_type": "uuid", "value": "evidence.output_object_id"}
                            ]
                        }
                    ]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    })
}

pub(super) fn showcase_directory_name_batch() -> Result<RecordBatch, String> {
    RecordBatch::try_from_iter(vec![
        (
            "id",
            Arc::new(StringArray::from(vec!["p1", "p2"])) as ArrayRef,
        ),
        (
            "name",
            Arc::new(StringArray::from(vec!["Ada Directory", "Linus Directory"])) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())
}

pub(super) fn covemap_json_section(
    kind: SectionKind,
    value: Value,
) -> Result<CovemapSection, String> {
    let payload =
        serde_json::to_vec(&covemap_payload_value(kind, value)).map_err(|err| err.to_string())?;
    Ok(CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: CompressionCodec::None as u8,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    })
}

pub(super) fn covemap_payload_value(kind: SectionKind, mut value: Value) -> Value {
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

pub(super) fn events_batch(row_count: usize) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(row_count);
    let mut amounts = Vec::with_capacity(row_count);
    let mut buckets = Vec::with_capacity(row_count);
    let mut names = Vec::with_capacity(row_count);
    let mut active = Vec::with_capacity(row_count);
    for row in 0..row_count {
        ids.push(row as i64);
        amounts.push(((row * 37) % 10_000) as i64);
        buckets.push(format!("bucket-{:02}", row % 16));
        names.push(match row % 5 {
            0 => "alpha",
            1 => "beta",
            2 => "gamma",
            3 => "delta",
            _ => "omega",
        });
        active.push(row % 3 != 0);
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("amount", Arc::new(Int64Array::from(amounts)) as ArrayRef),
        ("bucket", Arc::new(StringArray::from(buckets)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("active", Arc::new(BooleanArray::from(active)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

pub(super) fn write_parquet_file(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(properties))
        .map_err(|err| err.to_string())?;
    writer.write(batch).map_err(|err| err.to_string())?;
    writer.close().map_err(|err| err.to_string())?;
    Ok(())
}

pub(super) fn decode_single_arrow_projection_batch(bytes: &[u8]) -> Result<RecordBatch, String> {
    if let Ok(reader) = FileReader::try_new(Cursor::new(bytes.to_vec()), None) {
        let batches = reader
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| format!("cannot decode Arrow IPC file projection: {err}"))?;
        return batches
            .into_iter()
            .next()
            .ok_or_else(|| "Arrow IPC file projection did not contain any batches".to_string());
    }

    let reader = StreamReader::try_new(Cursor::new(bytes.to_vec()), None)
        .map_err(|err| format!("cannot decode Arrow IPC projection as file or stream: {err}"))?;
    let batches = reader
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot decode Arrow IPC stream projection: {err}"))?;
    batches
        .into_iter()
        .next()
        .ok_or_else(|| "Arrow IPC stream projection did not contain any batches".to_string())
}

pub(super) fn write_orc_file(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let mut writer = OrcWriterBuilder::new(file, batch.schema())
        .try_build()
        .map_err(|err| format!("cannot open ORC writer: {err}"))?;
    writer
        .write(batch)
        .map_err(|err| format!("cannot write ORC batch: {err}"))?;
    writer
        .close()
        .map_err(|err| format!("cannot finish ORC writer: {err}"))?;
    Ok(())
}

pub(super) fn validate_orc_parity(path: &Path, batch: &RecordBatch) -> Result<(), String> {
    let file =
        fs::File::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    let builder = OrcReaderBuilder::try_new(file)
        .map_err(|err| format!("cannot read generated ORC {}: {err}", path.display()))?;
    if builder.schema().fields().len() != batch.schema().fields().len() {
        return Err("generated ORC schema column count does not match source batch".into());
    }
    let rows = builder
        .with_batch_size(4096)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot read generated ORC batches: {err}"))?
        .iter()
        .map(|batch| batch.num_rows())
        .sum::<usize>();
    if rows != batch.num_rows() {
        return Err(format!(
            "generated ORC row count {rows} does not match source batch {}",
            batch.num_rows()
        ));
    }
    Ok(())
}

pub(super) struct CoverageCacheFixture {
    pub(super) cove_bytes: Vec<u8>,
    pub(super) cache_bytes: Vec<u8>,
}

pub(super) fn coverage_cache_fixture() -> Result<CoverageCacheFixture, String> {
    let cove_bytes = primitive_events_file_with_name_gamma_coverage(false);
    let state = cove_datafusion::bootstrap::bootstrap_bytes("synthetic-cache", cove_bytes.clone())
        .map_err(|err| err.to_string())?;
    let file_digest =
        compute_digest(DigestAlgorithm::Sha256, &cove_bytes).map_err(|err| err.to_string())?;
    let mut seed = Vec::with_capacity(28 + file_digest.len());
    seed.extend_from_slice(state.file_id());
    seed.extend_from_slice(&state.file_len().to_le_bytes());
    seed.extend_from_slice(&state.footer_crc32c().to_le_bytes());
    seed.extend_from_slice(&file_digest);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed).map_err(|err| err.to_string())?;
    let mut snapshot_id = [0u8; 16];
    snapshot_id.copy_from_slice(&digest[..16]);
    let dataset_id = *state.file_id();
    let cache = CoverageCacheV2 {
        header: CoveCoverageCacheHeaderV2 {
            cache_format_namespace_ref: 1,
            cache_format_version_major: 2,
            cache_format_version_minor: 0,
            flags: 0,
            cache_id: [7u8; 16],
            dataset_id,
            snapshot_id,
            entry_count: 1,
            created_at_us: 0,
            producer_engine_ref: 0,
            reserved: [0; 32],
            checksum: 0,
        },
        entries: vec![CoverageCacheEntryV2 {
            entry_id: 1,
            dataset_id,
            snapshot_id,
            predicate_normal_form_ref: 1,
            interval_normal_form_ref: u32::MAX,
            coverage_set_ref: 1,
            coverage_granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            actual_coverage_size_bytes: 64,
            actual_read_cost_ns: 1,
            created_at_us: 0,
            valid_until_snapshot_ref: u32::MAX,
            producer_engine_ref: 0,
            checksum: 0,
        }],
    };
    Ok(CoverageCacheFixture {
        cove_bytes,
        cache_bytes: cache.serialize().map_err(|err| err.to_string())?,
    })
}

pub(super) fn primitive_events_file_with_name_gamma_coverage(bad_checksum: bool) -> Vec<u8> {
    let mut writer = primitive_events_writer();
    for section in name_gamma_coverage_sections(1, bad_checksum) {
        writer.push_extra_section(section);
    }
    let placeholder = writer.write().unwrap();
    let placeholder_state =
        cove_datafusion::bootstrap::bootstrap_bytes("synthetic-cache", placeholder).unwrap();
    let snapshot_validity_ref = placeholder_state
        .pruning()
        .selected_coverage_snapshot_validity_ref
        .expect("coverage fixture has embedded coverage metadata");

    let mut writer = primitive_events_writer();
    for section in name_gamma_coverage_sections(snapshot_validity_ref, bad_checksum) {
        writer.push_extra_section(section);
    }
    writer.write().unwrap()
}

pub(super) fn primitive_events_writer() -> ScanProfileCoveWriter {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(1, "id", CoveLogicalType::Int64, CovePhysicalKind::NumCode),
                column(2, "name", CoveLogicalType::Utf8, CovePhysicalKind::VarBytes),
                column(
                    3,
                    "active",
                    CoveLogicalType::Bool,
                    CovePhysicalKind::Boolean,
                ),
            ],
        }],
    };
    let mut first = ScanSegment::new(1, 0, 0, 2, 3);
    first.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[1, 2]))]);
    first.set_column_pages(2, vec![varbytes_page(2, varbytes(&["alpha", "beta"]))]);
    first.set_column_pages(3, vec![bool_page(2, bools(&[true, false]))]);

    let mut second = ScanSegment::new(1, 1, 2, 1, 3);
    second.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[3]))]);
    second.set_column_pages(2, vec![varbytes_page(1, varbytes(&["gamma"]))]);
    second.set_column_pages(3, vec![bool_page(1, bools(&[true]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(first);
    writer.push_segment(second);
    writer
}

pub(super) fn column(
    column_id: u32,
    name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
) -> ColumnEntry {
    ColumnEntry {
        column_id,
        name: name.into(),
        logical,
        physical,
        nullable: false,
        sort_order: 0,
        collation_id: 0,
        precision: 0,
        scale: 0,
        flags: 0,
    }
}

pub(super) fn name_gamma_coverage_sections(
    snapshot_validity_ref: u32,
    bad_checksum: bool,
) -> Vec<SectionPayload> {
    let predicate_form_ref = 1;
    let provider_id = 1;
    let coverage_set_id = 1;
    let predicate_form_section =
        predicate_normal_form_ast_section(predicate_form_ref, 1, name_eq_gamma_ast_payload());

    let provider = CoverageProviderDescriptorV2 {
        provider_id,
        provider_kind: CoverageProofKindV2::ValueToFragmentIndex as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        granularity: CoverageGranularityV2::Morsel,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: 2,
        referenced_path_ref: u32::MAX,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref,
        predicate_form_ref,
        producer_ref: u32::MAX,
        checksum: 0,
    };
    let coverage_set = CoverageSetV2 {
        header: CoverageSetHeaderV2 {
            coverage_set_id,
            provider_id,
            granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            predicate_form_ref,
            snapshot_validity_ref,
            total_fragment_count: 2,
            covered_fragment_count: 0,
            required_fragment_count_estimate: 0,
            coverage_degree_ppm: 500_000,
            tightness_degree_ppm: 1_000_000,
            entries_offset: 0,
            entries_length: 0,
            checksum: 0,
        },
        entries: vec![CoverageSetEntryV2 {
            target_kind: CoverageGranularityV2::Morsel,
            flags: 0,
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 0,
            page_ref: u32::MAX,
            object_type_id: u32::MAX,
            path_ref: u32::MAX,
            dimensional_bucket_ref: u32::MAX,
            row_start: 0,
            row_count: 0,
            row_ordinal_bitmap_ref: u32::MAX,
            byte_range_ref: u32::MAX,
            checksum: 0,
        }],
    };
    let coverage_set_bytes = coverage_set.serialize().unwrap();
    let mut coverage_set_checksum = coverage_set_payload_checksum(&coverage_set_bytes);
    if bad_checksum {
        coverage_set_checksum ^= 1;
    }
    let proof = CoverageProofRecordV2 {
        proof_id: 1,
        provider_id,
        coverage_set_id,
        predicate_form_ref,
        proof_kind: CoverageProofKindV2::ValueToFragmentIndex,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::Morsel,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref,
        coverage_set_checksum,
        proof_payload_ref: u32::MAX,
        checksum: 0,
    };

    vec![
        coverage_section(
            SectionKind::CoverageProviderRegistry,
            1,
            provider.serialize().to_vec(),
        ),
        coverage_section(SectionKind::CoverageSet, 1, coverage_set_bytes),
        coverage_section(
            SectionKind::CoverageProofRecord,
            1,
            proof.serialize().unwrap().to_vec(),
        ),
        predicate_form_section,
    ]
}

pub(super) fn predicate_normal_form_ast_section(
    predicate_form_id: u32,
    table_id: u32,
    payload: Vec<u8>,
) -> SectionPayload {
    let form = PredicateNormalFormV2 {
        predicate_form_id,
        form_kind: PredicateFormKindV2::PredicateAst,
        flags: 0,
        logical_context_ref: table_id,
        payload_offset: PredicateNormalFormV2::LEN as u64,
        payload_length: payload.len() as u64,
        checksum: 0,
    };
    let mut data = Vec::with_capacity(PredicateNormalFormV2::LEN + payload.len());
    data.extend_from_slice(&form.serialize().unwrap());
    data.extend_from_slice(&payload);
    coverage_section(SectionKind::PredicateNormalForm, 1, data)
}

pub(super) fn name_eq_gamma_ast_payload() -> Vec<u8> {
    let canonical = CanonicalValue::Utf8("gamma").encode().unwrap();
    let node_offset = PredicateAstPayloadHeaderV2::LEN;
    let literal_offset = node_offset + PredicateAstNodeV2::LEN;
    let operand_ref_offset = literal_offset + PredicateLiteralV2::LEN;
    let canonical_offset = operand_ref_offset + 2 * PredicateAstOperandRefV2::LEN;

    let mut payload = Vec::new();
    payload.extend_from_slice(&predicate_ast_header(
        node_offset as u64,
        literal_offset as u64,
        operand_ref_offset as u64,
    ));
    payload.extend_from_slice(&predicate_ast_node());
    payload.extend_from_slice(&predicate_ast_literal(
        canonical_offset as u64,
        canonical.len() as u32,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        0,
        PredicateOperandKindV2::ColumnOrPath,
        2,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        1,
        PredicateOperandKindV2::Literal,
        0,
    ));
    payload.extend_from_slice(&canonical);
    payload
}

pub(super) fn predicate_ast_header(
    node_offset: u64,
    literal_offset: u64,
    operand_ref_offset: u64,
) -> [u8; PredicateAstPayloadHeaderV2::LEN] {
    let mut out = [0u8; PredicateAstPayloadHeaderV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..8].copy_from_slice(&1u32.to_le_bytes());
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..32].copy_from_slice(&node_offset.to_le_bytes());
    out[32..40].copy_from_slice(&literal_offset.to_le_bytes());
    out[56..64].copy_from_slice(&operand_ref_offset.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[68..72].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn predicate_ast_node() -> [u8; PredicateAstNodeV2::LEN] {
    let mut out = [0u8; PredicateAstNodeV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(PredicateOpV2::Eq as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(CoveLogicalType::Bool as u16).to_le_bytes());
    out[12] = PredicateNullPolicyV2::SqlWhere as u8;
    out[14..16].copy_from_slice(&2u16.to_le_bytes());
    out[16..20].copy_from_slice(&0u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..28].copy_from_slice(&0u32.to_le_bytes());
    out[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    out[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[36..40].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn predicate_ast_literal(
    canonical_value_offset: u64,
    canonical_value_length: u32,
) -> [u8; PredicateLiteralV2::LEN] {
    let mut out = [0u8; PredicateLiteralV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(ValueTag::Utf8 as u16).to_le_bytes());
    out[6..8].copy_from_slice(&(CoveLogicalType::Utf8 as u16).to_le_bytes());
    out[12..20].copy_from_slice(&canonical_value_offset.to_le_bytes());
    out[20..24].copy_from_slice(&canonical_value_length.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[24..28].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn predicate_ast_operand_ref(
    ordinal: u16,
    operand_kind: PredicateOperandKindV2,
    ref_id: u32,
) -> [u8; PredicateAstOperandRefV2::LEN] {
    let mut out = [0u8; PredicateAstOperandRefV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&ordinal.to_le_bytes());
    out[6] = operand_kind as u8;
    out[8..12].copy_from_slice(&ref_id.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn coverage_section(
    kind: SectionKind,
    item_count: u64,
    data: Vec<u8>,
) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data,
    }
}

pub(super) fn numcode_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::NumCode as u32)
}

pub(super) fn varbytes_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::VarBytes as u32)
}

pub(super) fn bool_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::PlainFixed as u32)
}

pub(super) fn numcode_i64(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| (*value as u64).to_le_bytes())
        .collect()
}

pub(super) fn varbytes(values: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

pub(super) fn bools(values: &[bool]) -> Vec<u8> {
    values.iter().map(|value| u8::from(*value)).collect()
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
