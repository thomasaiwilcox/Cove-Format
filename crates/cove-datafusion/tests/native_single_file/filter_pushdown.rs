use super::*;

#[test]
fn filter_pushdown_classifies_supported_numeric_and_null_exact() {
    let path = write_temp_cove("nullable_classification", nullable_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let nullable_col = Expr::Column(Column::from_name("maybe"));
    let is_null = Expr::IsNull(Box::new(nullable_col.clone()));
    let is_not_null = Expr::IsNotNull(Box::new(nullable_col));
    let comparison = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("id"))),
        Operator::Gt,
        Box::new(Expr::Literal(ScalarValue::Int64(Some(1)), None)),
    ));

    let support = provider
        .supports_filters_pushdown(&[&is_null, &is_not_null, &comparison])
        .unwrap();

    assert_eq!(
        support,
        vec![
            TableProviderFilterPushDown::Exact,
            TableProviderFilterPushDown::Exact,
            TableProviderFilterPushDown::Exact
        ]
    );
    assert!(support.contains(&TableProviderFilterPushDown::Exact));
    fs::remove_file(path).unwrap();
}

#[test]
fn filter_pushdown_classifies_between_as_two_exact_bounds() {
    let path = write_temp_cove("between_classification", primitive_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let between = Expr::Between(Between::new(
        Box::new(Expr::Column(Column::from_name("id"))),
        false,
        Box::new(Expr::Literal(ScalarValue::Int64(Some(2)), None)),
        Box::new(Expr::Literal(ScalarValue::Int64(Some(2)), None)),
    ));

    let support = provider.supports_filters_pushdown(&[&between]).unwrap();

    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    fs::remove_file(path).unwrap();
}

#[test]
fn filter_pushdown_classifies_numeric_in_as_exact() {
    let path = write_temp_cove("numeric_in_classification", primitive_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let in_list = Expr::InList(InList::new(
        Box::new(Expr::Column(Column::from_name("id"))),
        vec![
            Expr::Literal(ScalarValue::Int64(Some(1)), None),
            Expr::Literal(ScalarValue::Int64(Some(3)), None),
        ],
        false,
    ));

    let support = provider.supports_filters_pushdown(&[&in_list]).unwrap();

    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    fs::remove_file(path).unwrap();
}

#[test]
fn filter_pushdown_classifies_fixed_bytes_equality_exact() {
    let path = write_temp_cove("fixed_bytes_classification", fixed_uuid_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let equality = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("uid"))),
        Operator::Eq,
        Box::new(Expr::Literal(
            ScalarValue::Utf8(Some("00000000-0000-0000-0000-000000000002".into())),
            None,
        )),
    ));

    let support = provider.supports_filters_pushdown(&[&equality]).unwrap();

    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    fs::remove_file(path).unwrap();
}

