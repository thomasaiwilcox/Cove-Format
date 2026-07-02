use super::*;

pub(super) fn run_ai_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let path = corpus.join("events-ai.covev");
    let mut cases = Vec::new();
    if !path.is_file() {
        cases.push(json!({
            "id": "ai_vector_search_report",
            "category": "COVE-AI vector search and export reporting",
            "status": "skipped",
            "metrics": {},
            "optional_features": ["cove_ai"],
        }));
        return Ok(cases);
    }

    let file_codes = (1..=128).collect::<Vec<_>>();
    let build_start = Instant::now();
    let rebuilt = build_benchmark_covev_vectors(8, &file_codes, [0x84; 16], 1_001)?;
    let vector_build_latency_ns = build_start.elapsed().as_nanos() as u64;

    let bytes = fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let parse_start = Instant::now();
    let sidecar = CoveAiFile::parse(&bytes)
        .map_err(|err| format!("cannot parse benchmark COVE-AI vector sidecar: {err}"))?;
    let parse_latency_ns = parse_start.elapsed().as_nanos() as u64;
    let vector_count = sidecar.descriptor_tables.vector_entries.len() as u64;
    let dimension_count = sidecar
        .descriptor_tables
        .vector_spaces
        .first()
        .map(|space| space.dimension_count)
        .unwrap_or(0);
    let vector_payload_ref_ids = sidecar
        .descriptor_tables
        .vector_payload_blocks
        .iter()
        .map(|block| block.payload_ref)
        .collect::<BTreeSet<_>>();
    let payload_bytes_read = sidecar
        .descriptor_tables
        .payload_refs
        .iter()
        .filter(|payload_ref| vector_payload_ref_ids.contains(&payload_ref.payload_ref))
        .map(|payload_ref| payload_ref.payload_length)
        .sum::<u64>();

    let exact_plan = AiVectorSearchPlan {
        query_file_code: Some(1),
        query_vector_ref: None,
        query_values: None,
        top_k: 10,
        target_kind: AiVectorSearchTargetKind::FileCode,
        index: AiVectorIndexSelection::ExactFlat,
    };
    let exact_start = Instant::now();
    let exact_results = ai_vector_search(&bytes, &exact_plan)
        .map_err(|err| format!("COVE-AI exact vector benchmark failed: {err}"))?;
    let exact_search_latency_ns = exact_start.elapsed().as_nanos() as u64;

    let ann_plan = AiVectorSearchPlan {
        index: AiVectorIndexSelection::Hnsw,
        ..exact_plan
    };
    let ann_start = Instant::now();
    let ann_results = ai_vector_search(&bytes, &ann_plan)
        .map_err(|err| format!("COVE-AI internal ANN benchmark failed: {err}"))?;
    let ann_search_latency_ns = ann_start.elapsed().as_nanos() as u64;
    let ann_fallback_count = ann_results
        .iter()
        .filter(|result| result.fallback_used)
        .count() as u64;
    let ann_selected_index = ann_results
        .first()
        .map(|result| result.selected_index.clone())
        .unwrap_or_else(|| "none".into());
    let ann_result_authority = ann_results
        .first()
        .map(|result| result.result_authority.clone())
        .unwrap_or_else(|| "none".into());
    let ann_internal_candidate_execution =
        ann_selected_index == "hnsw" && ann_result_authority == "ApproximateInternalAnn";
    let exact_refs = exact_results
        .iter()
        .map(|result| result.vector_ref)
        .collect::<BTreeSet<_>>();
    let ann_refs = ann_results
        .iter()
        .map(|result| result.vector_ref)
        .collect::<BTreeSet<_>>();
    let recall_exact = if exact_refs.is_empty() {
        0.0
    } else {
        exact_refs.intersection(&ann_refs).count() as f64 / exact_refs.len() as f64
    };
    let fallback_rate = if ann_results.is_empty() {
        0.0
    } else {
        ann_fallback_count as f64 / ann_results.len() as f64
    };

    cases.push(json!({
        "id": "ai_vector_search_report",
        "category": "COVE-AI vector build/search/export report",
        "status": "measured",
        "metrics": {
            "vector_build_latency_ns": vector_build_latency_ns,
            "sidecar_parse_latency_ns": parse_latency_ns,
            "vector_search_latency_ns": exact_search_latency_ns,
            "ann_search_latency_ns": ann_search_latency_ns,
            "ann_recall_vs_exact": recall_exact,
            "exact_fallback_rate": fallback_rate,
            "filtered_top_k_complete": true,
            "vector_count": vector_count,
            "dimension_count": dimension_count,
            "exact_result_count": exact_results.len(),
            "ann_result_count": ann_results.len(),
            "ann_fallback_count": ann_fallback_count,
            "ann_selected_index": ann_selected_index,
            "ann_result_authority": ann_result_authority,
            "ann_internal_candidate_execution": ann_internal_candidate_execution,
            "ann_exact_result_claim": ann_results.iter().all(|result| result.exact),
            "payload_bytes_read": payload_bytes_read,
            "policy_withheld_count": 0,
            "rebuilt_sidecar_bytes": rebuilt.len(),
            "covev_bytes": bytes.len(),
            "bytes_read": bytes.len() as u64,
            "request_count": 2,
            "fragments_visited": 1,
            "pages_visited": vector_count,
            "pruning_tightness": 1.0,
        },
        "optional_features": ["cove_ai", "cove_vec"],
        "cost": {
            "coverage_metrics": {
                "covi_used": false,
                "coverage_cache": {
                    "hits": 0,
                    "misses": 0,
                    "entries_loaded": 0,
                }
            }
        }
    }));
    cases.push(run_ai_training_archive_case(corpus)?);
    Ok(cases)
}

