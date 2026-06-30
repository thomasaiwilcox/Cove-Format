use super::*;

pub(super) fn attach_covi_sidecar_metrics(
    cost: &mut Value,
    stats: cove_datafusion::dataset_state::DatasetBootstrapStats,
) {
    if let Some(metrics) = cost
        .get_mut("coverage_metrics")
        .and_then(Value::as_object_mut)
    {
        metrics.insert(
            "covi".into(),
            json!({
                "loaded": stats.covi_sidecars_loaded,
                "stale": stats.covi_sidecars_stale,
                "ignored": stats.covi_sidecars_ignored,
                "candidate_pruned": stats.covi_candidate_pruned,
                "index_only_answers": stats.covi_index_only_answers,
            }),
        );
    }
}

pub(super) fn normalize_case_metrics(case: &mut Value) {
    let Some(object) = case.as_object_mut() else {
        return;
    };
    let cost = object.get("cost").cloned().unwrap_or(Value::Null);
    let metrics = object
        .entry("metrics")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("metrics object");
    let planning = metrics
        .get("planning_ns")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let scan = metrics.get("scan_ns").and_then(Value::as_u64).unwrap_or(0);
    let elapsed = metrics
        .get("end_to_end_ns")
        .and_then(Value::as_u64)
        .unwrap_or(planning.saturating_add(scan));
    metrics.entry("end_to_end_ns").or_insert(json!(elapsed));
    metrics.entry("elapsed_time_ns").or_insert(json!(elapsed));
    let metadata_bytes = cost
        .pointer("/observed/metadata_bytes_read")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let data_bytes = cost
        .pointer("/observed/data_bytes_read")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let bytes_read = metadata_bytes.saturating_add(data_bytes);
    metrics.entry("bytes_read").or_insert(json!(bytes_read));
    let request_count = cost
        .pointer("/observed/range_requests")
        .and_then(Value::as_u64)
        .or_else(|| {
            cost.pointer("/range_plan/original_range_requests")
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);
    metrics
        .entry("request_count")
        .or_insert(json!(request_count));
    metrics.entry("fragments_visited").or_insert(json!(cost
        .pointer("/observed/scan_tasks")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("pages_visited").or_insert(json!(cost
        .pointer("/observed/pages_decoded")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    let considered = cost
        .pointer("/observed/morsels_considered")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let pruned = cost
        .pointer("/observed/morsels_pruned")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    metrics
        .entry("pruning_tightness")
        .or_insert(json!(if considered == 0 {
            0.0
        } else {
            pruned as f64 / considered as f64
        }));
    metrics.entry("coverage_cache").or_insert_with(|| {
        cost.pointer("/coverage_metrics/coverage_cache")
            .cloned()
            .unwrap_or(json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }))
    });
    metrics.entry("coverage_cache_hit").or_insert(json!(cost
        .pointer("/coverage_metrics/coverage_cache/hits")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("coverage_cache_miss").or_insert(json!(cost
        .pointer("/coverage_metrics/coverage_cache/misses")
        .and_then(Value::as_u64)
        .unwrap_or(0)));
    metrics.entry("index_use").or_insert(json!({
        "covi_used": cost.pointer("/coverage_metrics/covi_used").and_then(Value::as_bool).unwrap_or(false),
        "lookup_hits": cost.pointer("/observed/lookup_index_hits").and_then(Value::as_u64).unwrap_or(0),
        "lookup_misses": cost.pointer("/observed/lookup_index_misses").and_then(Value::as_u64).unwrap_or(0),
        "index_fallbacks": cost.pointer("/observed/index_fallbacks").and_then(Value::as_u64).unwrap_or(0),
    }));
    metrics.entry("memory_peak_bytes").or_insert(Value::Null);
    let artifact_sizes = json!({
        "cove_bytes": metrics.get("cove_bytes").and_then(Value::as_u64).unwrap_or(0),
        "parquet_bytes": metrics.get("parquet_bytes").and_then(Value::as_u64).unwrap_or(0),
        "orc_bytes": metrics.get("orc_bytes").and_then(Value::as_u64).unwrap_or(0),
        "covx_bytes": metrics.get("covx_bytes").and_then(Value::as_u64).unwrap_or(0),
    });
    metrics.entry("artifact_sizes").or_insert(artifact_sizes);
}

pub(super) fn validate_report_cases(cases: &[Value]) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(PUBLIC_MANIFEST).map_err(|err| err.to_string())?;
    if let Some(groups) = manifest.get("query_groups").and_then(Value::as_array) {
        for group in groups.iter().filter_map(Value::as_str) {
            require_measured_case(cases, group)?;
        }
    }
    if let Some(skipped) = cases
        .iter()
        .find(|case| case.get("status").and_then(Value::as_str) == Some("skipped"))
    {
        return Err(format!(
            "benchmark case {} was skipped",
            skipped
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    let required = [
        "full_numeric_scan",
        "string_category_scan",
        "equality_filter",
        "range_filter",
        "top_n",
        "point_lookup",
        "ai_vector_search_report",
        "ai_training_archive_report",
        "covi_index_latency",
        "covi_index_only_count",
        "object_store_cold_warm",
        "semantic_projection_object_store_compare",
        "semantic_showcase_bundle_object_store_compare",
        "parquet_conversion_cost",
        "orc_conversion_cost",
        "orc_full_scan_readback",
        "orc_file_size_overhead",
        "coverage_cache_disabled",
        "coverage_cache_hit",
        "coverage_cache_hit_miss_invalidation",
        "filecode_group_by",
        "execution_code_remap_overhead",
        "registered_codec_decode_predicate_kernel",
        "fallback_payload_overhead",
        "page_cluster_range_coalescing",
        "zero_copy_success_fallback_rate",
        "coverage_degree_tightness",
        "tpch_style_queries",
        "tpcds_style_queries",
        "medical_operational_queries",
        "negative_corrupt_validation",
        "canonicalisation_vectors",
        "semantic_mapping_corpus",
        "cove_o_delta_artifact_metrics",
        "cove_map_build_tiny",
        "cove_map_build_medium",
        "cove_map_build_messy_multisource",
        "cove_o_overlap_stress",
        "cove_o_overlap_scale_1_table",
        "cove_o_overlap_scale_2_tables",
        "cove_o_overlap_scale_4_tables",
        "cove_o_overlap_scale_8_tables",
        "cove_o_overlap_scale_8_tables_large",
        "cove_o_overlap_partial_0pct",
        "cove_o_overlap_partial_25pct",
        "cove_o_overlap_partial_50pct",
        "cove_o_overlap_partial_75pct",
        "cove_o_overlap_partial_100pct",
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "projection_covi_missing_sidecar_fallback",
        "projection_covi_stale_sidecar_fallback",
        "projection_covi_unsupported_predicate_fallback",
        "semantic_projection_object_store_compare",
        "semantic_showcase_bundle_object_store_compare",
        "customer360_projection_scan",
        "customer360_selective_filter",
        "customer360_event_filter",
        "customer360_object_store_compare",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
        "customer360_projection_covi_compound_valid",
        "proof_suite_customer360",
        "proof_suite_claims",
        "proof_suite_catalog",
    ];
    for id in required {
        if !cases.iter().any(|case| case.get("id") == Some(&json!(id))) {
            return Err(format!("benchmark report missing required case {id}"));
        }
    }
    let required_metric_fields = [
        "elapsed_time_ns",
        "bytes_read",
        "request_count",
        "fragments_visited",
        "pages_visited",
        "pruning_tightness",
        "coverage_cache",
        "index_use",
        "memory_peak_bytes",
        "artifact_sizes",
    ];
    for case in cases {
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!(
                    "benchmark case {} is missing metrics",
                    case.get("id").and_then(Value::as_str).unwrap_or("unknown")
                )
            })?;
        for field in required_metric_fields {
            if !metrics.contains_key(field) {
                return Err(format!(
                    "benchmark case {} missing required metric {field}",
                    case.get("id").and_then(Value::as_str).unwrap_or("unknown")
                ));
            }
        }
    }
    let cache_hit = cases
        .iter()
        .find(|case| case.get("id") == Some(&json!("coverage_cache_hit")))
        .and_then(|case| {
            case.pointer("/cost/coverage_metrics/coverage_cache/hits")
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);
    if cache_hit == 0 {
        return Err("coverage_cache_hit did not record a COVE-CACHE hit".into());
    }
    let covi_lookup = require_measured_case(cases, "covi_index_latency")?;
    if !case_bool(covi_lookup, "/cost/coverage_metrics/covi_used") {
        return Err("covi_index_latency did not use COVI candidates".into());
    }
    if case_u64(covi_lookup, "/cost/coverage_metrics/covi_candidates") == 0 {
        return Err("covi_index_latency did not produce any COVI candidates".into());
    }
    if case_u64(covi_lookup, "/cost/coverage_metrics/covi/loaded") == 0 {
        return Err("covi_index_latency did not load a COVI sidecar".into());
    }

    let covi_count = require_measured_case(cases, "covi_index_only_count")?;
    if case_u64(covi_count, "/cost/coverage_metrics/covi/loaded") == 0 {
        return Err("covi_index_only_count did not load a COVI sidecar".into());
    }
    if case_u64(covi_count, "/cost/coverage_metrics/covi/index_only_answers") == 0 {
        return Err("covi_index_only_count did not record COVI index-only evidence".into());
    }
    if covi_count.pointer("/proof/kind").and_then(Value::as_str) != Some("CoviIndexOnlyCount") {
        return Err("covi_index_only_count did not prove CoviIndexOnlyCount".into());
    }
    validate_projection_covi_benchmark_cases(cases)?;
    validate_ai_benchmark_case(cases)?;
    validate_overlap_stress_benchmark_case(cases)?;
    validate_overlap_scale_benchmark_cases(cases)?;
    validate_overlap_partial_benchmark_cases(cases)?;
    validate_proof_suite_benchmark_cases(cases)?;
    validate_cove_o_delta_benchmark_case(cases)?;
    Ok(())
}

pub(super) fn validate_cove_o_delta_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "cove_o_delta_artifact_metrics")?;
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "cove_o_delta_artifact_metrics is missing metrics".to_string())?;
    let required_metrics = [
        "bytes_written_per_update",
        "full_rewrite_bytes_per_update",
        "total_bytes_stored",
        "writer_finalization_ns",
        "publication_latency_ns",
        "validation_time_ns",
        "latest_state_point_lookup_p95_artifacts",
        "object_history_query_selected_deltas",
        "projection_readback_property_skips",
        "object_store_request_count",
        "chain_summary_range_requests",
        "delta_artifacts_opened",
        "delta_artifacts_skipped_before_open",
        "source_publication_pruning_effectiveness",
        "dictionary_alias_resolution_count",
        "compaction_throughput_rows_per_ns",
        "compacted_output_bytes",
        "index_rebuild_candidate_count",
        "delta_chain_depth",
        "selected_delta_count",
        "skipped_delta_count",
        "chain_summary_bytes",
        "base_file_bytes",
        "total_delta_bytes",
        "patch_rows_applied",
        "materialized_property_count",
        "checkpoint_recommended",
        "compaction_recommended",
        "snapshot_index_recommended",
        "recommendations",
    ];
    for field in required_metrics {
        if !metrics.contains_key(field) {
            return Err(format!(
                "cove_o_delta_artifact_metrics missing metric {field}"
            ));
        }
    }
    if case_u64(case, "/metrics/delta_chain_depth") == 0 {
        return Err("cove_o_delta_artifact_metrics did not measure any deltas".into());
    }
    if case_u64(case, "/metrics/delta_artifacts_skipped_before_open") == 0 {
        return Err("cove_o_delta_artifact_metrics did not record pruning".into());
    }
    if case_u64(case, "/metrics/chain_summary_bytes") == 0 {
        return Err("cove_o_delta_artifact_metrics did not encode a chain summary".into());
    }
    if !case_bool(case, "/metrics/compaction_recommended") {
        return Err("cove_o_delta_artifact_metrics did not trigger compaction guidance".into());
    }
    if !case_bool(case, "/metrics/checkpoint_recommended") {
        return Err("cove_o_delta_artifact_metrics did not trigger checkpoint guidance".into());
    }
    Ok(())
}

pub(super) fn validate_ai_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "ai_vector_search_report")?;
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "ai_vector_search_report is missing metrics".to_string())?;
    for field in [
        "vector_build_latency_ns",
        "sidecar_parse_latency_ns",
        "vector_search_latency_ns",
        "ann_search_latency_ns",
        "ann_recall_vs_exact",
        "exact_fallback_rate",
        "filtered_top_k_complete",
        "ann_selected_index",
        "ann_result_authority",
        "ann_internal_candidate_execution",
        "ann_exact_result_claim",
        "vector_count",
        "dimension_count",
        "exact_result_count",
        "ann_result_count",
        "ann_fallback_count",
        "payload_bytes_read",
        "policy_withheld_count",
    ] {
        if !metrics.contains_key(field) {
            return Err(format!("ai_vector_search_report missing metric {field}"));
        }
    }
    if case_u64(case, "/metrics/vector_count") == 0 {
        return Err("ai_vector_search_report did not measure any vectors".into());
    }
    if case_u64(case, "/metrics/exact_result_count") == 0 {
        return Err("ai_vector_search_report did not return exact vector results".into());
    }
    if case_u64(case, "/metrics/payload_bytes_read") == 0 {
        return Err("ai_vector_search_report did not report vector payload bytes".into());
    }
    if case_u64(case, "/metrics/ann_fallback_count") != 0 {
        return Err("ai_vector_search_report unexpectedly fell back from indexed ANN".into());
    }
    if !case_bool(case, "/metrics/ann_internal_candidate_execution") {
        return Err("ai_vector_search_report did not exercise internal ANN candidates".into());
    }
    if case_bool(case, "/metrics/ann_exact_result_claim") {
        return Err("ai_vector_search_report claimed exactness for approximate ANN".into());
    }
    if !(0.0..=1.0).contains(&case_f64(case, "/metrics/ann_recall_vs_exact")) {
        return Err("ai_vector_search_report recall was outside 0..1".into());
    }
    if !case_bool(case, "/metrics/filtered_top_k_complete") {
        return Err("ai_vector_search_report did not mark filtered top-k completeness".into());
    }
    let training_case = require_measured_case(cases, "ai_training_archive_report")?;
    let training_metrics = training_case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "ai_training_archive_report is missing metrics".to_string())?;
    for field in [
        "sample_count",
        "train_sample_count",
        "ai_import_samples_per_sec",
        "ai_verify_samples_per_sec",
        "ai_stream_samples_per_sec",
        "ai_export_samples_per_sec",
        "ai_payload_bytes_read",
        "ai_policy_withheld_count",
        "ai_context_latency_ms",
        "ai_vector_search_latency_ms",
        "ai_export_format",
        "export_bytes",
    ] {
        if !training_metrics.contains_key(field) {
            return Err(format!("ai_training_archive_report missing metric {field}"));
        }
    }
    if case_u64(training_case, "/metrics/sample_count") == 0 {
        return Err("ai_training_archive_report did not measure samples".into());
    }
    if case_u64(training_case, "/metrics/train_sample_count") == 0 {
        return Err("ai_training_archive_report did not stream train samples".into());
    }
    if case_u64(training_case, "/metrics/ai_payload_bytes_read") == 0 {
        return Err("ai_training_archive_report did not read payload bytes".into());
    }
    if training_case
        .pointer("/metrics/ai_export_format")
        .and_then(Value::as_str)
        != Some("hf-jsonl")
    {
        return Err("ai_training_archive_report did not report hf-jsonl export".into());
    }
    Ok(())
}

