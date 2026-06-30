use super::*;

pub(super) fn run_query_case(
    id: &str,
    category: &str,
    path: &Path,
    options: ExplainOptions,
) -> Result<Value, String> {
    let plan_start = Instant::now();
    let planned = plan_local_file(path, options).map_err(|err| err.to_string())?;
    let planning_ns = plan_start.elapsed().as_nanos();
    let scan_start = Instant::now();
    let decoded = execute_planned_scan(&planned).map_err(|err| err.to_string())?;
    let scan_ns = scan_start.elapsed().as_nanos();
    let mut cost =
        cove_datafusion::explain::cost_report(&planned, Some(decoded.stats)).to_json_value();
    attach_covi_sidecar_metrics(&mut cost, planned.state.bootstrap_stats());
    Ok(json!({
        "id": id,
        "category": category,
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": scan_ns,
            "end_to_end_ns": planning_ns + scan_ns,
            "batches": decoded.batches.len(),
            "rows_materialized": decoded.batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        },
        "cost": cost,
    }))
}

pub(super) fn run_orc_readback_case(path: &Path) -> Result<Value, String> {
    let start = Instant::now();
    let file =
        fs::File::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    let builder = OrcReaderBuilder::try_new(file)
        .map_err(|err| format!("cannot open ORC {}: {err}", path.display()))?;
    let columns = builder.schema().fields().len();
    let batches = builder
        .with_batch_size(4096)
        .build()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("cannot read ORC batches: {err}"))?;
    let scan_ns = start.elapsed().as_nanos();
    let rows = batches.iter().map(|batch| batch.num_rows()).sum::<usize>();
    Ok(json!({
        "id": "orc_full_scan_readback",
        "category": "ORC full-scan materialisation/readback",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": scan_ns,
            "end_to_end_ns": scan_ns,
            "rows_materialized": rows,
            "columns_materialized": columns,
            "orc_bytes": fs::metadata(path).map_err(|err| err.to_string())?.len(),
        },
        "optional_features": ["orc_compare"],
    }))
}

pub(super) fn run_covi_index_only_count_case(path: &Path) -> Result<Value, String> {
    let options = CoveTableOptions::default().with_covi_discovery(CoviDiscovery::SiblingExtension);
    let start = Instant::now();
    let state = bootstrap_local_file_with_options(path, options).map_err(|err| err.to_string())?;
    let plan = exact_unfiltered_counts(state.as_ref(), &[None])
        .map_err(|err| err.to_string())?
        .ok_or_else(|| {
            "COVE-I exact COUNT did not produce a metadata aggregate plan".to_string()
        })?;
    let planning_ns = start.elapsed().as_nanos();
    let proof = plan.proof().clone();
    if proof.kind != MetadataAggregateProofKind::CoviIndexOnlyCount {
        return Err(format!(
            "COVE-I exact COUNT used {:?} instead of CoviIndexOnlyCount",
            proof.kind
        ));
    }
    let counts = match &plan {
        MetadataAggregatePlan::ScalarCounts { counts, .. } => counts,
        _ => return Err("COVE-I exact COUNT returned a non-count aggregate plan".into()),
    };
    let stats = state.bootstrap_stats();
    if stats.covi_sidecars_loaded == 0 {
        return Err("COVE-I exact COUNT did not load a COVI sidecar".into());
    }
    Ok(json!({
        "id": "covi_index_only_count",
        "category": "COVE-I exact index-only COUNT",
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": 0,
            "end_to_end_ns": planning_ns,
            "batches": 0,
            "rows_materialized": plan.output_rows(),
            "count": counts.first().copied().unwrap_or(0),
        },
        "cost": {
            "coverage_metrics": {
                "covi_used": true,
                "covi": {
                    "loaded": stats.covi_sidecars_loaded,
                    "stale": stats.covi_sidecars_stale,
                    "ignored": stats.covi_sidecars_ignored,
                    "candidate_pruned": stats.covi_candidate_pruned,
                    "index_only_answers": 1,
                },
            },
        },
        "proof": {
            "kind": format!("{:?}", proof.kind),
            "reason": proof.reason,
        },
        "optional_features": ["covi"],
    }))
}