#[test]
fn filter_pushdown_classifies_varbytes_equality_exact() {
    let path = write_temp_cove("varbytes_classification", primitive_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let varbytes_equality = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("beta".into())), None)),
    ));
    let varbytes_range = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        Operator::GtEq,
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("beta".into())), None)),
    ));

    let support = provider
        .supports_filters_pushdown(&[&varbytes_equality, &varbytes_range])
        .unwrap();

    assert_eq!(
        support,
        vec![
            TableProviderFilterPushDown::Exact,
            TableProviderFilterPushDown::Unsupported
        ]
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn filter_pushdown_classifies_varbytes_prefix_like_exact() {
    let path = write_temp_cove("varbytes_prefix_classification", primitive_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let like = Expr::Like(Like::new(
        false,
        Box::new(Expr::Column(Column::from_name("name"))),
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("ga%".into())), None)),
        None,
        false,
    ));
    let wildcard = Expr::Like(Like::new(
        false,
        Box::new(Expr::Column(Column::from_name("name"))),
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("g_m%".into())), None)),
        None,
        false,
    ));

    let support = provider
        .supports_filters_pushdown(&[&like, &wildcard])
        .unwrap();

    assert_eq!(
        support,
        vec![
            TableProviderFilterPushDown::Exact,
            TableProviderFilterPushDown::Unsupported
        ]
    );
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn sql_filter_uses_matched_coverage_metadata_for_morsel_pruning() {
    let bytes = primitive_events_file_with_name_gamma_coverage(false);
    let state = bootstrap_bytes("coverage_sql_gamma", bytes.clone()).unwrap();
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("gamma".into()))),
        },
        "name = 'gamma'",
    );
    let plan = plan_scan(&state, Some(&[0, 1]), vec![filter]).unwrap();
    assert!(plan.coverage_expr.is_some());
    let decoded = decode_scan(&state, &plan).unwrap();
    assert_eq!(decoded.stats.morsels_pruned, 1);

    let path = write_temp_cove("coverage_sql_gamma", bytes);
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT id, name FROM events WHERE name = 'gamma'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();

    let expected = [
        "+----+-------+",
        "| id | name  |",
        "+----+-------+",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn coverage_metadata_bad_checksum_fails_open() {
    let bytes = primitive_events_file_with_name_gamma_coverage(true);
    let state = bootstrap_bytes("coverage_sql_bad_checksum", bytes.clone()).unwrap();
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("gamma".into()))),
        },
        "name = 'gamma'",
    );
    let plan = plan_scan(&state, Some(&[0, 1]), vec![filter]).unwrap();
    let decoded = decode_scan(&state, &plan).unwrap();
    assert_eq!(decoded.stats.morsels_pruned, 0);

    let path = write_temp_cove("coverage_sql_bad_checksum", bytes);
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, morsels_pruned) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name FROM events WHERE name = 'gamma'",
        "cove_morsels_pruned",
    )
    .await;

    let expected = [
        "+----+-------+",
        "| id | name  |",
        "+----+-------+",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(morsels_pruned, 0);
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn coverage_metadata_stale_snapshot_fails_open() {
    let bytes = primitive_events_file_with_name_gamma_coverage_snapshot(false, 2);
    let state = bootstrap_bytes("coverage_sql_stale_snapshot", bytes.clone()).unwrap();
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("gamma".into()))),
        },
        "name = 'gamma'",
    );
    let plan = plan_scan(&state, Some(&[0, 1]), vec![filter]).unwrap();
    let decoded = decode_scan(&state, &plan).unwrap();
    assert_eq!(decoded.stats.morsels_pruned, 0);

    let path = write_temp_cove("coverage_sql_stale_snapshot", bytes);
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let (batches, morsels_pruned) = collect_sql_with_cove_metric(
        &ctx,
        "SELECT id, name FROM events WHERE name = 'gamma'",
        "cove_morsels_pruned",
    )
    .await;

    let expected = [
        "+----+-------+",
        "| id | name  |",
        "+----+-------+",
        "| 3  | gamma |",
        "+----+-------+",
    ];
    assert_batches_eq!(expected, &batches);
    assert_eq!(morsels_pruned, 0);
    fs::remove_file(path).unwrap();
}

#[test]
fn sibling_coverage_cache_is_explicit_and_records_planner_hits() {
    let bytes = primitive_events_file_with_name_gamma_coverage(false);
    let path = write_temp_cove("coverage_cache_hit", bytes.clone());
    let base_state = bootstrap_local_file(&path).unwrap();
    assert!(!base_state.coverage_cache().runtime_stats().enabled);

    let cache_bytes = coverage_cache_bytes_for_state(base_state.as_ref(), &bytes);
    let cache_path = PathBuf::from(format!("{}.cache", path.display()));
    fs::write(&cache_path, cache_bytes).unwrap();

    let cached_state = cove_datafusion::bootstrap::bootstrap_local_file_with_options(
        &path,
        CoveTableOptions::default().with_sibling_coverage_cache(),
    )
    .unwrap();
    assert_eq!(
        cached_state.bootstrap_stats().coverage_cache_entries_loaded,
        1
    );
    let filter = lower_filter(
        &cached_state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("gamma".into()))),
        },
        "name = 'gamma'",
    );
    let plan = plan_scan(&cached_state, Some(&[0, 1]), vec![filter]).unwrap();
    let cache_stats = cached_state.coverage_cache().runtime_stats();
    assert_eq!(cache_stats.hits, 1);
    assert_eq!(cache_stats.misses, 0);

    let graph = build_task_graph(&cached_state, &plan).unwrap();
    assert_eq!(graph.morsels_pruned, 1);
    fs::remove_file(path).unwrap();
    fs::remove_file(cache_path).unwrap();
}

