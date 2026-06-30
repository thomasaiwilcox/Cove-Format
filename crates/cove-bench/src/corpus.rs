use super::*;

pub(super) fn generate_corpus(profile: &str, out: &Path) -> Result<(), String> {
    let row_count = match profile {
        "ci" => 2_048,
        "standard" => 32_768,
        "publication" => 262_144,
        other => return Err(format!("unknown benchmark profile {other:?}")),
    };
    fs::create_dir_all(out).map_err(|err| format!("cannot create {}: {err}", out.display()))?;
    fs::write(out.join("public-corpus.json"), PUBLIC_MANIFEST)
        .map_err(|err| format!("cannot write manifest: {err}"))?;

    let batch = events_batch(row_count)?;
    let conversion_options = ParquetConversionOptions {
        table_name: "events".into(),
        namespace: "bench".into(),
        morsel_row_count: 512,
        segment_row_count: 2048,
        dictionary_policy: ParquetDictionaryPolicy::Auto,
        stats_policy: ParquetStatsPolicy::Recompute,
        acceleration_policy: ParquetAccelerationPolicy::Auto,
        point_lookup_columns: vec!["id".into(), "name".into()],
        cluster_columns: vec!["bucket".into()],
        topn_columns: vec!["amount".into()],
        aggregate_policy: ParquetAggregatePolicy::Auto,
        aggregate_columns: vec!["amount".into()],
        emit_covx: true,
        emit_covm: true,
        ..ParquetConversionOptions::default()
    };
    let converted = convert_arrow_record_batches(
        "generated-arrow",
        format!("events-{profile}-{row_count}"),
        batch.schema(),
        vec![batch.clone()],
        &conversion_options,
    )
    .map_err(|err| err.to_string())?;
    durable::durable_replace(&out.join("events.cove"), &converted.cove_bytes)
        .map_err(|err| format!("cannot publish events.cove: {err}"))?;
    if let Some(covx) = converted.covx_bytes {
        durable::durable_replace(&out.join("events.covx"), &covx)
            .map_err(|err| format!("cannot publish events.covx: {err}"))?;
    }
    if let Some(covm) = converted.covm_bytes {
        durable::durable_replace(&out.join("events.covm"), &covm)
            .map_err(|err| format!("cannot publish events.covm: {err}"))?;
    }
    let covi_bytes = build_covi_from_cove_bytes(
        &converted.cove_bytes,
        &CoviBuildOptions {
            column_ids: vec![1, 4],
            include_index_only_counts: true,
            include_index_only_min_max: true,
            include_index_only_distinct_count: true,
            include_index_only_exists: true,
            ..CoviBuildOptions::default()
        },
    )
    .map_err(|err| format!("cannot build events.covi: {err}"))?;
    durable::durable_replace(&out.join("events.covi"), &covi_bytes)
        .map_err(|err| format!("cannot publish events.covi: {err}"))?;
    let ai_vector_file_codes = (1..=128).collect::<Vec<_>>();
    let ai_vector_dimension_count = 8;
    let ai_vector_bytes = build_benchmark_covev_vectors(
        ai_vector_dimension_count,
        &ai_vector_file_codes,
        [0x83; 16],
        1_000,
    )?;
    durable::durable_replace(&out.join("events-ai.covev"), &ai_vector_bytes)
        .map_err(|err| format!("cannot publish events-ai.covev: {err}"))?;
    let ai_training_source = out.join("ai-training-source.jsonl");
    let ai_training_archive = out.join("ai-training.coveai");
    fs::write(
        &ai_training_source,
        benchmark_training_source_jsonl(match profile {
            "ci" => 128,
            "standard" => 2_048,
            "publication" => 8_192,
            _ => unreachable!(),
        }),
    )
    .map_err(|err| format!("cannot publish AI training source: {err}"))?;
    import_jsonl(
        &ai_training_source,
        Some(&ai_training_archive),
        AiImportOptions {
            schema: AiImportSchema::Instruction,
            split_column: Some("split".to_string()),
            publish_covm: true,
            artifact_id: Some([0x85; 16]),
            created_at_us: Some(1_002),
            ..AiImportOptions::default()
        },
    )
    .map_err(|err| format!("cannot build benchmark AI training archive: {err}"))?;
    write_parquet_file(&out.join("events.parquet"), &batch)?;
    write_orc_file(&out.join("events.orc"), &batch)?;
    validate_orc_parity(&out.join("events.orc"), &batch)?;
    let mut publication_locks = generate_publication_gap_datasets(profile, row_count, out)?;

    let cache_fixture = coverage_cache_fixture()?;
    durable::durable_replace(&out.join("synthetic-cache.cove"), &cache_fixture.cove_bytes)
        .map_err(|err| format!("cannot publish synthetic-cache.cove: {err}"))?;
    durable::durable_replace(
        &out.join("synthetic-cache.cove.cache"),
        &cache_fixture.cache_bytes,
    )
    .map_err(|err| format!("cannot publish synthetic-cache.cove.cache: {err}"))?;

    let mut lock_entries = vec![
        dataset_lock("events", "events.cove", &converted.cove_bytes)?,
        dataset_lock(
            "events-orc",
            "events.orc",
            &fs::read(out.join("events.orc")).map_err(|err| err.to_string())?,
        )?,
        dataset_lock("events-covi", "events.covi", &covi_bytes)?,
        dataset_lock("events-ai", "events-ai.covev", &ai_vector_bytes)?,
        dataset_lock(
            "ai-training",
            "ai-training.coveai",
            &fs::read(&ai_training_archive).map_err(|err| err.to_string())?,
        )?,
        dataset_lock(
            "synthetic-cache",
            "synthetic-cache.cove",
            &cache_fixture.cove_bytes,
        )?,
    ];
    lock_entries.append(&mut publication_locks);
    let lock = json!({
        "version": 1,
        "profile": profile,
        "manifest_sha256": hex(&compute_digest(DigestAlgorithm::Sha256, PUBLIC_MANIFEST.as_bytes()).map_err(|err| err.to_string())?),
        "datasets": lock_entries,
    });
    fs::write(
        out.join("corpus.lock.json"),
        serde_json::to_vec_pretty(&lock).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write corpus lock: {err}"))?;
    Ok(())
}

pub(super) fn dataset_lock(name: &str, path: &str, bytes: &[u8]) -> Result<Value, String> {
    Ok(json!({
        "name": name,
        "path": path,
        "bytes": bytes.len(),
        "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, bytes).map_err(|err| err.to_string())?),
    }))
}

pub(super) fn build_benchmark_covev_vectors(
    dimension_count: u32,
    file_codes: &[u32],
    artifact_id: [u8; 16],
    created_at_us: i64,
) -> Result<Vec<u8>, String> {
    write_covev_filecode_vectors_with_index(
        &CoveVecFileCodeVectorBuild {
            artifact_id,
            created_at_us,
            dimension_count,
            file_codes: file_codes.to_vec(),
            vector_payload: benchmark_vector_payload(dimension_count, file_codes),
        },
        1,
    )
    .map_err(|err| format!("cannot build benchmark COVE-VEC sidecar: {err}"))
}

pub(super) fn benchmark_vector_payload(dimension_count: u32, file_codes: &[u32]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(file_codes.len() * dimension_count as usize * 4);
    for file_code in file_codes {
        for dim in 0..dimension_count {
            let seed = (*file_code as f32 * 0.03125) + (dim as f32 * 0.125);
            let value = seed.sin() + seed.cos() * 0.5;
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    payload
}

pub(super) fn benchmark_training_source_jsonl(sample_count: usize) -> String {
    let mut out = String::new();
    for index in 0..sample_count {
        let split = if index % 100 == 0 {
            "test"
        } else if index % 50 == 0 {
            "validation"
        } else {
            "train"
        };
        let row = json!({
            "sample_id": format!("bench-{index:08}"),
            "instruction": format!("Explain governed training archive sample {index}."),
            "input": format!("source_record={index}; split={split}"),
            "output": format!("COVE-AI keeps sample {index} reproducible and policy-auditable."),
            "split": split,
            "generator": {
                "provider": "cove-bench",
                "model": "deterministic-fixture"
            },
            "policy": {
                "payload_permission": index % 37 != 0,
                "diagnostic": if index % 37 == 0 { "benchmark_policy_withheld" } else { "allowed" }
            }
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    out
}

pub(super) fn generate_publication_gap_datasets(
    profile: &str,
    row_count: usize,
    out: &Path,
) -> Result<Vec<Value>, String> {
    let mut locks = Vec::new();
    let runnable = [
        ("tpch-style", "tpch_style", row_count),
        (
            "tpcds-style",
            "tpcds_style",
            row_count.saturating_div(2).max(64),
        ),
        (
            "medical-operational",
            "medical_operational",
            row_count.saturating_div(2).max(64),
        ),
    ];
    for (dataset_id, table_name, rows) in runnable {
        let batch = events_batch(rows)?;
        let options = ParquetConversionOptions {
            table_name: table_name.into(),
            namespace: "bench_publication".into(),
            morsel_row_count: 512,
            segment_row_count: 2048,
            dictionary_policy: ParquetDictionaryPolicy::Auto,
            stats_policy: ParquetStatsPolicy::Recompute,
            acceleration_policy: ParquetAccelerationPolicy::Auto,
            point_lookup_columns: vec!["id".into(), "name".into()],
            cluster_columns: vec!["bucket".into()],
            topn_columns: vec!["amount".into()],
            aggregate_policy: ParquetAggregatePolicy::Auto,
            aggregate_columns: vec!["amount".into()],
            emit_covx: true,
            emit_covm: true,
            ..ParquetConversionOptions::default()
        };
        let converted = convert_arrow_record_batches(
            "generated-arrow",
            format!("{dataset_id}-{profile}-{rows}"),
            batch.schema(),
            vec![batch.clone()],
            &options,
        )
        .map_err(|err| err.to_string())?;
        let cove_path = out.join(format!("{dataset_id}.cove"));
        let parquet_path = out.join(format!("{dataset_id}.parquet"));
        let orc_path = out.join(format!("{dataset_id}.orc"));
        let report_path = out.join(format!("{dataset_id}.report.json"));
        durable::durable_replace(&cove_path, &converted.cove_bytes)
            .map_err(|err| format!("cannot publish {dataset_id}.cove: {err}"))?;
        write_parquet_file(&parquet_path, &batch)?;
        write_orc_file(&orc_path, &batch)?;
        validate_orc_parity(&orc_path, &batch)?;
        let parquet_bytes = fs::read(&parquet_path).map_err(|err| err.to_string())?;
        let orc_bytes = fs::read(&orc_path).map_err(|err| err.to_string())?;
        let report = json!({
            "version": 1,
            "dataset": dataset_id,
            "profile": profile,
            "rows": rows,
            "artifacts": {
                "cove": {
                    "path": format!("{dataset_id}.cove"),
                    "bytes": converted.cove_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &converted.cove_bytes).map_err(|err| err.to_string())?),
                },
                "parquet": {
                    "path": format!("{dataset_id}.parquet"),
                    "bytes": parquet_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &parquet_bytes).map_err(|err| err.to_string())?),
                },
                "orc": {
                    "path": format!("{dataset_id}.orc"),
                    "bytes": orc_bytes.len(),
                    "sha256": hex(&compute_digest(DigestAlgorithm::Sha256, &orc_bytes).map_err(|err| err.to_string())?),
                },
            },
            "generation": "deterministic public v2 generated analog",
        });
        let report_bytes = serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?;
        fs::write(&report_path, &report_bytes)
            .map_err(|err| format!("cannot write {}: {err}", report_path.display()))?;

        locks.push(dataset_lock(
            dataset_id,
            &format!("{dataset_id}.cove"),
            &converted.cove_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-parquet"),
            &format!("{dataset_id}.parquet"),
            &parquet_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-orc"),
            &format!("{dataset_id}.orc"),
            &orc_bytes,
        )?);
        locks.push(dataset_lock(
            &format!("{dataset_id}-report"),
            &format!("{dataset_id}.report.json"),
            &report_bytes,
        )?);
    }

    let corrupt_bytes = b"not-a-cove-v2-file\n".to_vec();
    durable::durable_replace(&out.join("negative-corrupt.cove"), &corrupt_bytes)
        .map_err(|err| format!("cannot publish negative-corrupt.cove: {err}"))?;
    let corrupt_metadata = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "negative-corrupt",
        "expected": "reject",
        "expected_error_class": "invalid_cove_artifact",
        "artifact": "negative-corrupt.cove",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(
        out.join("negative-corrupt.expected.json"),
        &corrupt_metadata,
    )
    .map_err(|err| format!("cannot write negative-corrupt metadata: {err}"))?;
    locks.push(dataset_lock(
        "negative-corrupt",
        "negative-corrupt.cove",
        &corrupt_bytes,
    )?);
    locks.push(dataset_lock(
        "negative-corrupt-expected",
        "negative-corrupt.expected.json",
        &corrupt_metadata,
    )?);

    let canonicalisation = canonicalisation_fixture()?;
    let canonicalisation_bytes =
        serde_json::to_vec_pretty(&canonicalisation).map_err(|err| err.to_string())?;
    fs::write(out.join("canonicalisation.json"), &canonicalisation_bytes)
        .map_err(|err| format!("cannot write canonicalisation fixture: {err}"))?;
    locks.push(dataset_lock(
        "canonicalisation",
        "canonicalisation.json",
        &canonicalisation_bytes,
    )?);

    let semantic_dir = out.join("semantic-mapping");
    fs::create_dir_all(&semantic_dir)
        .map_err(|err| format!("cannot create semantic mapping dir: {err}"))?;
    let covemap_bytes = bench_covemap_bytes()?;
    durable::durable_replace(&semantic_dir.join("people.covemap"), &covemap_bytes)
        .map_err(|err| format!("cannot publish semantic mapping COVE-MAP: {err}"))?;
    let mut csv = String::from("id,name\n");
    for row in 0..512 {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(semantic_dir.join("people.csv"), csv.as_bytes())
        .map_err(|err| format!("cannot write semantic mapping CSV: {err}"))?;
    let semantic_map_path = semantic_dir.join("people.covemap");
    let semantic_csv_path = semantic_dir.join("people.csv");
    let semantic_mapped_cove_o =
        cove_o_from_paths(&semantic_map_path, std::slice::from_ref(&semantic_csv_path))
            .map_err(|err| format!("cannot build semantic mapping mapped COVE-O: {err}"))?;
    durable::durable_replace(
        &semantic_dir.join("people_mapped.cove"),
        &semantic_mapped_cove_o,
    )
    .map_err(|err| format!("cannot publish semantic mapping mapped COVE-O: {err}"))?;
    let semantic_cove_t = projected_output_from_paths(
        &semantic_map_path,
        std::slice::from_ref(&semantic_csv_path),
        ProjectionFormat::CoveT,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic mapping projected COVE-T: {err}"))?;
    durable::durable_replace(
        &semantic_dir.join("people_projection.cove"),
        &semantic_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic mapping projected COVE-T: {err}"))?;
    let semantic_arrow = projected_output_from_paths(
        &semantic_map_path,
        std::slice::from_ref(&semantic_csv_path),
        ProjectionFormat::Arrow,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic mapping Arrow projection: {err}"))?;
    let semantic_projection_batch = decode_single_arrow_projection_batch(&semantic_arrow)?;
    write_parquet_file(
        &semantic_dir.join("people_projection.parquet"),
        &semantic_projection_batch,
    )?;
    let semantic_expected = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "semantic-mapping",
        "expected_rows": 512,
        "mapping_id": "bench-map",
        "mapping_version": "bench/v1",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(semantic_dir.join("expected.json"), &semantic_expected)
        .map_err(|err| format!("cannot write semantic mapping metadata: {err}"))?;
    locks.push(dataset_lock(
        "semantic-mapping-covemap",
        "semantic-mapping/people.covemap",
        &covemap_bytes,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-csv",
        "semantic-mapping/people.csv",
        csv.as_bytes(),
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-mapped-cove-o",
        "semantic-mapping/people_mapped.cove",
        &semantic_mapped_cove_o,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-cove-t",
        "semantic-mapping/people_projection.cove",
        &semantic_cove_t,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-parquet",
        "semantic-mapping/people_projection.parquet",
        &fs::read(semantic_dir.join("people_projection.parquet")).map_err(|err| err.to_string())?,
    )?);
    locks.push(dataset_lock(
        "semantic-mapping-expected",
        "semantic-mapping/expected.json",
        &semantic_expected,
    )?);

    let showcase_dir = out.join("semantic-showcase");
    fs::create_dir_all(&showcase_dir)
        .map_err(|err| format!("cannot create semantic showcase dir: {err}"))?;
    let showcase_map_bytes = showcase_multi_source_covemap()?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&showcase_dir.join("showcase.covemap"), &showcase_map_bytes)
        .map_err(|err| format!("cannot publish semantic showcase COVE-MAP: {err}"))?;
    fs::write(
        showcase_dir.join("crm.csv"),
        b"id,name\np1,Ada CRM\np2,Linus CRM\n",
    )
    .map_err(|err| format!("cannot write semantic showcase CRM CSV: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("directory.parquet"),
        &showcase_directory_name_batch()?,
    )?;
    fs::write(
        showcase_dir.join("subscription.csv"),
        b"id,name\np1,Ada\np2,Linus\n",
    )
    .map_err(|err| format!("cannot write semantic showcase subscription CSV: {err}"))?;
    let showcase_map_path = showcase_dir.join("showcase.covemap");
    let showcase_sources = vec![
        showcase_dir.join("crm.csv"),
        showcase_dir.join("directory.parquet"),
        showcase_dir.join("subscription.csv"),
    ];
    let showcase_object_bytes = cove_o_from_paths(&showcase_map_path, &showcase_sources)
        .map_err(|err| format!("cannot build semantic showcase mapped COVE-O: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("showcase_mapped.cove"),
        &showcase_object_bytes,
    )
    .map_err(|err| format!("cannot publish semantic showcase mapped COVE-O: {err}"))?;
    let showcase_object_path = showcase_dir.join("showcase_mapped.cove");
    let showcase_people_cove_t = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::CoveT,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase people COVE-T: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("people_projection.cove"),
        &showcase_people_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic showcase people COVE-T: {err}"))?;
    let showcase_evidence_cove_t = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::CoveT,
        Some("evidence_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase evidence COVE-T: {err}"))?;
    durable::durable_replace(
        &showcase_dir.join("evidence_projection.cove"),
        &showcase_evidence_cove_t,
    )
    .map_err(|err| format!("cannot publish semantic showcase evidence COVE-T: {err}"))?;
    let showcase_people_arrow = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::Arrow,
        Some("person_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase people Arrow: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("people_projection.parquet"),
        &decode_single_arrow_projection_batch(&showcase_people_arrow)?,
    )?;
    let showcase_evidence_arrow = projected_output_from_cove_o_path(
        &showcase_object_path,
        None,
        ProjectionFormat::Arrow,
        Some("evidence_projection"),
    )
    .map_err(|err| format!("cannot build semantic showcase evidence Arrow: {err}"))?;
    write_parquet_file(
        &showcase_dir.join("evidence_projection.parquet"),
        &decode_single_arrow_projection_batch(&showcase_evidence_arrow)?,
    )?;
    let showcase_expected = serde_json::to_vec_pretty(&json!({
        "version": 1,
        "dataset": "semantic-showcase",
        "expected_people_rows": 2,
        "expected_evidence_rows": 6,
        "mapping_id": "showcase-map",
        "mapping_version": "bench/showcase.v1",
    }))
    .map_err(|err| err.to_string())?;
    fs::write(showcase_dir.join("expected.json"), &showcase_expected)
        .map_err(|err| format!("cannot write semantic showcase metadata: {err}"))?;
    locks.push(dataset_lock(
        "semantic-showcase-covemap",
        "semantic-showcase/showcase.covemap",
        &showcase_map_bytes,
    )?);
    for (name, rel) in [
        ("semantic-showcase-crm", "semantic-showcase/crm.csv"),
        (
            "semantic-showcase-directory",
            "semantic-showcase/directory.parquet",
        ),
        (
            "semantic-showcase-subscription",
            "semantic-showcase/subscription.csv",
        ),
        (
            "semantic-showcase-mapped-cove-o",
            "semantic-showcase/showcase_mapped.cove",
        ),
        (
            "semantic-showcase-people-cove-t",
            "semantic-showcase/people_projection.cove",
        ),
        (
            "semantic-showcase-evidence-cove-t",
            "semantic-showcase/evidence_projection.cove",
        ),
        (
            "semantic-showcase-people-parquet",
            "semantic-showcase/people_projection.parquet",
        ),
        (
            "semantic-showcase-evidence-parquet",
            "semantic-showcase/evidence_projection.parquet",
        ),
        (
            "semantic-showcase-expected",
            "semantic-showcase/expected.json",
        ),
    ] {
        locks.push(dataset_lock(
            name,
            rel,
            &fs::read(out.join(rel)).map_err(|err| err.to_string())?,
        )?);
    }

    let customer360_dir = out.join("customer360");
    let customer360_profile = match profile {
        "ci" => Customer360Profile::Quick,
        "standard" => Customer360Profile::Standard,
        "publication" => Customer360Profile::Publication,
        other => return Err(format!("unknown benchmark profile {other:?}")),
    };
    let customer360_manifest = generate_customer360(&Customer360Options {
        out_dir: customer360_dir.clone(),
        profile: customer360_profile,
        force: true,
    })
    .map_err(|err| format!("cannot build Customer 360 benchmark corpus: {err}"))?;
    let customer360_manifest_bytes =
        serde_json::to_vec_pretty(&customer360_manifest).map_err(|err| err.to_string())?;
    for (name, rel) in [
        ("customer360-crm", "customer360/crm.csv"),
        ("customer360-support", "customer360/support.jsonl"),
        ("customer360-billing", "customer360/billing.parquet"),
        ("customer360-reconciled", "customer360/customers_360.jsonl"),
        ("customer360-events-jsonl", "customer360/events.jsonl"),
        ("customer360-events-cove", "customer360/events.cove"),
        ("customer360-covemap", "customer360/customer360.covemap"),
        (
            "customer360-readback-covemap",
            "customer360/customer360_readback.covemap",
        ),
        ("customer360-mapped-cove-o", "customer360/customers.cove"),
        (
            "customer360-customers-cove-t",
            "customer360/customers_projection.cove",
        ),
        (
            "customer360-evidence-cove-t",
            "customer360/evidence_projection.cove",
        ),
        (
            "customer360-customers-parquet",
            "customer360/customers_projection.parquet",
        ),
        (
            "customer360-evidence-parquet",
            "customer360/evidence_projection.parquet",
        ),
        (
            "customer360-notebook-script",
            "customer360/notebooks/customer360_analysis.py",
        ),
    ] {
        locks.push(dataset_lock(
            name,
            rel,
            &fs::read(out.join(rel)).map_err(|err| err.to_string())?,
        )?);
    }
    locks.push(dataset_lock(
        "customer360-manifest",
        "customer360/customer360-manifest.json",
        &customer360_manifest_bytes,
    )?);

    let proof_suite_dir = out.join("proof-suite");
    let proof_suite_manifest = generate_proof_suite(&ProofSuiteOptions {
        out_dir: proof_suite_dir,
        profile: customer360_profile,
        scenario: ProofSuiteScenario::All,
        force: true,
    })
    .map_err(|err| format!("cannot build COVE-O proof-suite benchmark corpus: {err}"))?;
    let proof_suite_manifest_bytes =
        serde_json::to_vec_pretty(&proof_suite_manifest).map_err(|err| err.to_string())?;
    locks.push(dataset_lock(
        "proof-suite-manifest",
        "proof-suite/proof-suite-manifest.json",
        &proof_suite_manifest_bytes,
    )?);
    for scenario in ["customer360", "claims", "catalog"] {
        for (name_suffix, rel_suffix) in [
            ("doctor", "doctor-report.json"),
            ("size", "proof-size-comparison.json"),
            (
                "bundle-manifest",
                "map-build-bundle/map-build-manifest.json",
            ),
            ("bundle-report", "map-build-bundle/map-build-report.json"),
        ] {
            let rel = format!("proof-suite/{scenario}/{rel_suffix}");
            locks.push(dataset_lock(
                &format!("proof-suite-{scenario}-{name_suffix}"),
                &rel,
                &fs::read(out.join(&rel)).map_err(|err| err.to_string())?,
            )?);
        }
    }

    Ok(locks)
}

pub(super) fn canonicalisation_fixture() -> Result<Value, String> {
    let cases = vec![
        (
            "utf8_nfc_source",
            "utf8",
            CanonicalValue::Utf8("cafe\u{301}"),
        ),
        (
            "signed_width_normalisation",
            "int64",
            CanonicalValue::Int {
                width: 2,
                value: -123,
            },
        ),
        (
            "list_order_preserved",
            "list",
            CanonicalValue::List(vec![
                CanonicalValue::Utf8("alpha"),
                CanonicalValue::Utf8("beta"),
            ]),
        ),
        (
            "map_sorted_by_canonical_key",
            "map",
            CanonicalValue::Map(vec![
                (
                    CanonicalValue::Utf8("b"),
                    CanonicalValue::Int { width: 8, value: 2 },
                ),
                (
                    CanonicalValue::Utf8("a"),
                    CanonicalValue::Int { width: 8, value: 1 },
                ),
            ]),
        ),
    ];
    let mut encoded = Vec::new();
    for (id, logical, value) in cases {
        encoded.push(json!({
            "id": id,
            "logical": logical,
            "value_tag": format!("{:?}", value.value_tag()),
            "canonical_hex": hex(&value.encode().map_err(|err| err.to_string())?),
        }));
    }
    Ok(json!({
        "version": 1,
        "dataset": "canonicalisation",
        "cases": encoded,
    }))
}

pub(super) fn run_corpus(
    corpus: &Path,
    report_json: &Path,
    report_md: &Path,
) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(PUBLIC_MANIFEST).map_err(|err| err.to_string())?;
    let mut cases = Vec::new();
    cases.extend(run_events_cases(corpus)?);
    cases.extend(run_ai_cases(corpus)?);
    cases.extend(run_cache_cases(corpus)?);
    cases.push(run_cove_o_delta_artifact_metrics_case()?);
    cases.extend(run_publication_gap_cases(corpus)?);
    for case in &mut cases {
        normalize_case_metrics(case);
    }
    validate_report_cases(&cases)?;
    let report = json!({
        "version": 1,
        "manifest": manifest,
        "corpus": corpus.display().to_string(),
        "environment": environment_report(),
        "feature_disclosure": {
            "covx": corpus.join("events.covx").is_file(),
            "covi": corpus.join("events.covi").is_file(),
            "coverage_cache": true,
            "cove_map": true,
            "layout": true,
            "parquet_compare": true,
            "orc_compare": corpus.join("events.orc").is_file(),
            "publication_corpora": true,
            "object_store_harness": true,
            "cove_o_delta_artifacts": true,
            "cove_ai": corpus.join("events-ai.covev").is_file(),
        },
        "cases": cases,
    });
    if let Some(parent) = report_json.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("cannot create report dir: {err}"))?;
    }
    fs::write(
        report_json,
        serde_json::to_vec_pretty(&report).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", report_json.display()))?;
    fs::write(report_md, markdown_report(&report))
        .map_err(|err| format!("cannot write {}: {err}", report_md.display()))?;
    Ok(())
}

pub(super) fn run_events_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("events.cove");
    let mut cases = Vec::new();
    let queries = vec![
        (
            "full_numeric_scan",
            "full numeric scan",
            ExplainOptions {
                projection: Some(vec!["id".into(), "amount".into()]),
                ..ExplainOptions::default()
            },
        ),
        (
            "string_category_scan",
            "string/category scan",
            ExplainOptions {
                projection: Some(vec!["name".into(), "bucket".into()]),
                ..ExplainOptions::default()
            },
        ),
        (
            "equality_filter",
            "equality predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "id".into(),
                    op: FilterOp::Eq,
                    value: Some("17".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "point_lookup",
            "point lookup predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "id".into(),
                    op: FilterOp::Eq,
                    value: Some("1024".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "range_filter",
            "range predicate",
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "amount".into(),
                    op: FilterOp::Gte,
                    value: Some("1000".into()),
                }],
                ..ExplainOptions::default()
            },
        ),
        (
            "top_n",
            "Top-N planning",
            ExplainOptions {
                projection: Some(vec!["id".into(), "amount".into()]),
                top_n: Some(TopNDsl {
                    column: "amount".into(),
                    fetch: 10,
                    descending: true,
                }),
                ..ExplainOptions::default()
            },
        ),
    ];
    for (id, category, options) in queries {
        cases.push(run_query_case(id, category, &path, options)?);
    }
    let parquet = corpus.join("events.parquet");
    let orc = corpus.join("events.orc");
    cases.push(json!({
        "id": "parquet_conversion_cost",
        "category": "Parquet conversion cost and file-size overhead",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "parquet_bytes": fs::metadata(&parquet).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["parquet_compare"],
    }));
    cases.push(json!({
        "id": "orc_conversion_cost",
        "category": "ORC conversion cost and file-size overhead",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "orc_bytes": fs::metadata(&orc).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }));
    cases.push(run_orc_readback_case(&orc)?);
    cases.push(json!({
        "id": "file_size_overhead",
        "category": "COVE file-size overhead vs Parquet",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "parquet_bytes": fs::metadata(&parquet).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["parquet_compare"],
    }));
    cases.push(json!({
        "id": "orc_file_size_overhead",
        "category": "COVE file-size overhead vs ORC",
        "status": "measured",
        "metrics": {
            "cove_bytes": fs::metadata(&path).map_err(|err| err.to_string())?.len(),
            "orc_bytes": fs::metadata(&orc).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }));
    if corpus.join("events.covm").is_file() {
        cases.push(json!({
            "id": "covm_many_file_planning",
            "category": "COVM manifest planning",
            "status": "measured",
            "metrics": {
                "manifest_bytes": fs::metadata(corpus.join("events.covm")).map_err(|err| err.to_string())?.len(),
            },
            "optional_features": ["covm"],
        }));
    }
    cases.push(run_query_case(
        "in_filter",
        "IN predicate",
        &path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "bucket".into(),
                op: FilterOp::In,
                value: Some("bucket-01|bucket-03|bucket-05".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_metadata_count_min_max_case(&path)?);
    cases.push(run_object_store_cold_warm_case(corpus, &path)?);
    cases.push(json!({
        "id": "covx_acceleration",
        "category": "COVX acceleration",
        "status": if corpus.join("events.covx").is_file() { "measured" } else { "skipped" },
        "metrics": {
            "covx_present": corpus.join("events.covx").is_file(),
            "covx_bytes": fs::metadata(corpus.join("events.covx")).map(|meta| meta.len()).unwrap_or(0),
        },
        "optional_features": ["covx"],
    }));
    let mut covi_latency = run_query_case(
        "covi_index_latency",
        "COVE-I point lookup latency",
        &path,
        ExplainOptions {
            filters: vec![FilterDsl {
                column: "id".into(),
                op: FilterOp::Eq,
                value: Some("1024".into()),
            }],
            table_options: CoveTableOptions::default()
                .with_covi_discovery(CoviDiscovery::SiblingExtension),
            ..ExplainOptions::default()
        },
    )?;
    if let Some(case) = covi_latency.as_object_mut() {
        case.insert("optional_features".into(), json!(["covi"]));
    }
    cases.push(covi_latency);
    cases.push(run_covi_index_only_count_case(&path)?);
    cases.push(run_cove_map_identity_case(corpus)?);
    cases.push(json!({
        "id": "layout_scan_split",
        "category": "layout and scan-split planning",
        "status": "measured",
        "metrics": {
            "layout_disclosed": true,
        },
        "optional_features": ["layout"],
    }));
    cases.extend(run_spec_gap_cases(&path)?);
    Ok(cases)
}

pub(super) fn run_spec_gap_cases(path: &Path) -> Result<Vec<Value>, String> {
    Ok(vec![
        run_query_case(
            "filecode_group_by",
            "FileCode group-by/export dictionary path",
            path,
            ExplainOptions {
                projection: Some(vec!["bucket".into(), "name".into()]),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "execution_code_remap_overhead",
            "ExecutionCode remap overhead",
            path,
            ExplainOptions {
                projection: Some(vec!["name".into()]),
                table_options: CoveTableOptions::default(),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "registered_codec_decode_predicate_kernel",
            "registered codec decode and predicate-kernel cost",
            path,
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "amount".into(),
                    op: FilterOp::Lt,
                    value: Some("500".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "fallback_payload_overhead",
            "fallback payload overhead",
            path,
            ExplainOptions {
                projection: Some(vec!["id".into(), "active".into()]),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "page_cluster_range_coalescing",
            "page-cluster range coalescing",
            path,
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "bucket".into(),
                    op: FilterOp::In,
                    value: Some("bucket-01|bucket-02".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "zero_copy_success_fallback_rate",
            "zero-copy success and fallback rate",
            path,
            ExplainOptions {
                projection: Some(vec!["id".into(), "amount".into(), "name".into()]),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "coverage_degree_tightness",
            "coverage degree and pruning tightness",
            path,
            ExplainOptions {
                filters: vec![FilterDsl {
                    column: "id".into(),
                    op: FilterOp::Gte,
                    value: Some("1024".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
    ])
}