pub(super) fn run_metadata_count_min_max_case(path: &Path) -> Result<Value, String> {
    let options = CoveTableOptions::default().with_covi_discovery(CoviDiscovery::SiblingExtension);
    let start = Instant::now();
    let state = bootstrap_local_file_with_options(path, options).map_err(|err| err.to_string())?;
    let counts = exact_unfiltered_counts(state.as_ref(), &[None, Some(1)])
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "metadata count did not produce an exact plan".to_string())?;
    let min_max = exact_unfiltered_aggregate_synopses(
        state.as_ref(),
        &[
            (1, MetadataSynopsisAggregateKind::Min),
            (1, MetadataSynopsisAggregateKind::Max),
        ],
    )
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "metadata min/max did not produce an exact synopsis plan".to_string())?;
    let planning_ns = start.elapsed().as_nanos();
    let count_values = match &counts {
        MetadataAggregatePlan::ScalarCounts { counts, .. } => counts.clone(),
        _ => return Err("metadata count returned a non-count plan".into()),
    };
    let min_max_values = match &min_max {
        MetadataAggregatePlan::ScalarValues { values, .. } => values.len(),
        _ => return Err("metadata min/max returned a non-value plan".into()),
    };
    Ok(json!({
        "id": "metadata_count_min_max",
        "category": "metadata-only count/min/max",
        "status": "measured",
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": 0,
            "end_to_end_ns": planning_ns,
            "rows_materialized": 1,
            "count_values": count_values,
            "min_max_values": min_max_values,
        },
        "proofs": {
            "count": format!("{:?}", counts.proof().kind),
            "min_max": format!("{:?}", min_max.proof().kind),
        },
    }))
}

#[derive(Debug, Default, Clone)]
pub(super) struct OfflineObjectStoreStats {
    pub(super) object_gets: u64,
    pub(super) range_gets: u64,
    pub(super) bytes_requested: u64,
    pub(super) bytes_returned: u64,
    pub(super) cache_hits: u64,
    pub(super) cache_misses: u64,
    pub(super) original_ranges: u64,
    pub(super) coalesced_ranges: u64,
}

#[derive(Debug, Default)]
pub(super) struct OfflineObjectStoreHarness {
    objects: BTreeMap<String, Vec<u8>>,
    range_cache: BTreeSet<(String, u64, u64)>,
    stats: OfflineObjectStoreStats,
}

impl OfflineObjectStoreHarness {
    pub(super) fn put_object(&mut self, key: impl Into<String>, bytes: Vec<u8>) {
        self.objects.insert(key.into(), bytes);
    }

    fn get_object(&mut self, key: &str) -> Result<Vec<u8>, String> {
        let bytes = self
            .objects
            .get(key)
            .ok_or_else(|| format!("offline object {key:?} does not exist"))?
            .clone();
        self.stats.object_gets = self.stats.object_gets.saturating_add(1);
        self.stats.bytes_requested = self
            .stats
            .bytes_requested
            .saturating_add(bytes.len() as u64);
        self.stats.bytes_returned = self.stats.bytes_returned.saturating_add(bytes.len() as u64);
        Ok(bytes)
    }

    fn range_get(&mut self, key: &str, range: Range<u64>) -> Result<Vec<u8>, String> {
        let bytes = self
            .objects
            .get(key)
            .ok_or_else(|| format!("offline object {key:?} does not exist"))?;
        if range.start > range.end || range.end as usize > bytes.len() {
            return Err(format!(
                "range {}..{} is outside object {key:?} length {}",
                range.start,
                range.end,
                bytes.len()
            ));
        }
        let len = range.end.saturating_sub(range.start);
        self.stats.range_gets = self.stats.range_gets.saturating_add(1);
        self.stats.bytes_requested = self.stats.bytes_requested.saturating_add(len);
        let cache_key = (key.to_string(), range.start, range.end);
        if self.range_cache.insert(cache_key) {
            self.stats.cache_misses = self.stats.cache_misses.saturating_add(1);
            self.stats.bytes_returned = self.stats.bytes_returned.saturating_add(len);
        } else {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
        }
        Ok(bytes[range.start as usize..range.end as usize].to_vec())
    }

