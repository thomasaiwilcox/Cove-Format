use super::*;

#[test]
fn m4d_bootstrap_parses_aggregate_composite_and_topn_metadata() {
    let state = bootstrap_bytes("m4d_metadata", dictionary_items_file_with_m4d_metadata()).unwrap();

    assert_eq!(state.aggregate_entries_for(1).len(), 1);
    assert_eq!(state.composite_indexes().count(), 1);
    assert_eq!(state.topn_for(1).len(), 1);
}

#[tokio::test]
async fn m4d_metadata_count_star_rewrites_to_memtable() {
    let path = write_temp_cove("m4d_count_star", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 3    |", "+------+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        !explain_text.contains("CoveExec"),
        "metadata COUNT should not scan COVE data: {explain_text}"
    );
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m4d_metadata_count_nullable_column_uses_synopsis_or_native_count() {
    let exact_path = write_temp_cove(
        "m4d_count_nullable_exact",
        nullable_events_file_with_count(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &exact_path).unwrap();

    let batches = ctx
        .sql("SELECT COUNT(maybe) AS present FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+---------+",
        "| present |",
        "+---------+",
        "| 2       |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &batches);
    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(maybe) AS present FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(exact_path).unwrap();

    let fallback_path = write_temp_cove("m4d_count_nullable_fallback", nullable_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &fallback_path).unwrap();
    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(maybe) AS present FROM events")
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
    fs::remove_file(fallback_path).unwrap();
}

#[test]
fn m4d_composite_tuple_prunes_multi_column_filecode_filters() {
    let state = bootstrap_bytes("composite", composite_pairs_file()).unwrap();
    let projection = vec![2];
    let left = FilterPlan::pruning_file_code_in(0, vec![0], "left = 'red'");
    let right = FilterPlan::pruning_file_code_in(1, vec![1], "right = 'blue'");
    let plan = plan_scan(&state, Some(&projection), vec![left, right]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| hit     |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.morsels_considered, 2);
    assert_eq!(decoded.stats.morsels_pruned, 1);
}

#[tokio::test]
async fn m4d_topn_optimizer_rewrites_single_i64_projection_to_native_order() {
    let path = write_temp_cove("m4d_topn", topn_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id FROM events ORDER BY id DESC LIMIT 1",
        &[
            "cove_native_sort_kernels",
            "cove_native_sort_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = ["+----+", "| id |", "+----+", "| 9  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events ORDER BY id DESC LIMIT 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64OrderExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("SortExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_i64_order_without_limit_uses_native_order_exec() {
    let path = write_temp_cove("native_i64_order_without_limit", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id FROM events ORDER BY id DESC",
        &[
            "cove_native_sort_kernels",
            "cove_native_sort_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+----+", "| id |", "+----+", "| 3  |", "| 2  |", "| 1  |", "+----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert_eq!(metrics[1], 3, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events ORDER BY id DESC")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64OrderExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("SortExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_topn_order_uses_native_filter_and_order_exec() {
    let path = write_temp_cove("filtered_native_i64_topn_order", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT id FROM events WHERE id >= 2 ORDER BY id DESC LIMIT 1",
        &[
            "cove_native_lane_predicates",
            "cove_native_sort_kernels",
            "cove_native_sort_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = ["+----+", "| id |", "+----+", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 1, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE id >= 2 ORDER BY id DESC LIMIT 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64OrderExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("SortExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_i64_inner_join_uses_native_join_exec() {
    let left_path = write_temp_cove("native_i64_join_left", topn_events_file());
    let right_path = write_temp_cove("native_i64_join_right", topn_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_events", &left_path).unwrap();
    register_cove_file(&ctx, "right_events", &right_path).unwrap();

    let sql = "SELECT l.id AS lid, r.id AS rid \
               FROM left_events AS l JOIN right_events AS r ON l.id = r.id \
               ORDER BY lid";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-----+-----+",
        "| lid | rid |",
        "+-----+-----+",
        "| 1   | 1   |",
        "| 9   | 9   |",
        "+-----+-----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 3, "{metrics:?}");
    assert_eq!(metrics[1], 6, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.id AS lid, r.id AS rid FROM left_events AS l JOIN right_events AS r ON l.id = r.id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64JoinExec"),
        "{explain_text}"
    );
    assert_typed_i64_native_contract(&explain_text);
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_filecode_inner_join_uses_native_join_exec_across_swapped_dictionaries() {
    let left_path = write_temp_cove(
        "native_filecode_join_left",
        dictionary_items_file(sample_dictionary()),
    );
    let right_path = write_temp_cove(
        "native_filecode_join_right",
        dictionary_items_file(swapped_dictionary()),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_items", &left_path).unwrap();
    register_cove_file(&ctx, "right_items", &right_path).unwrap();

    let sql = "SELECT l.name AS lname, r.name AS rname \
               FROM left_items AS l JOIN right_items AS r ON l.name = r.name \
               ORDER BY lname";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-------+-------+",
        "| lname | rname |",
        "+-------+-------+",
        "| blue  | blue  |",
        "| red   | red   |",
        "+-------+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 3, "{metrics:?}");
    assert_eq!(metrics[1], 6, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.name AS lname, r.name AS rname FROM left_items AS l JOIN right_items AS r ON l.name = r.name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeFileCodeJoinExec"),
        "{explain_text}"
    );
    assert_filecode_join_native_contract(&explain_text);
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_filecode_left_semi_and_anti_join_use_native_join_exec() {
    let left_path = write_temp_cove(
        "native_filecode_join_semi_anti_left",
        filecode_key_file(sample_dictionary(), &[0, 1, 0]),
    );
    let right_path = write_temp_cove(
        "native_filecode_join_semi_anti_right",
        filecode_key_file(swapped_dictionary(), &[1]),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_items", &left_path).unwrap();
    register_cove_file(&ctx, "right_items", &right_path).unwrap();

    let semi_sql = "SELECT l.name AS name \
                    FROM left_items AS l LEFT SEMI JOIN right_items AS r ON l.name = r.name \
                    ORDER BY name";
    let (semi_batches, semi_metrics) = collect_sql_with_cove_metrics(
        &ctx,
        semi_sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let semi_expected = [
        "+------+", "| name |", "+------+", "| red  |", "| red  |", "+------+",
    ];
    assert_batches_eq!(semi_expected, &semi_batches);
    assert_eq!(semi_metrics[0], 3, "{semi_metrics:?}");
    assert_eq!(semi_metrics[1], 6, "{semi_metrics:?}");
    assert_eq!(semi_metrics[2], 0, "{semi_metrics:?}");

    let anti_sql = "SELECT l.name AS name \
                    FROM left_items AS l LEFT ANTI JOIN right_items AS r ON l.name = r.name \
                    ORDER BY name";
    let (anti_batches, anti_metrics) = collect_sql_with_cove_metrics(
        &ctx,
        anti_sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let anti_expected = ["+------+", "| name |", "+------+", "| blue |", "+------+"];
    assert_batches_eq!(anti_expected, &anti_batches);
    assert_eq!(anti_metrics[0], 3, "{anti_metrics:?}");
    assert_eq!(anti_metrics[1], 5, "{anti_metrics:?}");
    assert_eq!(anti_metrics[2], 0, "{anti_metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.name AS name FROM left_items AS l LEFT SEMI JOIN right_items AS r ON l.name = r.name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeFileCodeJoinExec"),
        "{explain_text}"
    );
    assert_filecode_join_native_contract(&explain_text);
    assert!(explain_text.contains("kind=left_semi"), "{explain_text}");
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_inner_join_skips_null_keys() {
    let left_path = write_temp_cove("native_i64_join_nulls_left", nullable_i64_key_file());
    let right_path = write_temp_cove("native_i64_join_nulls_right", nullable_i64_key_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT l.maybe AS left_maybe, r.maybe AS right_maybe \
         FROM left_keys AS l JOIN right_keys AS r ON l.maybe = r.maybe \
         ORDER BY left_maybe",
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+------------+-------------+",
        "| left_maybe | right_maybe |",
        "+------------+-------------+",
        "| 10         | 10          |",
        "| 40         | 40          |",
        "+------------+-------------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 3, "{metrics:?}");
    assert_eq!(metrics[1], 10, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_inner_join_uses_row_pair_kernel_for_duplicate_negative_keys() {
    let left_path = write_temp_cove(
        "native_i64_join_duplicate_negative_left",
        i64_key_file(&[-2, 3, -2]),
    );
    let right_path = write_temp_cove(
        "native_i64_join_duplicate_negative_right",
        i64_key_file(&[-2, -2, 4]),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT l.id AS lid, r.id AS rid \
         FROM left_keys AS l JOIN right_keys AS r ON l.id = r.id \
         ORDER BY lid, rid",
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_native_join_dispatch_scalar",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-----+-----+",
        "| lid | rid |",
        "+-----+-----+",
        "| -2  | -2  |",
        "| -2  | -2  |",
        "| -2  | -2  |",
        "| -2  | -2  |",
        "+-----+-----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(metrics[0], 3, "{metrics:?}");
    assert_eq!(metrics[1], 10, "{metrics:?}");
    assert_eq!(metrics[2], 3, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_left_semi_and_anti_join_use_native_join_exec() {
    let left_path = write_temp_cove(
        "native_i64_join_semi_anti_left",
        i64_key_file(&[1, 2, 3, 3]),
    );
    let right_path = write_temp_cove("native_i64_join_semi_anti_right", i64_key_file(&[1, 3]));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let semi_sql = "SELECT l.id AS id \
                    FROM left_keys AS l LEFT SEMI JOIN right_keys AS r ON l.id = r.id \
                    ORDER BY id";
    let (semi_batches, semi_metrics) = collect_sql_with_cove_metrics(
        &ctx,
        semi_sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let semi_expected = [
        "+----+", "| id |", "+----+", "| 1  |", "| 3  |", "| 3  |", "+----+",
    ];
    assert_batches_eq!(semi_expected, &semi_batches);
    assert_eq!(semi_metrics[0], 3, "{semi_metrics:?}");
    assert_eq!(semi_metrics[1], 9, "{semi_metrics:?}");
    assert_eq!(semi_metrics[2], 0, "{semi_metrics:?}");

    let anti_sql = "SELECT l.id AS id \
                    FROM left_keys AS l LEFT ANTI JOIN right_keys AS r ON l.id = r.id \
                    ORDER BY id";
    let (anti_batches, anti_metrics) = collect_sql_with_cove_metrics(
        &ctx,
        anti_sql,
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let anti_expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(anti_expected, &anti_batches);
    assert_eq!(anti_metrics[0], 3, "{anti_metrics:?}");
    assert_eq!(anti_metrics[1], 7, "{anti_metrics:?}");
    assert_eq!(anti_metrics[2], 0, "{anti_metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.id AS id FROM left_keys AS l LEFT SEMI JOIN right_keys AS r ON l.id = r.id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64JoinExec"),
        "{explain_text}"
    );
    assert!(explain_text.contains("kind=left_semi"), "{explain_text}");
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_left_anti_join_keeps_unmatched_null_left_keys() {
    let left_path = write_temp_cove("native_i64_left_anti_null_left", nullable_i64_key_file());
    let right_path = write_temp_cove("native_i64_left_anti_null_right", i64_key_file(&[10]));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT l.maybe AS maybe \
         FROM left_keys AS l LEFT ANTI JOIN right_keys AS r ON l.maybe = r.id \
         ORDER BY maybe NULLS FIRST",
        &[
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-------+",
        "| maybe |",
        "+-------+",
        "|       |",
        "|       |",
        "| 40    |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(metrics[0], 3, "{metrics:?}");
    assert_eq!(metrics[1], 8, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.maybe AS maybe FROM left_keys AS l LEFT ANTI JOIN right_keys AS r ON l.maybe = r.id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64JoinExec"),
        "{explain_text}"
    );
    assert!(explain_text.contains("kind=left_anti"), "{explain_text}");
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_not_in_with_null_subquery_does_not_use_native_anti_join() {
    let left_path = write_temp_cove("native_i64_not_in_null_left", i64_key_file(&[10, 40, 50]));
    let right_path = write_temp_cove("native_i64_not_in_null_right", nullable_i64_key_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let batches = ctx
        .sql(
            "SELECT id FROM left_keys \
             WHERE id NOT IN (SELECT maybe FROM right_keys) \
             ORDER BY id",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        0
    );

    let explain = ctx
        .sql(
            "EXPLAIN SELECT id FROM left_keys \
             WHERE id NOT IN (SELECT maybe FROM right_keys)",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        !explain_text.contains("CoveNativeI64JoinExec"),
        "{explain_text}"
    );
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn filtered_native_i64_inner_join_inputs_use_native_join_exec() {
    let left_path = write_temp_cove("filtered_native_i64_join_left", i64_key_file(&[1, 2, 3]));
    let right_path = write_temp_cove("filtered_native_i64_join_right", i64_key_file(&[2, 3]));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "left_keys", &left_path).unwrap();
    register_cove_file(&ctx, "right_keys", &right_path).unwrap();

    let sql = "SELECT l.id AS lid, r.id AS rid \
               FROM (SELECT id FROM left_keys WHERE id >= 2) AS l \
               JOIN right_keys AS r ON l.id = r.id \
               ORDER BY lid";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_lane_predicates",
            "cove_native_join_kernels",
            "cove_native_join_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-----+-----+",
        "| lid | rid |",
        "+-----+-----+",
        "| 2   | 2   |",
        "| 3   | 3   |",
        "+-----+-----+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert!(metrics[1] >= 3, "{metrics:?}");
    assert_eq!(metrics[2], 6, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT l.id AS lid, r.id AS rid FROM (SELECT id FROM left_keys WHERE id >= 2) AS l JOIN right_keys AS r ON l.id = r.id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64JoinExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("HashJoinExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(left_path).unwrap();
    fs::remove_file(right_path).unwrap();
}

#[tokio::test]
async fn native_i64_topn_order_respects_explicit_null_order() {
    let path = write_temp_cove("native_i64_topn_order_nulls", nullable_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT maybe FROM events ORDER BY maybe ASC NULLS FIRST LIMIT 3",
        &[
            "cove_native_sort_kernels",
            "cove_native_sort_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+-------+",
        "| maybe |",
        "+-------+",
        "|       |",
        "|       |",
        "| 10    |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert_eq!(metrics[1], 4, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT maybe FROM events ORDER BY maybe ASC NULLS FIRST LIMIT 3")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeI64OrderExec"),
        "{explain_text}"
    );
    assert!(!explain_text.contains("SortExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m4e_overlay_snapshot_applies_file_and_row_visibility() {
    let dir = make_temp_dir("m4e_overlay");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(&first, primitive_events_file()).unwrap();
    fs::write(&second, primitive_events_file()).unwrap();
    let first_state = cove_table_from_path(&first).unwrap();
    let second_state = cove_table_from_path(&second).unwrap();
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "overlay-1".into(),
        files: vec![
            OverlayFile {
                uri: first.display().to_string().into(),
                expected_identity: Some(identity_for_state(first_state.state())),
                visibility: RowVisibility::DeletedRanges(vec![RowRange { start: 1, len: 1 }]),
            },
            OverlayFile {
                uri: second.display().to_string().into(),
                expected_identity: Some(identity_for_state(second_state.state())),
                visibility: RowVisibility::VisibleRanges(Vec::new()),
            },
        ],
    };

    let ctx = SessionContext::new();
    let provider =
        register_cove_overlay_snapshot(&ctx, "events", snapshot, CoveTableOptions::default())
            .unwrap();
    assert_eq!(provider.state().file_count(), 1);
    assert_eq!(provider.state().bootstrap_stats().overlay_files_hidden, 1);
    assert_eq!(provider.statistics().unwrap().num_rows, Precision::Exact(2));

    let batches = ctx
        .sql("SELECT id FROM events ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+----+", "| id |", "+----+", "| 1  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let count = ctx
        .sql("SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 2    |", "+------+"];
    assert_batches_eq!(expected, &count);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn m4e_overlay_rejects_stale_identity_unless_file_is_hidden() {
    let path = write_temp_cove("m4e_overlay_stale", primitive_events_file());
    let mut identity = identity_for_state(cove_table_from_path(&path).unwrap().state());
    identity.footer_crc32c ^= 1;

    let visible = CoveOverlaySnapshot {
        snapshot_id: "overlay-stale".into(),
        files: vec![OverlayFile {
            uri: path.display().to_string().into(),
            expected_identity: Some(identity.clone()),
            visibility: RowVisibility::All,
        }],
    };
    let ctx = SessionContext::new();
    let err = register_cove_overlay_snapshot(&ctx, "events", visible, CoveTableOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("overlay identity mismatch"), "{err}");

    let hidden = CoveOverlaySnapshot {
        snapshot_id: "overlay-hidden-stale".into(),
        files: vec![
            OverlayFile {
                uri: path.display().to_string().into(),
                expected_identity: Some(identity),
                visibility: RowVisibility::VisibleRanges(Vec::new()),
            },
            OverlayFile {
                uri: path.display().to_string().into(),
                expected_identity: None,
                visibility: RowVisibility::All,
            },
        ],
    };
    register_cove_overlay_snapshot(&ctx, "events_ok", hidden, CoveTableOptions::default()).unwrap();
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m5_cove_e_metadata_survives_full_range_and_overlay_bootstrap() {
    let bytes = dictionary_items_file_with_lookup_and_cove_e(sample_dictionary(), true);
    let state = bootstrap_bytes("items_bytes", bytes.clone()).unwrap();
    assert_eq!(
        state.mounted().engine_metadata.execution_descriptors.len(),
        1
    );
    assert_eq!(
        state.mounted().engine_metadata.engine_mount_policies.len(),
        1
    );

    let path = write_temp_cove("m5_cove_e_range", bytes);
    let provider = cove_table_from_path(&path).unwrap();
    assert_eq!(
        provider.state().files()[0]
            .mounted()
            .engine_metadata
            .execution_descriptors
            .len(),
        1
    );

    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "m5-overlay".into(),
        files: vec![OverlayFile {
            uri: path.display().to_string().into(),
            expected_identity: Some(identity_for_state(provider.state())),
            visibility: RowVisibility::All,
        }],
    };
    let ctx = SessionContext::new();
    let overlay =
        register_cove_overlay_snapshot(&ctx, "items", snapshot, CoveTableOptions::default())
            .unwrap();
    assert_eq!(
        overlay.state().files()[0]
            .mounted()
            .engine_metadata
            .execution_descriptors
            .len(),
        1
    );
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m5_execution_code_policy_controls_unsupported_filecode_filters() {
    let path = write_temp_cove(
        "m5_unsupported_cove_e",
        dictionary_items_file_with_lookup_and_cove_e(sample_dictionary(), false),
    );
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "items_disabled",
        &path,
        CoveTableOptions::default().with_execution_code_policy(ExecutionCodePolicy::Disabled),
    )
    .unwrap();
    let batches = ctx
        .sql("SELECT payload FROM items_disabled WHERE name = 'red'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &batches);

    register_cove_file_with_options(
        &ctx,
        "items_required",
        &path,
        CoveTableOptions::default()
            .with_execution_code_policy(ExecutionCodePolicy::RequireSupported),
    )
    .unwrap();
    let err = ctx
        .sql("SELECT payload FROM items_required WHERE name = 'red'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("COVE_E_BAD_ENGINE_PROFILE"), "{err}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m5_metadata_filecode_count_uses_cove_metadata_exec() {
    let path = write_temp_cove(
        "m5_count_filecode",
        dictionary_items_file_with_lookup_and_cove_e(sample_dictionary(), true),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM items WHERE name = 'red'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 1    |", "+------+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT COUNT(*) AS rows FROM items WHERE name = 'red'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveMetadataExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m4e_overlay_rejects_uri_scheme_paths() {
    let path = write_temp_cove("m4e_overlay_file_uri", primitive_events_file());
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "overlay-file-uri".into(),
        files: vec![OverlayFile {
            uri: format!("file://{}", path.display()).into(),
            expected_identity: None,
            visibility: RowVisibility::All,
        }],
    };
    let ctx = SessionContext::new();
    let err = register_cove_overlay_snapshot(&ctx, "events", snapshot, CoveTableOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("URI scheme"), "{err}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn m5_metadata_filecode_group_by_counts_logical_values_across_files() {
    let dir = make_temp_dir("m5_group_overlay");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(
        &first,
        dictionary_items_file_with_lookup_and_cove_e(sample_dictionary(), true),
    )
    .unwrap();
    fs::write(
        &second,
        dictionary_items_file_with_lookup_and_cove_e(swapped_dictionary(), true),
    )
    .unwrap();
    let first_state = cove_table_from_path(&first).unwrap();
    let second_state = cove_table_from_path(&second).unwrap();
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "m5-group-overlay".into(),
        files: vec![
            OverlayFile {
                uri: first.display().to_string().into(),
                expected_identity: Some(identity_for_state(first_state.state())),
                visibility: RowVisibility::All,
            },
            OverlayFile {
                uri: second.display().to_string().into(),
                expected_identity: Some(identity_for_state(second_state.state())),
                visibility: RowVisibility::All,
            },
        ],
    };
    let ctx = SessionContext::new();
    register_cove_overlay_snapshot(&ctx, "items", snapshot, CoveTableOptions::default()).unwrap();
    let batches = ctx
        .sql("SELECT name, COUNT(*) AS rows FROM items GROUP BY name ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+------+------+",
        "| name | rows |",
        "+------+------+",
        "| blue | 2    |",
        "| red  | 2    |",
        "+------+------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT name, COUNT(*) AS rows FROM items GROUP BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_dir_all(dir).unwrap();
}
