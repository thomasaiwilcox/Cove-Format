use super::*;

#[tokio::test]
async fn select_star_reads_single_file_multi_segment() {
    let path = write_temp_cove("events", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT * FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+----+-------+--------+",
        "| id | name  | active |",
        "+----+-------+--------+",
        "| 1  | alpha | true   |",
        "| 2  | beta  | false  |",
        "| 3  | gamma | true   |",
        "+----+-------+--------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn registered_utf8_page_scans_through_stable_decoder() {
    let path = write_temp_cove(
        "registered_utf8_supported",
        registered_names_file(true, true),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "names", &path).unwrap();

    let batches = ctx
        .sql("SELECT name FROM names")
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
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn registered_utf8_page_scans_through_core_fallback_without_descriptor() {
    let path = write_temp_cove(
        "registered_utf8_fallback",
        registered_names_file(false, true),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "names", &path).unwrap();

    let batches = ctx
        .sql("SELECT name FROM names")
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
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn register_cove_o_projections_queries_people_projection() {
    let object_path = write_temp_mapped_cove(
        "mapped-people",
        "cove_map_execution.covemap",
        &["people.parquet"],
    );
    let ctx = SessionContext::new();
    let registered = register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].table_name, "people_projection");
    assert_eq!(registered[0].projection_id, "person_projection");

    let batches = ctx
        .sql("SELECT name, membership_count FROM people_projection ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+-------+------------------+",
        "| name  | membership_count |",
        "+-------+------------------+",
        "| Ada   | 1                |",
        "| Linus | 1                |",
        "+-------+------------------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(object_path).unwrap();
}

#[tokio::test]
async fn register_cove_o_projections_uses_cove_projection_exec() {
    let object_path = write_temp_mapped_cove(
        "mapped-people-exec",
        "cove_map_execution.covemap",
        &["people.parquet"],
    );
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();

    let (batches, range_requests) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection ORDER BY name",
        "cove_range_requests",
    )
    .await;
    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| Ada   |",
        "| Linus |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert!(range_requests > 0);

    let explain_batches = ctx
        .sql("EXPLAIN SELECT name FROM people_projection ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain = pretty_format_batches(&explain_batches).unwrap().to_string();
    assert!(explain.contains("CoveProjectionExec"));
    assert!(!explain.contains("MemTableExec"));

    fs::remove_file(object_path).unwrap();
}

#[tokio::test]
async fn register_cove_o_projections_uses_sparse_range_plan() {
    let object_path = write_temp_mapped_cove(
        "mapped-people-range-plan",
        "cove_map_execution.covemap",
        &["people.parquet"],
    );
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();

    let (batches, sparse_plan_count) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection ORDER BY name",
        "cove_range_plan_sparse",
    )
    .await;
    let (_, dense_plan_count) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection ORDER BY name",
        "cove_range_plan_dense",
    )
    .await;
    let (_, range_requests) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection ORDER BY name",
        "cove_range_requests",
    )
    .await;

    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| Ada   |",
        "| Linus |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(sparse_plan_count, 1);
    assert_eq!(dense_plan_count, 0);
    assert!(range_requests > 1);

    fs::remove_file(object_path).unwrap();
}