    pub(super) fn take_stats(&mut self) -> OfflineObjectStoreStats {
        std::mem::take(&mut self.stats)
    }
}

pub(super) fn deterministic_object_ranges(file_len: u64) -> Vec<Range<u64>> {
    let mut ranges = Vec::new();
    let mut push = |start: u64, end: u64| {
        if start < end
            && !ranges
                .iter()
                .any(|range: &Range<u64>| range.start == start && range.end == end)
        {
            ranges.push(start..end);
        }
    };
    push(0, file_len.min(4096));
    push(4096.min(file_len), file_len.min(8192));
    let middle = file_len / 2;
    push(middle, middle.saturating_add(4096).min(file_len));
    push(file_len.saturating_sub(4096), file_len);
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges
}

pub(super) fn coalesce_object_ranges(
    ranges: &[Range<u64>],
    max_gap: u64,
    max_span: u64,
) -> Vec<Range<u64>> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|range| (range.start, range.end));
    let mut coalesced: Vec<Range<u64>> = Vec::new();
    for range in sorted {
        let Some(last) = coalesced.last_mut() else {
            coalesced.push(range);
            continue;
        };
        let gap = range.start.saturating_sub(last.end);
        let span = range.end.saturating_sub(last.start);
        if range.start <= last.end || (gap <= max_gap && span <= max_span) {
            last.end = last.end.max(range.end);
        } else {
            coalesced.push(range);
        }
    }
    coalesced
}

pub(super) fn read_harness_ranges(
    harness: &mut OfflineObjectStoreHarness,
    key: &str,
    ranges: &[Range<u64>],
) -> Result<(), String> {
    for range in ranges {
        harness.range_get(key, range.clone())?;
    }
    Ok(())
}

pub(super) fn object_store_stats_json(stats: &OfflineObjectStoreStats) -> Value {
    json!({
        "object_gets": stats.object_gets,
        "range_gets": stats.range_gets,
        "bytes_requested": stats.bytes_requested,
        "bytes_returned": stats.bytes_returned,
        "cache_hits": stats.cache_hits,
        "cache_misses": stats.cache_misses,
        "original_ranges": stats.original_ranges,
        "coalesced_ranges": stats.coalesced_ranges,
    })
}