#[test]
fn null_pruning_uses_page_indexes_without_materializing_predicate_columns() {
    let state = bootstrap_bytes("nullable", nullable_events_file()).unwrap();
    let projection = vec![0];
    let filter = FilterPlan::pruning_null(1, NullPredicateKind::IsNull, "maybe IS NULL");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    assert_eq!(decoded.stats.predicate_pages_checked, 3);
    assert_eq!(decoded.stats.morsels_pruned, 1);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert_eq!(decoded.stats.pages_decoded, 4);
    assert!(decoded.batches.iter().all(|batch| batch.num_columns() == 1));
    let expected = ["+----+", "| id |", "+----+", "| 2  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
}

#[test]
fn numeric_row_selection_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_numeric(
        0,
        NumericPredicateOp::Gt,
        PredicateLiteral::Int64(2),
        "id > 2",
    );
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| gamma |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.rows_materialized, 1);
    assert!(decoded.stats.native_table_batches >= 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert!(
        decoded.stats.native_lane_predicate_dispatch_scalar
            + decoded.stats.native_lane_predicate_dispatch_avx2
            + decoded.stats.native_lane_predicate_dispatch_neon
            >= 1
    );
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.native_projection_batches >= 1);
    assert!(decoded.stats.native_projection_pages >= 1);
    assert_eq!(decoded.stats.native_projection_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn numeric_in_row_selection_uses_native_numcode_kernel() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("id".into())),
            list: vec![
                LowerExpr::Literal(LowerLiteral::Int64(1)),
                LowerExpr::Literal(LowerLiteral::Int64(3)),
            ],
            negated: false,
        },
        "id IN (1, 3)",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::NumericIn { literals, .. }) => {
            assert_eq!(literals.len(), 2);
        }
        other => panic!("expected numeric IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| alpha |",
        "| gamma |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.rows_materialized, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert!(decoded.stats.native_lane_predicate_rows_seen >= decoded.stats.rows_selected);
    assert!(decoded.stats.native_lane_predicate_rows_matched >= decoded.stats.rows_selected);
    assert!(decoded.stats.native_lane_predicate_bytes_touched > 0);
    assert!(
        decoded.stats.native_lane_predicate_dispatch_scalar
            + decoded.stats.native_lane_predicate_dispatch_avx2
            + decoded.stats.native_lane_predicate_dispatch_neon
            >= 1
    );
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn numeric_not_equal_row_selection_uses_native_numcode_complement_kernel() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("id".into())),
            op: LowerOperator::NotEq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Int64(2))),
        },
        "id != 2",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::NumericNotIn { literals, .. }) => {
            assert_eq!(literals.len(), 1);
        }
        other => panic!("expected numeric NOT IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+-------+",
        "| name  |",
        "+-------+",
        "| alpha |",
        "| gamma |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.rows_materialized, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn numeric_not_in_row_selection_uses_native_numcode_complement_kernel() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("id".into())),
            list: vec![
                LowerExpr::Literal(LowerLiteral::Int64(1)),
                LowerExpr::Literal(LowerLiteral::Int64(3)),
            ],
            negated: true,
        },
        "id NOT IN (1, 3)",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::NumericNotIn { literals, .. }) => {
            assert_eq!(literals.len(), 2);
        }
        other => panic!("expected numeric NOT IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+------+", "| name |", "+------+", "| beta |", "+------+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
}

#[tokio::test]
async fn numeric_not_in_filter_is_pushed_down_exactly() {
    let path = write_temp_cove("events_numeric_not_in_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT name FROM events WHERE id NOT IN (1, 3)")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+------+", "| name |", "+------+", "| beta |", "+------+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT name FROM events WHERE id NOT IN (1, 3)")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=2"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn empty_numeric_in_selects_no_rows_without_page_decode() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_numeric_in(0, Vec::new(), "id IN ()");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    assert!(decoded.batches.is_empty());
    assert_eq!(decoded.stats.rows_selected, 0);
    assert_eq!(decoded.stats.rows_materialized, 0);
    assert_eq!(decoded.stats.pages_decoded, 0);
}

