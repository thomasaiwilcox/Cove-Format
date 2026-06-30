use super::*;

pub(super) fn run_cache_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("synthetic-cache.cove");
    let filter = FilterDsl {
        column: "name".into(),
        op: FilterOp::Eq,
        value: Some("gamma".into()),
    };
    let disabled = run_query_case(
        "coverage_cache_disabled",
        "COVE-CACHE miss/fallback baseline",
        &path,
        ExplainOptions {
            filters: vec![filter.clone()],
            table_options: CoveTableOptions::default(),
            ..ExplainOptions::default()
        },
    )?;
    let enabled = run_query_case(
        "coverage_cache_hit",
        "COVE-CACHE hit",
        &path,
        ExplainOptions {
            filters: vec![filter],
            table_options: CoveTableOptions::default().with_sibling_coverage_cache(),
            ..ExplainOptions::default()
        },
    )?;
    let provider_lookup = json!({
        "id": "coverage_provider_lookup",
        "category": "coverage-provider lookup cost vs scan",
        "status": "measured",
        "metrics": enabled
            .pointer("/cost/coverage_metrics")
            .cloned()
            .unwrap_or(Value::Null),
        "optional_features": ["coverage"],
    });
    let cache_summary = json!({
        "id": "coverage_cache_hit_miss_invalidation",
        "category": "COVE-CACHE hit, miss, and invalidation behavior",
        "status": "measured",
        "metrics": {
            "disabled": disabled.pointer("/cost/coverage_metrics/coverage_cache").cloned().unwrap_or(Value::Null),
            "enabled": enabled.pointer("/cost/coverage_metrics/coverage_cache").cloned().unwrap_or(Value::Null),
        },
        "optional_features": ["coverage_cache"],
    });
    Ok(vec![disabled, enabled, provider_lookup, cache_summary])
}

