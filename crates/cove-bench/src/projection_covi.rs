use super::*;

pub(super) const PROJECTION_COVI_BENCH_ROWS: usize = 1_024;
pub(super) const PROJECTION_COVI_METRICS: &[&str] = &[
    "cove_projection_covi_sidecars_found",
    "cove_covi_sidecars_loaded",
    "cove_covi_sidecars_stale",
    "cove_projection_covi_sidecars_ignored",
    "cove_projection_covi_validation_bytes",
    "cove_projection_covi_root_count",
    "cove_projection_covi_eligible_filters",
    "cove_lookup_index_hits",
    "cove_lookup_index_misses",
    "cove_projection_covi_candidate_rows",
    "cove_projection_covi_rows_skipped",
    "cove_projection_covi_residual_rows_checked",
    "cove_projection_covi_fallback_no_sidecar",
    "cove_projection_covi_fallback_no_eligible_filter",
    "cove_projection_covi_fallback_lookup_failed",
    "cove_projection_covi_fallback_stale",
    "cove_projection_covi_fallback_unavailable",
];

#[derive(Clone, Copy)]
pub(super) enum ProjectionCoviSidecarState {
    Valid,
    Missing,
    Stale,
}

pub(super) struct ProjectionCoviQueryOutcome {
    planning_ns: u128,
    scan_ns: u128,
    rows: usize,
    metrics: BTreeMap<String, usize>,
}

pub(super) fn run_projection_covi_measured_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let root = corpus.join("semantic-map-builds").join("projection-covi");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("people_projection.covemap");
    let source_path = root.join("people.csv");
    durable::durable_replace(&map_path, &projection_covi_covemap_bytes()?)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let mut csv = String::from("id,name,status,score\n");
    let statuses = ["active", "trial", "paused", "closed"];
    for row in 0..PROJECTION_COVI_BENCH_ROWS {
        csv.push_str(&format!(
            "p{row:04},person-{row:04},{},{}\n",
            statuses[row % statuses.len()],
            row
        ));
    }
    fs::write(&source_path, csv.as_bytes())
        .map_err(|err| format!("cannot write {}: {err}", source_path.display()))?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("projection COVE-I benchmark build failed: {err}"))?;
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "projection COVE-I manifest missing object path".to_string())?;
    let object_path = out_dir.join(object_rel);
    let sidecar_path = out_dir.join("indexes").join("projection_columns.covi");
    let sidecar_bytes = fs::read(&sidecar_path)
        .map_err(|err| format!("cannot read {}: {err}", sidecar_path.display()))?;
    let source_bytes = fs::metadata(&source_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let cove_o_bytes = fs::metadata(&object_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let projection_sidecar_bytes = sidecar_bytes.len() as u64;
    let duplication_ratio = if source_bytes == 0 {
        0.0
    } else {
        total_bundle_bytes as f64 / source_bytes as f64
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            format!("cannot create Tokio runtime for projection COVE-I benchmark: {err}")
        })?;
    let case_specs = [
        (
            "projection_covi_equality_valid",
            "Projection COVE-I equality filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Valid,
            1usize,
        ),
        (
            "projection_covi_in_valid",
            "Projection COVE-I IN filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE status IN ('active', 'trial')",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS / 2,
        ),
        (
            "projection_covi_range_valid",
            "Projection COVE-I numeric range filter with valid sidecar",
            "SELECT id, name, status, score FROM people_projection WHERE score >= 900",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS - 900,
        ),
        (
            "projection_covi_missing_sidecar_fallback",
            "Projection COVE-I missing sidecar materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Missing,
            1usize,
        ),
        (
            "projection_covi_stale_sidecar_fallback",
            "Projection COVE-I stale sidecar materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name = 'person-0042'",
            ProjectionCoviSidecarState::Stale,
            1usize,
        ),
        (
            "projection_covi_unsupported_predicate_fallback",
            "Projection COVE-I unsupported predicate materialized fallback",
            "SELECT id, name, status, score FROM people_projection WHERE name != 'person-0042'",
            ProjectionCoviSidecarState::Valid,
            PROJECTION_COVI_BENCH_ROWS - 1,
        ),
    ];
    let mut cases = Vec::with_capacity(case_specs.len());
    for (id, category, sql, sidecar_state, expected_rows) in case_specs {
        set_projection_covi_sidecar_state(&sidecar_path, &sidecar_bytes, sidecar_state)?;
        let outcome = runtime.block_on(run_projection_covi_sql_case(&object_path, sql))?;
        durable::durable_replace(&sidecar_path, &sidecar_bytes)
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display()))?;
        if outcome.rows != expected_rows {
            return Err(format!(
                "{id} returned {} rows; expected {expected_rows}",
                outcome.rows
            ));
        }
        cases.push(projection_covi_case_report(ProjectionCoviCaseReportInput {
            id,
            category,
            sql,
            outcome: &outcome,
            source_bytes,
            cove_o_bytes,
            projection_sidecar_bytes,
            total_bundle_bytes,
            duplication_ratio,
        }));
    }
    Ok(cases)
}