#[tokio::test]
async fn numeric_in_filter_is_pushed_down_exactly() {
    let path = write_temp_cove("events_numeric_in_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT name FROM events WHERE id IN (1, 3) ORDER BY id")
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
        "| gamma |",
        "+-------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT name FROM events WHERE id IN (1, 3)")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn fixed_bytes_equality_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", fixed_uuid_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![2];
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("uid".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8(
                "00000000-0000-0000-0000-000000000002".into(),
            ))),
        },
        "uid = '00000000-0000-0000-0000-000000000002'",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FixedBytesEq { literal, .. }) => {
            assert_eq!(literal.as_slice(), &uuid_bytes(2));
        }
        other => panic!("expected FixedBytes predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| beta    |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.rows_materialized, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn fixed_bytes_in_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", fixed_uuid_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![2];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("uid".into())),
            list: vec![
                LowerExpr::Literal(LowerLiteral::Utf8(
                    "00000000-0000-0000-0000-000000000001".into(),
                )),
                LowerExpr::Literal(LowerLiteral::Utf8(
                    "00000000-0000-0000-0000-000000000003".into(),
                )),
            ],
            negated: false,
        },
        "uid IN ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003')",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FixedBytesIn { literals, .. }) => {
            assert_eq!(literals, &vec![uuid_bytes(1), uuid_bytes(3)]);
        }
        other => panic!("expected FixedBytes IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| alpha   |",
        "| gamma   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.rows_materialized, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn fixed_bytes_same_column_or_lowers_to_native_in() {
    let state = bootstrap_bytes("events", fixed_uuid_events_file()).unwrap();
    let projection = vec![2];
    let filter = lower_filter(
        &state,
        &LowerExpr::Or(vec![
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("uid".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8(
                    "00000000-0000-0000-0000-000000000001".into(),
                ))),
            },
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("uid".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8(
                    "00000000-0000-0000-0000-000000000003".into(),
                ))),
            },
        ]),
        "uid = '00000000-0000-0000-0000-000000000001' OR uid = '00000000-0000-0000-0000-000000000003'",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FixedBytesIn { literals, .. }) => {
            assert_eq!(literals, &vec![uuid_bytes(1), uuid_bytes(3)]);
        }
        other => panic!("expected FixedBytes IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| alpha   |",
        "| gamma   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
}

#[test]
fn filter_pushdown_classifies_fixed_bytes_in_exact() {
    let path = write_temp_cove("fixed_bytes_in_classification", fixed_uuid_events_file());
    let provider = cove_table_from_path(&path).unwrap();
    let in_list = Expr::InList(InList::new(
        Box::new(Expr::Column(Column::from_name("uid"))),
        vec![
            Expr::Literal(
                ScalarValue::Utf8(Some("00000000-0000-0000-0000-000000000001".into())),
                None,
            ),
            Expr::Literal(
                ScalarValue::Utf8(Some("00000000-0000-0000-0000-000000000003".into())),
                None,
            ),
        ],
        false,
    ));

    let support = provider.supports_filters_pushdown(&[&in_list]).unwrap();

    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    fs::remove_file(path).unwrap();
}

#[test]
fn varbytes_prefix_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![0];
    let filter = lower_filter(
        &state,
        &LowerExpr::Like {
            expr: Box::new(LowerExpr::Column("name".into())),
            pattern: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("ga%".into()))),
            negated: false,
            case_insensitive: false,
            escape_char: None,
        },
        "name LIKE 'ga%'",
    );
    assert!(matches!(
        filter.predicate,
        Some(CovePredicate::VarBytesPrefix { .. })
    ));
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.rows_materialized, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[tokio::test]
async fn varbytes_prefix_like_filter_is_pushed_down_exactly() {
    let path = write_temp_cove("events_varbytes_prefix_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT id FROM events WHERE name LIKE 'ga%'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+----+", "| id |", "+----+", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE name LIKE 'ga%'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn varbytes_equality_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![0];
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("beta".into()))),
        },
        "name = 'beta'",
    );
    assert!(matches!(
        filter.predicate,
        Some(CovePredicate::VarBytesEq { .. })
    ));
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.rows_materialized, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_table_batches >= 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.native_projection_batches >= 1);
    assert!(decoded.stats.native_projection_pages >= 1);
    assert_eq!(decoded.stats.native_projection_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn varbytes_in_late_materializes_projected_columns() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![0];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("name".into())),
            list: vec![
                LowerExpr::Literal(LowerLiteral::Utf8("alpha".into())),
                LowerExpr::Literal(LowerLiteral::Utf8("gamma".into())),
            ],
            negated: false,
        },
        "name IN ('alpha', 'gamma')",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::VarBytesIn { literals, .. }) => {
            assert_eq!(literals, &vec![b"alpha".to_vec(), b"gamma".to_vec()]);
        }
        other => panic!("expected VarBytes IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 1  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.rows_materialized, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert!(decoded.stats.native_table_batches >= 1);
    assert!(decoded.stats.native_lane_predicates >= 1);
    assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn varbytes_same_column_or_lowers_to_native_in() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let projection = vec![0];
    let filter = lower_filter(
        &state,
        &LowerExpr::Or(vec![
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("name".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("alpha".into()))),
            },
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("name".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("gamma".into()))),
            },
        ]),
        "name = 'alpha' OR name = 'gamma'",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::VarBytesIn { literals, .. }) => {
            assert_eq!(literals, &vec![b"alpha".to_vec(), b"gamma".to_vec()]);
        }
        other => panic!("expected VarBytes IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 1  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
}

