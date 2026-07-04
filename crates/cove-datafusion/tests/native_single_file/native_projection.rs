use super::*;

#[test]
fn byte_bootstrap_selects_table_from_multi_table_file() {
    let bytes = multiple_tables_file();
    let state = bootstrap_bytes_with_options(
        "memory://multi",
        bytes.clone(),
        CoveTableOptions::default().with_table_id(2),
    )
    .unwrap();
    assert_eq!(state.table().table_id, 2);
    assert_eq!(state.table().name, "second");

    let state = bootstrap_bytes_with_options(
        "memory://multi",
        bytes,
        CoveTableOptions::default().with_table_name(Some("public".into()), "first".into()),
    )
    .unwrap();
    assert_eq!(state.table().table_id, 1);
}

#[test]
fn byte_bootstrap_rejects_unselected_missing_and_ambiguous_tables() {
    let err = bootstrap_bytes("memory://multi", multiple_tables_file())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("requires cove.table_id or cove.table_name"),
        "{err}"
    );

    let err = bootstrap_bytes_with_options(
        "memory://multi",
        multiple_tables_file(),
        CoveTableOptions::default().with_table_id(99),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("selected table_id 99 not found"), "{err}");

    let err = bootstrap_bytes_with_options(
        "memory://ambiguous",
        ambiguous_table_names_file(),
        CoveTableOptions::default().with_table_name(None, "events".into()),
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("selected table name events is ambiguous"),
        "{err}"
    );
}