pub(super) fn run_customer360_projection_covi_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    let dir = corpus.join("customer360");
    let map_path = dir.join("customer360_readback.covemap");
    let source_path = dir.join("customers_360.jsonl");
    let out_dir = dir.join("projection-covi-bundle");
    let customer_count = customer360_manifest_customer_count(&dir)?;
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let result = build_from_paths(&map_path, std::slice::from_ref(&source_path), options)
        .map_err(|err| format!("customer360 projection COVE-I benchmark build failed: {err}"))?;
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "customer360 projection COVE-I manifest missing object path".to_string())?;
    let object_path = out_dir.join(object_rel);
    let sidecar_path = out_dir.join("indexes").join("projection_columns.covi");
    let sidecar_bytes = fs::read(&sidecar_path)
        .map_err(|err| format!("cannot read {}: {err}", sidecar_path.display()))?;
    let source_bytes = fs::metadata(&source_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let cove_o_bytes = fs::metadata(&object_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let projection_sidecar_bytes = sidecar_bytes.len() as u64;
    let duplication_ratio = if source_bytes == 0 {
        0.0
    } else {
        total_bundle_bytes as f64 / source_bytes as f64
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| {
            format!(
                "cannot create Tokio runtime for Customer 360 projection COVE-I benchmark: {err}"
            )
        })?;
    let case_specs = [
        (
            "customer360_projection_covi_score_range_valid",
            "Customer 360 projection COVE-I high-score range filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE score >= 80",
            customer360_score_range_count(customer_count, 80),
        ),
        (
            "customer360_projection_covi_status_eq_valid",
            "Customer 360 projection COVE-I status equality filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE status = 'active'",
            customer360_status_active_count(customer_count),
        ),
        (
            "customer360_projection_covi_tier_in_valid",
            "Customer 360 projection COVE-I tier IN filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE tier IN ('gold', 'platinum')",
            customer360_tier_gold_platinum_count(customer_count),
        ),
        (
            "customer360_projection_covi_compound_valid",
            "Customer 360 projection COVE-I compound score/status filter",
            "SELECT customer_id, tier, score, status, mrr FROM customers WHERE score >= 80 AND status = 'active'",
            customer360_score_active_count(customer_count, 80),
        ),
    ];
    let mut cases = Vec::with_capacity(case_specs.len());
    for (id, category, sql, expected_rows) in case_specs {
        set_projection_covi_sidecar_state(
            &sidecar_path,
            &sidecar_bytes,
            ProjectionCoviSidecarState::Valid,
        )?;
        let outcome = runtime.block_on(run_projection_covi_sql_case(&object_path, sql))?;
        durable::durable_replace(&sidecar_path, &sidecar_bytes)
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display()))?;
        if outcome.rows != expected_rows {
            return Err(format!(
                "{id} returned {} rows; expected {expected_rows}",
                outcome.rows
            ));
        }
        cases.push(projection_covi_case_report(ProjectionCoviCaseReportInput {
            id,
            category,
            sql,
            outcome: &outcome,
            source_bytes,
            cove_o_bytes,
            projection_sidecar_bytes,
            total_bundle_bytes,
            duplication_ratio,
        }));
    }
    Ok(cases)
}