#[tokio::test]
async fn register_cove_o_projections_pushes_exact_scalar_filters() {
    let object_path = write_temp_mapped_cove(
        "mapped-people-filter",
        "cove_map_execution.covemap",
        &["people.parquet"],
    );
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();

    let batches = ctx
        .sql("SELECT membership_count FROM people_projection WHERE name = 'Ada'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+------------------+",
        "| membership_count |",
        "+------------------+",
        "| 1                |",
        "+------------------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain_batches = ctx
        .sql("EXPLAIN SELECT membership_count FROM people_projection WHERE name = 'Ada'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain = pretty_format_batches(&explain_batches).unwrap().to_string();
    assert!(explain.contains("CoveProjectionExec"));
    assert!(
        !explain.contains("FilterExec"),
        "exact pushed filter should not leave a FilterExec: {explain}"
    );

    fs::remove_file(object_path).unwrap();
}

#[cfg(feature = "covi")]
#[tokio::test]
async fn register_cove_o_projection_uses_projection_column_covi_sidecar() {
    let dir = make_temp_dir("mapped_projection_column_covi");
    let bundle = dir.join("bundle");
    let mapping_path = conformance_accept_path("cove_map_execution.covemap");
    let source_paths = vec![conformance_accept_path("people.parquet")];
    let mut options = cove_map::MapBuildOptions::new(bundle.clone());
    options.projection_output = cove_map::MapBuildProjectionOutput::None;
    let result = cove_map::build_from_paths(&mapping_path, &source_paths, options).unwrap();
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .unwrap();
    let object_path = bundle.join(object_rel);
    assert!(bundle
        .join("indexes")
        .join("projection_columns.covi")
        .is_file());

    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let (batches, loaded) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT membership_count FROM people_projection WHERE name = 'Ada'",
        "cove_covi_sidecars_loaded",
    )
    .await;
    let expected = [
        "+------------------+",
        "| membership_count |",
        "+------------------+",
        "| 1                |",
        "+------------------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(loaded, 1);
    let (_, hits) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT membership_count FROM people_projection WHERE name = 'Ada'",
        "cove_lookup_index_hits",
    )
    .await;
    assert_eq!(hits, 1);
    let (_, candidate_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT membership_count FROM people_projection WHERE name = 'Ada'",
        "cove_projection_covi_candidate_rows",
    )
    .await;
    assert_eq!(candidate_rows, 1);
    let (_, skipped_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT membership_count FROM people_projection WHERE name = 'Ada'",
        "cove_projection_covi_rows_skipped",
    )
    .await;
    assert_eq!(skipped_rows, 1);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covi")]
#[tokio::test]
async fn register_cove_o_projection_reports_projection_covi_fallbacks() {
    let dir = make_temp_dir("mapped_projection_column_covi_fallbacks");
    let bundle = dir.join("bundle");
    let mapping_path = conformance_accept_path("cove_map_execution.covemap");
    let source_paths = vec![conformance_accept_path("people.parquet")];
    let mut options = cove_map::MapBuildOptions::new(bundle.clone());
    options.projection_output = cove_map::MapBuildProjectionOutput::None;
    let result = cove_map::build_from_paths(&mapping_path, &source_paths, options).unwrap();
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .unwrap();
    let object_path = bundle.join(object_rel);
    let sidecar_path = bundle.join("indexes").join("projection_columns.covi");
    let sidecar_bytes = fs::read(&sidecar_path).unwrap();

    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let (batches, metrics) = collect_sql_with_cove_metrics(
        &ctx,
        "SELECT name FROM people_projection WHERE name IN ('Ada', 'Linus') AND name = 'Ada'",
        &[
            "cove_projection_covi_eligible_filters",
            "cove_lookup_index_hits",
            "cove_projection_covi_candidate_rows",
            "cove_projection_covi_rows_skipped",
        ],
    )
    .await;
    let expected = ["+------+", "| name |", "+------+", "| Ada  |", "+------+"];
    assert_batches_eq!(expected, &batches);
    assert_eq!(metrics, vec![2, 2, 1, 1]);

    fs::remove_file(&sidecar_path).unwrap();
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let (batches, no_sidecar) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection WHERE name = 'Ada'",
        "cove_projection_covi_fallback_no_sidecar",
    )
    .await;
    assert_batches_eq!(expected, &batches);
    assert_eq!(no_sidecar, 1);

    fs::write(&sidecar_path, &sidecar_bytes).unwrap();
    let mut stale = sidecar_bytes;
    stale[0] ^= 0x01;
    fs::write(&sidecar_path, stale).unwrap();
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let (batches, stale_count) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection WHERE name = 'Ada'",
        "cove_projection_covi_fallback_stale",
    )
    .await;
    assert_batches_eq!(expected, &batches);
    assert_eq!(stale_count, 1);

    fs::write(
        &sidecar_path,
        fs::read(bundle.join("indexes").join("object_properties.covi")).unwrap(),
    )
    .unwrap();
    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let (batches, unsupported) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT name FROM people_projection WHERE name != 'Ada'",
        "cove_projection_covi_fallback_no_eligible_filter",
    )
    .await;
    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| Linus |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(unsupported, 1);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn mapped_cove_o_end_to_end_builds_registers_and_queries_in_datafusion() {
    let mapping_path = conformance_accept_path("cove_map_execution.covemap");
    let source_paths = vec![conformance_accept_path("people.parquet")];
    let object_bytes = cove_o_from_paths(&mapping_path, &source_paths).unwrap();
    let object_path = write_temp_cove("mapped-e2e", object_bytes);

    let ctx = SessionContext::new();
    let registered = register_cove_o_projections(&ctx, &object_path, None, Some("demo")).unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].table_name, "demo__people_projection");
    assert_eq!(registered[0].projection_id, "person_projection");

    let batches = ctx
        .sql("SELECT name, membership_count FROM demo__people_projection ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+-------+------------------+",
        "| name  | membership_count |",
        "+-------+------------------+",
        "| Ada   | 1                |",
        "| Linus | 1                |",
        "+-------+------------------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(object_path).unwrap();
}