#[tokio::test]
async fn metadata_cache_scopes_entries_by_selected_table() {
    let bytes = multiple_tables_file();
    let reader = MemoryRangeReader::new(bytes.clone());
    let cache = CoveMetadataCache::default();

    let first = bootstrap_range_reader_with_options(
        "memory://multi-table-cache",
        bytes.len() as u64,
        &reader,
        CoveTableOptions::default().with_table_id(1),
        Some(&cache),
    )
    .await
    .unwrap();
    let second = bootstrap_range_reader_with_options(
        "memory://multi-table-cache",
        bytes.len() as u64,
        &reader,
        CoveTableOptions::default().with_table_id(2),
        Some(&cache),
    )
    .await
    .unwrap();

    assert_eq!(first.table().table_id, 1);
    assert_eq!(second.table().table_id, 2);
    assert!(!Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn compatibility_filters_are_residual_and_correct() {
    let dir = make_temp_dir("listing_filters");
    fs::write(dir.join("part1.cove"), nullable_events_file()).unwrap();
    let ctx = SessionContext::new();
    register_cove_listing_table(&ctx, "events", dir.to_str().unwrap())
        .await
        .unwrap();

    let batches = ctx
        .sql("SELECT id FROM events WHERE maybe IS NULL ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+----+", "| id |", "+----+", "| 2  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE maybe IS NULL")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("FilterExec") || explain_text.contains("Filter"));
    assert!(
        explain_text.contains("cove_advisory_filters=1"),
        "{explain_text}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn listing_exact_residual_policy_elides_only_proven_exact_filters() {
    let dir = make_temp_dir("listing_exact_residual");
    fs::write(dir.join("part1.cove"), primitive_events_file()).unwrap();
    let ctx = SessionContext::new();
    register_cove_listing_table_with_options(
        &ctx,
        "events",
        dir.to_str().unwrap(),
        CoveTableOptions::default()
            .with_filter_residual_policy(FilterResidualPolicy::ElideExactWhenProven),
    )
    .await
    .unwrap();

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE id = 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("cove_advisory_filters=1"),
        "{explain_text}"
    );
    assert!(
        !explain_text.contains("FilterExec"),
        "exact pushed filter should not leave a FilterExec: {explain_text}"
    );

    let batches = ctx
        .sql("SELECT id, name FROM events WHERE id = 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+----+------+",
        "| id | name |",
        "+----+------+",
        "| 2  | beta |",
        "+----+------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn compatibility_uses_range_reads_and_projection_reads_fewer_bytes() {
    let projected = query_counting_store("SELECT name FROM events").await;
    let full = query_counting_store("SELECT * FROM events").await;

    assert_eq!(projected.full_gets, 0);
    assert_eq!(full.full_gets, 0);
    assert!(projected.range_gets > 0);
    assert!(full.range_gets > 0);
    assert!(
        projected.bytes_returned < full.bytes_returned,
        "projected={} full={}",
        projected.bytes_returned,
        full.bytes_returned
    );
}

#[tokio::test]
async fn compatibility_dictionary_output_is_option_aware() {
    let dir = make_temp_dir("listing_dictionary");
    fs::write(
        dir.join("part1.cove"),
        dictionary_items_file(sample_dictionary()),
    )
    .unwrap();
    fs::write(
        dir.join("part2.cove"),
        dictionary_items_file(swapped_dictionary()),
    )
    .unwrap();
    let ctx = SessionContext::new();
    register_cove_listing_table_with_options(
        &ctx,
        "items",
        dir.to_str().unwrap(),
        CoveTableOptions::default().with_arrow_dictionary_output(),
    )
    .await
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM items")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert!(batches.iter().all(|batch| {
        batch
            .column(0)
            .as_any()
            .downcast_ref::<DictionaryArray<UInt32Type>>()
            .is_some()
    }));

    let filtered = ctx
        .sql("SELECT name FROM items WHERE name = 'red' ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let filtered_expected = [
        "+------+", "| name |", "+------+", "| red  |", "| red  |", "+------+",
    ];
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
        "| blue | 2 |",
        "| red  | 2 |",
        "+------+---+",
    ];
    assert_batches_eq!(grouped_expected, &grouped);

    let ordered = ctx
        .sql("SELECT name FROM items ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let ordered_expected = [
        "+------+", "| name |", "+------+", "| blue |", "| blue |", "| red  |", "| red  |",
        "+------+",
    ];
    assert_batches_eq!(ordered_expected, &ordered);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn filtered_filecode_group_count_uses_native_group_exec_across_swapped_dictionaries() {
    let dir = make_temp_dir("native_filecode_group_swapped");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(&first, dictionary_items_file(sample_dictionary())).unwrap();
    fs::write(&second, dictionary_items_file(swapped_dictionary())).unwrap();
    let first_state = cove_table_from_path(&first).unwrap();
    let second_state = cove_table_from_path(&second).unwrap();
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "native-filecode-group-swapped".into(),
        files: vec![
            OverlayFile {
                uri: local_manifest_uri(&first).into(),
                expected_identity: Some(identity_for_state(first_state.state())),
                visibility: RowVisibility::All,
            },
            OverlayFile {
                uri: local_manifest_uri(&second).into(),
                expected_identity: Some(identity_for_state(second_state.state())),
                visibility: RowVisibility::All,
            },
        ],
    };
    let ctx = SessionContext::new();
    register_cove_overlay_snapshot(&ctx, "items", snapshot, CoveTableOptions::default()).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT name, COUNT(*) AS n FROM items WHERE name = 'red' GROUP BY name ORDER BY name",
        &[
            "cove_native_lane_predicates",
            "cove_native_group_kernels",
            "cove_native_group_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+------+---+",
        "| name | n |",
        "+------+---+",
        "| red  | 2 |",
        "+------+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 2, "{metrics:?}");
    assert!(metrics[1] >= 2, "{metrics:?}");
    assert_eq!(metrics[2], 2, "{metrics:?}");
    assert_eq!(metrics[3], 0, "{metrics:?}");

    let explain = ctx
        .sql("EXPLAIN SELECT name, COUNT(*) AS n FROM items WHERE name = 'red' GROUP BY name")
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
    assert!(explain_text.contains("representation=filecode_utf8"));
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn filtered_filecode_distinct_uses_native_group_exec_across_swapped_dictionaries() {
    let dir = make_temp_dir("native_filecode_distinct_swapped");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(&first, dictionary_items_file(sample_dictionary())).unwrap();
    fs::write(&second, dictionary_items_file(swapped_dictionary())).unwrap();
    let first_state = cove_table_from_path(&first).unwrap();
    let second_state = cove_table_from_path(&second).unwrap();
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "native-filecode-distinct-swapped".into(),
        files: vec![
            OverlayFile {
                uri: local_manifest_uri(&first).into(),
                expected_identity: Some(identity_for_state(first_state.state())),
                visibility: RowVisibility::All,
            },
            OverlayFile {
                uri: local_manifest_uri(&second).into(),
                expected_identity: Some(identity_for_state(second_state.state())),
                visibility: RowVisibility::All,
            },
        ],
    };
    let ctx = SessionContext::new();
    register_cove_overlay_snapshot(&ctx, "items", snapshot, CoveTableOptions::default()).unwrap();

    for sql in [
        "SELECT name FROM items WHERE name = 'red' GROUP BY name ORDER BY name",
        "SELECT DISTINCT name FROM items WHERE name = 'red' ORDER BY name",
    ] {
        let (batches, metrics) = collect_sql_with_cove_metrics(
            &ctx,
            sql,
            &[
                "cove_native_lane_predicates",
                "cove_native_group_kernels",
                "cove_native_group_rows_matched",
                "cove_rows_materialized",
            ],
        )
        .await;
        let expected = ["+------+", "| name |", "+------+", "| red  |", "+------+"];
        assert_batches_eq!(expected, &batches);
        assert!(metrics[0] >= 2, "{sql}: {metrics:?}");
        assert!(metrics[1] >= 2, "{sql}: {metrics:?}");
        assert_eq!(metrics[2], 2, "{sql}: {metrics:?}");
        assert_eq!(metrics[3], 0, "{sql}: {metrics:?}");

        let explain = ctx
            .sql(&format!("EXPLAIN {sql}"))
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
        assert!(explain_text.contains("representation=filecode_utf8"));
        assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
        assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn filecode_i64_group_aggregates_use_native_exec_across_swapped_dictionaries() {
    let dir = make_temp_dir("native_filecode_i64_group_aggs_swapped");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(
        &first,
        scored_dictionary_items_file(sample_dictionary(), [10, 20]),
    )
    .unwrap();
    fs::write(
        &second,
        scored_dictionary_items_file(swapped_dictionary(), [30, 40]),
    )
    .unwrap();
    let first_state = cove_table_from_path(&first).unwrap();
    let second_state = cove_table_from_path(&second).unwrap();
    let snapshot = CoveOverlaySnapshot {
        snapshot_id: "native-filecode-i64-group-aggs-swapped".into(),
        files: vec![
            OverlayFile {
                uri: local_manifest_uri(&first).into(),
                expected_identity: Some(identity_for_state(first_state.state())),
                visibility: RowVisibility::All,
            },
            OverlayFile {
                uri: local_manifest_uri(&second).into(),
                expected_identity: Some(identity_for_state(second_state.state())),
                visibility: RowVisibility::All,
            },
        ],
    };
    let ctx = SessionContext::new();
    register_cove_overlay_snapshot(&ctx, "items", snapshot, CoveTableOptions::default()).unwrap();

    let sql =
        "SELECT name, SUM(score) AS total, MIN(score) AS lo, MAX(score) AS hi, COUNT(score) AS c \
               FROM items GROUP BY name ORDER BY name";
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        sql,
        &[
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+------+-------+----+----+---+",
        "| name | total | lo | hi | c |",
        "+------+-------+----+----+---+",
        "| blue | 50    | 20 | 30 | 2 |",
        "| red  | 50    | 10 | 40 | 2 |",
        "+------+-------+----+----+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 2, "{metrics:?}");
    assert_eq!(metrics[1], 4, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql(
            "EXPLAIN SELECT name, SUM(score) AS total, MIN(score) AS lo, MAX(score) AS hi, \
             COUNT(score) AS c FROM items GROUP BY name",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeFileCodeI64GroupAggregateExec"),
        "{explain_text}"
    );
    assert_filecode_i64_group_aggregate_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn filecode_i64_group_aggregates_use_native_exec_over_local_codebook() {
    let path = write_temp_cove(
        "local_codebook_filecode_i64_group_aggs",
        local_codebook_scored_dictionary_items_file(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT name, SUM(score) AS total, COUNT(score) AS c \
         FROM items GROUP BY name ORDER BY name",
        &[
            "cove_native_aggregate_kernels",
            "cove_native_aggregate_rows_matched",
            "cove_rows_materialized",
        ],
    )
    .await;
    let expected = [
        "+------+-------+---+",
        "| name | total | c |",
        "+------+-------+---+",
        "| blue | 60    | 2 |",
        "| red  | 40    | 2 |",
        "+------+-------+---+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(metrics[0] >= 1, "{metrics:?}");
    assert_eq!(metrics[1], 4, "{metrics:?}");
    assert_eq!(metrics[2], 0, "{metrics:?}");

    let explain = ctx
        .sql(
            "EXPLAIN SELECT name, SUM(score) AS total, COUNT(score) AS c \
             FROM items GROUP BY name",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("CoveNativeFileCodeI64GroupAggregateExec"),
        "{explain_text}"
    );
    assert_filecode_i64_group_aggregate_native_contract(&explain_text);
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("CoveMetadataExec"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn select_projected_column_returns_only_projection() {
    let path = write_temp_cove("events_projection", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT name FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| alpha |",
        "| beta  |",
        "| gamma |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(batches.iter().all(|batch| batch.num_columns() == 1));
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn arrow_view_output_returns_view_arrays_and_preserves_values() {
    let path = write_temp_cove("arrow_view_output", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "events",
        &path,
        CoveTableOptions::default().with_arrow_view_output(),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM events WHERE name >= 'beta'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches[0].schema().field(0).data_type(),
        &datafusion::arrow::datatypes::DataType::Utf8View
    );
    let names = batches
        .iter()
        .flat_map(|batch| {
            let array = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringViewArray>()
                .unwrap();
            (0..array.len())
                .map(|row| array.value(row).to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["beta", "gamma"]);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn arrow_view_output_returns_binary_view_arrays() {
    let path = write_temp_cove("arrow_binary_view_output", binary_events_file());
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "events",
        &path,
        CoveTableOptions::default().with_arrow_view_output(),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT payload FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches[0].schema().field(0).data_type(),
        &datafusion::arrow::datatypes::DataType::BinaryView
    );
    let payloads = batches
        .iter()
        .flat_map(|batch| {
            let array = batch
                .column(0)
                .as_any()
                .downcast_ref::<BinaryViewArray>()
                .unwrap();
            (0..array.len())
                .map(|row| array.value(row).to_vec())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        payloads,
        vec![b"short".to_vec(), b"long-binary-payload".to_vec()]
    );
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn arrow_view_output_supports_sort_group_and_topn() {
    let path = write_temp_cove("arrow_view_sort_group", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "events",
        &path,
        CoveTableOptions::default().with_arrow_view_output(),
    )
    .unwrap();

    let sorted = ctx
        .sql("SELECT name FROM events ORDER BY name DESC LIMIT 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        sorted[0].schema().field(0).data_type(),
        &datafusion::arrow::datatypes::DataType::Utf8View
    );
    let expected_sorted = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| gamma |",
        "| beta  |",
        "+-------+",
    ];
    assert_batches_eq!(expected_sorted, &sorted);

    let grouped = ctx
        .sql("SELECT name, COUNT(*) AS n FROM events GROUP BY name ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_grouped = [
        "+-------+---+",
        "| name  | n |",
        "+-------+---+",
        "| alpha | 1 |",
        "| beta  | 1 |",
        "| gamma | 1 |",
        "+-------+---+",
    ];
    assert_batches_eq!(expected_grouped, &grouped);
    fs::remove_file(path).unwrap();
}

#[test]
fn decode_projection_pushdown_decodes_fewer_pages() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full_plan = plan_scan(&state, None, Vec::new()).unwrap();
    let projection = vec![1];
    let projected_plan = plan_scan(&state, Some(&projection), Vec::new()).unwrap();

    let full = decode_scan(&state, &full_plan).unwrap();
    let projected = decode_scan(&state, &projected_plan).unwrap();

    assert_eq!(full.stats.pages_decoded, 6);
    assert_eq!(projected.stats.pages_decoded, 2);
    assert_eq!(projected_plan.scan_projection, vec![1]);
    assert!(projected
        .batches
        .iter()
        .all(|batch| batch.num_columns() == 1));
}

#[test]
fn m6_task_graph_partitions_follow_target_morsel_option() {
    let state = cove_datafusion::bootstrap::bootstrap_bytes_with_options(
        "events",
        primitive_events_file(),
        CoveTableOptions::default().with_target_morsels_per_partition(1),
    )
    .unwrap();
    let plan = plan_scan(&state, None, Vec::new()).unwrap();
    let graph = build_task_graph(&state, &plan).unwrap();

    assert_eq!(graph.tasks.len(), 2);
    assert_eq!(graph.partitions.len(), 2);
    assert!(graph
        .partitions
        .iter()
        .all(|partition| partition.tasks.len() == 1));
}

#[tokio::test]
async fn m6_partitioned_native_scan_preserves_results_under_sort() {
    let path = write_temp_cove("m6_partitioned", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file_with_options(
        &ctx,
        "events",
        &path,
        CoveTableOptions::default().with_target_morsels_per_partition(1),
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT id, name FROM events ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+----+-------+",
        "| id | name  |",
        "+----+-------+",
        "| 1  | alpha |",
        "| 2  | beta  |",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[test]
fn m6_range_coalescing_thresholds_are_configurable() {
    let ranges = vec![0..8, 16..24, 4096..4104];
    let default_count = coalesced_range_count(&ranges, RangeCoalescingOptions::default()).unwrap();
    let tight_count = coalesced_range_count(
        &ranges,
        RangeCoalescingOptions {
            max_gap: 0,
            max_span: 1024 * 1024,
        },
    )
    .unwrap();

    assert_eq!(default_count, 1);
    assert_eq!(tight_count, 3);
}

#[tokio::test]
async fn projection_order_and_exact_filter_are_correct() {
    let path = write_temp_cove("events_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT name, id FROM events WHERE id > 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+-------+----+",
        "| name  | id |",
        "+-------+----+",
        "| beta  | 2  |",
        "| gamma | 3  |",
        "+-------+----+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT name, id FROM events WHERE id > 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"));
    assert!(!explain_text.contains("FilterExec"));
    assert!(explain_text.contains("scan_program="));
    assert!(explain_text.contains("exact_filters=1"));
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn float_numcode_filters_compare_logical_values() {
    let path = write_temp_cove("float_metrics", float_metrics_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "metrics", &path).unwrap();

    let batches = ctx
        .sql("SELECT id FROM metrics WHERE f64 > 2.0 ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let batches = ctx
        .sql("SELECT id FROM metrics WHERE f32 = CAST(2.25 AS REAL) ORDER BY id")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn stats_only_constant_int_and_uint_pages_scan_repeated_values() {
    let path = write_temp_cove("stats_only_numeric", stats_only_numeric_metrics_file(true));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "metrics", &path).unwrap();

    let batches = ctx
        .sql("SELECT signed, unsigned FROM metrics ORDER BY signed, unsigned")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+--------+----------+",
        "| signed | unsigned |",
        "+--------+----------+",
        "| -42    | 42       |",
        "| -42    | 42       |",
        "| -42    | 42       |",
        "+--------+----------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn stats_only_constant_float32_page_preserves_numcode_bits() {
    let nan_bits = 0x7fc1_2345u32;
    let path = write_temp_cove("stats_only_float32", stats_only_float32_file(nan_bits));
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "metrics", &path).unwrap();

    let batches = ctx
        .sql("SELECT f32 FROM metrics")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(batches.len(), 1);
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Float32Array>()
        .unwrap();
    assert_eq!(values.len(), 3);
    for row in 0..values.len() {
        assert_eq!(values.value(row).to_bits(), nan_bits);
    }
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn stats_only_constant_page_without_required_stats_fails_closed() {
    let bytes = stats_only_numeric_metrics_file(false);
    assert!(
        matches!(
            bootstrap_bytes("stats_only_missing_stats", bytes.clone()),
            Err(CoveError::PageCorrupt)
        ),
        "stats-only all-non-null pages must require validated stats"
    );

    let path = write_temp_cove("stats_only_missing_stats", bytes);
    let ctx = SessionContext::new();
    let err = register_cove_file(&ctx, "metrics", &path).unwrap_err();
    assert!(
        err.to_string().contains("PAGE_CORRUPT"),
        "unexpected error: {err}"
    );
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn stats_only_all_null_page_scans_without_value_stream() {
    let path = write_temp_cove(
        "stats_only_all_null",
        include_bytes!(
            "../../../../conformance/accept/cove_t_payload_elision_stats_only_all_null_valid.cove"
        )
        .to_vec(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) AS rows, COUNT(status_code) AS non_nulls FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+------+-----------+",
        "| rows | non_nulls |",
        "+------+-----------+",
        "| 6    | 0         |",
        "+------+-----------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn between_filter_uses_inclusive_lower_and_upper_bounds() {
    let path = write_temp_cove("events_between", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT id FROM events WHERE id BETWEEN 2 AND 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}