pub(super) fn validate_proof_suite_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required_metrics = [
        "build_time_ns",
        "validation_time_ns",
        "parity_time_ns",
        "source_bytes",
        "source_parquet_bundle_bytes",
        "normalized_parquet_bundle_bytes",
        "denormalized_parquet_bytes",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
        "duplication_ratio",
        "doctor_status_ok",
        "parity_status_ok",
        "parity_report_count",
    ];
    for id in [
        "proof_suite_customer360",
        "proof_suite_claims",
        "proof_suite_catalog",
    ] {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required_metrics {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing proof-suite metric {field}"));
            }
        }
        if !case_bool(case, "/metrics/doctor_status_ok") {
            return Err(format!("{id} doctor report was not ok"));
        }
        if !case_bool(case, "/metrics/parity_status_ok") {
            return Err(format!("{id} parity reports were not ok"));
        }
        if case_u64(case, "/metrics/parity_report_count") == 0 {
            return Err(format!("{id} did not include parity reports"));
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
        if case_u64(case, "/metrics/covi_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-I bytes"));
        }
        if case_u64(case, "/metrics/covm_bytes") == 0 {
            return Err(format!("{id} did not emit COVM bytes"));
        }
    }
    Ok(())
}

pub(super) fn validate_overlap_stress_benchmark_case(cases: &[Value]) -> Result<(), String> {
    let case = require_measured_case(cases, "cove_o_overlap_stress")?;
    let required_metrics = [
        "source_table_count",
        "row_count",
        "overlap_fraction",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "unique_payload_bytes",
        "duplicate_payload_bytes",
        "cove_o_bytes",
        "compressed_cove_o_bytes",
        "uncompressed_cove_o_bytes",
        "compact_cove_o_bytes",
        "expanded_cove_o_bytes",
        "section_compression_saved_bytes",
        "section_compression_uncompressed_bytes",
        "section_compression_emitted_bytes",
        "section_compression_compressed_section_count",
        "section_compression_ratio",
        "compact_vs_expanded_cove_o_ratio",
        "compact_evidence_index_bytes",
        "expanded_evidence_json_bytes",
        "compact_evidence_vs_expanded_json_ratio",
        "total_bundle_bytes",
        "uncompressed_total_bundle_bytes",
        "expanded_total_bundle_bytes",
        "compressed_vs_uncompressed_bundle_ratio",
        "compact_vs_expanded_bundle_ratio",
        "cove_o_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
        "evidence_to_property_ratio",
    ];
    let metrics = case
        .get("metrics")
        .and_then(Value::as_object)
        .ok_or_else(|| "cove_o_overlap_stress is missing metrics".to_string())?;
    for field in required_metrics {
        if !metrics.contains_key(field) {
            return Err(format!("cove_o_overlap_stress missing metric {field}"));
        }
    }
    if case_u64(case, "/metrics/source_table_count") < 2 {
        return Err("cove_o_overlap_stress did not use multiple source tables".into());
    }
    if case_u64(case, "/metrics/duplicate_payload_bytes") == 0 {
        return Err("cove_o_overlap_stress did not generate duplicate payload".into());
    }
    if case_u64(case, "/metrics/cove_o_bytes") == 0 {
        return Err("cove_o_overlap_stress did not produce COVE-O bytes".into());
    }
    if case_u64(case, "/metrics/compressed_cove_o_bytes")
        >= case_u64(case, "/metrics/uncompressed_cove_o_bytes")
    {
        return Err("cove_o_overlap_stress section compression did not reduce COVE-O bytes".into());
    }
    if case_u64(case, "/metrics/section_compression_saved_bytes") == 0 {
        return Err("cove_o_overlap_stress did not record section compression savings".into());
    }
    if case_u64(
        case,
        "/metrics/section_compression_compressed_section_count",
    ) == 0
    {
        return Err("cove_o_overlap_stress did not compress any COVE-O sections".into());
    }
    if case_u64(case, "/metrics/compact_evidence_index_bytes")
        >= case_u64(case, "/metrics/expanded_evidence_json_bytes")
    {
        return Err(
            "cove_o_overlap_stress compact evidence was not smaller than expanded evidence".into(),
        );
    }
    if case_u64(case, "/metrics/compact_cove_o_bytes")
        >= case_u64(case, "/metrics/expanded_cove_o_bytes")
    {
        return Err(
            "cove_o_overlap_stress compact COVE-O was not smaller than expanded COVE-O".into(),
        );
    }
    Ok(())
}