pub(super) fn run_object_store_cold_warm_case(corpus: &Path, path: &Path) -> Result<Value, String> {
    let options = ExplainOptions {
        projection: Some(vec!["id".into(), "amount".into()]),
        filters: vec![FilterDsl {
            column: "amount".into(),
            op: FilterOp::Gte,
            value: Some("1000".into()),
        }],
        ..ExplainOptions::default()
    };
    let cold = run_query_case(
        "object_store_cold_probe",
        "object-store cold probe",
        path,
        options.clone(),
    )?;
    let warm = run_query_case(
        "object_store_warm_probe",
        "object-store warm probe",
        path,
        options,
    )?;
    let events_bytes = fs::read(path).map_err(|err| format!("cannot read events object: {err}"))?;
    let mut harness = OfflineObjectStoreHarness::default();
    harness.put_object("events.cove", events_bytes.clone());
    if let Ok(covm_bytes) = fs::read(corpus.join("events.covm")) {
        harness.put_object("events.covm", covm_bytes);
        let _ = harness.get_object("events.covm")?;
    }
    let original_ranges = deterministic_object_ranges(events_bytes.len() as u64);
    let coalesced_ranges = coalesce_object_ranges(&original_ranges, 1024, 16 * 1024);
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, "events.cove", &coalesced_ranges)?;
    let cold_store = harness.take_stats();
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, "events.cove", &coalesced_ranges)?;
    let warm_store = harness.take_stats();

    let mut coverage_harness = OfflineObjectStoreHarness::default();
    let coverage_bytes = fs::read(corpus.join("synthetic-cache.cove"))
        .map_err(|err| format!("cannot read synthetic-cache object: {err}"))?;
    coverage_harness.put_object("synthetic-cache.cove", coverage_bytes.clone());
    let coverage_ranges = deterministic_object_ranges(coverage_bytes.len() as u64);
    let pruned_ranges: Vec<_> = coverage_ranges.into_iter().take(1).collect();
    coverage_harness.stats.original_ranges = 4;
    coverage_harness.stats.coalesced_ranges = pruned_ranges.len() as u64;
    read_harness_ranges(
        &mut coverage_harness,
        "synthetic-cache.cove",
        &pruned_ranges,
    )?;
    let coverage_store = coverage_harness.take_stats();

    Ok(json!({
        "id": "object_store_cold_warm",
        "category": "object-store cold and warm scans",
        "status": "measured",
        "metrics": {
            "planning_ns": case_u64(&cold, "/metrics/planning_ns") + case_u64(&warm, "/metrics/planning_ns"),
            "scan_ns": case_u64(&cold, "/metrics/scan_ns") + case_u64(&warm, "/metrics/scan_ns"),
            "end_to_end_ns": case_u64(&cold, "/metrics/end_to_end_ns") + case_u64(&warm, "/metrics/end_to_end_ns"),
            "rows_materialized": case_u64(&cold, "/metrics/rows_materialized") + case_u64(&warm, "/metrics/rows_materialized"),
            "cold": cold["metrics"].clone(),
            "warm": warm["metrics"].clone(),
            "object_store_requests": cold_store.range_gets + cold_store.object_gets + warm_store.range_gets + warm_store.object_gets,
            "object_store_bytes_requested": cold_store.bytes_requested + warm_store.bytes_requested,
            "object_store_bytes_returned": cold_store.bytes_returned + warm_store.bytes_returned,
        },
        "cost": {
            "cold": cold["cost"].clone(),
            "warm": warm["cost"].clone(),
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "cold": object_store_stats_json(&cold_store),
                "warm": object_store_stats_json(&warm_store),
                "coverage_pruned": object_store_stats_json(&coverage_store),
                "page_cluster": {
                    "original_ranges": original_ranges.len(),
                    "coalesced_ranges": coalesced_ranges.len(),
                    "request_reduction": original_ranges.len().saturating_sub(coalesced_ranges.len()),
                },
                "caveat": "Hermetic object-store semantics, not live S3 or MinIO performance.",
            },
        },
    }))
}

pub(super) fn run_semantic_projection_object_store_case(corpus: &Path) -> Result<Value, String> {
    let semantic_dir = corpus.join("semantic-mapping");
    let mapped_path = semantic_dir.join("people_mapped.cove");
    let cove_t_path = semantic_dir.join("people_projection.cove");
    let parquet_path = semantic_dir.join("people_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read semantic mapping mapped COVE-O: {err}"))?;
    let cove_t_bytes = fs::read(&cove_t_path)
        .map_err(|err| format!("cannot read semantic mapping projected COVE-T: {err}"))?;
    let parquet_bytes = fs::read(&parquet_path)
        .map_err(|err| format!("cannot read semantic mapping projected Parquet: {err}"))?;
    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("people_mapped.cove", mapped_bytes.clone())?;
    let (cove_t_cold, cove_t_warm, cove_t_original, cove_t_coalesced) =
        simulate_object_store_cold_warm("people_projection.cove", cove_t_bytes.clone())?;
    let (parquet_cold, parquet_warm, parquet_original, parquet_coalesced) =
        simulate_object_store_cold_warm("people_projection.parquet", parquet_bytes.clone())?;
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "semantic_projection_object_store_compare",
        "category": "semantic-mapping mapped COVE-O vs projected COVE-T vs Parquet object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": 512,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": cove_t_bytes.len(),
            "parquet_bytes": parquet_bytes.len(),
            "bytes_read": mapped_cold.bytes_requested + cove_t_cold.bytes_requested + parquet_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + cove_t_cold.range_gets + parquet_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": cove_t_bytes.len(),
                "parquet_bytes": parquet_bytes.len(),
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet": parquet_bytes.len() as i64 - mapped_bytes.len() as i64,
                "mapped_cold_request_delta": parquet_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta": parquet_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "bytes_saved_vs_parquet": parquet_bytes.len() as i64 - cove_t_bytes.len() as i64,
                "cold_request_delta": parquet_cold.range_gets as i64 - cove_t_cold.range_gets as i64,
                "cold_bytes_requested_delta": parquet_cold.bytes_requested as i64 - cove_t_cold.bytes_requested as i64,
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {
                        "original": mapped_original.len(),
                        "coalesced": mapped_coalesced.len(),
                    }
                },
                "projected_cove_t": {
                    "file_bytes": cove_t_bytes.len(),
                    "cold": object_store_stats_json(&cove_t_cold),
                    "warm": object_store_stats_json(&cove_t_warm),
                    "ranges": {
                        "original": cove_t_original.len(),
                        "coalesced": cove_t_coalesced.len(),
                    }
                },
                "parquet": {
                    "file_bytes": parquet_bytes.len(),
                    "cold": object_store_stats_json(&parquet_cold),
                    "warm": object_store_stats_json(&parquet_warm),
                    "ranges": {
                        "original": parquet_original.len(),
                        "coalesced": parquet_coalesced.len(),
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness"],
    }))
}

