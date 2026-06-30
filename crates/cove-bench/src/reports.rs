use super::*;

pub(super) fn markdown_report(report: &Value) -> String {
    let mut out = String::from("# COVE v2 Public Benchmark Report\n\n");
    out.push_str("| Case | Status | Planning ns | Scan ns | Rows |\n");
    out.push_str("| --- | --- | ---: | ---: | ---: |\n");
    if let Some(cases) = report.get("cases").and_then(Value::as_array) {
        for case in cases {
            let id = case.get("id").and_then(Value::as_str).unwrap_or("unknown");
            let status = case
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let metrics = case.get("metrics").unwrap_or(&Value::Null);
            let planning = metrics
                .get("planning_ns")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let scan = metrics.get("scan_ns").and_then(Value::as_u64).unwrap_or(0);
            let rows = metrics
                .get("rows_materialized")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            out.push_str(&format!(
                "| `{id}` | {status} | {planning} | {scan} | {rows} |\n"
            ));
        }
        if let Some(ai_case) = cases
            .iter()
            .find(|case| case.get("id") == Some(&json!("ai_vector_search_report")))
        {
            let metrics = ai_case.get("metrics").unwrap_or(&Value::Null);
            out.push_str("\n## COVE-AI Vector Report\n\n");
            out.push_str("| Vectors | Dimensions | Exact results | ANN index | ANN fallback count | Recall vs exact | Payload bytes |\n");
            out.push_str("| ---: | ---: | ---: | --- | ---: | ---: | ---: |\n");
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {:.3} | {} |\n",
                metrics
                    .get("vector_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("dimension_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("exact_result_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ann_selected_index")
                    .and_then(Value::as_str)
                    .unwrap_or("none"),
                metrics
                    .get("ann_fallback_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ann_recall_vs_exact")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("payload_bytes_read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            ));
        }
        if let Some(ai_training_case) = cases
            .iter()
            .find(|case| case.get("id") == Some(&json!("ai_training_archive_report")))
        {
            let metrics = ai_training_case.get("metrics").unwrap_or(&Value::Null);
            out.push_str("\n## COVE-AI Training Archive Report\n\n");
            out.push_str("| Samples | Train samples | Import rows/s | Verify rows/s | Stream rows/s | Export rows/s | Payload bytes | Withheld | Format |\n");
            out.push_str("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
            out.push_str(&format!(
                "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {} | {} | {} |\n",
                metrics
                    .get("sample_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("train_sample_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ai_import_samples_per_sec")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("ai_verify_samples_per_sec")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("ai_stream_samples_per_sec")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("ai_export_samples_per_sec")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                metrics
                    .get("ai_payload_bytes_read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ai_policy_withheld_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                metrics
                    .get("ai_export_format")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
            ));
        }
        let overlap_scale_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("cove_o_overlap_scale_"))
            })
            .collect::<Vec<_>>();
        if !overlap_scale_cases.is_empty() {
            out.push_str("\n## COVE-O Overlap Scale\n\n");
            out.push_str("| Case | Tables | Rows | COVE-O bytes | Source Parquet bytes | Bundle bytes | COVE-O / Parquet | Bundle / Parquet |\n");
            out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
            for case in overlap_scale_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("cove_o_overlap_scale_unknown")
                    .trim_start_matches("cove_o_overlap_scale_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let tables = metrics
                    .get("source_table_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let rows = metrics
                    .get("row_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_ratio = metrics
                    .get("cove_o_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let bundle_ratio = metrics
                    .get("bundle_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                out.push_str(&format!(
                    "| `{id}` | {tables} | {rows} | {cove_o} | {parquet} | {bundle} | {cove_ratio:.3} | {bundle_ratio:.3} |\n"
                ));
            }
        }
        let overlap_partial_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("cove_o_overlap_partial_"))
            })
            .collect::<Vec<_>>();
        if !overlap_partial_cases.is_empty() {
            out.push_str("\n## COVE-O Partial Overlap\n\n");
            out.push_str("| Case | Overlap | Tables | Rows/table | Unique objects | COVE-O bytes | Source Parquet bytes | Bundle bytes | COVE-O / Parquet | Bundle / Parquet |\n");
            out.push_str(
                "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
            );
            for case in overlap_partial_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("cove_o_overlap_partial_unknown")
                    .trim_start_matches("cove_o_overlap_partial_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let overlap = metrics
                    .get("overlap_percent")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let tables = metrics
                    .get("source_table_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let rows = metrics
                    .get("row_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let unique_objects = metrics
                    .get("unique_entity_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cove_ratio = metrics
                    .get("cove_o_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let bundle_ratio = metrics
                    .get("bundle_vs_parquet_bundle_ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                out.push_str(&format!(
                    "| `{id}` | {overlap}% | {tables} | {rows} | {unique_objects} | {cove_o} | {parquet} | {bundle} | {cove_ratio:.3} | {bundle_ratio:.3} |\n"
                ));
            }
        }
        let proof_cases = cases
            .iter()
            .filter(|case| {
                case.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("proof_suite_"))
            })
            .collect::<Vec<_>>();
        if !proof_cases.is_empty() {
            out.push_str("\n## COVE-O Proof Suite\n\n");
            out.push_str("| Scenario | COVE-O bytes | Source bytes | Source Parquet bytes | Bundle bytes | Doctor | Parity |\n");
            out.push_str("| --- | ---: | ---: | ---: | ---: | --- | --- |\n");
            for case in proof_cases {
                let id = case
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("proof_suite_unknown")
                    .trim_start_matches("proof_suite_");
                let metrics = case.get("metrics").unwrap_or(&Value::Null);
                let cove_o = metrics
                    .get("cove_o_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let source = metrics
                    .get("source_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let parquet = metrics
                    .get("source_parquet_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let bundle = metrics
                    .get("total_bundle_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let doctor = metrics
                    .get("doctor_status_ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let parity = metrics
                    .get("parity_status_ok")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.push_str(&format!(
                    "| `{id}` | {cove_o} | {source} | {parquet} | {bundle} | {} | {} |\n",
                    if doctor { "ok" } else { "fail" },
                    if parity { "ok" } else { "fail" },
                ));
            }
        }
    }
    out
}

pub(super) fn environment_report() -> Value {
    json!({
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
    })
}