pub(super) fn validate_overlap_scale_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required = [
        "source_table_count",
        "row_count",
        "overlap_fraction",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "source_parquet_redundancy_ratio",
        "duplicate_payload_bytes",
        "duplicate_payload_ratio",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "cove_o_vs_source_csv_ratio",
        "bundle_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "bundle_vs_parquet_bundle_ratio",
        "cove_o_vs_unique_parquet_ratio",
        "bundle_vs_unique_parquet_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
    ];
    let ids = [
        "cove_o_overlap_scale_1_table",
        "cove_o_overlap_scale_2_tables",
        "cove_o_overlap_scale_4_tables",
        "cove_o_overlap_scale_8_tables",
        "cove_o_overlap_scale_8_tables_large",
    ];
    for id in ids {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing overlap-scale metric {field}"));
            }
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
        if case_u64(case, "/metrics/source_parquet_bundle_bytes") == 0 {
            return Err(format!("{id} did not emit source Parquet baselines"));
        }
        if case_u64(case, "/metrics/object_count") == 0 {
            return Err(format!("{id} did not materialize objects"));
        }
    }

    let two = require_measured_case(cases, "cove_o_overlap_scale_2_tables")?;
    let four = require_measured_case(cases, "cove_o_overlap_scale_4_tables")?;
    let eight = require_measured_case(cases, "cove_o_overlap_scale_8_tables")?;
    if case_f64(eight, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(two, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "overlap scale did not improve COVE-O/source-Parquet ratio from 2 to 8 tables".into(),
        );
    }
    if case_f64(eight, "/metrics/bundle_vs_parquet_bundle_ratio")
        >= case_f64(four, "/metrics/bundle_vs_parquet_bundle_ratio")
    {
        return Err(
            "overlap scale did not improve bundle/source-Parquet ratio from 4 to 8 tables".into(),
        );
    }
    if case_f64(eight, "/metrics/cove_o_vs_parquet_bundle_ratio") >= 1.0 {
        return Err("8-table overlap scale did not make COVE-O smaller than source Parquet".into());
    }
    Ok(())
}

