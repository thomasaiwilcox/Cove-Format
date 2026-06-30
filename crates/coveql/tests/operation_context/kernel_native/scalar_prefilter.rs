use super::*;

#[test]
fn kernel_compare_mode_matches_materialized_object_json_rows() {
    let bytes = include_bytes!("../../../../../conformance/accept/cove_o_temporal_valid.cove");
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(
        kernel.kernel_report.materialized_fingerprint.as_deref(),
        Some(materialized.output_fingerprint.as_str())
    );
    assert_eq!(
        kernel.kernel_report.metrics.rows_scanned,
        kernel.kernel_report.counters.rows_scanned
    );
    assert_eq!(
        kernel.kernel_report.metrics.rows_pruned_by_bitmap,
        kernel
            .kernel_report
            .counters
            .rows_scanned
            .saturating_sub(kernel.kernel_report.counters.rows_after_bitmap)
    );
    assert_eq!(
        kernel.kernel_report.metrics.typed_predicate_rows,
        kernel.kernel_report.counters.typed_predicate_rows
    );
    assert_eq!(
        kernel.kernel_report.metrics.final_materialization_rows,
        kernel.kernel_report.counters.output_rows
    );
    assert!(kernel.explain_json()["execution"]["kernel_report"].is_object());
    assert!(kernel.explain_text().contains("kernel.decision"));
    let operator_contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    assert!(operator_contracts.iter().any(|contract| {
        contract["operator"] == "root_scan" && contract["representation_class"].as_str().is_some()
    }));
    let select_contract = operator_contracts
        .iter()
        .find(|contract| contract["operator"] == "select")
        .expect("direct path select contract is present");
    assert_eq!(select_contract["representation_class"], json!("code_pure"));
    assert_eq!(select_contract["exact"], true);
    assert_eq!(select_contract["residual_required"], false);
    assert!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .is_some_and(|boundaries| boundaries.is_empty())
    );
}

#[test]
fn kernel_native_scalar_bool_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active == true).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let diagnostic = kernel
        .executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED")
        .expect("native scalar prune diagnostic is present");
    assert_eq!(diagnostic.safe_details["predicate_count"], json!(1));
    assert_eq!(diagnostic.safe_details["page_count"], json!(1));
    assert_eq!(diagnostic.safe_details["matched_rows"], json!(2));
    assert_eq!(
        diagnostic.safe_details["residual_verification"],
        json!(false)
    );
    assert_eq!(diagnostic.safe_details["predicate_kernels"], json!(1));
    let predicate_dispatch_total = diagnostic.safe_details["predicate_kernel_scalar"]
        .as_u64()
        .unwrap()
        + diagnostic.safe_details["predicate_kernel_avx2"]
            .as_u64()
            .unwrap()
        + diagnostic.safe_details["predicate_kernel_neon"]
            .as_u64()
            .unwrap();
    assert_eq!(predicate_dispatch_total, 1);
    let decision = kernel
        .kernel_report
        .decisions
        .iter()
        .find(|decision| {
            decision
                .reason
                .contains("native scalar page-lane predicate")
        })
        .expect("native scalar prune decision is present");
    assert!(
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["matched_rows"] == json!(2)
            && decision.safe_details["residual_verification"] == json!(false)
            && decision.safe_details["predicate_kernels"] == json!(1)
    );
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_varbytes_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(name == "Ada").select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
}

#[test]
fn kernel_native_scalar_varbytes_in_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(name in ["Ada", "Bob"]).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["varbytes_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_varbytes_not_in_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(!(name in ["Ada"])).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Bob"})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["varbytes_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_varbytes_not_equal_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(name != "Ada").select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Bob"})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["varbytes_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_null_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = "Person.where(name.isNull()).select(name)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
}

#[test]
fn kernel_native_scalar_numcode_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric >= 10).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(3)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 3);
}

