use super::*;

#[cfg(feature = "covi")]
#[test]
fn generated_covi_min_max_answers_feed_datafusion_metadata_path() {
    let bytes = dictionary_items_file_with_lookup_index();
    let covi = cove_index::build::build_covi_from_cove_bytes(
        &bytes,
        &cove_index::build::CoviBuildOptions {
            all_columns: true,
            include_index_only_min_max: true,
            ..cove_index::build::CoviBuildOptions::default()
        },
    )
    .unwrap();
    let plain_state = bootstrap_bytes("items_plain", bytes.clone()).unwrap();
    let identity = plain_state.file(0).unwrap().identity();
    let digest =
        cove_core::digest::compute_digest(cove_core::constants::DigestAlgorithm::Sha256, &bytes)
            .expect("sha256 digest");
    let context = cove_index::execution::CoviValidationContextV2::for_file(
        identity.file_id,
        identity.file_len,
        identity.footer_crc32c,
    )
    .with_dataset_id(identity.file_id)
    .with_file_code_keys(true)
    .with_file_digest(cove_core::constants::DigestAlgorithm::Sha256, digest);
    cove_index::execution::ValidatedCoviArtifactV2::parse_and_validate(&covi, context).unwrap();
    let state = bootstrap_bytes_with_covi_artifacts(
        "items",
        bytes,
        vec![covi],
        CoveTableOptions::default(),
    )
    .unwrap();
    assert_eq!(state.bootstrap_stats().covi_sidecars_loaded, 1);

    let plan = exact_covi_unfiltered_min_max(
        &state,
        &[
            (0, cove_index::execution::CoviAggregateKindV2::Min),
            (0, cove_index::execution::CoviAggregateKindV2::Max),
        ],
    )
    .unwrap();

    assert!(plan.is_some());
}