pub(super) fn validate_overlap_partial_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let required = [
        "source_table_count",
        "row_count",
        "source_input_row_count",
        "overlap_fraction",
        "overlap_percent",
        "shared_row_count",
        "source_unique_rows_per_table",
        "unique_entity_count",
        "object_dedupe_ratio",
        "source_csv_bytes",
        "source_parquet_bundle_bytes",
        "unique_parquet_bytes",
        "source_parquet_redundancy_ratio",
        "duplicate_payload_bytes",
        "duplicate_payload_ratio",
        "cove_o_bytes",
        "cove_t_bytes",
        "covi_bytes",
        "covm_bytes",
        "total_bundle_bytes",
        "cove_o_vs_source_csv_ratio",
        "bundle_vs_source_csv_ratio",
        "cove_o_vs_parquet_bundle_ratio",
        "bundle_vs_parquet_bundle_ratio",
        "cove_o_vs_unique_parquet_ratio",
        "bundle_vs_unique_parquet_ratio",
        "object_count",
        "property_value_count",
        "evidence_entry_count",
    ];
    let ids = [
        "cove_o_overlap_partial_0pct",
        "cove_o_overlap_partial_25pct",
        "cove_o_overlap_partial_50pct",
        "cove_o_overlap_partial_75pct",
        "cove_o_overlap_partial_100pct",
    ];
    for id in ids {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing overlap-partial metric {field}"));
            }
        }
        if case_u64(case, "/metrics/object_count") != case_u64(case, "/metrics/unique_entity_count")
        {
            return Err(format!(
                "{id} object count did not match unique entity count"
            ));
        }
        if case_u64(case, "/metrics/cove_o_bytes") == 0 {
            return Err(format!("{id} did not emit COVE-O bytes"));
        }
    }

    let zero = require_measured_case(cases, "cove_o_overlap_partial_0pct")?;
    let fifty = require_measured_case(cases, "cove_o_overlap_partial_50pct")?;
    let hundred = require_measured_case(cases, "cove_o_overlap_partial_100pct")?;
    if case_f64(hundred, "/metrics/object_dedupe_ratio")
        <= case_f64(zero, "/metrics/object_dedupe_ratio")
    {
        return Err("partial overlap object dedupe ratio did not improve from 0% to 100%".into());
    }
    if case_f64(hundred, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(zero, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "partial overlap COVE-O/source-Parquet ratio did not improve from 0% to 100%".into(),
        );
    }
    if case_f64(fifty, "/metrics/cove_o_vs_parquet_bundle_ratio")
        >= case_f64(zero, "/metrics/cove_o_vs_parquet_bundle_ratio")
    {
        return Err(
            "partial overlap COVE-O/source-Parquet ratio did not improve by 50% overlap".into(),
        );
    }
    Ok(())
}

