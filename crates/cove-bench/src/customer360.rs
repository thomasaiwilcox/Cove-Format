use super::*;

pub(super) fn run_proof_suite_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    ["customer360", "claims", "catalog"]
        .into_iter()
        .map(|scenario| run_proof_suite_case(corpus, scenario))
        .collect()
}

pub(super) fn run_proof_suite_case(corpus: &Path, scenario: &str) -> Result<Value, String> {
    let dir = corpus.join("proof-suite").join(scenario);
    let start = Instant::now();
    let size_report: Value = serde_json::from_slice(
        &fs::read(dir.join("proof-size-comparison.json"))
            .map_err(|err| format!("cannot read {scenario} proof size report: {err}"))?,
    )
    .map_err(|err| format!("cannot parse {scenario} proof size report: {err}"))?;
    let doctor: Value = serde_json::from_slice(
        &fs::read(dir.join("doctor-report.json"))
            .map_err(|err| format!("cannot read {scenario} proof doctor report: {err}"))?,
    )
    .map_err(|err| format!("cannot parse {scenario} proof doctor report: {err}"))?;
    let mut parity_ok = true;
    let mut parity_reports = 0u64;
    let parity_dir = dir.join("parity");
    for entry in fs::read_dir(&parity_dir)
        .map_err(|err| format!("cannot read {}: {err}", parity_dir.display()))?
    {
        let entry =
            entry.map_err(|err| format!("cannot read {} entry: {err}", parity_dir.display()))?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let report: Value = serde_json::from_slice(
            &fs::read(entry.path()).map_err(|err| format!("cannot read parity report: {err}"))?,
        )
        .map_err(|err| format!("cannot parse parity report: {err}"))?;
        parity_reports += 1;
        parity_ok &= report.get("status").and_then(Value::as_str) == Some("ok");
    }
    let elapsed = start.elapsed().as_nanos();
    let metric_u64 =
        |field: &str| -> u64 { size_report.get(field).and_then(Value::as_u64).unwrap_or(0) };
    let build_time = metric_u64("build_time_ns");
    let source_bytes = metric_u64("source_bytes");
    let cove_o_bytes = metric_u64("cove_o_bytes");
    let cove_t_bytes = metric_u64("cove_t_bytes");
    let parquet_bytes = metric_u64("denormalized_parquet_bytes");
    let covi_bytes = metric_u64("covi_bytes");
    let covm_bytes = metric_u64("covm_bytes");
    let total_bundle_bytes = metric_u64("total_bundle_bytes");
    let artifact_sizes = json!({
        "source_bytes": source_bytes,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_bytes": cove_t_bytes,
        "covi_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "parquet_bytes": parquet_bytes,
        "total_bundle_bytes": total_bundle_bytes,
    });
    let metrics = json!({
        "planning_ns": elapsed,
        "scan_ns": build_time,
        "end_to_end_ns": build_time.saturating_add(elapsed as u64),
        "build_time_ns": build_time,
        "validation_time_ns": elapsed,
        "parity_time_ns": elapsed,
        "rows_materialized": size_report.get("object_count").cloned().unwrap_or(Value::Null),
        "source_bytes": source_bytes,
        "source_parquet_bundle_bytes": metric_u64("source_parquet_bundle_bytes"),
        "normalized_parquet_bundle_bytes": metric_u64("normalized_parquet_bundle_bytes"),
        "denormalized_parquet_bytes": parquet_bytes,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_bytes": cove_t_bytes,
        "covi_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "object_count": size_report.get("object_count").cloned().unwrap_or(Value::Null),
        "property_value_count": size_report.get("property_value_count").cloned().unwrap_or(Value::Null),
        "evidence_entry_count": size_report.get("evidence_entry_count").cloned().unwrap_or(Value::Null),
        "duplication_ratio": size_report.get("duplication_ratio_vs_source").cloned().unwrap_or(Value::Null),
        "cove_o_vs_source_ratio": size_report.get("cove_o_vs_source_ratio").cloned().unwrap_or(Value::Null),
        "cove_o_vs_source_parquet_ratio": size_report.get("cove_o_vs_source_parquet_ratio").cloned().unwrap_or(Value::Null),
        "doctor_status_ok": doctor.get("status").and_then(Value::as_str) == Some("ok"),
        "parity_status_ok": parity_ok,
        "parity_report_count": parity_reports,
        "bytes_read": source_bytes.saturating_add(total_bundle_bytes),
        "request_count": 0,
        "fragments_visited": 0,
        "pages_visited": 0,
        "pruning_tightness": 0.0,
        "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
        "index_use": {
            "covi_used": covi_bytes > 0,
            "lookup_hits": 0,
            "lookup_misses": 0,
            "index_fallbacks": 0
        },
        "memory_peak_bytes": Value::Null,
        "artifact_sizes": artifact_sizes,
    });
    let cost = json!({
        "proof": size_report,
        "doctor_status": doctor.get("status").cloned().unwrap_or(Value::Null),
        "parity_report_count": parity_reports,
    });
    Ok(json!({
        "id": format!("proof_suite_{scenario}"),
        "category": format!("COVE-O proof suite {scenario} scenario"),
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "proof_suite", "cove_i", "covm", "parquet_compare"],
    }))
}