#[tokio::test]
async fn varbytes_equality_filter_is_pushed_down_exactly() {
    let path = write_temp_cove("events_varbytes_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT id FROM events WHERE name = 'beta'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+----+", "| id |", "+----+", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE name = 'beta'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn varbytes_in_filter_is_pushed_down_exactly() {
    let path = write_temp_cove("events_varbytes_in_filter", primitive_events_file());
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "events", &path).unwrap();

    let batches = ctx
        .sql("SELECT id FROM events WHERE name IN ('alpha', 'gamma')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = ["+----+", "| id |", "+----+", "| 1  |", "| 3  |", "+----+"];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT id FROM events WHERE name IN ('alpha', 'gamma')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn absent_filecode_literal_selects_no_rows_without_page_decode() {
    let state = bootstrap_bytes("items", dictionary_items_file(sample_dictionary())).unwrap();
    let projection = vec![0];
    let filter = FilterPlan::pruning_file_code_in(0, Vec::new(), "name = 'green'");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    assert!(decoded.batches.is_empty());
    assert_eq!(decoded.stats.pages_decoded, 0);
    assert_eq!(decoded.stats.rows_selected, 0);
    assert_eq!(decoded.stats.rows_materialized, 0);
}

#[test]
fn direct_decode_resolves_canonical_filecode_filters_for_single_file_state() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("red".into()))),
        },
        "name = 'red'",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FileCodeIn {
            file_codes,
            canonical_values,
            ..
        }) => {
            assert!(file_codes.is_empty());
            assert_eq!(canonical_values.len(), 1);
        }
        other => panic!("expected FileCode predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.lookup_index_hits, 1);
    assert_eq!(decoded.stats.index_rows_selected, 1);
}

#[test]
fn filecode_same_column_or_lowers_to_native_in() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::Or(vec![
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("name".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("red".into()))),
            },
            LowerExpr::Binary {
                left: Box::new(LowerExpr::Column("name".into())),
                op: LowerOperator::Eq,
                right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("blue".into()))),
            },
        ]),
        "name = 'red' OR name = 'blue'",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FileCodeIn {
            file_codes,
            canonical_values,
            ..
        }) => {
            assert!(file_codes.is_empty());
            assert_eq!(canonical_values.len(), 2);
        }
        other => panic!("expected FileCode IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 2);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
}

#[test]
fn native_filecode_group_count_scan_groups_codes_then_decodes_labels() {
    let state = bootstrap_bytes("items", dictionary_items_file(sample_dictionary())).unwrap();
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("name".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Utf8("red".into()))),
        },
        "name = 'red'",
    );
    let projection = Vec::new();
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let scanned = native_filecode_group_count_scan(&state, 0, &plan).unwrap();

    assert_eq!(scanned.groups.counts.get(&canonical_utf8("red")), Some(&1));
    assert_eq!(scanned.groups.counts.get(&canonical_utf8("blue")), None);
    assert_eq!(scanned.groups.null_count, 0);
    assert_eq!(scanned.groups.rows_grouped, 1);
    assert!(scanned.stats.native_group_kernels >= 1);
    assert_eq!(scanned.stats.native_group_rows_matched, 1);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[test]