pub(super) fn run_semantic_showcase_bundle_object_store_case(
    corpus: &Path,
) -> Result<Value, String> {
    let showcase_dir = corpus.join("semantic-showcase");
    let mapped_path = showcase_dir.join("showcase_mapped.cove");
    let people_cove_t_path = showcase_dir.join("people_projection.cove");
    let evidence_cove_t_path = showcase_dir.join("evidence_projection.cove");
    let people_parquet_path = showcase_dir.join("people_projection.parquet");
    let evidence_parquet_path = showcase_dir.join("evidence_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read semantic showcase mapped COVE-O: {err}"))?;
    let people_cove_t_bytes = fs::read(&people_cove_t_path)
        .map_err(|err| format!("cannot read semantic showcase people COVE-T: {err}"))?;
    let evidence_cove_t_bytes = fs::read(&evidence_cove_t_path)
        .map_err(|err| format!("cannot read semantic showcase evidence COVE-T: {err}"))?;
    let people_parquet_bytes = fs::read(&people_parquet_path)
        .map_err(|err| format!("cannot read semantic showcase people Parquet: {err}"))?;
    let evidence_parquet_bytes = fs::read(&evidence_parquet_path)
        .map_err(|err| format!("cannot read semantic showcase evidence Parquet: {err}"))?;

    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("showcase_mapped.cove", mapped_bytes.clone())?;
    let (people_cove_t_cold, people_cove_t_warm, people_cove_t_original, people_cove_t_coalesced) =
        simulate_object_store_cold_warm("people_projection.cove", people_cove_t_bytes.clone())?;
    let (
        evidence_cove_t_cold,
        evidence_cove_t_warm,
        evidence_cove_t_original,
        evidence_cove_t_coalesced,
    ) = simulate_object_store_cold_warm("evidence_projection.cove", evidence_cove_t_bytes.clone())?;
    let (
        people_parquet_cold,
        people_parquet_warm,
        people_parquet_original,
        people_parquet_coalesced,
    ) = simulate_object_store_cold_warm("people_projection.parquet", people_parquet_bytes.clone())?;
    let (
        evidence_parquet_cold,
        evidence_parquet_warm,
        evidence_parquet_original,
        evidence_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "evidence_projection.parquet",
        evidence_parquet_bytes.clone(),
    )?;

    let projected_cove_t_cold =
        sum_offline_object_store_stats(&[people_cove_t_cold.clone(), evidence_cove_t_cold.clone()]);
    let projected_cove_t_warm =
        sum_offline_object_store_stats(&[people_cove_t_warm.clone(), evidence_cove_t_warm.clone()]);
    let parquet_bundle_cold = sum_offline_object_store_stats(&[
        people_parquet_cold.clone(),
        evidence_parquet_cold.clone(),
    ]);
    let parquet_bundle_warm = sum_offline_object_store_stats(&[
        people_parquet_warm.clone(),
        evidence_parquet_warm.clone(),
    ]);

    let projected_cove_t_bytes = people_cove_t_bytes.len() + evidence_cove_t_bytes.len();
    let parquet_bundle_bytes = people_parquet_bytes.len() + evidence_parquet_bytes.len();
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "semantic_showcase_bundle_object_store_compare",
        "category": "semantic-showcase mapped COVE-O vs projected bundle vs Parquet bundle object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": 8,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": projected_cove_t_bytes,
            "parquet_bytes": parquet_bundle_bytes,
            "bytes_read": mapped_cold.bytes_requested + projected_cove_t_cold.bytes_requested + parquet_bundle_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + projected_cove_t_cold.range_gets + parquet_bundle_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": projected_cove_t_bytes,
                "parquet_bytes": parquet_bundle_bytes,
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - mapped_bytes.len() as i64,
                "projected_bundle_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - projected_cove_t_bytes as i64,
                "mapped_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "projected_bundle_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - projected_cove_t_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "projected_bundle_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - projected_cove_t_cold.bytes_requested as i64
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {
                        "original": mapped_original.len(),
                        "coalesced": mapped_coalesced.len(),
                    }
                },
                "projected_cove_t_bundle": {
                    "file_bytes": projected_cove_t_bytes,
                    "cold": object_store_stats_json(&projected_cove_t_cold),
                    "warm": object_store_stats_json(&projected_cove_t_warm),
                    "artifacts": {
                        "people_projection": {
                            "file_bytes": people_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&people_cove_t_cold),
                            "warm": object_store_stats_json(&people_cove_t_warm),
                            "ranges": {
                                "original": people_cove_t_original.len(),
                                "coalesced": people_cove_t_coalesced.len(),
                            }
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&evidence_cove_t_cold),
                            "warm": object_store_stats_json(&evidence_cove_t_warm),
                            "ranges": {
                                "original": evidence_cove_t_original.len(),
                                "coalesced": evidence_cove_t_coalesced.len(),
                            }
                        }
                    }
                },
                "parquet_bundle": {
                    "file_bytes": parquet_bundle_bytes,
                    "cold": object_store_stats_json(&parquet_bundle_cold),
                    "warm": object_store_stats_json(&parquet_bundle_warm),
                    "artifacts": {
                        "people_projection": {
                            "file_bytes": people_parquet_bytes.len(),
                            "cold": object_store_stats_json(&people_parquet_cold),
                            "warm": object_store_stats_json(&people_parquet_warm),
                            "ranges": {
                                "original": people_parquet_original.len(),
                                "coalesced": people_parquet_coalesced.len(),
                            }
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_parquet_bytes.len(),
                            "cold": object_store_stats_json(&evidence_parquet_cold),
                            "warm": object_store_stats_json(&evidence_parquet_warm),
                            "ranges": {
                                "original": evidence_parquet_original.len(),
                                "coalesced": evidence_parquet_coalesced.len(),
                            }
                        }
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness"],
    }))
}

