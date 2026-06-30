use super::*;

#[cfg(feature = "datafusion")]
#[test]
fn datafusion_projection_report_separates_pushed_and_residual_filters() {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use datafusion::{
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator},
    };

    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("active", DataType::Boolean, false),
        Field::new("score", DataType::Float64, false),
    ]));
    let supported = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let residual = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("score"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Float64(Some(1.0)), None)),
    ));

    let report = coveql::datafusion_projection_pushdown_report_for_plan(
        &schema,
        &[supported, residual],
        &planned,
    )
    .unwrap();

    assert_eq!(
        report.report_version,
        coveql::DATAFUSION_COVEQL_REPORT_VERSION
    );
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 1);
    assert_eq!(report.received_filters.len(), 2);
    assert_eq!(report.filter_outcomes.len(), 2);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert_eq!(
        report.filter_outcomes[1].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::ResidualRejected
    );
    assert_eq!(report.filter_outcomes[0].diagnostic_code, None);
    assert_eq!(
        report.filter_outcomes[1].diagnostic_code.as_deref(),
        Some(coveql::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE)
    );
    assert!(report.filter_outcomes[0].lowered_coveql_predicates[0].contains("projection.active"));
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert_eq!(report.residual_filters.len(), 1);
    assert_eq!(report.rejected_filters, report.residual_filters);
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("projection.active"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert_eq!(report.decode_boundaries.len(), 1);
    assert!(!report.trusted);
}

#[cfg(feature = "datafusion")]
#[test]
fn datafusion_coveql_memtable_registers_materialized_coveql_output() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_projection_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_memtable_for_plan(
        &ctx,
        "people_coveql",
        bytes,
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    assert_eq!(
        report.report_version,
        coveql::DATAFUSION_COVEQL_REPORT_VERSION
    );
    assert_eq!(report.provider_kind, "coveql_memtable");
    assert_eq!(report.root_kind, "projection");
    assert!(report.materialized_coveql_before_registration);
    assert!(report.residual_verification);
    assert!(report.scan_residual_verification_required);
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::MaterializedBaseline
    );
    assert!(report.coveql_scan_materialized_fallback);
    assert!(report.coveql_scan_residual_required);
    assert_eq!(
        report.scan_execution_policy,
        "materialized_arrow_memtable_before_datafusion"
    );
    assert!(!report.scan_filter_pushdown_supported);
    assert!(!report.scan_projection_pushdown_supported);
    assert!(report.unhandled_residuals.is_empty());
    assert_eq!(report.batch_count, 1);
}