fn native_filecode_i64_group_aggregate_scan_merges_by_canonical_label() {
    let state = bootstrap_bytes(
        "items",
        scored_dictionary_items_file(sample_dictionary(), [10, 20]),
    )
    .unwrap();
    let projection = Vec::new();
    let plan = plan_scan(&state, Some(&projection), Vec::new()).unwrap();

    let scanned = native_filecode_i64_group_aggregate_scan(&state, 0, 1, &plan).unwrap();

    let red = scanned.groups.groups.get(&canonical_utf8("red")).unwrap();
    let blue = scanned.groups.groups.get(&canonical_utf8("blue")).unwrap();
    assert_eq!(red.row_count, 1);
    assert_eq!(red.aggregate.sum, 10);
    assert_eq!(blue.row_count, 1);
    assert_eq!(blue.aggregate.sum, 20);
    assert_eq!(scanned.groups.null_row_count, 0);
    assert!(scanned.stats.native_aggregate_kernels >= 1);
    assert_eq!(scanned.stats.native_aggregate_rows_matched, 2);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[test]
fn native_filecode_i64_group_aggregate_scan_merges_local_codebook_by_canonical_label() {
    let state = bootstrap_bytes("items", local_codebook_scored_dictionary_items_file()).unwrap();
    let projection = Vec::new();
    let plan = plan_scan(&state, Some(&projection), Vec::new()).unwrap();

    let scanned = native_filecode_i64_group_aggregate_scan(&state, 0, 1, &plan).unwrap();

    let red = scanned.groups.groups.get(&canonical_utf8("red")).unwrap();
    let blue = scanned.groups.groups.get(&canonical_utf8("blue")).unwrap();
    assert_eq!(red.row_count, 2);
    assert_eq!(red.aggregate.sum, 40);
    assert_eq!(blue.row_count, 2);
    assert_eq!(blue.aggregate.sum, 60);
    assert_eq!(scanned.groups.null_row_count, 0);
    assert!(scanned.stats.native_aggregate_kernels >= 1);
    assert_eq!(scanned.stats.native_aggregate_rows_matched, 4);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[test]
fn native_i64_i64_group_aggregate_scan_merges_numeric_groups() {
    let state = bootstrap_bytes("scores", numeric_scores_file()).unwrap();
    let projection = Vec::new();
    let plan = plan_scan(
        &state,
        Some(&projection),
        vec![FilterPlan::pruning_numeric(
            1,
            NumericPredicateOp::GtEq,
            PredicateLiteral::Int64(20),
            "score >= 20",
        )],
    )
    .unwrap();

    let scanned = native_i64_i64_group_aggregate_scan(&state, 0, 1, &plan).unwrap();

    let one = scanned.groups.aggregates.get(&1).unwrap();
    let two = scanned.groups.aggregates.get(&2).unwrap();
    assert_eq!(scanned.groups.row_counts.get(&1), Some(&2));
    assert_eq!(one.count, 2);
    assert_eq!(one.sum, 80);
    assert_eq!(one.min, Some(30));
    assert_eq!(one.max, Some(50));
    assert_eq!(scanned.groups.row_counts.get(&2), Some(&2));
    assert_eq!(two.count, 2);
    assert_eq!(two.sum, 60);
    assert_eq!(two.min, Some(20));
    assert_eq!(two.max, Some(40));
    assert_eq!(scanned.groups.null_row_count, 0);
    assert!(scanned.stats.native_lane_predicates >= 1);
    assert!(scanned.stats.native_aggregate_kernels >= 1);
    assert_eq!(scanned.stats.native_aggregate_rows_matched, 4);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[test]
fn native_bool_group_count_scan_groups_plain_bool_lane() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let projection = Vec::new();
    let plan = plan_scan(
        &state,
        Some(&projection),
        vec![FilterPlan::pruning_numeric(
            0,
            NumericPredicateOp::GtEq,
            PredicateLiteral::Int64(2),
            "id >= 2",
        )],
    )
    .unwrap();

    let scanned = native_bool_group_count_scan(&state, 2, &plan).unwrap();

    assert_eq!(scanned.groups.counts, vec![1, 1]);
    assert_eq!(scanned.groups.null_count, 0);
    assert_eq!(scanned.groups.rows_grouped, 2);
    assert!(scanned.stats.native_lane_predicates >= 1);
    assert!(scanned.stats.native_group_kernels >= 1);
    assert_eq!(scanned.stats.native_group_rows_matched, 2);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[test]
fn native_bool_i64_group_aggregate_scan_uses_dense_bool_groups() {
    let state = bootstrap_bytes("events", primitive_events_file()).unwrap();
    let projection = Vec::new();
    let plan = plan_scan(
        &state,
        Some(&projection),
        vec![FilterPlan::pruning_numeric(
            0,
            NumericPredicateOp::GtEq,
            PredicateLiteral::Int64(2),
            "id >= 2",
        )],
    )
    .unwrap();

    let scanned = native_bool_i64_group_aggregate_scan(&state, 2, 0, &plan).unwrap();

    assert_eq!(scanned.groups.row_counts, vec![1, 1]);
    assert_eq!(scanned.groups.aggregates[0].count, 1);
    assert_eq!(scanned.groups.aggregates[0].sum, 2);
    assert_eq!(scanned.groups.aggregates[1].count, 1);
    assert_eq!(scanned.groups.aggregates[1].sum, 3);
    assert_eq!(scanned.groups.null_row_count, 0);
    assert!(scanned.stats.native_lane_predicates >= 1);
    assert!(scanned.stats.native_aggregate_kernels >= 1);
    assert_eq!(scanned.stats.native_aggregate_rows_matched, 2);
    assert_eq!(scanned.stats.rows_materialized, 0);
}

#[tokio::test]
async fn filecode_same_column_or_filter_is_pushed_down_exactly() {
    let path = write_temp_cove(
        "items_filecode_or_filter",
        dictionary_items_file_with_lookup_index(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let batches = ctx
        .sql("SELECT payload FROM items WHERE name = 'red' OR name = 'blue'")
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
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT payload FROM items WHERE name = 'red' OR name = 'blue'")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn filecode_not_in_uses_native_complement_kernel() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("name".into())),
            list: vec![LowerExpr::Literal(LowerLiteral::Utf8("red".into()))],
            negated: true,
        },
        "name NOT IN ('red')",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FileCodeNotIn {
            file_codes,
            canonical_values,
            ..
        }) => {
            assert!(file_codes.is_empty());
            assert_eq!(canonical_values.len(), 1);
        }
        other => panic!("expected FileCode NOT IN predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert_eq!(decoded.stats.rows_materialized, 1);
    assert_eq!(decoded.stats.residual_predicates, 0);
    assert_eq!(decoded.stats.exact_predicates, 1);
    assert_eq!(decoded.stats.lookup_index_hits, 0);
    assert!(decoded.stats.native_lane_predicates >= 1);
}

#[test]
fn filecode_not_in_with_null_literal_remains_residual() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("name".into())),
            list: vec![LowerExpr::Literal(LowerLiteral::Null)],
            negated: true,
        },
        "name NOT IN (NULL)",
    );

    assert_eq!(filter.use_kind, CoveFilterUse::Unsupported);
    assert!(filter.predicate.is_none());
}