pub(super) fn run_customer360_object_store_case(corpus: &Path) -> Result<Value, String> {
    let dir = corpus.join("customer360");
    let mapped_path = dir.join("customers.cove");
    let customers_cove_t_path = dir.join("customers_projection.cove");
    let evidence_cove_t_path = dir.join("evidence_projection.cove");
    let customers_parquet_path = dir.join("customers_projection.parquet");
    let evidence_parquet_path = dir.join("evidence_projection.parquet");
    let start = Instant::now();
    let mapped_bytes = fs::read(&mapped_path)
        .map_err(|err| format!("cannot read Customer 360 mapped COVE-O: {err}"))?;
    let customers_cove_t_bytes = fs::read(&customers_cove_t_path)
        .map_err(|err| format!("cannot read Customer 360 customers COVE-T: {err}"))?;
    let evidence_cove_t_bytes = fs::read(&evidence_cove_t_path)
        .map_err(|err| format!("cannot read Customer 360 evidence COVE-T: {err}"))?;
    let customers_parquet_bytes = fs::read(&customers_parquet_path)
        .map_err(|err| format!("cannot read Customer 360 customers Parquet: {err}"))?;
    let evidence_parquet_bytes = fs::read(&evidence_parquet_path)
        .map_err(|err| format!("cannot read Customer 360 evidence Parquet: {err}"))?;

    let (mapped_cold, mapped_warm, mapped_original, mapped_coalesced) =
        simulate_object_store_cold_warm("customer360/customers.cove", mapped_bytes.clone())?;
    let (
        customers_cove_t_cold,
        customers_cove_t_warm,
        customers_cove_t_original,
        customers_cove_t_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/customers_projection.cove",
        customers_cove_t_bytes.clone(),
    )?;
    let (
        evidence_cove_t_cold,
        evidence_cove_t_warm,
        evidence_cove_t_original,
        evidence_cove_t_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/evidence_projection.cove",
        evidence_cove_t_bytes.clone(),
    )?;
    let (
        customers_parquet_cold,
        customers_parquet_warm,
        customers_parquet_original,
        customers_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/customers_projection.parquet",
        customers_parquet_bytes.clone(),
    )?;
    let (
        evidence_parquet_cold,
        evidence_parquet_warm,
        evidence_parquet_original,
        evidence_parquet_coalesced,
    ) = simulate_object_store_cold_warm(
        "customer360/evidence_projection.parquet",
        evidence_parquet_bytes.clone(),
    )?;

    let projected_cove_t_cold = sum_offline_object_store_stats(&[
        customers_cove_t_cold.clone(),
        evidence_cove_t_cold.clone(),
    ]);
    let projected_cove_t_warm = sum_offline_object_store_stats(&[
        customers_cove_t_warm.clone(),
        evidence_cove_t_warm.clone(),
    ]);
    let parquet_bundle_cold = sum_offline_object_store_stats(&[
        customers_parquet_cold.clone(),
        evidence_parquet_cold.clone(),
    ]);
    let parquet_bundle_warm = sum_offline_object_store_stats(&[
        customers_parquet_warm.clone(),
        evidence_parquet_warm.clone(),
    ]);

    let projected_cove_t_bytes = customers_cove_t_bytes.len() + evidence_cove_t_bytes.len();
    let parquet_bundle_bytes = customers_parquet_bytes.len() + evidence_parquet_bytes.len();
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "id": "customer360_object_store_compare",
        "category": "Customer 360 mapped COVE-O vs projected COVE-T bundle vs Parquet bundle object-store comparison",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": Value::Null,
            "mapped_cove_o_bytes": mapped_bytes.len(),
            "cove_bytes": projected_cove_t_bytes,
            "parquet_bytes": parquet_bundle_bytes,
            "bytes_read": mapped_cold.bytes_requested + projected_cove_t_cold.bytes_requested + parquet_bundle_cold.bytes_requested,
            "request_count": mapped_cold.range_gets + projected_cove_t_cold.range_gets + parquet_bundle_cold.range_gets,
            "fragments_visited": 0,
            "pages_visited": 0,
            "pruning_tightness": 0.0,
            "coverage_cache": {"hits": 0, "misses": 0, "entries_loaded": 0},
            "index_use": {"covi_used": false, "lookup_hits": 0, "lookup_misses": 0, "index_fallbacks": 0},
            "memory_peak_bytes": Value::Null,
            "artifact_sizes": {
                "mapped_cove_o_bytes": mapped_bytes.len(),
                "cove_bytes": projected_cove_t_bytes,
                "parquet_bytes": parquet_bundle_bytes,
                "orc_bytes": 0,
                "covx_bytes": 0
            },
            "delta": {
                "mapped_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - mapped_bytes.len() as i64,
                "projected_bundle_bytes_saved_vs_parquet_bundle": parquet_bundle_bytes as i64 - projected_cove_t_bytes as i64,
                "mapped_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - mapped_cold.range_gets as i64,
                "projected_bundle_cold_request_delta_vs_parquet_bundle": parquet_bundle_cold.range_gets as i64 - projected_cove_t_cold.range_gets as i64,
                "mapped_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - mapped_cold.bytes_requested as i64,
                "projected_bundle_cold_bytes_requested_delta_vs_parquet_bundle": parquet_bundle_cold.bytes_requested as i64 - projected_cove_t_cold.bytes_requested as i64
            }
        },
        "cost": {
            "simulation": "offline deterministic object-store harness",
            "object_store_harness": {
                "mapped_cove_o": {
                    "file_bytes": mapped_bytes.len(),
                    "cold": object_store_stats_json(&mapped_cold),
                    "warm": object_store_stats_json(&mapped_warm),
                    "ranges": {"original": mapped_original.len(), "coalesced": mapped_coalesced.len()}
                },
                "projected_cove_t_bundle": {
                    "file_bytes": projected_cove_t_bytes,
                    "cold": object_store_stats_json(&projected_cove_t_cold),
                    "warm": object_store_stats_json(&projected_cove_t_warm),
                    "artifacts": {
                        "customers_projection": {
                            "file_bytes": customers_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&customers_cove_t_cold),
                            "warm": object_store_stats_json(&customers_cove_t_warm),
                            "ranges": {"original": customers_cove_t_original.len(), "coalesced": customers_cove_t_coalesced.len()}
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_cove_t_bytes.len(),
                            "cold": object_store_stats_json(&evidence_cove_t_cold),
                            "warm": object_store_stats_json(&evidence_cove_t_warm),
                            "ranges": {"original": evidence_cove_t_original.len(), "coalesced": evidence_cove_t_coalesced.len()}
                        }
                    }
                },
                "parquet_bundle": {
                    "file_bytes": parquet_bundle_bytes,
                    "cold": object_store_stats_json(&parquet_bundle_cold),
                    "warm": object_store_stats_json(&parquet_bundle_warm),
                    "artifacts": {
                        "customers_projection": {
                            "file_bytes": customers_parquet_bytes.len(),
                            "cold": object_store_stats_json(&customers_parquet_cold),
                            "warm": object_store_stats_json(&customers_parquet_warm),
                            "ranges": {"original": customers_parquet_original.len(), "coalesced": customers_parquet_coalesced.len()}
                        },
                        "evidence_projection": {
                            "file_bytes": evidence_parquet_bytes.len(),
                            "cold": object_store_stats_json(&evidence_parquet_cold),
                            "warm": object_store_stats_json(&evidence_parquet_warm),
                            "ranges": {"original": evidence_parquet_original.len(), "coalesced": evidence_parquet_coalesced.len()}
                        }
                    }
                },
                "caveat": "Hermetic object-store semantics for corpus artifacts, not live cloud storage performance."
            }
        },
        "optional_features": ["cove_map", "parquet_compare", "object_store_harness", "customer360"],
    }))
}