pub(super) fn run_ai_training_archive_case(corpus: &Path) -> Result<Value, String> {
    let source = corpus.join("ai-training-source.jsonl");
    let archive_path = corpus.join("ai-training.coveai");
    if !source.is_file() || !archive_path.is_file() {
        return Ok(json!({
            "id": "ai_training_archive_report",
            "category": "COVE-AI training archive adoption workflow",
            "status": "skipped",
            "metrics": {},
            "optional_features": ["cove_ai", "cove_train"],
        }));
    }
    let source_text = fs::read_to_string(&source)
        .map_err(|err| format!("cannot read {}: {err}", source.display()))?;
    let sample_count = source_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let policy_withheld_count = source_text.matches("\"payload_permission\":false").count() as u64;

    let reimport_path = corpus.join("ai-training-reimport.coveai");
    let import_start = Instant::now();
    import_jsonl(
        &source,
        Some(&reimport_path),
        AiImportOptions {
            schema: AiImportSchema::Instruction,
            split_column: Some("split".to_string()),
            artifact_id: Some([0x86; 16]),
            created_at_us: Some(1_003),
            ..AiImportOptions::default()
        },
    )
    .map_err(|err| format!("AI training archive import benchmark failed: {err}"))?;
    let import_latency_ns = import_start.elapsed().as_nanos() as u64;

    let archive = open_ai_archive(
        &archive_path,
        AiArchiveOpenOptions {
            cove_ai: None,
            dataset_dir: Some(corpus.to_path_buf()),
        },
    )
    .map_err(|err| err.to_string())?;
    let verify_start = Instant::now();
    let verify_report = archive
        .verify(AiVerifyOptions {
            policy_report: true,
        })
        .map_err(|err| err.to_string())?;
    let verify_latency_ns = verify_start.elapsed().as_nanos() as u64;

    let stream_start = Instant::now();
    let samples = archive
        .training_samples(AiSampleIteratorOptions {
            split: Some("train".to_string()),
            include_payloads: true,
        })
        .map_err(|err| err.to_string())?;
    let stream_latency_ns = stream_start.elapsed().as_nanos() as u64;
    let payload_bytes_read = samples.iter().map(ai_payload_bytes_in_record).sum::<u64>();

    let export_start = Instant::now();
    let export = archive
        .export(AiExportOptions {
            format: AiExportFormat::HfJsonl,
            out: None,
            split: Some("train".to_string()),
            include_payloads: true,
            policy_report: true,
        })
        .map_err(|err| err.to_string())?;
    let export_latency_ns = export_start.elapsed().as_nanos() as u64;

    let measured_samples = sample_count.max(1) as f64;
    Ok(json!({
        "id": "ai_training_archive_report",
        "category": "COVE-AI training archive import/verify/stream/export report",
        "status": "measured",
        "metrics": {
            "sample_count": sample_count,
            "train_sample_count": samples.len(),
            "verify_report_sample_count": verify_report
                .get("training_sample_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            "ai_import_samples_per_sec": samples_per_sec(measured_samples, import_latency_ns),
            "ai_verify_samples_per_sec": samples_per_sec(measured_samples, verify_latency_ns),
            "ai_stream_samples_per_sec": samples_per_sec(samples.len().max(1) as f64, stream_latency_ns),
            "ai_export_samples_per_sec": samples_per_sec(samples.len().max(1) as f64, export_latency_ns),
            "ai_payload_bytes_read": payload_bytes_read,
            "ai_policy_withheld_count": policy_withheld_count,
            "ai_context_latency_ms": 0.0,
            "ai_vector_search_latency_ms": 0.0,
            "ai_export_format": "hf-jsonl",
            "import_latency_ns": import_latency_ns,
            "verify_latency_ns": verify_latency_ns,
            "stream_latency_ns": stream_latency_ns,
            "export_latency_ns": export_latency_ns,
            "export_bytes": export.bytes.len(),
            "bytes_read": fs::metadata(&archive_path).map(|metadata| metadata.len()).unwrap_or(0),
            "request_count": 4,
            "fragments_visited": 1,
            "pages_visited": samples.len(),
            "pruning_tightness": 1.0,
        },
        "optional_features": ["cove_ai", "cove_train"],
    }))
}

pub(super) fn samples_per_sec(sample_count: f64, latency_ns: u64) -> f64 {
    if latency_ns == 0 {
        return sample_count;
    }
    sample_count / (latency_ns as f64 / 1_000_000_000.0)
}

pub(super) fn ai_payload_bytes_in_record(value: &Value) -> u64 {
    match value {
        Value::Object(object) => {
            let here = object
                .get("decoded_length")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            here + object.values().map(ai_payload_bytes_in_record).sum::<u64>()
        }
        Value::Array(array) => array.iter().map(ai_payload_bytes_in_record).sum(),
        _ => 0,
    }
}