pub(super) fn customer360_manifest_customer_count(dir: &Path) -> Result<usize, String> {
    let path = dir.join("customer360-manifest.json");
    let bytes = fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("invalid {}: {err}", path.display()))?;
    let count = manifest
        .pointer("/row_counts/canonical_customers")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            format!(
                "{} is missing row_counts.canonical_customers",
                path.display()
            )
        })?;
    usize::try_from(count).map_err(|_| {
        format!(
            "{} row_counts.canonical_customers is too large",
            path.display()
        )
    })
}

pub(super) fn customer360_score(index: usize) -> i64 {
    ((index * 37) % 100) as i64
}

pub(super) fn customer360_status(index: usize) -> &'static str {
    if index.is_multiple_of(13) {
        "dormant"
    } else if index.is_multiple_of(5) {
        "watch"
    } else {
        "active"
    }
}

pub(super) fn customer360_tier(index: usize) -> &'static str {
    ["bronze", "silver", "gold", "platinum"][(index + 1) % 4]
}

pub(super) fn customer360_score_range_count(rows: usize, threshold: i64) -> usize {
    (0..rows)
        .filter(|index| customer360_score(*index) >= threshold)
        .count()
}

pub(super) fn customer360_status_active_count(rows: usize) -> usize {
    (0..rows)
        .filter(|index| customer360_status(*index) == "active")
        .count()
}

pub(super) fn customer360_tier_gold_platinum_count(rows: usize) -> usize {
    (0..rows)
        .filter(|index| matches!(customer360_tier(*index), "gold" | "platinum"))
        .count()
}

pub(super) fn customer360_score_active_count(rows: usize, threshold: i64) -> usize {
    (0..rows)
        .filter(|index| {
            customer360_score(*index) >= threshold && customer360_status(*index) == "active"
        })
        .count()
}

pub(super) fn set_projection_covi_sidecar_state(
    sidecar_path: &Path,
    original_bytes: &[u8],
    state: ProjectionCoviSidecarState,
) -> Result<(), String> {
    match state {
        ProjectionCoviSidecarState::Valid => durable::durable_replace(sidecar_path, original_bytes)
            .map(|_| ())
            .map_err(|err| format!("cannot restore {}: {err}", sidecar_path.display())),
        ProjectionCoviSidecarState::Missing => match fs::remove_file(sidecar_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("cannot remove {}: {err}", sidecar_path.display())),
        },
        ProjectionCoviSidecarState::Stale => {
            let mut stale = original_bytes.to_vec();
            if let Some(first) = stale.first_mut() {
                *first ^= 0x01;
            }
            durable::durable_replace(sidecar_path, &stale)
                .map(|_| ())
                .map_err(|err| format!("cannot write stale {}: {err}", sidecar_path.display()))
        }
    }
}