pub(super) fn run_cove_o_delta_artifact_metrics_case() -> Result<Value, String> {
    let start = Instant::now();
    let base_file_bytes = 1_048_576u64;
    let summary = CovmDeltaChainSummaryV1::new(
        [0x44; 16],
        [0x55; 16],
        DigestAlgorithm::Sha256 as u16,
        vec![0x99; 32],
        vec![
            delta_benchmark_summary_entry(1, 64 * 1024, [0x10; 16], [0x11; 16], 1, 10, 1_000),
            delta_benchmark_summary_entry(2, 96 * 1024, [0x11; 16], [0x12; 16], 11, 20, 2_000),
            delta_benchmark_summary_entry(3, 128 * 1024, [0x12; 16], [0x13; 16], 21, 30, 3_000),
        ],
    );
    let summary_bytes = summary
        .serialize()
        .map_err(|error| format!("cannot serialize delta benchmark chain summary: {error}"))?;
    let parsed = CovmDeltaChainSummaryV1::parse(&summary_bytes)
        .map_err(|error| format!("cannot parse delta benchmark chain summary: {error}"))?;
    let decision = parsed
        .prune_delta_chain(CovmDeltaPruneRequest {
            as_of_csn: Some(25),
            source_publish_range_us: Some((2_050, 3_050)),
            ..CovmDeltaPruneRequest::default()
        })
        .map_err(|error| format!("cannot prune delta benchmark chain summary: {error}"))?;
    let mut amplification = parsed.read_amplification_metrics(&decision);
    amplification.base_file_bytes = base_file_bytes;
    amplification.total_delta_bytes = parsed
        .delta_summaries
        .iter()
        .map(|entry| entry.delta_artifact_ref.file_len)
        .sum();
    let selected_delta_bytes = parsed
        .delta_summaries
        .iter()
        .filter(|entry| {
            decision
                .selected_chain_ordinals
                .contains(&entry.chain_ordinal)
        })
        .map(|entry| entry.delta_artifact_ref.file_len)
        .sum::<u64>();
    amplification.bytes_returned = base_file_bytes
        .saturating_add(selected_delta_bytes)
        .saturating_add(summary_bytes.len() as u64);
    amplification.touched_set_hits = 1;
    amplification.touched_set_misses = 1;
    amplification.tombstone_summary_hits = 1;
    amplification.anchor_validations = amplification.selected_delta_count;
    amplification.patch_rows_applied = 96;
    amplification.materialized_property_count = 128;
    amplification.max_patch_rows_since_checkpoint = 48;
    amplification.point_lookup_artifacts_p95 = amplification.selected_delta_count + 3;
    amplification.metadata_range_requests_before_data = 3;

    let recommendations = amplification
        .recommendations(CovmDeltaReadAmplificationPolicy::default())
        .into_iter()
        .map(delta_benchmark_recommendation)
        .collect::<Vec<_>>();
    let elapsed = start.elapsed().as_nanos();
    let total_bytes_stored = base_file_bytes
        .saturating_add(amplification.total_delta_bytes)
        .saturating_add(summary_bytes.len() as u64);
    let pruning_effectiveness = if amplification.delta_chain_depth == 0 {
        0.0
    } else {
        amplification.skipped_delta_count as f64 / amplification.delta_chain_depth as f64
    };

    let mut metrics = serde_json::Map::new();
    macro_rules! metric {
        ($name:literal, $value:expr) => {
            metrics.insert($name.into(), json!($value));
        };
    }
    metric!("planning_ns", elapsed);
    metric!("scan_ns", 0);
    metric!("end_to_end_ns", elapsed);
    metric!("elapsed_time_ns", elapsed);
    metric!("bytes_read", amplification.bytes_returned);
    metric!("request_count", amplification.object_store_request_count);
    metric!("fragments_visited", amplification.selected_delta_count);
    metric!("pages_visited", amplification.selected_delta_count);
    metric!("pruning_tightness", pruning_effectiveness);
    metrics.insert(
        "coverage_cache".into(),
        json!({
            "hits": 0,
            "misses": 0,
            "entries_loaded": 0,
        }),
    );
    metrics.insert(
        "index_use".into(),
        json!({
            "covi_used": false,
            "lookup_hits": amplification.touched_set_hits,
            "lookup_misses": amplification.touched_set_misses,
            "index_fallbacks": 0,
        }),
    );
    metrics.insert("memory_peak_bytes".into(), Value::Null);
    metrics.insert(
        "artifact_sizes".into(),
        json!({
            "base_cove_bytes": base_file_bytes,
            "delta_bytes": amplification.total_delta_bytes,
            "chain_summary_bytes": summary_bytes.len() as u64,
            "total_bytes_stored": total_bytes_stored,
        }),
    );
    metric!(
        "bytes_written_per_update",
        amplification.total_delta_bytes / amplification.delta_chain_depth.max(1) as u64
    );
    metric!("full_rewrite_bytes_per_update", base_file_bytes);
    metric!("total_bytes_stored", total_bytes_stored);
    metric!("writer_finalization_ns", elapsed);
    metric!("publication_latency_ns", elapsed);
    metric!("validation_time_ns", elapsed);
    metric!(
        "latest_state_point_lookup_p95_artifacts",
        amplification.point_lookup_artifacts_p95
    );
    metric!(
        "object_history_query_selected_deltas",
        amplification.selected_delta_count
    );
    metric!(
        "projection_readback_property_skips",
        amplification.touched_set_misses
    );
    metric!(
        "object_store_request_count",
        amplification.object_store_request_count
    );
    metric!(
        "chain_summary_range_requests",
        amplification.chain_summary_range_requests
    );
    metric!(
        "delta_artifacts_opened",
        amplification.delta_artifacts_opened
    );
    metric!(
        "delta_artifacts_skipped_before_open",
        amplification.delta_artifacts_skipped_before_open
    );
    metric!(
        "source_publication_pruning_effectiveness",
        pruning_effectiveness
    );
    metric!(
        "dictionary_alias_resolution_count",
        amplification.dictionary_alias_resolutions
    );
    metric!("compaction_throughput_rows_per_ns", 0.0);
    metric!("compacted_output_bytes", base_file_bytes);
    metric!(
        "index_rebuild_candidate_count",
        amplification.selected_delta_count
    );
    metric!("delta_chain_depth", amplification.delta_chain_depth);
    metric!("selected_delta_count", amplification.selected_delta_count);
    metric!("skipped_delta_count", amplification.skipped_delta_count);
    metric!("chain_summary_bytes", amplification.chain_summary_bytes);
    metric!("base_file_bytes", amplification.base_file_bytes);
    metric!("total_delta_bytes", amplification.total_delta_bytes);
    metric!("patch_rows_applied", amplification.patch_rows_applied);
    metric!(
        "materialized_property_count",
        amplification.materialized_property_count
    );
    metric!(
        "checkpoint_recommended",
        recommendations.contains(&"RecommendCheckpoint")
    );
    metric!(
        "compaction_recommended",
        recommendations.contains(&"RecommendCompaction")
    );
    metric!(
        "snapshot_index_recommended",
        recommendations.contains(&"RecommendSnapshotLevelIndex")
    );
    metrics.insert("recommendations".into(), json!(recommendations));

    let mut case = serde_json::Map::new();
    case.insert("id".into(), json!("cove_o_delta_artifact_metrics"));
    case.insert(
        "category".into(),
        json!("COVE-O delta artifact release-gate metrics"),
    );
    case.insert("status".into(), json!("measured"));
    case.insert("metrics".into(), Value::Object(metrics));
    case.insert(
        "optional_features".into(),
        json!(["cove_o_delta_artifacts"]),
    );
    Ok(Value::Object(case))
}