#[test]
fn kernel_native_scalar_numcode_in_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric in [5, 20]).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"metric": 5}), json!({"metric": 20})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["numcode_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_numcode_not_in_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(!(metric in [5, 20])).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"metric": 10}), json!({"metric": 30})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["numcode_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_numcode_not_equal_page_lane_prefilter_matches_materialized() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric != 20).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"metric": 5}),
            json!({"metric": 10}),
            json!({"metric": 30})
        ]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(3)
            && diagnostic.safe_details["predicate_order"] == json!(["numcode_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 3);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_negated_numcode_range_uses_inverted_page_lane_prefilter() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(!(metric >= 10)).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"metric": 5})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["numcode"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_same_path_numeric_or_uses_numcode_in_prefilter() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric == 5 || metric == 20).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["numcode_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_csn_row_directory_prefilter_matches_materialized() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(csn >= 2).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["matched_rows"] == json!(2)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_branch_row_directory_prefilter_matches_materialized() {
    let bytes = object_file_with_bool_records_on_branches(&[true, false, true], &[0, 7, 7]);
    let query = "Thing.where(branch_key == 7).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["system_numeric"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_timestamp_row_directory_prefilter_matches_materialized() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(timestamp_us >= 11).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["system_numeric"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_predicates_are_cost_ordered_before_bitmap_intersection() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active == true && csn >= 2).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(2)
            && diagnostic.safe_details["page_count"] == json!(2)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["system_numeric", "bool_eq"])
            && diagnostic.safe_details["bitmap_intersections"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["predicate_order"] == json!(["system_numeric", "bool_eq"])
            && decision.safe_details["matched_rows"] == json!(1)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
}

#[test]
fn kernel_native_scalar_predicates_short_circuit_after_empty_ordered_bitmap() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active == true && csn > 99).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(2)
            && diagnostic.safe_details["executed_predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(0)
            && diagnostic.safe_details["predicate_order"] == json!(["system_numeric", "bool_eq"])
            && diagnostic.safe_details["bitmap_intersections"] == json!(0)
            && diagnostic.safe_details["short_circuited"] == json!(true)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["executed_predicate_count"] == json!(1)
            && decision.safe_details["short_circuited"] == json!(true)
            && decision.safe_details["matched_rows"] == json!(0)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 0);
}

#[test]
fn kernel_native_scalar_goid_row_directory_prefilter_matches_materialized_without_pushdown() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = r#"Thing.where(goid == uuid"00000000-0000-0000-0000-000000000000").select(active)"#;
    let execution_options = ExecutionOptions {
        pushdown: PushdownOptions {
            enabled: false,
            ..PushdownOptions::default()
        },
        ..ExecutionOptions::default()
    };
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options.clone(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        execution_options,
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
}

#[test]
fn kernel_native_scalar_fixed_bytes_page_lane_prefilter_matches_materialized() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = object_file_with_plain_fixed_uuid_records(&[first, second, first]);
    let query =
        r#"UuidFixedThing.where(uid == uuid"01010101-0101-0101-0101-010101010101").select(uid)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["matched_rows"] == json!(2)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn retained_kernel_native_scalar_numcode_page_lane_prefilter_uses_retained_buffers() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric >= 10).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::new(bytes.clone())),
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(3)
            && diagnostic.safe_details["retained_page_buffers"] == json!(true)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["retained_page_buffers"] == json!(true)
            && decision.safe_details["matched_rows"] == json!(3)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 3);
}

#[test]
fn retained_kernel_native_scalar_csn_row_directory_prefilter_uses_retained_buffers() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(csn >= 2).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::new(bytes.clone())),
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["retained_page_buffers"] == json!(true)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["retained_page_buffers"] == json!(true)
            && decision.safe_details["matched_rows"] == json!(2)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}

#[test]
fn kernel_native_scalar_fixed_bytes_in_page_lane_prefilter_matches_materialized() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = object_file_with_plain_fixed_uuid_records(&[first, second, first]);
    let query = r#"UuidFixedThing.where(uid in [uuid"01010101-0101-0101-0101-010101010101", uuid"02020202-0202-0202-0202-020202020202"]).select(uid)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(3)
            && diagnostic.safe_details["predicate_order"] == json!(["fixed_bytes_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 3);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_fixed_bytes_not_in_page_lane_prefilter_matches_materialized() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = object_file_with_plain_fixed_uuid_records(&[first, second, first]);
    let query = r#"UuidFixedThing.where(!(uid in [uuid"01010101-0101-0101-0101-010101010101"])).select(uid)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"uid": "02020202020202020202020202020202"})]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["fixed_bytes_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_fixed_bytes_not_equal_page_lane_prefilter_matches_materialized() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = object_file_with_plain_fixed_uuid_records(&[first, second, first]);
    let query =
        r#"UuidFixedThing.where(uid != uuid"01010101-0101-0101-0101-010101010101").select(uid)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"uid": "02020202020202020202020202020202"})]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["fixed_bytes_not_in"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn retained_kernel_native_scalar_fixed_bytes_page_lane_prefilter_uses_retained_buffers() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = object_file_with_plain_fixed_uuid_records(&[first, second, first]);
    let query =
        r#"UuidFixedThing.where(uid == uuid"01010101-0101-0101-0101-010101010101").select(uid)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::new(bytes.clone())),
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["page_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["retained_page_buffers"] == json!(true)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("native scalar page-lane predicate")
            && decision.safe_details["retained_page_buffers"] == json!(true)
            && decision.safe_details["matched_rows"] == json!(2)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
}