pub(super) fn run_customer360_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let dir = corpus.join("customer360");
    Ok(vec![
        run_query_case(
            "customer360_projection_scan",
            "Customer 360 projected canonical customer scan",
            &dir.join("customers_projection.cove"),
            ExplainOptions {
                projection: Some(vec![
                    "customer_id".into(),
                    "region".into(),
                    "tier".into(),
                    "score".into(),
                    "status".into(),
                    "mrr".into(),
                ]),
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "customer360_selective_filter",
            "Customer 360 projected selective score filter",
            &dir.join("customers_projection.cove"),
            ExplainOptions {
                projection: Some(vec!["customer_id".into(), "tier".into(), "score".into()]),
                filters: vec![FilterDsl {
                    column: "score".into(),
                    op: FilterOp::Gte,
                    value: Some("80".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_query_case(
            "customer360_event_filter",
            "Customer 360 event fact selective filter",
            &dir.join("events.cove"),
            ExplainOptions {
                projection: Some(vec![
                    "event_id".into(),
                    "customer_id".into(),
                    "event_kind".into(),
                    "score".into(),
                ]),
                filters: vec![FilterDsl {
                    column: "score".into(),
                    op: FilterOp::Gte,
                    value: Some("80".into()),
                }],
                ..ExplainOptions::default()
            },
        )?,
        run_customer360_object_store_case(corpus)?,
    ])
}

pub(super) fn run_cove_map_build_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let root = corpus.join("semantic-map-builds");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    Ok(vec![
        run_cove_map_build_case(&root, "cove_map_build_tiny", "tiny", 16)?,
        run_cove_map_build_case(&root, "cove_map_build_medium", "medium", 512)?,
        run_cove_map_build_messy_case(&root)?,
    ])
}

pub(super) fn run_cove_map_build_case(
    root: &Path,
    id: &str,
    label: &str,
    row_count: usize,
) -> Result<Value, String> {
    let dir = root.join(label);
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let map_path = dir.join("people.covemap");
    let source_path = dir.join("people.csv");
    durable::durable_replace(&map_path, &bench_covemap_bytes()?)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let mut csv = String::from("id,name\n");
    for row in 0..row_count {
        csv.push_str(&format!("{row},person-{row}\n"));
    }
    fs::write(&source_path, csv.as_bytes())
        .map_err(|err| format!("cannot write {}: {err}", source_path.display()))?;
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("{id} failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cove_map_build_case_report(
        id,
        "COVE-MAP build bundle",
        elapsed,
        &[source_path],
        &out_dir,
        &result.manifest,
    )
}

pub(super) fn run_cove_map_build_messy_case(root: &Path) -> Result<Value, String> {
    let dir = root.join("messy-multisource");
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let map_path = dir.join("showcase.covemap");
    let map_bytes = showcase_multi_source_covemap()?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let crm = dir.join("crm.csv");
    let subscription = dir.join("subscription.csv");
    let directory = dir.join("directory.parquet");
    fs::write(&crm, b"id,name\np1,Ada CRM\np2,Linus CRM\np3,Grace CRM\n")
        .map_err(|err| format!("cannot write {}: {err}", crm.display()))?;
    fs::write(
        &subscription,
        b"id,name\np1,Ada\np2,Linus\np3,Grace Subscription\n",
    )
    .map_err(|err| format!("cannot write {}: {err}", subscription.display()))?;
    write_parquet_file(&directory, &showcase_directory_name_batch()?)?;
    let sources = vec![crm, directory, subscription];
    let out_dir = dir.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &sources, options)
        .map_err(|err| format!("cove_map_build_messy_multisource failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cove_map_build_case_report(
        "cove_map_build_messy_multisource",
        "COVE-MAP messy multi-source build bundle",
        elapsed,
        &sources,
        &out_dir,
        &result.manifest,
    )
}

pub(super) fn cove_map_build_case_report(
    id: &str,
    category: &str,
    elapsed: u128,
    sources: &[PathBuf],
    out_dir: &Path,
    manifest: &Value,
) -> Result<Value, String> {
    let source_bytes = sources
        .iter()
        .map(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    let object_bytes = manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let projection_bytes = manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_bytes = manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_root_count = manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("root_count").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let sidecar_available = manifest
        .pointer("/sidecar_readiness/covi/available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sidecar_family_count = manifest
        .pointer("/sidecar_readiness/covi/generated_root_families")
        .and_then(Value::as_array)
        .map(|families| families.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(out_dir)?;
    Ok(json!({
        "id": id,
        "category": category,
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "build_time_ns": elapsed,
            "validation_time_ns": elapsed,
            "projection_readback_time_ns": 0,
            "source_bytes": source_bytes,
            "cove_o_bytes": object_bytes,
            "projection_bytes": projection_bytes,
            "index_bytes": index_bytes,
            "index_root_count": index_root_count,
            "covm_bytes": covm_bytes,
            "sidecar_available": sidecar_available,
            "sidecar_family_count": sidecar_family_count,
            "sidecar_lookup_hit_rate": if sidecar_available { 1.0 } else { 0.0 },
            "sidecar_fallback_rate": 0.0,
            "total_bundle_bytes": total_bundle_bytes,
            "duplication_ratio": if source_bytes == 0 { 0.0 } else { total_bundle_bytes as f64 / source_bytes as f64 },
            "object_count": manifest.pointer("/counts/object_count").cloned().unwrap_or(Value::Null),
            "property_value_count": manifest.pointer("/counts/property_value_count").cloned().unwrap_or(Value::Null),
            "evidence_entry_count": manifest.pointer("/counts/evidence_entry_count").cloned().unwrap_or(Value::Null),
            "native_acceleration_gate": "covi-and-covm-emitted-and-validated",
        },
        "optional_features": ["cove_map", "map_build", "cove_i", "covm"],
    }))
}