#[tokio::test]
async fn mapped_cove_o_projection_queries_zstd_compressed_map_build_object() {
    let dir = make_temp_dir("mapped-zstd-map-build");
    let bundle = dir.join("bundle");
    let mapping_path = conformance_accept_path("cove_map_execution.covemap");
    let people_csv = dir.join("people.csv");
    let mut csv = String::from("person_id,person_name,team_id,team_name,valid_from,valid_to\n");
    for index in 0..512 {
        csv.push_str(&format!(
            "p{index},person_{index:04},t{},Team {},2026-01-01,2026-12-31\n",
            index % 16,
            index % 16
        ));
    }
    fs::write(&people_csv, csv).unwrap();
    let mut options = cove_map::MapBuildOptions::new(bundle.clone());
    options.projection_output = cove_map::MapBuildProjectionOutput::None;
    let result = cove_map::build_from_paths(&mapping_path, &[people_csv], options).unwrap();
    assert!(
        result
            .manifest
            .pointer("/compression_summary/compressed_section_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    let object_rel = result
        .manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .unwrap();
    let object_path = bundle.join(object_rel);

    let ctx = SessionContext::new();
    register_cove_o_projections(&ctx, &object_path, None, None).unwrap();
    let batches = ctx
        .sql("SELECT name, membership_count FROM people_projection WHERE name = 'person_0042'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+-------------+------------------+",
        "| name        | membership_count |",
        "+-------------+------------------+",
        "| person_0042 | 1                |",
        "+-------------+------------------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn mapped_cove_o_showcase_spans_multiple_sources_and_projections_in_datafusion() {
    let showcase_dir = std::env::temp_dir().join(format!(
        "cove-datafusion-showcase-{}-{}",
        std::process::id(),
        NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&showcase_dir).unwrap();
    let mapping_path = showcase_dir.join("showcase.covemap");
    fs::write(
        &mapping_path,
        showcase_multi_source_covemap().serialize().unwrap(),
    )
    .unwrap();
    let crm_path = showcase_dir.join("crm.csv");
    fs::write(&crm_path, b"id,name\np1,Ada CRM\np2,Linus CRM\n").unwrap();
    let directory_path = showcase_dir.join("directory.parquet");
    fs::write(
        &directory_path,
        write_parquet_batch(showcase_directory_name_batch()),
    )
    .unwrap();
    let subscriptions_path = showcase_dir.join("subscription.csv");
    fs::write(&subscriptions_path, b"id,name\np1,Ada\np2,Linus\n").unwrap();

    let source_paths = vec![
        crm_path.clone(),
        directory_path.clone(),
        subscriptions_path.clone(),
    ];
    let object_bytes = cove_o_from_paths(&mapping_path, &source_paths).unwrap();
    let object_path = write_temp_cove("mapped-showcase-object", object_bytes);

    let ctx = SessionContext::new();
    let registered = register_cove_o_projections(&ctx, &object_path, None, Some("demo")).unwrap();
    assert_eq!(registered.len(), 2);
    assert_eq!(
        registered
            .iter()
            .map(|projection| projection.table_name.as_str())
            .collect::<Vec<_>>(),
        vec!["demo__people_projection", "demo__evidence_projection",]
    );

    let people_batches = ctx
        .sql("SELECT name FROM demo__people_projection ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_people = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| Ada   |",
        "| Linus |",
        "+-------+",
    ];
    assert_batches_eq!(expected_people, &people_batches);

    let directory_batches = ctx
        .sql(
            "SELECT source_id, COUNT(DISTINCT source_row_identity) AS evidence_count \
             FROM demo__evidence_projection \
             GROUP BY source_id ORDER BY source_id",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_directory = [
        "+--------------+----------------+",
        "| source_id    | evidence_count |",
        "+--------------+----------------+",
        "| crm          | 2              |",
        "| directory    | 2              |",
        "| subscription | 2              |",
        "+--------------+----------------+",
    ];
    assert_batches_eq!(expected_directory, &directory_batches);

    let joined_batches = ctx
        .sql(
            "SELECT DISTINCT p.name, e.source_id, e.source_row_identity \
             FROM demo__people_projection p \
             JOIN demo__evidence_projection e \
               ON p.person_goid = e.output_object_id \
             ORDER BY p.name, e.source_id, e.source_row_identity",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected_joined = [
        "+-------+--------------+---------------------+",
        "| name  | source_id    | source_row_identity |",
        "+-------+--------------+---------------------+",
        "| Ada   | crm          | crm:0               |",
        "| Ada   | directory    | directory:0         |",
        "| Ada   | subscription | subscription:0      |",
        "| Linus | crm          | crm:1               |",
        "| Linus | directory    | directory:1         |",
        "| Linus | subscription | subscription:1      |",
        "+-------+--------------+---------------------+",
    ];
    assert_batches_eq!(expected_joined, &joined_batches);

    fs::remove_file(object_path).unwrap();
    fs::remove_dir_all(showcase_dir).unwrap();
}

#[tokio::test]
async fn register_cove_o_projection_queries_mixed_format_object_with_mapping_path() {
    let object_path = write_temp_mapped_cove(
        "mapped-mixed",
        "cove_map_source_priority_projectable.covemap",
        &["cove_map_crm.parquet", "cove_map_support.orc"],
    );
    let mapping_path = conformance_accept_path("cove_map_source_priority_projectable.covemap");
    let ctx = SessionContext::new();
    register_cove_o_projection(
        &ctx,
        "priority_people",
        &object_path,
        Some(&mapping_path),
        "person_projection",
    )
    .unwrap();

    let batches = ctx
        .sql("SELECT name FROM priority_people")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+--------------+",
        "| name         |",
        "+--------------+",
        "| Support Name |",
        "+--------------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(object_path).unwrap();
}

#[tokio::test]
async fn registered_required_unprojected_page_does_not_block_count_scan() {
    let path = write_temp_cove(
        "registered_utf8_required_unprojected",
        registered_names_file(false, false),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "names", &path).unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) FROM names")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+----------+",
        "| count(*) |",
        "+----------+",
        "| 3        |",
        "+----------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_limit_pushdown_materializes_only_requested_rows() {
    let path = write_temp_cove("events_limit", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (full_batches, full_materialized) =
        collect_sql_with_cove_metric(&ctx, "SELECT id FROM events", "cove_rows_materialized").await;
    let full_expected = [
        "+----+", "| id |", "+----+", "| 1  |", "| 2  |", "| 3  |", "+----+",
    ];
    assert_batches_eq!(full_expected, &full_batches);
    assert_eq!(full_materialized, 3);
    let (_, full_buffered_partitions) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id FROM events",
        "cove_materialization_buffered_partitions",
    )
    .await;
    assert_eq!(full_buffered_partitions, 1);

    let (limit_batches, limit_materialized) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id FROM events LIMIT 1",
        "cove_rows_materialized",
    )
    .await;
    let limit_expected = ["+----+", "| id |", "+----+", "| 1  |", "+----+"];
    assert_batches_eq!(limit_expected, &limit_batches);
    assert_eq!(limit_materialized, 1);
    assert!(limit_materialized < full_materialized);
    let (_, limit_streaming_partitions) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id FROM events LIMIT 1",
        "cove_materialization_streaming_partitions",
    )
    .await;
    assert_eq!(limit_streaming_partitions, 1);

    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_arrow_export_path_metrics_are_recorded() {
    let path = write_temp_cove("events_export_metrics", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (_, numcode_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name, active FROM events",
        "cove_arrow_export_direct_numcode_rows",
    )
    .await;
    let (_, varbytes_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name, active FROM events",
        "cove_arrow_export_direct_varbytes_rows",
    )
    .await;
    let (_, plainfixed_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name, active FROM events",
        "cove_arrow_export_direct_plainfixed_rows",
    )
    .await;
    let (_, fallback_rows) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name, active FROM events",
        "cove_arrow_export_fallback_rows",
    )
    .await;

    assert_eq!(numcode_rows, 3);
    assert_eq!(varbytes_rows, 3);
    assert_eq!(plainfixed_rows, 3);
    assert_eq!(fallback_rows, 0);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_nested_list_column_projects_as_arrow_list() {
    let mut builder = ListBuilder::new(Int32Builder::new());
    builder.values().append_value(1);
    builder.values().append_value(2);
    builder.append(true);
    builder.append(false);
    builder.values().append_value(3);
    builder.append(true);
    let batch = ArrowRecordBatch::try_from_iter(vec![(
        "tags",
        Arc::new(builder.finish()) as ArrowArrayRef,
    )])
    .unwrap();
    let result = convert_arrow_record_batches(
        "arrow-test",
        "test:native-nested-list".into(),
        batch.schema(),
        vec![batch],
        &ParquetConversionOptions::default(),
    )
    .unwrap();
    let path = write_temp_cove("native_nested_list", result.cove_bytes);
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT tags FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(batches.len(), 1);
    let array = batches[0].column(0);
    let list = array.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(list.value_offsets(), &[0, 2, 2, 3]);
    assert!(!list.is_null(0));
    assert!(list.is_null(1));
    assert!(!list.is_null(2));
    let values = list.values().as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(values.values(), &[1, 2, 3]);

    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn native_materialization_mode_selection_is_explained() {
    let path = write_temp_cove("materialization_modes", topn_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("materialization_mode=buffered"),
        "{explain_text}"
    );

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events LIMIT 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(
        explain_text.contains("materialization_mode=streaming"),
        "{explain_text}"
    );

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
    assert!(!explain_text.contains("CoveExec"), "{explain_text}");

    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn async_bootstrap_and_registration_match_sync_helpers() {
    let path = write_temp_cove("async_parity", primitive_events_file());

    let sync_state = bootstrap_local_file(&path).unwrap();
    let async_state = bootstrap_local_file_async(&path).await.unwrap();
    assert_eq!(sync_state.table().row_count, async_state.table().row_count);
    assert_eq!(sync_state.schema().as_ref(), async_state.schema().as_ref());

    let sync_provider = cove_table_from_path(&path).unwrap();
    let async_provider = cove_table_from_path_async(&path).await.unwrap();
    assert_eq!(
        sync_provider.state().bootstrap_stats(),
        async_provider.state().bootstrap_stats()
    );

    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events_sync", &path).unwrap();
    register_cove_file_async(&ctx, "events_async", &path)
        .await
        .unwrap();

    let sync_batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events_sync")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let async_batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events_async")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 3    |", "+------+"];
    assert_batches_eq!(expected, &sync_batches);
    assert_batches_eq!(expected, &async_batches);

    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn scoped_operation_required_feature_does_not_block_unrelated_scan() {
    let state = bootstrap_bytes(
        "feature_scope_unrelated_operation",
        primitive_events_file_with_scoped_feature(scoped_feature_entry(
            FeatureScopeV2::OperationRequired,
            OperationKindV2::CoveragePlanning,
            0,
            u64::MAX,
        )),
    )
    .unwrap();
    let plan = plan_scan(&state, None, Vec::new()).unwrap();
    let decoded = decode_scan(&state, &plan).unwrap();
    assert_eq!(
        decoded
            .batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>(),
        3
    );
}

#[tokio::test]
async fn scoped_operation_required_feature_rejects_matching_scan() {
    let path = write_temp_cove(
        "feature_scope_matching_operation",
        primitive_events_file_with_scoped_feature(scoped_feature_entry(
            FeatureScopeV2::OperationRequired,
            OperationKindV2::OrdinaryTableScan,
            0,
            u64::MAX,
        )),
    );

    assert!(matches!(
        bootstrap_local_file(&path),
        Err(CoveError::UnknownRequiredFeature(UNKNOWN_SCOPED_FEATURE))
    ));

    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn scoped_page_required_feature_rejects_exact_page_decode() {
    let state = bootstrap_bytes(
        "feature_scope_page_decode",
        primitive_events_file_with_scoped_feature(scoped_feature_entry(
            FeatureScopeV2::PageRequired,
            OperationKindV2::None,
            5,
            cove_column_page_target_ref(1, 0),
        )),
    )
    .unwrap();
    let plan = plan_scan(&state, None, Vec::new()).unwrap();
    assert!(matches!(
        decode_scan(&state, &plan),
        Err(CoveError::UnknownRequiredFeature(UNKNOWN_SCOPED_FEATURE))
    ));
}

#[tokio::test]
async fn listing_registration_reads_multiple_cove_files() {
    let dir = make_temp_dir("listing_multi");
    fs::write(dir.join("part1.cove"), primitive_events_file()).unwrap();
    fs::write(dir.join("part2.cove"), primitive_events_file()).unwrap();

    let ctx = SessionContext::new();
    register_cove_listing_table(&ctx, "events", dir.to_str().unwrap())
        .await
        .unwrap();

    let batches = ctx
        .sql("SELECT id, name FROM events ORDER BY id, name")
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
        "| 1  | alpha |",
        "| 2  | beta  |",
        "| 2  | beta  |",
        "| 3  | gamma |",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covm")]
#[tokio::test]
async fn covm_registration_reads_multiple_relative_files() {
    let dir = make_temp_dir("covm_multi");
    let nested = dir.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let first = dir.join("part1.cove");
    let second = nested.join("part2.cove");
    fs::write(&first, primitive_events_file()).unwrap();
    fs::write(&second, primitive_events_file()).unwrap();
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(
        &manifest,
        vec![
            covm_entry_for_path("part1.cove", &first),
            covm_entry_for_path("nested/part2.cove", &second),
        ],
    );

    let ctx = SessionContext::new();
    let provider = register_cove_covm(&ctx, "events", &manifest).unwrap();
    assert_eq!(provider.state().file_count(), 2);
    assert_eq!(provider.state().bootstrap_stats().files_validated, 2);

    let batches = ctx
        .sql("SELECT id, name FROM events ORDER BY id, name")
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
        "| 1  | alpha |",
        "| 2  | beta  |",
        "| 2  | beta  |",
        "| 3  | gamma |",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covm")]
#[tokio::test]
async fn covm_registration_rejects_member_uri_that_escapes_manifest_dir() {
    let dir = make_temp_dir("covm_member_escape");
    let path = dir.join("part1.cove");
    fs::write(&path, primitive_events_file()).unwrap();
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(&manifest, vec![covm_entry_for_path("../part1.cove", &path)]);

    let err = cove_table_from_covm_path(&manifest)
        .unwrap_err()
        .to_string();
    assert!(err.contains("parent-directory"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covm")]
#[tokio::test]
async fn covm_rejects_schema_mismatch() {
    let dir = make_temp_dir("covm_schema_mismatch");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(&first, primitive_events_file()).unwrap();
    fs::write(&second, nullable_events_file()).unwrap();
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(
        &manifest,
        vec![
            covm_entry_for_path("part1.cove", &first),
            covm_entry_for_path("part2.cove", &second),
        ],
    );

    let err = cove_table_from_covm_path(&manifest)
        .unwrap_err()
        .to_string();
    assert!(err.contains("schema mismatch"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covm")]
#[tokio::test]
async fn stale_covm_entry_cannot_exclude_file() {
    let dir = make_temp_dir("covm_stale");
    let path = dir.join("part1.cove");
    fs::write(&path, primitive_events_file()).unwrap();
    let mut entry = covm_entry_for_path("part1.cove", &path);
    entry.footer_crc32c ^= 0x55AA_0011;
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(&manifest, vec![entry]);

    let ctx = SessionContext::new();
    let provider = register_cove_covm(&ctx, "events", &manifest).unwrap();
    assert_eq!(provider.state().bootstrap_stats().covm_entries_stale, 1);
    let batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 3    |", "+------+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "covm")]
#[tokio::test]
async fn covm_filecode_filters_resolve_per_file_dictionary() {
    let dir = make_temp_dir("covm_filecode");
    let first = dir.join("part1.cove");
    let second = dir.join("part2.cove");
    fs::write(&first, dictionary_items_file(sample_dictionary())).unwrap();
    fs::write(&second, dictionary_items_file(swapped_dictionary())).unwrap();
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(
        &manifest,
        vec![
            covm_entry_for_path("part1.cove", &first),
            covm_entry_for_path("part2.cove", &second),
        ],
    );

    let ctx = SessionContext::new();
    register_cove_covm(&ctx, "items", &manifest).unwrap();
    let batches = ctx
        .sql("SELECT name FROM items WHERE name = 'red' ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+------+", "| name |", "+------+", "| red  |", "| red  |", "+------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(all(feature = "covm", feature = "covx"))]
#[tokio::test]
async fn covx_sibling_sidecar_validation_is_advisory() {
    let dir = make_temp_dir("covx_sidecar");
    let path = dir.join("part1.cove");
    fs::write(&path, primitive_events_file()).unwrap();
    let manifest = dir.join("dataset.covm");
    write_covm_manifest(&manifest, vec![covm_entry_for_path("part1.cove", &path)]);
    write_covx_sidecar(
        &PathBuf::from(format!("{}.covx", path.display())),
        vec![covx_entry_for_path(&path)],
    );

    let provider = cove_table_from_covm_path(&manifest).unwrap();
    assert_eq!(provider.state().bootstrap_stats().covx_sidecars_loaded, 1);

    let mut stale = covx_entry_for_path(&path);
    stale.file_len += 1;
    write_covx_sidecar(
        &PathBuf::from(format!("{}.covx", path.display())),
        vec![stale],
    );
    let ctx = SessionContext::new();
    let provider = register_cove_covm(&ctx, "events", &manifest).unwrap();
    assert_eq!(provider.state().bootstrap_stats().covx_sidecars_stale, 1);
    let batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 3    |", "+------+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sql_external_table_stored_as_cove_works_after_format_registration() {
    let dir = make_temp_dir("sql_external");
    fs::write(dir.join("part1.cove"), primitive_events_file()).unwrap();

    let ctx = SessionContext::new();
    register_cove_file_format(&ctx).unwrap();
    ctx.sql(&format!(
        "CREATE EXTERNAL TABLE events STORED AS COVE LOCATION '{}'",
        dir.display()
    ))
    .await
    .unwrap();

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
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sql_external_table_accepts_cove_format_options() {
    let dir = make_temp_dir("sql_external_options");
    fs::write(dir.join("part1.cove"), primitive_events_file()).unwrap();

    let ctx = SessionContext::new();
    register_cove_file_format(&ctx).unwrap();
    ctx.sql(&format!(
        "CREATE EXTERNAL TABLE events STORED AS COVE LOCATION '{}' OPTIONS (\
         'cove.filter_residual_policy' 'preserve_all', \
         'cove.arrow_output' 'standard', \
         'cove.arrow_string_validation' 'strict_or_cached_proof', \
         'cove.page_payload_validation' 'trusted', \
         'cove.local_file_read' 'mmap', \
         'cove.range_coalescing_max_gap' '64', \
         'cove.range_coalescing_max_span' '4096', \
         'cove.covx_discovery' 'disabled', \
         'cove.covi_discovery' 'disabled', \
         'cove.coverage_cache' 'disabled', \
         'cove.execution_code_policy' 'opportunistic', \
         'cove.target_morsels_per_partition' '4')",
        dir.display()
    ))
    .await
    .unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) AS rows FROM events")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| rows |", "+------+", "| 3    |", "+------+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sql_external_table_rejects_unknown_cove_format_options() {
    let dir = make_temp_dir("sql_external_options");
    fs::write(dir.join("part1.cove"), primitive_events_file()).unwrap();

    let ctx = SessionContext::new();
    register_cove_file_format(&ctx).unwrap();
    let err = ctx
        .sql(&format!(
            "CREATE EXTERNAL TABLE events STORED AS COVE LOCATION '{}' OPTIONS ('cove.foo' 'bar')",
            dir.display()
        ))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("COVE DataFusion v2 does not support SQL format option"),
        "{err}"
    );
    assert!(err.contains("cove.foo"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn copy_to_cove_writes_readable_bounded_file() {
    let dir = make_temp_dir("copy_to_cove");
    let path = dir.join("out.cove");
    let ctx = SessionContext::new();
    register_cove_file_format(&ctx).unwrap();
    ctx.sql(&format!(
        "COPY (\
         SELECT CAST(1 AS BIGINT) AS id, CAST('alpha' AS VARCHAR) AS name \
         UNION ALL \
         SELECT CAST(2 AS BIGINT) AS id, CAST('beta' AS VARCHAR) AS name\
         ) TO '{}' STORED AS COVE",
        path.display()
    ))
    .await
    .unwrap()
    .collect()
    .await
    .unwrap();

    let read_ctx = SessionContext::new();
    register_cove_file(&read_ctx, "written", &path).unwrap();
    let batches = read_ctx
        .sql("SELECT id, name FROM written ORDER BY id")
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
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sql_external_table_appends_partition_columns() {
    let dir = make_temp_dir("sql_external_partitions");
    let partition = dir.join("year=2026");
    fs::create_dir_all(&partition).unwrap();
    fs::write(partition.join("part1.cove"), primitive_events_file()).unwrap();

    let ctx = SessionContext::new();
    register_cove_file_format(&ctx).unwrap();
    ctx.sql(&format!(
        "CREATE EXTERNAL TABLE events(id BIGINT, name VARCHAR, active BOOLEAN) \
         STORED AS COVE PARTITIONED BY (year INT) LOCATION '{}'",
        dir.display()
    ))
    .await
    .unwrap();

    let batches = ctx
        .sql("SELECT year, name FROM events WHERE year = 2026 ORDER BY name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+------+-------+",
        "| year | name  |",
        "+------+-------+",
        "| 2026 | alpha |",
        "| 2026 | beta  |",
        "| 2026 | gamma |",
        "+------+-------+",
    ];
    assert_batches_eq!(expected, &batches);

    let partition_only = ctx
        .sql("SELECT year FROM events WHERE year = 2026 LIMIT 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| year |", "+------+", "| 2026 |", "+------+"];
    assert_batches_eq!(expected, &partition_only);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn listing_registration_rejects_schema_mismatch_and_empty_listing() {
    let mismatch = make_temp_dir("listing_mismatch");
    fs::write(mismatch.join("part1.cove"), primitive_events_file()).unwrap();
    fs::write(mismatch.join("part2.cove"), nullable_events_file()).unwrap();
    let ctx = SessionContext::new();
    let err = register_cove_listing_table(&ctx, "events", mismatch.to_str().unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("schema mismatch"), "{err}");
    fs::remove_dir_all(mismatch).unwrap();

    let empty = make_temp_dir("listing_empty");
    let err = register_cove_listing_table(&ctx, "empty_events", empty.to_str().unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("empty listing"), "{err}");
    fs::remove_dir_all(empty).unwrap();
}

#[tokio::test]
async fn listing_registration_rejects_multiple_tables_in_one_file() {
    let dir = make_temp_dir("listing_multi_table");
    fs::write(dir.join("bad.cove"), multiple_tables_file()).unwrap();
    let ctx = SessionContext::new();
    let err = register_cove_listing_table(&ctx, "bad", dir.to_str().unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("requires cove.table_id or cove.table_name"),
        "{err}"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn listing_registration_selects_table_from_multi_table_file() {
    let dir = make_temp_dir("listing_multi_table_selected");
    fs::write(dir.join("selected.cove"), multiple_tables_file()).unwrap();
    let ctx = SessionContext::new();
    register_cove_listing_table_with_options(
        &ctx,
        "selected",
        dir.to_str().unwrap(),
        CoveTableOptions::default().with_table_name(Some("public".into()), "second".into()),
    )
    .await
    .unwrap();

    let batches = ctx
        .sql("SELECT COUNT(*) AS n FROM selected")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+---+", "| n |", "+---+", "| 0 |", "+---+"];
    assert_batches_eq!(expected, &batches);
    fs::remove_dir_all(dir).unwrap();
}