pub(super) async fn run_projection_covi_sql_case(
    object_path: &Path,
    sql: &str,
) -> Result<ProjectionCoviQueryOutcome, String> {
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, object_path, None, None)
        .map_err(|err| format!("cannot register COVE-O projections: {err}"))?;
    let planning_start = Instant::now();
    let dataframe = ctx
        .sql(sql)
        .await
        .map_err(|err| format!("cannot plan projection COVE-I SQL {sql:?}: {err}"))?;
    let plan = dataframe
        .create_physical_plan()
        .await
        .map_err(|err| format!("cannot create projection COVE-I physical plan: {err}"))?;
    let planning_ns = planning_start.elapsed().as_nanos();
    let scan_start = Instant::now();
    let batches = collect_physical_plan(Arc::clone(&plan), ctx.task_ctx())
        .await
        .map_err(|err| format!("cannot execute projection COVE-I SQL {sql:?}: {err}"))?;
    let scan_ns = scan_start.elapsed().as_nanos();
    let rows = batches.iter().map(|batch| batch.num_rows()).sum::<usize>();
    let metrics = PROJECTION_COVI_METRICS
        .iter()
        .map(|name| ((*name).to_string(), execution_plan_metric_sum(&plan, name)))
        .collect();
    Ok(ProjectionCoviQueryOutcome {
        planning_ns,
        scan_ns,
        rows,
        metrics,
    })
}

pub(super) fn execution_plan_metric_sum(plan: &Arc<dyn ExecutionPlan>, metric_name: &str) -> usize {
    let own = plan
        .metrics()
        .and_then(|metrics| metrics.sum_by_name(metric_name))
        .map(|metric| metric.as_usize())
        .unwrap_or(0);
    own + plan
        .children()
        .into_iter()
        .map(|child| execution_plan_metric_sum(child, metric_name))
        .sum::<usize>()
}

pub(super) struct ProjectionCoviCaseReportInput<'a> {
    id: &'a str,
    category: &'a str,
    sql: &'a str,
    outcome: &'a ProjectionCoviQueryOutcome,
    source_bytes: u64,
    cove_o_bytes: u64,
    projection_sidecar_bytes: u64,
    total_bundle_bytes: u64,
    duplication_ratio: f64,
}