#[tokio::test]
async fn filecode_not_in_filter_is_pushed_down_exactly() {
    let path = write_temp_cove(
        "items_filecode_not_in_filter",
        dictionary_items_file_with_lookup_index(),
    );
    let ctx = SessionContext::new();
    register_cove_file(&ctx, "items", &path).unwrap();

    let batches = ctx
        .sql("SELECT payload FROM items WHERE name NOT IN ('red')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &batches);

    let explain = ctx
        .sql("EXPLAIN SELECT payload FROM items WHERE name NOT IN ('red')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let explain_text = pretty_format_batches(&explain).unwrap().to_string();
    assert!(explain_text.contains("CoveExec"), "{explain_text}");
    assert!(!explain_text.contains("FilterExec"), "{explain_text}");
    assert!(explain_text.contains("exact_filters=1"), "{explain_text}");
    fs::remove_file(path).unwrap();
}

#[test]
fn direct_decode_resolves_bool_filecode_filters_by_value_tag() {
    let state = bootstrap_bytes("items", bool_filecode_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::Binary {
            left: Box::new(LowerExpr::Column("active".into())),
            op: LowerOperator::Eq,
            right: Box::new(LowerExpr::Literal(LowerLiteral::Boolean(true))),
        },
        "active = true",
    );
    match filter.predicate.as_ref() {
        Some(CovePredicate::FileCodeIn {
            file_codes,
            canonical_values,
            canonical_keys,
            ..
        }) => {
            assert!(file_codes.is_empty());
            assert_eq!(canonical_values, &vec![Vec::<u8>::new()]);
            assert_eq!(canonical_keys, &vec![vec![ValueTag::BoolTrue as u8]]);
        }
        other => panic!("expected bool FileCode predicate, got {other:?}"),
    }
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.lookup_index_hits, 1);
    assert_eq!(decoded.stats.index_rows_selected, 1);
}

#[test]
fn task_graph_execution_resolves_canonical_filecode_filters() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("name".into())),
            list: vec![LowerExpr::Literal(LowerLiteral::Utf8("red".into()))],
            negated: false,
        },
        "name IN ('red')",
    );
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();
    let graph = build_task_graph(&state, &plan).unwrap();

    let decoded =
        decode_local_dataset_scan_tasks(&state, &plan, &graph.tasks, 0, graph.partitions.len())
            .unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert!(!graph.tasks.is_empty());
    assert_eq!(graph.tasks.len(), 1);
    assert_eq!(graph.tasks[0].row_selection.as_deref(), Some(&[0][..]));
    assert_eq!(decoded.stats.lookup_index_hits, 0);
    assert_eq!(decoded.stats.lookup_rowref_tasks, 1);
    assert_eq!(decoded.stats.selection_bitsets, 1);
}