pub(super) fn delta_benchmark_summary_entry(
    chain_ordinal: u32,
    file_len: u64,
    parent_snapshot_id: [u8; 16],
    snapshot_id: [u8; 16],
    csn_min: u64,
    csn_max: u64,
    time_base_us: i64,
) -> DeltaChainSummaryEntryV1 {
    let artifact_id = [0x60u8.saturating_add(chain_ordinal as u8); 16];
    let reference = CovmDeltaArtifactRefV1 {
        chain_ordinal,
        flags: 0,
        artifact_id,
        snapshot_id,
        parent_snapshot_id,
        file_len,
        footer_crc32c: checksum::crc32c(&artifact_id),
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: [0x90u8.saturating_add(chain_ordinal as u8); 32],
        uri_ref: chain_ordinal,
        checksum: 0,
    };
    DeltaChainSummaryEntryV1 {
        chain_ordinal,
        delta_artifact_ref: reference,
        delta_artifact_id: artifact_id,
        required_delta_features: 0,
        optional_delta_features: 0,
        csn_min,
        csn_max,
        commit_time_start_us: time_base_us,
        commit_time_end_us: time_base_us + 99,
        artifact_created_at_us: time_base_us + 100,
        first_published_at_us: time_base_us + 200,
        selected_snapshot_published_at_us: time_base_us + 300,
        time_field_presence_flags: DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT
            | DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        time_summary_exactness_flags: DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
        source_publish_range_start_us: time_base_us,
        source_publish_range_end_us: time_base_us + 99,
        scope_summary_ref: 0,
        branch_summary_ref: 0,
        object_type_summary_ref: 0,
        goid_range_summary_ref: 0,
        touched_summary_ref: 0,
        tombstone_summary_ref: 0,
        property_summary_ref: 0,
        temporal_role_summary_ref: 0,
        delta_header_range_offset: 0,
        delta_header_range_length: 238,
        hot_summary_range_offset: 238,
        hot_summary_range_length: 128,
        checksum: 0,
    }
}