pub(super) fn projection_covi_case_report(input: ProjectionCoviCaseReportInput<'_>) -> Value {
    let outcome = input.outcome;
    let metric = |name: &str| -> u64 { *outcome.metrics.get(name).unwrap_or(&0) as u64 };
    let fallback_no_sidecar = metric("cove_projection_covi_fallback_no_sidecar");
    let fallback_no_eligible = metric("cove_projection_covi_fallback_no_eligible_filter");
    let fallback_lookup_failed = metric("cove_projection_covi_fallback_lookup_failed");
    let fallback_stale = metric("cove_projection_covi_fallback_stale");
    let fallback_unavailable = metric("cove_projection_covi_fallback_unavailable");
    let fallback_count = fallback_no_sidecar
        .saturating_add(fallback_no_eligible)
        .saturating_add(fallback_lookup_failed)
        .saturating_add(fallback_stale)
        .saturating_add(fallback_unavailable);
    let fallback_reason = if fallback_no_sidecar > 0 {
        json!("missing_sidecar")
    } else if fallback_no_eligible > 0 {
        json!("no_eligible_filter")
    } else if fallback_lookup_failed > 0 {
        json!("lookup_failed")
    } else if fallback_stale > 0 {
        json!("stale_sidecar")
    } else if fallback_unavailable > 0 {
        json!("unavailable_sidecar")
    } else {
        Value::Null
    };
    let lookup_hits = metric("cove_lookup_index_hits");
    let lookup_misses = metric("cove_lookup_index_misses");
    let candidate_rows = metric("cove_projection_covi_candidate_rows");
    let skipped_rows = metric("cove_projection_covi_rows_skipped");
    let residual_rows = metric("cove_projection_covi_residual_rows_checked");
    let validation_bytes = metric("cove_projection_covi_validation_bytes");
    let root_count = metric("cove_projection_covi_root_count");
    let planning_ns = outcome.planning_ns;
    let scan_ns = outcome.scan_ns;
    json!({
        "id": input.id,
        "category": input.category,
        "status": "measured",
        "query": input.sql,
        "metrics": {
            "planning_ns": planning_ns,
            "scan_ns": scan_ns,
            "end_to_end_ns": planning_ns + scan_ns,
            "rows_materialized": outcome.rows,
            "result_rows": outcome.rows,
            "source_bytes": input.source_bytes,
            "cove_o_bytes": input.cove_o_bytes,
            "projection_sidecar_bytes": input.projection_sidecar_bytes,
            "total_bundle_bytes": input.total_bundle_bytes,
            "duplication_ratio": input.duplication_ratio,
            "sidecar_found": metric("cove_projection_covi_sidecars_found"),
            "sidecar_loaded": metric("cove_covi_sidecars_loaded"),
            "sidecar_stale": metric("cove_covi_sidecars_stale"),
            "sidecar_ignored": metric("cove_projection_covi_sidecars_ignored"),
            "validation_bytes": validation_bytes,
            "root_count": root_count,
            "eligible_filters": metric("cove_projection_covi_eligible_filters"),
            "lookup_hits": lookup_hits,
            "lookup_misses": lookup_misses,
            "candidate_rows": candidate_rows,
            "skipped_rows": skipped_rows,
            "residual_rows": residual_rows,
            "fallback_count": fallback_count,
            "fallback_reason": fallback_reason,
            "fallback_no_sidecar": fallback_no_sidecar,
            "fallback_no_eligible_filter": fallback_no_eligible,
            "fallback_lookup_failed": fallback_lookup_failed,
            "fallback_stale": fallback_stale,
            "fallback_unavailable": fallback_unavailable,
        },
        "cost": {
            "observed": {
                "metadata_bytes_read": validation_bytes,
                "data_bytes_read": input.cove_o_bytes,
                "range_requests": if metric("cove_projection_covi_sidecars_found") > 0 { 2 } else { 1 },
                "scan_tasks": 1,
                "pages_decoded": outcome.rows,
                "morsels_considered": root_count,
                "morsels_pruned": skipped_rows,
                "lookup_index_hits": lookup_hits,
                "lookup_index_misses": lookup_misses,
                "index_fallbacks": fallback_count,
            },
            "coverage_metrics": {
                "covi_used": lookup_hits > 0,
                "covi_candidates": candidate_rows,
                "projection_covi": {
                    "sidecar_found": metric("cove_projection_covi_sidecars_found"),
                    "sidecar_loaded": metric("cove_covi_sidecars_loaded"),
                    "sidecar_stale": metric("cove_covi_sidecars_stale"),
                    "sidecar_ignored": metric("cove_projection_covi_sidecars_ignored"),
                    "validation_bytes": validation_bytes,
                    "root_count": root_count,
                    "eligible_filters": metric("cove_projection_covi_eligible_filters"),
                    "lookup_hits": lookup_hits,
                    "lookup_misses": lookup_misses,
                    "candidate_rows": candidate_rows,
                    "skipped_rows": skipped_rows,
                    "residual_rows": residual_rows,
                    "fallback_count": fallback_count,
                    "fallback_reason": fallback_reason,
                    "fallback_no_sidecar": fallback_no_sidecar,
                    "fallback_no_eligible_filter": fallback_no_eligible,
                    "fallback_lookup_failed": fallback_lookup_failed,
                    "fallback_stale": fallback_stale,
                    "fallback_unavailable": fallback_unavailable,
                }
            }
        },
        "optional_features": ["cove_map", "cove_o_projection", "cove_i", "projection_covi"],
    })
}

pub(super) fn directory_size(path: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for entry in
        fs::read_dir(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?
    {
        let entry = entry.map_err(|err| format!("cannot read {} entry: {err}", path.display()))?;
        let metadata = entry
            .metadata()
            .map_err(|err| format!("cannot stat {}: {err}", entry.path().display()))?;
        if metadata.is_dir() {
            total = total
                .checked_add(directory_size(&entry.path())?)
                .ok_or_else(|| "directory size overflow".to_string())?;
        } else {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| "directory size overflow".to_string())?;
        }
    }
    Ok(total)
}