#[test]
fn filecode_domain_pruning_skips_non_matching_morsels() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_domain_stats()).unwrap();
    let full = decode_scan(&state, &plan_scan(&state, None, Vec::new()).unwrap()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_file_code_in(0, vec![0], "name = 'red'");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.morsels_considered, 2);
    assert_eq!(decoded.stats.morsels_pruned, 1);
    assert_eq!(decoded.stats.rows_selected, 1);
    assert!(decoded.stats.pages_decoded < full.stats.pages_decoded);
}

#[test]
fn lookup_filecode_equality_selects_rows_before_predicate_page_decode() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_file_code_in(0, vec![0], "name = 'red'");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.lookup_index_hits, 1);
    assert_eq!(decoded.stats.index_rows_selected, 1);
    assert_eq!(decoded.stats.pages_decoded, 1);
}

#[test]
fn absent_lookup_key_prunes_without_page_decode() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_lookup_index()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_file_code_in(0, vec![42], "name = 'green'");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    assert!(decoded.batches.is_empty());
    assert_eq!(decoded.stats.pages_decoded, 0);
    assert_eq!(decoded.stats.morsels_pruned, 1);
}

#[test]
fn inverted_filecode_in_prunes_morsels_before_decode() {
    let state = bootstrap_bytes("items", dictionary_items_file_with_inverted_index()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_file_code_in(0, vec![0], "name IN ('red')");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.morsels_considered, 2);
    assert_eq!(decoded.stats.morsels_pruned, 1);
}

#[test]
fn inverted_index_uses_file_global_morsel_ordinals() {
    let state = bootstrap_bytes(
        "items",
        dictionary_items_file_with_ambiguous_inverted_index(),
    )
    .unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_file_code_in(0, vec![1], "name IN ('blue')");
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| second  |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.morsels_considered, 2);
    assert_eq!(decoded.stats.morsels_pruned, 1);
    assert_eq!(decoded.stats.pages_decoded, 2);
}

#[test]
fn lookup_numcode_equality_uses_exact_key_conversion() {
    let state = bootstrap_bytes("events", numeric_lookup_events_file()).unwrap();
    let projection = vec![1];
    let filter = FilterPlan::pruning_numeric(
        0,
        NumericPredicateOp::Eq,
        PredicateLiteral::Int64(2),
        "id = 2",
    );
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| beta    |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.lookup_index_hits, 1);
    assert_eq!(decoded.stats.pages_decoded, 1);
}

#[test]
fn lookup_float_zero_equality_uses_both_signed_zero_keys() {
    let state = bootstrap_bytes("metrics", float_zero_lookup_file()).unwrap();
    let projection = vec![0];
    let filter = FilterPlan::pruning_numeric(
        1,
        NumericPredicateOp::Eq,
        PredicateLiteral::Float64(0.0),
        "f64 = 0.0",
    );
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = ["+----+", "| id |", "+----+", "| 1  |", "| 2  |", "+----+"];
    assert_batches_eq!(expected, &decoded.batches);
    assert_eq!(decoded.stats.lookup_index_hits, 2);
    assert_eq!(decoded.stats.index_rows_selected, 2);
}

#[cfg(feature = "covi")]
#[test]
fn generated_covi_filecode_utf8_filters_match_sidecar_keys() {
    let bytes = dictionary_items_file_with_lookup_index();
    let covi = cove_index::build::build_covi_from_cove_bytes(
        &bytes,
        &cove_index::build::CoviBuildOptions {
            all_columns: true,
            ..cove_index::build::CoviBuildOptions::default()
        },
    )
    .unwrap();
    let state = bootstrap_bytes_with_covi_artifacts(
        "items",
        bytes,
        vec![covi],
        CoveTableOptions::default(),
    )
    .unwrap();
    let projection = vec![1];
    let filter = lower_filter(
        &state,
        &LowerExpr::InList {
            expr: Box::new(LowerExpr::Column("name".into())),
            list: vec![LowerExpr::Literal(LowerLiteral::Utf8("red".into()))],
            negated: false,
        },
        "name IN ('red')",
    );
    let plan = plan_scan(&state, Some(&projection), vec![filter]).unwrap();
    assert_eq!(plan.covi_candidates.as_ref().map(Vec::len), Some(1));

    let decoded = decode_scan(&state, &plan).unwrap();

    let expected = [
        "+---------+",
        "| payload |",
        "+---------+",
        "| first   |",
        "+---------+",
    ];
    assert_batches_eq!(expected, &decoded.batches);
}