#[cfg(feature = "covi")]
#[tokio::test]
async fn generated_covi_sum_avg_answers_feed_datafusion_metadata_path() {
    let bytes = primitive_events_file();
    let covi = cove_index::build::build_covi_from_cove_bytes(
        &bytes,
        &cove_index::build::CoviBuildOptions {
            all_columns: true,
            include_index_only_sum_avg: true,
            ..cove_index::build::CoviBuildOptions::default()
        },
    )
    .unwrap();
    let path = write_temp_cove("generated_covi_sum_avg", bytes);
    let covi_path = PathBuf::from(format!("{}.covi", path.display()));
    fs::write(&covi_path, covi).unwrap();
    let ctx = SessionContext::new();
    let provider = register_cove_file(&ctx, "events", &path).unwrap();
    assert_eq!(provider.state().bootstrap_stats().covi_sidecars_loaded, 1);

    let batches = ctx
        .sql("SELECT SUM(id) AS total, AVG(id) AS mean FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+-------+------+",
        "| total | mean |",
        "+-------+------+",
        "| 6     | 2.0  |",
        "+-------+------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT SUM(id) AS total, AVG(id) AS mean FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
    fs::remove_file(covi_path).unwrap();
}

#[tokio::test]
async fn native_i64_scalar_aggregates_use_native_aggregate_exec() {
    let path = write_temp_cove("native_i64_aggregates", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT SUM(id) AS total, AVG(id) AS mean, MIN(id) AS lo, MAX(id) AS hi FROM events",
        &[
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-------+------+----+----+",
        "| total | mean | lo | hi |",
        "+-------+------+----+----+",
        "| 6     | 2.0  | 1  | 3  |",
        "+-------+------+----+----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 3, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT SUM(id), AVG(id), MIN(id), MAX(id) FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeAggregateExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_i64_group_count_uses_native_group_exec() {
    let path = write_temp_cove("native_i64_group_count", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id, COUNT(*) AS n FROM events GROUP BY id ORDER BY id",
        &[
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+----+---+",
        "| id | n |",
        "+----+---+",
        "| 1  | 1 |",
        "| 2  | 1 |",
        "| 3  | 1 |",
        "+----+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 3, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id, COUNT(*) AS n FROM events GROUP BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupCountExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_bool_group_count_uses_native_group_exec() {
    let path = write_temp_cove("native_bool_group_count", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT active, COUNT(*) AS n FROM events GROUP BY active ORDER BY active",
        &[
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+--------+---+",
        "| active | n |",
        "+--------+---+",
        "| false  | 1 |",
        "| true   | 2 |",
        "+--------+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert_eq!(metrics[1], 3, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT active, COUNT(*) AS n FROM events GROUP BY active")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupCountExec"),
        "{explain_text}"
    );
    assert_bool_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_bool_i64_group_aggregates_use_native_exec() {
    let path = write_temp_cove(
        "filtered_native_bool_i64_group_aggs",
        primitive_events_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let sql = "SELECT active, SUM(id) AS total, AVG(id) AS mean, MIN(id) AS lo, MAX(id) AS hi, COUNT(id) AS c \
               FROM events WHERE id >= 2 GROUP BY active ORDER BY active";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_lane_predicates",
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+--------+-------+------+----+----+---+",
        "| active | total | mean | lo | hi | c |",
        "+--------+-------+------+----+----+---+",
        "| false  | 2     | 2.0  | 2  | 2  | 1 |",
        "| true   | 3     | 3.0  | 3  | 3  | 1 |",
        "+--------+-------+------+----+----+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql(
            "EXPLAIN SELECT active, SUM(id) AS total, AVG(id) AS mean, MIN(id) AS lo, \
             MAX(id) AS hi, COUNT(id) AS c \
             FROM events WHERE id >= 2 GROUP BY active",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeBoolI64GroupAggregateExec"),
        "{explain_text}"
    );
    assert_bool_i64_group_aggregate_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_i64_group_aggregates_use_native_exec() {
    let path = write_temp_cove("filtered_native_i64_i64_group_aggs", numeric_scores_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "scores", &path).unwrap();

    let sql = "SELECT id, SUM(score) AS total, AVG(score) AS mean, MIN(score) AS lo, MAX(score) AS hi, COUNT(score) AS c \
               FROM scores WHERE score >= 20 GROUP BY id ORDER BY id";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_lane_predicates",
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+----+-------+------+----+----+---+",
        "| id | total | mean | lo | hi | c |",
        "+----+-------+------+----+----+---+",
        "| 1  | 80    | 40.0 | 30 | 50 | 2 |",
        "| 2  | 60    | 30.0 | 20 | 40 | 2 |",
        "+----+-------+------+----+----+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 4, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql(
            "EXPLAIN SELECT id, SUM(score) AS total, AVG(score) AS mean, MIN(score) AS lo, \
             MAX(score) AS hi, COUNT(score) AS c \
             FROM scores WHERE score >= 20 GROUP BY id",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64I64GroupAggregateExec"),
        "{explain_text}"
    );
    assert_i64_i64_group_aggregate_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_bool_distinct_group_uses_native_group_exec() {
    let path = write_temp_cove(
        "filtered_native_bool_distinct_group",
        primitive_events_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT active FROM events WHERE id >= 2 GROUP BY active ORDER BY active",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+--------+",
        "| active |",
        "+--------+",
        "| false  |",
        "| true   |",
        "+--------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT active FROM events WHERE id >= 2 GROUP BY active")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupDistinctExec"),
        "{explain_text}"
    );
    assert_bool_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_bool_select_distinct_uses_native_group_exec() {
    let path = write_temp_cove(
        "filtered_native_bool_select_distinct",
        primitive_events_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT DISTINCT active FROM events WHERE id >= 2 ORDER BY active",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+--------+",
        "| active |",
        "+--------+",
        "| false  |",
        "| true   |",
        "+--------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT DISTINCT active FROM events WHERE id >= 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupDistinctExec"),
        "{explain_text}"
    );
    assert_bool_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_distinct_group_uses_native_group_exec() {
    let path = write_temp_cove(
        "filtered_native_i64_distinct_group",
        primitive_events_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id FROM events WHERE id >= 2 GROUP BY id ORDER BY id",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = ["+----+", "| id |", "+----+", "| 2  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE id >= 2 GROUP BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupDistinctExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_select_distinct_uses_native_group_exec() {
    let path = write_temp_cove(
        "filtered_native_i64_select_distinct",
        primitive_events_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT DISTINCT id FROM events WHERE id >= 2 ORDER BY id",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = ["+----+", "| id |", "+----+", "| 2  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT DISTINCT id FROM events WHERE id >= 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupDistinctExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_scalar_aggregates_use_native_filter_and_aggregate_exec() {
    let path = write_temp_cove("filtered_native_i64_aggregates", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let sql =
        "SELECT SUM(id) AS total, AVG(id) AS mean, MIN(id) AS lo, MAX(id) AS hi FROM events WHERE id >= 2";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_lane_predicates",
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-------+------+----+----+",
        "| total | mean | lo | hi |",
        "+-------+------+----+----+",
        "| 5     | 2.5  | 2  | 3  |",
        "+-------+------+----+----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT SUM(id), AVG(id), MIN(id), MAX(id) FROM events WHERE id >= 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeAggregateExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_group_count_uses_native_filter_and_group_exec() {
    let path = write_temp_cove("filtered_native_i64_group_count", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id, COUNT(*) AS n FROM events WHERE id >= 2 GROUP BY id ORDER BY id",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+----+---+",
        "| id | n |",
        "+----+---+",
        "| 2  | 1 |",
        "| 3  | 1 |",
        "+----+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id, COUNT(*) AS n FROM events WHERE id >= 2 GROUP BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeGroupCountExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_count_column_uses_native_filter_and_aggregate_exec() {
    let path = write_temp_cove("filtered_native_i64_count_column", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT COUNT(id) AS present FROM events WHERE id >= 2",
        &[
            "cove_native_lane_predicates",
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+---------+",
        "| present |",
        "+---------+",
        "| 2       |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(id) AS present FROM events WHERE id >= 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeAggregateExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_count_star_uses_native_filter_and_count_exec() {
    let path = write_temp_cove("filtered_native_count_star", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT COUNT(*) AS rows FROM events WHERE id >= 2",
        &[
            "cove_native_lane_predicates",
            "cove_native_count_scans",
            "cove_native_count_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = ["+------+", "| rows |", "+------+", "| 2    |", "+------+"];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(*) AS rows FROM events WHERE id >= 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeCountExec"),
        "{explain_text}"
    );
    assert_rowset_count_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[cfg(feature = "covi")]
#[tokio::test]
async fn covi_sum_avg_answers_feed_datafusion_metadata_path() {
    let bytes = float_metrics_file();
    let covi = float_metric_sum_avg_covi_artifact(&bytes);
    let path = write_temp_cove("covi_sum_avg_metrics", bytes);
    let covi_path = PathBuf::from(format!("{}.covi", path.display()));
    fs::write(&covi_path, covi).unwrap();
    let ctx = SessionContext::new();
    let provider = register_cove_file(&ctx, "metrics", &path).unwrap();
    assert_eq!(provider.state().bootstrap_stats().covi_sidecars_loaded, 1);

    let batches = ctx
        .sql("SELECT SUM(f64) AS total, AVG(f64) AS mean FROM metrics")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+-------+------+",
        "| total | mean |",
        "+-------+------+",
        "| 0.75  | 0.25 |",
        "+-------+------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT SUM(f64) AS total, AVG(f64) AS mean FROM metrics")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
    fs::remove_file(covi_path).unwrap();
}

#[tokio::test]
async fn exact_null_filters_push_down_and_remain_correct() {
    let path = write_temp_cove("nullable_residual", nullable_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let is_null = ctx
        .sql("SELECT id FROM events WHERE maybe IS NULL ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_null = ["+----+", "| id |", "+----+", "| 2  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected_null, &is_null);

    let is_not_null = ctx
        .sql("SELECT id FROM events WHERE maybe IS NOT NULL ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_not_null = ["+----+", "| id |", "+----+", "| 1  |", "| 4  |", "+----+"];
    assert_batches_eq!(expected_not_null, &is_not_null);

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE maybe IS NULL")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"));
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn explain_select_star_mentions_cove_exec() {
    let path = write_temp_cove("events_explain", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("EXPLAIN SELECT * FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&batches).unwrap().to_string();

    assert!(explain_text.contains("CoveExec"));
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_values_are_decoded() {
    let path = write_temp_cove("items", dictionary_items_file(sample_dictionary()));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let (batches, decoded_fallback_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_decoded_fallback_rows",
    )
    .await;

    let expected = [
        "+------+", "| name |", "+------+", "| red  |", "| blue |", "+------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(decoded_fallback_rows, 2);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_output_is_opt_in() {
    let path = write_temp_cove(
        "items_dictionary",
        dictionary_items_file(sample_dictionary()),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let array = batches[0].column(0);
    assert!(array
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .is_some());
    let expected = [
        "+------+", "| name |", "+------+", "| red  |", "| blue |", "+------+",
    ];
    assert_batches_eq!(expected, &batches);

    let filtered = ctx
        .sql("SELECT name FROM items WHERE name = 'red'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let filtered_expected = ["+------+", "| name |", "+------+", "| red  |", "+------+"];
    assert_batches_eq!(filtered_expected, &filtered);

    let grouped = ctx
        .sql("SELECT name, COUNT(*) AS n FROM items GROUP BY name ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let grouped_expected = [
        "+------+---+",
        "| name | n |",
        "+------+---+",
        "| blue | 1 |",
        "| red  | 1 |",
        "+------+---+",
    ];
    assert_batches_eq!(grouped_expected, &grouped);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_output_uses_direct_key_export_and_value_cache() {
    let path = write_temp_cove(
        "items_dictionary_metrics",
        dictionary_items_file_with_domain_stats(),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .unwrap();

    let (batches, key_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_keys_rows",
    )
    .await;
    let expected = [
        "+------+", "| name |", "+------+", "| red  |", "| blue |", "+------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(key_rows, 2);

    let (_, value_bytes) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_values_bytes",
    )
    .await;
    let (_, cache_misses) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_value_cache_misses",
    )
    .await;
    let (_, cache_hits) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_value_cache_hits",
    )
    .await;

    assert!(value_bytes > 0);
    assert_eq!(cache_misses, 1);
    assert_eq!(cache_hits, 1);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_output_remaps_mixed_file_dictionary() {
    let path = write_temp_cove("mixed_dictionary", mixed_dictionary_items_file());
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .unwrap();

    let (batches, remapped_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM items",
        "cove_filecode_dictionary_remapped_rows",
    )
    .await;
    let expected = [
        "+------+", "| name |", "+------+", "| red  |", "| blue |", "+------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(remapped_rows, 2);

    let dictionary = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert_eq!(dictionary.keys().value(0), 0);
    assert_eq!(dictionary.keys().value(1), 1);
    let values = dictionary
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values.value(0), "red");
    assert_eq!(values.value(1), "blue");

    let blob_batches = ctx
        .sql("SELECT blob FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let blob_dictionary = blob_batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    let blob_values = blob_dictionary
        .values()
        .as_any()
        .downcast_ref::<BinaryArray>()
        .unwrap();
    assert_eq!(blob_values.len(), 1);
    assert_eq!(blob_values.value(0), &[0xaa, 0xbb]);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_output_ignores_view_values_for_filecode_columns() {
    let path = write_temp_cove(
        "items_dictionary_view_options",
        dictionary_items_file(sample_dictionary()),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default()
            .with_arrow_dictionary_output()
            .with_arrow_view_output(),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let dictionary = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert!(dictionary
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .is_some());
    assert!(dictionary
        .values()
        .as_any()
        .downcast_ref::<StringViewArray>()
        .is_none());
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn unrelated_redacted_dictionary_entry_does_not_block_projection() {
    let path = write_temp_cove(
        "redacted_mixed_dictionary",
        redacted_mixed_dictionary_items_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| name |", "+------+", "| red  |", "+------+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_dictionary_output_redacted_values_fail_projection() {
    let path = write_temp_cove(
        "redacted_dictionary",
        dictionary_items_file(redacted_dictionary()),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items",
        &path,
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .unwrap();

    let err = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("COVE_E_REDACTION_POLICY"), "{err}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn redacted_dictionary_value_fails_projection() {
    let path = write_temp_cove("redacted", dictionary_items_file(redacted_dictionary()));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let err = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("COVE_E_REDACTION_POLICY"), "{err}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn redacted_filecode_count_column_does_not_use_metadata_fast_path() {
    let path = write_temp_cove(
        "redacted_count",
        dictionary_items_file(redacted_dictionary()),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let err = ctx
        .sql("SELECT COUNT(name) AS present FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("COVE_E_REDACTION_POLICY"), "{err}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filecode_without_dictionary_is_rejected_at_registration() {
    let bytes =
        include_bytes!("../../../../conformance/reject/cove_t_filecode_missing_dictionary.cove");
    assert!(
        matches!(
            bootstrap_bytes("filecode_missing_dictionary", bytes.to_vec()),
            Err(CoveError::BadFileCode)
        ),
        "missing FileCode dictionary must fail ordinary table scan validation"
    );

    let path = write_temp_cove("filecode_missing_dictionary", bytes.to_vec());
    let ctx = SessionContext::new();
    let err = register_cove_file(&ctx, "items", &path).unwrap_err();

    assert!(err.to_string().contains("BAD_FILECODE"), "{err}");
    fs::remove_file(path).unwrap();
}