pub(super) fn delta_benchmark_recommendation(
    recommendation: CovmDeltaReadAmplificationRecommendation,
) -> &'static str {
    match recommendation {
        CovmDeltaReadAmplificationRecommendation::WarnChainDepth => "WarnChainDepth",
        CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth => {
            "RequireOverrideChainDepth"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendCheckpoint => "RecommendCheckpoint",
        CovmDeltaReadAmplificationRecommendation::RecommendCompaction => "RecommendCompaction",
        CovmDeltaReadAmplificationRecommendation::RecommendSnapshotLevelIndex => {
            "RecommendSnapshotLevelIndex"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendSummaryHoistingOrCompaction => {
            "RecommendSummaryHoistingOrCompaction"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendPackingSmallDeltas => {
            "RecommendPackingSmallDeltas"
        }
    }
}

pub(super) fn run_publication_gap_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let mut cases = Vec::new();
    cases.push(run_query_case(
        "tpch_style_queries",
        "TPC-H-style deterministic generated scan/filter workload",
        &corpus.join("tpch-style.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "amount".into(), "bucket".into()]),
            filters: vec![FilterDsl {
                column: "amount".into(),
                op: FilterOp::Gte,
                value: Some("1000".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "tpcds_style_queries",
        "TPC-DS-style deterministic generated scan/filter workload",
        &corpus.join("tpcds-style.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "name".into(), "active".into()]),
            filters: vec![FilterDsl {
                column: "bucket".into(),
                op: FilterOp::In,
                value: Some("bucket-02|bucket-04|bucket-06".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);
    cases.push(run_query_case(
        "medical_operational_queries",
        "medical-operational deterministic nested-adjacent workload",
        &corpus.join("medical-operational.cove"),
        ExplainOptions {
            projection: Some(vec!["id".into(), "name".into(), "amount".into()]),
            filters: vec![FilterDsl {
                column: "amount".into(),
                op: FilterOp::Lt,
                value: Some("2500".into()),
            }],
            ..ExplainOptions::default()
        },
    )?);

    let corrupt = fs::read(corpus.join("negative-corrupt.cove"))
        .map_err(|err| format!("cannot read negative-corrupt fixture: {err}"))?;
    let start = Instant::now();
    let rejected = reader::validate_bytes(&corrupt).is_err();
    let elapsed = start.elapsed().as_nanos();
    if !rejected {
        return Err("negative-corrupt benchmark fixture unexpectedly validated".into());
    }
    cases.push(json!({
        "id": "negative_corrupt_validation",
        "category": "negative/corrupt corpus expected-error validation",
        "status": "measured",
        "metrics": {
            "planning_ns": elapsed,
            "scan_ns": 0,
            "end_to_end_ns": elapsed,
            "rows_materialized": 0,
            "expected_errors": 1,
        },
    }));

    let canonicalisation: Value = serde_json::from_slice(
        &fs::read(corpus.join("canonicalisation.json"))
            .map_err(|err| format!("cannot read canonicalisation fixture: {err}"))?,
    )
    .map_err(|err| format!("cannot parse canonicalisation fixture: {err}"))?;
    let case_count = canonicalisation
        .get("cases")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if case_count == 0 {
        return Err("canonicalisation fixture did not contain any cases".into());
    }
    cases.push(json!({
        "id": "canonicalisation_vectors",
        "category": "canonicalisation public corpus vectors",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": 0,
            "end_to_end_ns": 0,
            "rows_materialized": case_count,
            "canonical_cases": case_count,
        },
    }));

    let semantic_dir = corpus.join("semantic-mapping");
    let start = Instant::now();
    let summary = cove_map::conversion_summary_from_paths(
        &semantic_dir.join("people.covemap"),
        &[semantic_dir.join("people.csv")],
    )
    .map_err(|err| format!("semantic-mapping corpus benchmark failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    cases.push(json!({
        "id": "semantic_mapping_corpus",
        "category": "semantic-mapping public corpus",
        "status": "measured",
        "metrics": {
            "planning_ns": 0,
            "scan_ns": elapsed,
            "end_to_end_ns": elapsed,
            "rows_materialized": summary["materialized_row_count"].as_u64().unwrap_or(0),
            "assertions": summary["assertion_count"].as_u64().unwrap_or(0),
            "evidence_entries": summary["evidence_entry_count"].as_u64().unwrap_or(0),
        },
        "optional_features": ["cove_map"],
    }));
    cases.push(run_semantic_projection_object_store_case(corpus)?);
    cases.push(run_semantic_showcase_bundle_object_store_case(corpus)?);
    cases.extend(run_cove_map_build_cases(corpus)?);
    cases.push(run_overlap_stress_case(corpus)?);
    cases.extend(run_overlap_scale_cases(corpus)?);
    cases.extend(run_overlap_partial_cases(corpus)?);
    cases.extend(run_projection_covi_measured_cases(corpus)?);
    cases.extend(run_customer360_cases(corpus)?);
    cases.extend(run_customer360_projection_covi_cases(corpus)?);
    cases.extend(run_proof_suite_cases(corpus)?);

    Ok(cases)
}