#[cfg(feature = "datafusion")]
#[test]
fn datafusion_coveql_provider_rejects_manifest_scoped_single_buffer_registration() {
    let left = object_file_with_bool_records_with_file_id([0xA1; 16], &[true]);
    let right = object_file_with_bool_records_with_file_id([0xB2; 16], &[false]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let scope = coveql::build_manifest_dataset_scope_context(
        &manifest,
        &[
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
        ],
        coveql::ManifestDatasetScopeOptions::default(),
    )
    .unwrap();
    let mut planned = parse_resolve_and_plan_query(
        &left,
        "Thing",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;

    let err = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(left),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("manifest-scoped plans"));
    assert!(err
        .to_string()
        .contains("dedicated manifest DataFusion provider"));
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_planned_coveql_at_scan_time() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_projection_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_provider_for_plan(
        &ctx,
        "people_coveql_provider",
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    assert_eq!(report.provider_kind, "coveql_table_provider");
    assert_eq!(report.root_kind, "projection");
    assert!(!report.materialized_coveql_before_registration);
    assert!(report.residual_verification);
    assert!(report.scan_filter_pushdown_supported);
    assert!(report.scan_projection_pushdown_supported);
    assert_eq!(
        report.scan_execution_policy,
        "datafusion_projection_readback_fast_path_when_negotiated"
    );
    assert!(report.unhandled_residuals.is_empty());
    assert!(report.limit_pushdown_policy.contains("trusted exact"));
    assert_eq!(report.batch_count, 1);

    let dataframe = ctx
        .sql("select active from people_coveql_provider limit 1")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 1);
    assert_eq!(batches[0].schema().field(0).name(), "active");
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        0
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_table_lookup_without_projection_fast_path() {
    use datafusion::catalog::TableProvider;

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records_and_projection(&[false, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes.clone()),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert_eq!(report.root_kind, "table");
    assert!(!report.scan_filter_pushdown_supported);
    assert!(!report.scan_projection_pushdown_supported);
    assert_eq!(report.scan_execution_policy, "planned_coveql_scan");
    assert!(report.unhandled_residuals.iter().any(|residual| {
        residual.contains("table lookup joins execute inside materialized table semantics")
    }));

    let negotiation = provider
        .scan_negotiation_report(Some(&[0]), &[], None)
        .unwrap();
    assert!(!negotiation.projection_pushdown_supported);
    assert!(!negotiation.projection_pushed_to_coveql);
    assert_eq!(negotiation.scan_execution_policy, "planned_coveql_scan");

    ctx.register_table("thing_lookup", provider as Arc<dyn TableProvider>)
        .unwrap();
    let batches = ctx
        .sql("select left_active, right_active from thing_lookup")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_graph_traversal_without_row_pushdown() {
    use datafusion::catalog::TableProvider;

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_person_and_association_record();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "node(Person) as c.traverse(out(edge(CustomerPlacedOrder) as placed)).select(customer: c.goid, target: placed.target_goid)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes.clone()),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert_eq!(report.root_kind, "node");
    assert!(!report.scan_filter_pushdown_supported);
    assert!(!report.scan_projection_pushdown_supported);
    assert_eq!(report.scan_execution_policy, "planned_coveql_scan");
    assert!(report.unhandled_residuals.iter().any(|residual| {
        residual.contains("graph traversal executes inside materialized graph semantics")
    }));

    ctx.register_table("person_traverse", provider as Arc<dyn TableProvider>)
        .unwrap();
    let batches = ctx
        .sql("select customer, target from person_traverse")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    assert_eq!(batches[0].num_columns(), 2);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_temporal_history_with_exact_kernel_probe() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.history(mode: records).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_provider_for_plan(
        &ctx,
        "thing_history_coveql_provider",
        Arc::new(bytes.to_vec()),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    assert_eq!(report.provider_kind, "coveql_table_provider");
    assert_eq!(report.root_kind, "object");
    assert!(!report.materialized_coveql_before_registration);
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!report.coveql_scan_materialized_fallback);
    assert!(!report.coveql_scan_residual_required);
    assert!(!report.scan_filter_pushdown_supported);
    assert!(report.limit_pushdown_policy.contains("filterless scans"));
    assert!(report.notes.iter().any(|note| note
        .contains("planned CoveQL physical execution is attempted inside the provider scan")));

    let explain_batches = ctx
        .sql("explain select active from thing_history_coveql_provider limit 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(explain_text.contains("CoveQlExec"), "{explain_text}");
    assert!(
        explain_text.contains("coveql_scan_authority_probe=ExactOptimizedKernel"),
        "{explain_text}"
    );

    let dataframe = ctx
        .sql("select active from thing_history_coveql_provider limit 1")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert_eq!(values.len(), 1);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_pushes_filterless_scan_projection_to_coveql_readback() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_two_column_projection_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(active, enabled)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_provider_for_plan(
        &ctx,
        "people_projection_coveql_provider",
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    assert!(report.scan_projection_pushdown_supported);

    let explain_batches = ctx
        .sql("explain select active from people_projection_coveql_provider")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(
        explain_text.contains("projection_pushed_to_coveql=true"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains(r#"pushed_projection_columns=["active"]"#),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("scan_execution_policy=datafusion_projection_readback_fast_path"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filter_authority=no_datafusion_filters"),
        "{explain_text}"
    );
    assert!(
        !explain_text.contains("residual_authority=materialized_coveql"),
        "{explain_text}"
    );

    let dataframe = ctx
        .sql("select active from people_projection_coveql_provider")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 1);
    assert_eq!(batches[0].schema().field(0).name(), "active");
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_direct_projection_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records_and_projection(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(thing_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter.clone()]).unwrap();
    assert_eq!(report.root_kind, "projection");
    assert_eq!(report.root_id.as_deref(), Some("thing_projection"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.received_filters.len(), 1);
    assert_eq!(report.filter_outcomes.len(), 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert!(report.filter_outcomes[0].lowered_coveql_predicates[0].contains("projection.active"));
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert!(report.rejected_filters.is_empty());
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);
    let negotiation = provider
        .scan_negotiation_report(Some(&[0]), &[filter.clone()], Some(1))
        .unwrap();
    assert_eq!(
        negotiation.report_version,
        coveql::DATAFUSION_COVEQL_REPORT_VERSION
    );
    assert_eq!(negotiation.provider_kind, "coveql_table_provider");
    assert_eq!(negotiation.root_kind, "projection");
    assert_eq!(
        negotiation.received_projection_columns,
        Some(vec!["active".into()])
    );
    assert!(negotiation.projection_pushdown_supported);
    assert!(negotiation.projection_pushed_to_coveql);
    assert_eq!(negotiation.pushed_projection_columns, vec!["active"]);
    assert_eq!(negotiation.received_filters.len(), 1);
    assert_eq!(negotiation.trusted_filters.len(), 1);
    assert!(negotiation.residual_filters.is_empty());
    assert!(negotiation.filters_trusted_exact);
    assert_eq!(negotiation.received_limit, Some(1));
    assert!(negotiation.limit_pushed_to_coveql);
    assert_eq!(negotiation.pushed_limit, Some(1));
    assert_eq!(
        negotiation.residual_filter_authority,
        "trusted_exact_coveql_pushdown"
    );
    assert_eq!(
        negotiation.scan_execution_policy,
        "datafusion_projection_readback_fast_path"
    );
    assert!(negotiation.unhandled_residuals.is_empty());

    ctx.register_table("thing_coveql_provider", provider as Arc<dyn TableProvider>)
        .unwrap();
    let explain_batches = ctx
        .sql("explain select active from thing_coveql_provider where active = true limit 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(explain_text.contains("trusted_filters=1"), "{explain_text}");
    assert!(
        explain_text.contains("scan_execution_policy=datafusion_projection_readback_fast_path"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filter_authority=trusted_exact_coveql_pushdown"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("received_limit=Some(1)"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("limit_pushed_to_coveql=true"),
        "{explain_text}"
    );

    let dataframe = ctx
        .sql("select active from thing_coveql_provider where active = true limit 1")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(values.value(0));
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_same_column_or_projection_filters_to_in_list() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let bytes = object_file_with_bool_records_and_projection(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(thing_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let true_filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let false_filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(false)), None)),
    ));
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(true_filter),
        Operator::Or,
        Box::new(false_filter),
    ));

    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter.clone()]).unwrap();
    assert_eq!(report.root_kind, "projection");
    assert_eq!(report.root_id.as_deref(), Some("thing_projection"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("projection.active in [2 literals]"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_keeps_computed_projection_filters_as_residuals() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
        physical_plan::displayable,
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_projection_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(value: coalesce(active, false))",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert!(!report.scan_filter_pushdown_supported);
    assert!(!report.scan_projection_pushdown_supported);

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("value"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Unsupported]);
    let report = provider.filter_pushdown_report(&[filter.clone()]).unwrap();
    assert_eq!(report.supported_filter_count, 0);
    assert_eq!(report.residual_filter_count, 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::ResidualRejected
    );
    assert_eq!(
        report.filter_outcomes[0].diagnostic_code.as_deref(),
        Some(coveql::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE)
    );
    assert!(report.pushed_filters.is_empty());
    assert_eq!(report.rejected_filters.len(), 1);
    let negotiation = provider
        .scan_negotiation_report(Some(&[0]), &[filter.clone()], Some(1))
        .unwrap();
    assert_eq!(
        negotiation.report_version,
        coveql::DATAFUSION_COVEQL_REPORT_VERSION
    );
    assert_eq!(
        negotiation.received_projection_columns,
        Some(vec!["value".into()])
    );
    assert!(!negotiation.projection_pushdown_supported);
    assert!(!negotiation.projection_pushed_to_coveql);
    assert!(negotiation.pushed_projection_columns.is_empty());
    assert_eq!(negotiation.received_filters.len(), 1);
    assert_eq!(
        negotiation.filter_outcomes[0].diagnostic_code.as_deref(),
        Some(coveql::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE)
    );
    assert_eq!(negotiation.residual_filters.len(), 1);
    assert_eq!(negotiation.rejected_filters.len(), 1);
    assert!(!negotiation.filters_trusted_exact);
    assert_eq!(negotiation.received_limit, Some(1));
    assert!(!negotiation.limit_pushed_to_coveql);
    assert_eq!(negotiation.pushed_limit, None);
    assert_eq!(
        negotiation.residual_filter_authority,
        "datafusion_residual_verification"
    );
    assert_eq!(negotiation.scan_execution_policy, "planned_coveql_scan");
    assert!(negotiation.unhandled_residuals.iter().any(|residual| {
        residual.contains("DataFusion scan projection remains outside CoveQL")
    }));
    assert!(negotiation
        .unhandled_residuals
        .iter()
        .any(|residual| { residual.contains("DataFusion scan limit remains outside CoveQL") }));

    let state = ctx.state();
    let exec = TableProvider::scan(provider.as_ref(), &state, None, &[filter], Some(1))
        .await
        .unwrap();
    let explain_text = displayable(exec.as_ref()).one_line().to_string();
    assert!(explain_text.contains("CoveQlExec"), "{explain_text}");
    assert!(
        explain_text.contains("received_filters=1"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filters=1"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("rejected_filters=1"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("received_limit=Some(1)"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("limit_pushed_to_coveql=false"),
        "{explain_text}"
    );
    assert!(explain_text.contains("limit=None"), "{explain_text}");
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_keeps_aliased_projection_filters_as_residuals() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let bytes = minimal_object_with_projection_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(flag: active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert!(!report.scan_filter_pushdown_supported);
    assert!(!report.scan_projection_pushdown_supported);

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("flag"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Unsupported]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 0);
    assert_eq!(report.residual_filter_count, 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::ResidualRejected
    );
    assert_eq!(
        report.filter_outcomes[0].diagnostic_code.as_deref(),
        Some(coveql::DATAFUSION_PUSH_FILTER_UNSAFE_DIAGNOSTIC_CODE)
    );
    assert!(report.pushed_filters.is_empty());
    assert_eq!(report.rejected_filters.len(), 1);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_object_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let provider_report = provider.report();
    assert_eq!(provider_report.root_kind, "object");
    assert_eq!(provider_report.root_id.as_deref(), Some("Thing"));
    assert_eq!(provider_report.dataset_file_count, 1);
    assert!(provider_report.scan_filter_pushdown_supported);
    assert!(provider_report.scan_projection_pushdown_supported);
    assert_eq!(
        provider_report.scan_execution_policy,
        "coveql_physical_or_materialized_scan"
    );
    assert!(provider_report
        .residual_filter_authority
        .contains("DataFusion retains SQL filters"));
    assert!(provider_report
        .residual_filter_authority
        .contains("projection guards"));

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.root_id.as_deref(), Some("Thing"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.received_filters.len(), 1);
    assert_eq!(report.filter_outcomes.len(), 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert!(report.filter_outcomes[0].lowered_coveql_predicates[0].contains("object.active"));
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.rejected_filters.is_empty());
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.active"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);

    ctx.register_table(
        "thing_object_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let explain_batches = ctx
        .sql("explain select active from thing_object_coveql_provider where active = true")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(explain_text.contains("CoveQlExec"), "{explain_text}");
    assert!(explain_text.contains("pushed_filters=1"), "{explain_text}");
    assert!(explain_text.contains("trusted_filters=1"), "{explain_text}");
    assert!(
        explain_text.contains("residual_filters=0"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("rejected_filters=0"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("received_filters=1"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("scan_execution_policy=coveql_physical_or_materialized_scan"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filter_authority=trusted_exact_coveql_pushdown"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("coveql_scan_authority_probe="),
        "{explain_text}"
    );
    assert!(
        !explain_text.contains("residual_authority=materialized_coveql"),
        "{explain_text}"
    );

    let limit_explain_batches = ctx
        .sql("explain select active from thing_object_coveql_provider where active = true limit 1")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut limit_explain_text = String::new();
    for batch in &limit_explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        limit_explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(
        limit_explain_text.contains("trusted_filters=1"),
        "{limit_explain_text}"
    );
    assert!(
        limit_explain_text.contains("received_limit=Some(1)"),
        "{limit_explain_text}"
    );
    assert!(
        limit_explain_text.contains("limit_pushed_to_coveql=true"),
        "{limit_explain_text}"
    );
    assert!(
        limit_explain_text.contains("limit=Some(1)"),
        "{limit_explain_text}"
    );

    let dataframe = ctx
        .sql("select active from thing_object_coveql_provider where active = true limit 1")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(values.value(0));
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_manifest_coveql_provider_executes_validated_members_with_residual_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let left = object_file_with_bool_records_with_file_id([0xA1; 16], &[false, true]);
    let right = object_file_with_bool_records_with_file_id([0xB2; 16], &[true, false, true]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let scope = coveql::build_manifest_dataset_scope_context(
        &manifest,
        &[
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
        ],
        coveql::ManifestDatasetScopeOptions::default(),
    )
    .unwrap();
    let mut planned = parse_resolve_and_plan_query(
        &left,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let provider = coveql::datafusion_manifest_coveql_provider_for_plan(
        vec![
            coveql::CoveQlRetainedManifestMember::from_vec("right.cove", right),
            coveql::CoveQlRetainedManifestMember::from_vec("left.cove", left),
        ],
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert_eq!(report.provider_kind, "manifest_coveql_table_provider");
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.root_id.as_deref(), Some("Thing"));
    assert_eq!(report.dataset_file_count, 2);
    assert!(report.scan_filter_pushdown_supported);
    assert!(report.scan_projection_pushdown_supported);
    assert_eq!(
        report.scan_execution_policy,
        "manifest_coveql_physical_or_materialized_scan"
    );
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::MaterializedBaseline
    );
    assert!(report.coveql_scan_materialized_fallback);
    assert!(report.coveql_scan_residual_required);
    assert!(report
        .residual_filter_authority
        .contains("manifest physical CoveQL kernel"));
    assert!(report
        .residual_filter_authority
        .contains("materialized CoveQL oracle"));
    assert_eq!(report.row_count, 5);

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let pushdown_report = provider.filter_pushdown_report(&[filter.clone()]).unwrap();
    assert_eq!(pushdown_report.root_kind, "object");
    assert_eq!(pushdown_report.root_id.as_deref(), Some("Thing"));
    assert!(pushdown_report.trusted);
    assert_eq!(pushdown_report.supported_filter_count, 1);
    assert_eq!(pushdown_report.residual_filter_count, 0);
    assert_eq!(
        pushdown_report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    let negotiation = provider
        .scan_negotiation_report(Some(&[0]), &[filter.clone()], Some(2))
        .unwrap();
    assert_eq!(negotiation.provider_kind, "manifest_coveql_table_provider");
    assert_eq!(negotiation.root_kind, "object");
    assert_eq!(negotiation.dataset_file_count, 2);
    assert_eq!(
        negotiation.received_projection_columns,
        Some(vec!["active".into()])
    );
    assert!(negotiation.projection_pushdown_supported);
    assert!(negotiation.projection_pushed_to_coveql);
    assert_eq!(negotiation.pushed_projection_columns, vec!["active"]);
    assert_eq!(negotiation.received_filters.len(), 1);
    assert_eq!(negotiation.trusted_filters.len(), 1);
    assert!(negotiation.residual_filters.is_empty());
    assert!(negotiation.filters_trusted_exact);
    assert_eq!(negotiation.received_limit, Some(2));
    assert!(negotiation.limit_pushed_to_coveql);
    assert_eq!(negotiation.pushed_limit, Some(2));
    assert_eq!(
        negotiation.residual_filter_authority,
        "trusted_exact_coveql_pushdown"
    );
    assert_eq!(
        negotiation.scan_execution_policy,
        "manifest_coveql_physical_or_materialized_scan"
    );

    ctx.register_table(
        "manifest_thing_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let explain_batches = ctx
        .sql(
            "explain select active from manifest_thing_coveql_provider where active = true limit 2",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(
        explain_text.contains("ManifestCoveQlExec"),
        "{explain_text}"
    );
    assert!(explain_text.contains("pushed_filters=1"), "{explain_text}");
    assert!(explain_text.contains("trusted_filters=1"), "{explain_text}");
    assert!(
        explain_text.contains("residual_filters=0"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("received_limit=Some(2)"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("limit_pushed_to_coveql=true"),
        "{explain_text}"
    );
    assert!(explain_text.contains("limit=Some(2)"), "{explain_text}");
    assert!(
        explain_text
            .contains("scan_execution_policy=manifest_coveql_physical_or_materialized_scan"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filter_authority=trusted_exact_coveql_pushdown"),
        "{explain_text}"
    );
    assert!(
        !explain_text.contains("residual_authority=manifest_materialized_coveql"),
        "{explain_text}"
    );

    let batches = ctx
        .sql("select active from manifest_thing_coveql_provider where active = true limit 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
    for batch in &batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        for row in 0..values.len() {
            assert!(values.value(row));
        }
    }
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_manifest_coveql_provider_reports_exact_kernel_with_validated_bridge() {
    use datafusion::catalog::TableProvider;

    let ctx = datafusion::execution::context::SessionContext::new();
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let scope = coveql::build_manifest_dataset_scope_context(
        &manifest,
        &[
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
        ],
        coveql::ManifestDatasetScopeOptions {
            tenant_id: Some("tenant-a".into()),
            security: SecurityContext {
                principal_or_session: Some("principal-a".into()),
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            code_domain_bridge_proofs: vec![coveql::ManifestCodeDomainBridgeProof {
                domain_id: "cove_e:org.example.coveql:exec-codes".into(),
                bridge_kind: "manifest_validated_canonical_remap".into(),
                exact: true,
                epoch: Some(1),
                reason: "manifest member dictionaries remap to the same canonical code domain"
                    .into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();
    let mut planned = parse_resolve_and_plan_query(
        &left,
        "Person.select(name)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let provider = coveql::datafusion_manifest_coveql_provider_for_plan(
        vec![
            coveql::CoveQlRetainedManifestMember::from_vec("right.cove", right),
            coveql::CoveQlRetainedManifestMember::from_vec("left.cove", left),
        ],
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert_eq!(report.provider_kind, "manifest_coveql_table_provider");
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.dataset_file_count, 2);
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert_eq!(
        report.scan_execution_policy,
        "manifest_coveql_physical_or_materialized_scan"
    );
    assert!(!report.coveql_scan_materialized_fallback);
    assert!(!report.coveql_scan_residual_required);

    ctx.register_table("manifest_people_exact", provider as Arc<dyn TableProvider>)
        .unwrap();
    let batches = ctx
        .sql("select name from manifest_people_exact order by name")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(
        (0..values.len())
            .map(|row| values.value(row))
            .collect::<Vec<_>>(),
        vec!["blue", "green", "red", "red"]
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_manifest_coveql_provider_lowers_direct_projection_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let left =
        object_file_with_bool_records_and_projection_with_file_id([0xA1; 16], &[false, true]);
    let right =
        object_file_with_bool_records_and_projection_with_file_id([0xB2; 16], &[true, false, true]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let scope = coveql::build_manifest_dataset_scope_context(
        &manifest,
        &[
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
        ],
        coveql::ManifestDatasetScopeOptions::default(),
    )
    .unwrap();
    let mut planned = parse_resolve_and_plan_query(
        &left,
        "projection(thing_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let provider = coveql::datafusion_manifest_coveql_provider_for_plan(
        vec![
            coveql::CoveQlRetainedManifestMember::from_vec("right.cove", right),
            coveql::CoveQlRetainedManifestMember::from_vec("left.cove", left),
        ],
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let report = provider.report();
    assert_eq!(report.provider_kind, "manifest_coveql_table_provider");
    assert_eq!(report.root_kind, "projection");
    assert!(report.scan_filter_pushdown_supported);
    assert!(report.scan_projection_pushdown_supported);
    assert_eq!(
        report.scan_execution_policy,
        "manifest_coveql_physical_or_materialized_scan"
    );
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!report.coveql_scan_materialized_fallback);
    assert!(!report.coveql_scan_residual_required);

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let pushdown_report = provider.filter_pushdown_report(&[filter.clone()]).unwrap();
    assert!(pushdown_report.trusted);
    assert_eq!(pushdown_report.supported_filter_count, 1);
    assert_eq!(pushdown_report.residual_filter_count, 0);
    assert_eq!(
        pushdown_report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    let negotiation = provider
        .scan_negotiation_report(Some(&[0]), &[filter.clone()], Some(2))
        .unwrap();
    assert_eq!(negotiation.provider_kind, "manifest_coveql_table_provider");
    assert_eq!(negotiation.root_kind, "projection");
    assert_eq!(negotiation.dataset_file_count, 2);
    assert_eq!(
        negotiation.received_projection_columns,
        Some(vec!["active".into()])
    );
    assert!(negotiation.projection_pushdown_supported);
    assert!(negotiation.projection_pushed_to_coveql);
    assert_eq!(negotiation.pushed_projection_columns, vec!["active"]);
    assert_eq!(negotiation.received_filters.len(), 1);
    assert_eq!(negotiation.trusted_filters.len(), 1);
    assert!(negotiation.residual_filters.is_empty());
    assert!(negotiation.filters_trusted_exact);
    assert_eq!(negotiation.received_limit, Some(2));
    assert!(negotiation.limit_pushed_to_coveql);
    assert_eq!(negotiation.pushed_limit, Some(2));
    assert_eq!(
        negotiation.residual_filter_authority,
        "trusted_exact_coveql_pushdown"
    );
    assert_eq!(
        negotiation.scan_execution_policy,
        "manifest_coveql_physical_or_materialized_scan"
    );

    ctx.register_table(
        "manifest_projection_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let explain_batches = ctx
        .sql(
            "explain select active from manifest_projection_coveql_provider where active = true limit 2",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(
        explain_text.contains("ManifestCoveQlExec"),
        "{explain_text}"
    );
    assert!(explain_text.contains("pushed_filters=1"), "{explain_text}");
    assert!(explain_text.contains("trusted_filters=1"), "{explain_text}");
    assert!(
        explain_text.contains("residual_filters=0"),
        "{explain_text}"
    );
    assert!(explain_text.contains("trusted=true"), "{explain_text}");
    assert!(
        explain_text.contains("received_limit=Some(2)"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("limit_pushed_to_coveql=true"),
        "{explain_text}"
    );
    assert!(explain_text.contains("limit=Some(2)"), "{explain_text}");
    assert!(
        explain_text
            .contains("scan_execution_policy=manifest_coveql_physical_or_materialized_scan"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("residual_filter_authority=trusted_exact_coveql_pushdown"),
        "{explain_text}"
    );

    let batches = ctx
        .sql("select active from manifest_projection_coveql_provider where active = true limit 2")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
    for batch in &batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        for row in 0..values.len() {
            assert!(values.value(row));
        }
    }
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_guards_row_projection_when_filters_need_residual() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(goid, active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Inexact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert!(report.trusted);

    ctx.register_table(
        "thing_projected_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let explain_batches = ctx
        .sql("explain select goid from thing_projected_coveql_provider where active = true")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut explain_text = String::new();
    for batch in &explain_batches {
        for column in batch.columns() {
            if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
                for row in 0..values.len() {
                    if !values.is_null(row) {
                        explain_text.push_str(values.value(row));
                    }
                }
            }
        }
    }
    assert!(
        explain_text.contains("projection_pushed_to_coveql=true"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains(r#"pushed_projection_columns=["goid", "active"]"#),
        "{explain_text}"
    );
    assert!(explain_text.contains("FilterExec"), "{explain_text}");
    assert!(
        explain_text.contains("projection=[goid@0]"),
        "{explain_text}"
    );
    assert!(explain_text.contains("trusted_filters=1"), "{explain_text}");

    let dataframe = ctx
        .sql("select goid from thing_projected_coveql_provider where active = true")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
    assert_eq!(batches[0].num_columns(), 1);
    assert_eq!(batches[0].schema().field(0).name(), "goid");
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_numeric_range_object_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "MetricThing.select(metric)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let lower = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("metric"))),
        Operator::Gt,
        Box::new(Expr::Literal(ScalarValue::Int64(Some(10)), None)),
    ));
    let upper = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("metric"))),
        Operator::LtEq,
        Box::new(Expr::Literal(ScalarValue::Int64(Some(30)), None)),
    ));
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(lower),
        Operator::And,
        Box::new(upper),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 2);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 2);
    assert_eq!(report.trusted_filters.len(), 2);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 2);
    assert!(report
        .lowered_coveql_predicates
        .iter()
        .any(|predicate| predicate.contains("object.metric >")));
    assert!(report
        .lowered_coveql_predicates
        .iter()
        .any(|predicate| predicate.contains("object.metric <=")));
    assert_eq!(
        report.proof_states,
        vec![
            PredicateProofState::ProvenExact,
            PredicateProofState::ProvenExact
        ]
    );
    assert_eq!(report.filter_outcomes.len(), 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.trusted);

    ctx.register_table(
        "metric_range_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql(
            "select metric from metric_range_coveql_provider \
             where metric > 10 and metric <= 30 order by metric",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(
        (0..values.len())
            .map(|index| values.value(index))
            .collect::<Vec<_>>(),
        vec![20, 30]
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_filecode_string_range_filters_with_collation_as_trusted_exact(
) {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Ada", "Nia", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        Operator::Lt,
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("M".into())), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.name <"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert_eq!(report.filter_outcomes.len(), 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report
        .decode_boundaries
        .iter()
        .any(|boundary| boundary.contains("effective UTF-8 bytewise collation")));
    assert!(report.trusted);

    ctx.register_table(
        "person_name_range_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql(
            "select name from person_name_range_coveql_provider \
             where name < 'M' order by name",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut names = Vec::new();
    for batch in &batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for row in 0..values.len() {
            names.push(values.value(row).to_string());
        }
    }
    assert_eq!(names, vec!["Ada".to_string(), "Bob".to_string()]);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_filecode_string_range_filters_with_default_collation_as_trusted_exact(
) {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Nia", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        Operator::Lt,
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("M".into())), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert_eq!(report.filter_outcomes.len(), 1);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report
        .decode_boundaries
        .iter()
        .any(|boundary| boundary.contains("effective UTF-8 bytewise collation")));
    assert!(report.trusted);

    ctx.register_table(
        "person_name_default_range_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql(
            "select name from person_name_default_range_coveql_provider \
             where name < 'M' order by name",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let mut names = Vec::new();
    for batch in &batches {
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for row in 0..values.len() {
            names.push(values.value(row).to_string());
        }
    }
    assert_eq!(names, vec!["Ada".to_string(), "Bob".to_string()]);
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_bare_boolean_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::Column,
        logical_expr::{Expr, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::Column(Column::from_name("active"));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.root_id.as_deref(), Some("Thing"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.active"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);

    ctx.register_table(
        "thing_bool_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let dataframe = ctx
        .sql("select active from thing_bool_coveql_provider where active")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_negated_boolean_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::Column,
        logical_expr::{Expr, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_bool_records(&[false, true, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::Not(Box::new(Expr::Column(Column::from_name("active"))));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.root_id.as_deref(), Some("Thing"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.active"));
    assert!(report.lowered_coveql_predicates[0].contains("Boolean(false)"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);

    ctx.register_table(
        "thing_negated_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let dataframe = ctx
        .sql("select active from thing_negated_coveql_provider where not active")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(!values.value(0));
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_not_equal_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes =
        object_file_with_nullable_bool_records(&[Some(false), Some(true), None, Some(true)]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::NotEq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.active"));
    assert!(report.lowered_coveql_predicates[0].contains("!="));
    assert!(report.lowered_coveql_predicates[0].contains("Boolean(true)"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);

    ctx.register_table(
        "thing_not_equal_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let dataframe = ctx
        .sql("select active from thing_not_equal_coveql_provider where active != true")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(!values.value(0));
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_not_of_equality_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "green", "red"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::Not(Box::new(Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Utf8(Some("red".into())), None)),
    ))));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "object");
    assert_eq!(report.root_id.as_deref(), Some("Person"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert!(report.rejected_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.name !="));
    assert!(report.lowered_coveql_predicates[0].contains(r#"Utf8("red")"#));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert!(report.decode_boundaries.is_empty());
    assert!(report.trusted);

    ctx.register_table(
        "person_not_eq_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql("select name from person_not_eq_coveql_provider where not (name = 'red')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(
        (0..values.len())
            .map(|index| values.value(index))
            .collect::<Vec<_>>(),
        vec!["blue", "green"]
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_is_true_false_boolean_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::Column,
        logical_expr::{Expr, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes =
        object_file_with_nullable_bool_records(&[Some(false), Some(true), None, Some(true)]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    for (filter, expected_literal) in [
        (
            Expr::IsTrue(Box::new(Expr::Column(Column::from_name("active")))),
            "Boolean(true)",
        ),
        (
            Expr::IsFalse(Box::new(Expr::Column(Column::from_name("active")))),
            "Boolean(false)",
        ),
    ] {
        let support =
            TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
        assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
        let report = provider
            .filter_pushdown_report(std::slice::from_ref(&filter))
            .unwrap();
        assert_eq!(report.supported_filter_count, 1);
        assert_eq!(report.residual_filter_count, 0);
        assert_eq!(report.pushed_filters.len(), 1);
        assert_eq!(report.trusted_filters.len(), 1);
        assert!(report.residual_filters.is_empty());
        assert_eq!(report.lowered_coveql_predicates.len(), 1);
        assert!(report.lowered_coveql_predicates[0].contains("object.active"));
        assert!(report.lowered_coveql_predicates[0].contains(expected_literal));
        assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
        assert!(report.trusted);
    }

    ctx.register_table(
        "thing_is_bool_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let true_batches = ctx
        .sql("select active from thing_is_bool_coveql_provider where active is true")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        true_batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>(),
        2
    );
    let false_batches = ctx
        .sql("select active from thing_is_bool_coveql_provider where active is false")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        false_batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>(),
        1
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_null_boolean_object_filters() {
    use datafusion::{
        catalog::TableProvider,
        common::Column,
        logical_expr::{Expr, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes =
        object_file_with_nullable_bool_records(&[Some(false), Some(true), None, Some(true)]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    for (filter, expected_summary) in [
        (
            Expr::IsNull(Box::new(Expr::Column(Column::from_name("active")))),
            "is null",
        ),
        (
            Expr::IsNotNull(Box::new(Expr::Column(Column::from_name("active")))),
            "is not null",
        ),
    ] {
        let support =
            TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
        assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
        let report = provider
            .filter_pushdown_report(std::slice::from_ref(&filter))
            .unwrap();
        assert_eq!(report.supported_filter_count, 1);
        assert_eq!(report.residual_filter_count, 0);
        assert_eq!(report.pushed_filters.len(), 1);
        assert_eq!(report.trusted_filters.len(), 1);
        assert!(report.residual_filters.is_empty());
        assert_eq!(report.lowered_coveql_predicates.len(), 1);
        assert!(report.lowered_coveql_predicates[0].contains("object.active"));
        assert!(report.lowered_coveql_predicates[0].contains(expected_summary));
        assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
        assert_eq!(report.filter_outcomes.len(), 1);
        assert_eq!(
            report.filter_outcomes[0].outcome,
            coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
        );
        assert!(report.filter_outcomes[0].trusted);
        assert!(report.trusted);
    }

    ctx.register_table(
        "thing_null_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let null_batches = ctx
        .sql("select active from thing_null_coveql_provider where active is null")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        null_batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>(),
        1
    );
    let null_values = null_batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(null_values.is_null(0));

    let not_null_batches = ctx
        .sql("select active from thing_null_coveql_provider where active is not null")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        not_null_batches
            .iter()
            .map(|batch| batch.num_rows())
            .sum::<usize>(),
        3
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_same_column_or_object_filters_to_in_list() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes =
        object_file_with_nullable_bool_records(&[Some(false), Some(true), None, Some(true)]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let true_filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(true)), None)),
    ));
    let false_filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("active"))),
        Operator::Eq,
        Box::new(Expr::Literal(ScalarValue::Boolean(Some(false)), None)),
    ));
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(true_filter),
        Operator::Or,
        Box::new(false_filter),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 1);
    assert!(report.lowered_coveql_predicates[0].contains("object.active in [2 literals]"));
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.trusted);

    ctx.register_table(
        "thing_or_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql(
            "select active from thing_or_coveql_provider \
             where active = true or active = false",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        3
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_not_in_object_filters_to_ne_conjunction() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{expr::InList, Expr, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "green", "red"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let filter = Expr::InList(InList::new(
        Box::new(Expr::Column(Column::from_name("name"))),
        vec![
            Expr::Literal(ScalarValue::Utf8(Some("red".into())), None),
            Expr::Literal(ScalarValue::Utf8(Some("green".into())), None),
        ],
        true,
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Exact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.supported_filter_count, 2);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 2);
    assert_eq!(report.trusted_filters.len(), 2);
    assert!(report.residual_filters.is_empty());
    assert!(report.rejected_filters.is_empty());
    assert_eq!(report.lowered_coveql_predicates.len(), 2);
    assert!(report
        .lowered_coveql_predicates
        .iter()
        .all(|predicate| predicate.contains("object.name !=")));
    assert_eq!(
        report.proof_states,
        vec![
            PredicateProofState::ProvenExact,
            PredicateProofState::ProvenExact
        ]
    );
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert!(report.decode_boundaries.is_empty());
    assert!(report.trusted);

    ctx.register_table(
        "person_not_in_coveql_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let batches = ctx
        .sql("select name from person_not_in_coveql_provider where name not in ('red', 'green')")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(values.value(0), "blue");
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_association_root_at_scan_time() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_person_and_association_record();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_provider_for_plan(
        &ctx,
        "association_coveql_provider",
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    assert_eq!(report.provider_kind, "coveql_table_provider");
    assert_eq!(report.root_kind, "association");
    assert_eq!(report.root_id.as_deref(), Some("CustomerPlacedOrder"));
    assert_eq!(report.dataset_file_count, 1);
    assert!(!report.materialized_coveql_before_registration);
    assert!(report.residual_verification);
    assert!(!report.scan_residual_verification_required);
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!report.coveql_scan_materialized_fallback);
    assert!(!report.coveql_scan_residual_required);
    assert!(report.scan_filter_pushdown_supported);
    assert_eq!(report.row_count, 1);

    let dataframe = ctx
        .sql("select source_goid, target_goid from association_coveql_provider")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 2);
    assert_eq!(batches[0].schema().field(0).name(), "source_goid");
    assert_eq!(batches[0].schema().field(1).name(), "target_goid");
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    let source_values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let target_values = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(source_values.value(0), "00000000000000000000000000000000");
    assert_eq!(target_values.value(0), "02020202020202020202020202020202");
}

#[test]
fn datafusion_table_provider_output_mode_is_valid_for_association_and_evidence_roots() {
    for (bytes, query) in [
        (
            object_file_with_person_and_association_record(),
            "association(CustomerPlacedOrder).select(source_goid)",
        ),
        (
            minimal_object_with_evidence_index_file(),
            "evidence(Person, grain: object).select(source_id)",
        ),
    ] {
        let err = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            ResolveOptions {
                output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
                ..ResolveOptions::default()
            },
            PlanOptions::default(),
            ExecutionOptions::default(),
            validation_options(),
        )
        .unwrap_err();

        assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_OUTPUT");
        assert!(err.diagnostics[0]
            .message
            .contains("DataFusion output is exposed through"));
    }
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_association_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = object_file_with_person_and_association_record();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("source_goid"))),
        Operator::Eq,
        Box::new(Expr::Literal(
            ScalarValue::Utf8(Some("00000000000000000000000000000000".into())),
            None,
        )),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Inexact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "association");
    assert_eq!(report.root_id.as_deref(), Some("CustomerPlacedOrder"));
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert!(report.lowered_coveql_predicates[0].contains("association.source_goid"));
    assert!(report.trusted);

    ctx.register_table(
        "association_coveql_filter_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let dataframe = ctx
        .sql(
            "select source_goid from association_coveql_filter_provider \
             where source_goid = '00000000000000000000000000000000'",
        )
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_executes_evidence_root_at_scan_time() {
    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_evidence_index_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "evidence(Person, grain: object).select(source_id, source_row_identity)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let report = coveql::register_datafusion_coveql_provider_for_plan(
        &ctx,
        "evidence_coveql_provider",
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    assert_eq!(report.provider_kind, "coveql_table_provider");
    assert_eq!(report.root_kind, "evidence");
    assert_eq!(report.root_id, None);
    assert_eq!(report.dataset_file_count, 1);
    assert!(!report.materialized_coveql_before_registration);
    assert!(report.residual_verification);
    assert!(report.scan_residual_verification_required);
    assert_eq!(
        report.coveql_scan_authority_source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!report.coveql_scan_materialized_fallback);
    assert!(report.coveql_scan_residual_required);
    assert!(report.scan_filter_pushdown_supported);
    assert_eq!(report.row_count, 1);

    let dataframe = ctx
        .sql("select source_id, source_row_identity from evidence_coveql_provider")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
    assert_eq!(batches[0].schema().field(0).name(), "source_id");
    assert_eq!(batches[0].schema().field(1).name(), "source_row_identity");
    let source_values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let identity_values = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(source_values.value(0), "crm.customers");
    assert_eq!(identity_values.value(0), "customer_id=1");
}

#[cfg(feature = "datafusion")]
#[tokio::test]
async fn datafusion_coveql_provider_lowers_evidence_filters_as_trusted_exact() {
    use datafusion::{
        catalog::TableProvider,
        common::{Column, ScalarValue},
        logical_expr::{BinaryExpr, Expr, Operator, TableProviderFilterPushDown},
    };

    let ctx = datafusion::execution::context::SessionContext::new();
    let bytes = minimal_object_with_evidence_index_file();
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "evidence(Person, grain: object).select(source_id, source_row_identity)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let provider = coveql::datafusion_coveql_provider_for_plan(
        Arc::new(bytes),
        &planned,
        ExecutionOptions::default(),
    )
    .unwrap();
    let filter = Expr::BinaryExpr(BinaryExpr::new(
        Box::new(Expr::Column(Column::from_name("source_id"))),
        Operator::Eq,
        Box::new(Expr::Literal(
            ScalarValue::Utf8(Some("crm.customers".into())),
            None,
        )),
    ));
    let support = TableProvider::supports_filters_pushdown(provider.as_ref(), &[&filter]).unwrap();
    assert_eq!(support, vec![TableProviderFilterPushDown::Inexact]);
    let report = provider.filter_pushdown_report(&[filter]).unwrap();
    assert_eq!(report.root_kind, "evidence");
    assert_eq!(report.root_id, None);
    assert_eq!(report.supported_filter_count, 1);
    assert_eq!(report.residual_filter_count, 0);
    assert_eq!(report.pushed_filters.len(), 1);
    assert_eq!(report.trusted_filters.len(), 1);
    assert!(report.residual_filters.is_empty());
    assert_eq!(report.proof_states, vec![PredicateProofState::ProvenExact]);
    assert_eq!(
        report.filter_outcomes[0].outcome,
        coveql::DataFusionCoveQlFilterOutcomeKind::TrustedExact
    );
    assert!(report.filter_outcomes[0].trusted);
    assert!(report.decode_boundaries.is_empty());
    assert!(report.lowered_coveql_predicates[0].contains("evidence.source_id"));
    assert!(report.trusted);

    ctx.register_table(
        "evidence_coveql_filter_provider",
        provider as Arc<dyn TableProvider>,
    )
    .unwrap();
    let dataframe = ctx
        .sql("select source_id from evidence_coveql_filter_provider where source_id = 'crm.customers'")
        .await
        .unwrap();
    let batches = dataframe.collect().await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
}

#[test]
fn logical_plan_text_printer_is_deterministic() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(planned.logical_plan_text(), planned.logical_plan_text());
}