type ColdWarmRangeStats = (
    OfflineObjectStoreStats,
    OfflineObjectStoreStats,
    Vec<Range<u64>>,
    Vec<Range<u64>>,
);

pub(super) fn simulate_object_store_cold_warm(
    key: &str,
    bytes: Vec<u8>,
) -> Result<ColdWarmRangeStats, String> {
    let mut harness = OfflineObjectStoreHarness::default();
    harness.put_object(key, bytes.clone());
    let original_ranges = deterministic_object_ranges(bytes.len() as u64);
    let coalesced_ranges = coalesce_object_ranges(&original_ranges, 1024, 16 * 1024);
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, key, &coalesced_ranges)?;
    let cold = harness.take_stats();
    harness.stats.original_ranges = original_ranges.len() as u64;
    harness.stats.coalesced_ranges = coalesced_ranges.len() as u64;
    read_harness_ranges(&mut harness, key, &coalesced_ranges)?;
    let warm = harness.take_stats();
    Ok((cold, warm, original_ranges, coalesced_ranges))
}

pub(super) fn sum_offline_object_store_stats(
    stats: &[OfflineObjectStoreStats],
) -> OfflineObjectStoreStats {
    let mut total = OfflineObjectStoreStats::default();
    for stat in stats {
        total.object_gets = total.object_gets.saturating_add(stat.object_gets);
        total.range_gets = total.range_gets.saturating_add(stat.range_gets);
        total.bytes_requested = total.bytes_requested.saturating_add(stat.bytes_requested);
        total.bytes_returned = total.bytes_returned.saturating_add(stat.bytes_returned);
        total.cache_hits = total.cache_hits.saturating_add(stat.cache_hits);
        total.cache_misses = total.cache_misses.saturating_add(stat.cache_misses);
        total.original_ranges = total.original_ranges.saturating_add(stat.original_ranges);
        total.coalesced_ranges = total.coalesced_ranges.saturating_add(stat.coalesced_ranges);
    }
    total
}