pub(super) fn validate_projection_covi_benchmark_cases(cases: &[Value]) -> Result<(), String> {
    let all_projection_cases = [
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "projection_covi_missing_sidecar_fallback",
        "projection_covi_stale_sidecar_fallback",
        "projection_covi_unsupported_predicate_fallback",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
        "customer360_projection_covi_compound_valid",
    ];
    let required_metrics = [
        "source_bytes",
        "cove_o_bytes",
        "projection_sidecar_bytes",
        "candidate_rows",
        "skipped_rows",
        "residual_rows",
        "result_rows",
        "lookup_hits",
        "lookup_misses",
        "fallback_count",
        "duplication_ratio",
    ];
    for id in all_projection_cases {
        let case = require_measured_case(cases, id)?;
        let metrics = case
            .get("metrics")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{id} is missing metrics"))?;
        for field in required_metrics {
            if !metrics.contains_key(field) {
                return Err(format!("{id} missing projection COVE-I metric {field}"));
            }
        }
    }
    for id in [
        "projection_covi_equality_valid",
        "projection_covi_in_valid",
        "projection_covi_range_valid",
        "customer360_projection_covi_score_range_valid",
        "customer360_projection_covi_status_eq_valid",
        "customer360_projection_covi_tier_in_valid",
    ] {
        let case = require_measured_case(cases, id)?;
        if case_u64(case, "/metrics/lookup_hits") == 0 {
            return Err(format!("{id} did not record projection COVE-I lookup hits"));
        }
        if case_u64(case, "/metrics/candidate_rows") == 0 {
            return Err(format!("{id} did not record projection COVE-I candidates"));
        }
        if case_u64(case, "/metrics/skipped_rows") == 0 {
            return Err(format!("{id} did not record projection COVE-I pruning"));
        }
        if case_u64(case, "/metrics/fallback_count") != 0 {
            return Err(format!(
                "{id} unexpectedly fell back from projection COVE-I"
            ));
        }
    }
    let missing = require_measured_case(cases, "projection_covi_missing_sidecar_fallback")?;
    if case_u64(missing, "/metrics/fallback_no_sidecar") == 0 {
        return Err(
            "projection_covi_missing_sidecar_fallback did not record missing-sidecar fallback"
                .into(),
        );
    }
    let stale = require_measured_case(cases, "projection_covi_stale_sidecar_fallback")?;
    if case_u64(stale, "/metrics/fallback_stale") == 0 {
        return Err(
            "projection_covi_stale_sidecar_fallback did not record stale-sidecar fallback".into(),
        );
    }
    if case_u64(stale, "/metrics/sidecar_ignored") == 0 {
        return Err("projection_covi_stale_sidecar_fallback did not record ignored sidecar".into());
    }
    let unsupported =
        require_measured_case(cases, "projection_covi_unsupported_predicate_fallback")?;
    if case_u64(unsupported, "/metrics/fallback_no_eligible_filter") == 0 {
        return Err(
            "projection_covi_unsupported_predicate_fallback did not record unsupported-filter fallback"
                .into(),
        );
    }
    if case_u64(unsupported, "/metrics/lookup_hits") != 0 {
        return Err("projection_covi_unsupported_predicate_fallback used sidecar lookup".into());
    }
    let compound = require_measured_case(cases, "customer360_projection_covi_compound_valid")?;
    if case_u64(compound, "/metrics/lookup_hits") < 2 {
        return Err(
            "customer360_projection_covi_compound_valid did not use both sidecar lookups".into(),
        );
    }
    if case_u64(compound, "/metrics/eligible_filters") < 2 {
        return Err(
            "customer360_projection_covi_compound_valid did not report both eligible filters"
                .into(),
        );
    }
    if case_u64(compound, "/metrics/skipped_rows") == 0 {
        return Err("customer360_projection_covi_compound_valid did not record pruning".into());
    }
    if case_u64(compound, "/metrics/fallback_count") != 0 {
        return Err("customer360_projection_covi_compound_valid unexpectedly fell back".into());
    }
    Ok(())
}

pub(super) fn require_measured_case<'a>(cases: &'a [Value], id: &str) -> Result<&'a Value, String> {
    let case = cases
        .iter()
        .find(|case| case.get("id") == Some(&json!(id)))
        .ok_or_else(|| format!("benchmark report missing required case {id}"))?;
    if case.get("status").and_then(Value::as_str) != Some("measured") {
        return Err(format!("{id} was not measured"));
    }
    Ok(case)
}

pub(super) fn case_u64(case: &Value, pointer: &str) -> u64 {
    case.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

pub(super) fn case_f64(case: &Value, pointer: &str) -> f64 {
    case.pointer(pointer).and_then(Value::as_f64).unwrap_or(0.0)
}

pub(super) fn case_bool(case: &Value, pointer: &str) -> bool {
    case.pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
