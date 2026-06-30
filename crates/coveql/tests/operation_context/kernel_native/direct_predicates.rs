use super::*;

#[test]
fn kernel_direct_object_projection_executes_with_exact_native_authority() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = "Thing.select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn kernel_single_file_filecode_equality_executes_with_exact_native_authority() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let query = r#"Person.where(name == "red").select(name)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
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
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON row output");
    };
    assert_eq!(rows, &vec![json!({"name": "red"}), json!({"name": "red"})]);
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );

    let operator_contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    assert!(operator_contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["representation_class"] == json!("code_pure")
            && contract["exact"] == json!(true)
            && contract["residual_required"] == json!(false)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_FILE_CODE_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["dataset_file_count"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("single-file FileCode literal predicate")
            && decision.safe_details["residual_verification"] == json!(false)
            && decision.safe_details["bridge_required"] == json!(false)
    }));
}

#[test]
fn kernel_direct_object_scalar_projection_executes_coded_safe_functions_with_exact_native_authority(
) {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), None],
        &["identity", "length", "startsWith"],
    );
    let query = r#"Person.select(name_len: length(name), starts_a: startsWith(name, "A"), id_name: identity(name))"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["residual_verification_required"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));

    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON row output");
    };
    assert_eq!(
        rows,
        &vec![
            json!({
                "name_len": 3,
                "starts_a": true,
                "id_name": "Ada"
            }),
            json!({
                "name_len": null,
                "starts_a": null,
                "id_name": null
            })
        ]
    );

    let operator_contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    for function_id in ["length", "startsWith", "identity"] {
        assert!(operator_contracts.iter().any(|contract| {
            contract["operator"] == format!("function:{function_id}")
                && contract["representation_class"] == json!("dictionary_lifted")
                && contract["exact"] == true
                && contract["residual_required"] == false
        }));
    }
}

#[test]
fn kernel_direct_object_string_scalar_projection_executes_registered_transforms_with_exact_native_authority(
) {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some(" Ada "), Some("Åsa"), None],
        &["lower", "upper", "trim"],
    );
    let query =
        "Person.select(lower_name: lower(name), upper_name: upper(name), trimmed: trim(name))";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["residual_verification_required"],
        json!(false)
    );

    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON row output");
    };
    assert_eq!(
        rows,
        &vec![
            json!({
                "lower_name": " ada ",
                "upper_name": " ADA ",
                "trimmed": "Ada"
            }),
            json!({
                "lower_name": "åsa",
                "upper_name": "ÅSA",
                "trimmed": "Åsa"
            }),
            json!({
                "lower_name": null,
                "upper_name": null,
                "trimmed": null
            })
        ]
    );

    let operator_contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    for function_id in ["lower", "upper", "trim"] {
        assert!(operator_contracts.iter().any(|contract| {
            contract["operator"] == format!("function:{function_id}")
                && contract["representation_class"] == json!("dictionary_lifted")
                && contract["exact"] == true
                && contract["residual_required"] == false
        }));
    }
}

#[test]
fn kernel_direct_object_bool_scalar_projection_executes_null_coalesce_and_cast_with_exact_native_authority(
) {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = r#"Thing.select(present: active.isNotNull(), absent: isNull(active), coalesced: coalesce(active, false), active_cast: cast(active, "bool"))"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["residual_verification_required"],
        json!(false)
    );

    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON row output");
    };
    assert_eq!(
        rows,
        &vec![
            json!({
                "present": true,
                "absent": false,
                "coalesced": true,
                "active_cast": true
            }),
            json!({
                "present": false,
                "absent": true,
                "coalesced": false,
                "active_cast": null
            }),
            json!({
                "present": true,
                "absent": false,
                "coalesced": false,
                "active_cast": false
            })
        ]
    );

    let operator_contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    for function_id in ["isNotNull", "isNull", "coalesce", "cast"] {
        assert!(operator_contracts.iter().any(|contract| {
            contract["operator"] == format!("function:{function_id}")
                && contract["exact"] == true
                && contract["residual_required"] == false
        }));
    }
}

#[test]
fn kernel_direct_object_scalar_projection_can_return_arrow_batches_with_exact_native_authority() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), None],
        &["length", "startsWith"],
    );
    let query = r#"Person.select(name_len: length(name), starts_a: startsWith(name, "A"))"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..json_resolve_options()
        },
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
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..json_resolve_options()
        },
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);

    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.schema().field(0).name(), "name_len");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
    assert_eq!(batch.schema().field(1).name(), "starts_a");
    assert_eq!(batch.schema().field(1).data_type(), &DataType::Boolean);
    let lengths = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(lengths.value(0), 3);
    assert!(lengths.is_null(1));
    let starts = batch
        .column(1)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(starts.value(0));
    assert!(starts.is_null(1));
}

#[test]
fn kernel_direct_object_projection_applies_default_order_before_skip_take() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.select(active).skip(1).take(1)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["residual_verification_required"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn kernel_auto_in_list_matches_materialized_output_without_compare_mode() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active in [true]).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
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
    assert!(!kernel.kernel_report.compared_with_materialized);
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert!(kernel.kernel_report.optimization_authority.authoritative);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
}

#[test]
fn kernel_native_scalar_bool_in_single_value_uses_bool_lane() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active in [true]).select(active)";
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
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["bool_eq"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_bool_not_in_single_value_uses_bool_lane() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(!(active in [true])).select(active)";
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
    assert_eq!(rows, vec![json!({"active": false})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["bool_eq"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_bool_not_equal_uses_bool_lane() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active != true).select(active)";
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
    assert_eq!(rows, vec![json!({"active": false})]);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"
            && diagnostic.safe_details["matched_rows"] == json!(1)
            && diagnostic.safe_details["predicate_order"] == json!(["bool_eq"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 1);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_native_scalar_bool_in_all_values_uses_validity_lane() {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = "Thing.where(active in [true, false]).select(active)";
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
            && diagnostic.safe_details["matched_rows"] == json!(2)
            && diagnostic.safe_details["predicate_order"] == json!(["null_check"])
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert_eq!(kernel.kernel_report.counters.rows_after_bitmap, 2);
    assert_eq!(kernel.kernel_report.counters.residual_rows_checked, 0);
}

#[test]
fn kernel_not_in_list_with_null_literal_keeps_residual_three_valued_logic() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = "Thing.where(!(active in [true, null])).select(active)";
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
    assert!(rows.is_empty());
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_NATIVE_SCALAR_LITERAL_PRUNE_EXECUTED"));
}

#[test]
fn kernel_starts_with_predicate_matches_materialized_output() {
    let (bytes, _) = object_file_with_filecode_records_and_function_registry(
        &["Ada", "Bob", "Ava"],
        &["startsWith"],
    );
    let query = r#"Person.where(startsWith(name, "A")).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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

    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Ada"}), json!({"name": "Ava"})]);
    assert!(kernel.kernel_report.compared_with_materialized);
}

#[test]
fn kernel_starts_with_compare_predicates_match_materialized_output() {
    let (bytes, _) = object_file_with_filecode_records_and_function_registry(
        &["Ada", "Bob", "Ava"],
        &["startsWith"],
    );
    for (query, expected) in [
        (
            r#"Person.where(startsWith(name, "A") == true).select(name)"#,
            vec![json!({"name": "Ada"}), json!({"name": "Ava"})],
        ),
        (
            r#"Person.where(startsWith(name, "A") != true).select(name)"#,
            vec![json!({"name": "Bob"})],
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
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
            &bytes,
            query,
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
        let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, &expected);
        assert_eq!(
            kernel.kernel_report.decision.kind,
            KernelDecisionKind::Applied
        );
        assert!(kernel.kernel_report.compared_with_materialized);

        let explain = kernel.explain_json();
        let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
            .as_array()
            .unwrap();
        assert!(contracts.iter().any(|contract| {
            contract["operator"] == "predicate_bool_compare"
                && contract["representation_class"] == "code_pure"
                && contract["exact"] == true
                && contract["residual_required"] == false
        }));
    }
}

#[test]
fn kernel_length_compare_predicate_matches_materialized_output() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), Some("Åsa"), Some("Bo"), None],
        &["length"],
    );
    let query = "Person.where(length(name) == 3).select(name)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"name": "Ada"}), json!({"name": "Åsa"})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let explain = kernel.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:length"
            && contract["representation_class"] == "dictionary_lifted"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_registered_string_scalar_compare_predicates_match_materialized_output() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), Some(" bob "), Some("ÅSA"), None],
        &["lower", "upper", "trim"],
    );
    for (query, expected, operator) in [
        (
            r#"Person.where(lower(name) == "åsa").select(name)"#,
            vec![json!({"name": "ÅSA"})],
            "predicate_function:lower",
        ),
        (
            r#"Person.where(trim(name) == "bob").select(name)"#,
            vec![json!({"name": " bob "})],
            "predicate_function:trim",
        ),
        (
            r#"Person.where(upper(name) != "ADA").select(name)"#,
            vec![json!({"name": " bob "}), json!({"name": "ÅSA"})],
            "predicate_function:upper",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
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
            &bytes,
            query,
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
        let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, &expected);
        assert_eq!(
            kernel.kernel_report.decision.kind,
            KernelDecisionKind::Applied
        );
        assert!(kernel.kernel_report.compared_with_materialized);

        let explain = kernel.explain_json();
        let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
            .as_array()
            .unwrap();
        assert!(contracts.iter().any(|contract| {
            contract["operator"] == operator
                && contract["representation_class"] == "dictionary_lifted"
                && contract["exact"] == true
                && contract["residual_required"] == false
        }));
    }
}

#[test]
fn kernel_numeric_range_predicates_match_materialized_output() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let query = "MetricThing.where(metric >= 10 && metric < 30).select(metric)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"metric": 10}), json!({"metric": 20})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);
}

#[test]
fn multiple_where_methods_match_single_conjunction() {
    let bytes = object_file_with_numcode_records(&[5, 10, 20, 30]);
    let chained = parse_resolve_plan_and_execute_query(
        &bytes,
        "MetricThing.where(metric >= 10).where(metric < 30).select(metric)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let conjunction = parse_resolve_plan_and_execute_query(
        &bytes,
        "MetricThing.where(metric >= 10 && metric < 30).select(metric)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(chained.output_fingerprint, conjunction.output_fingerprint);
    let CoveQlExecutionResult::JsonRows(rows) = chained.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"metric": 10}), json!({"metric": 20})]);
}

#[test]
fn kernel_null_check_predicate_matches_materialized_output() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = "Person.where(name.isNull()).select(name)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"name": null})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);
}

#[test]
fn kernel_function_null_check_predicate_matches_materialized_output() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = "Person.where(isNull(name)).select(name)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"name": null})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let explain = kernel.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_null_check"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
    let decode_boundaries = explain["execution"]["coded_execution"]["decode_boundaries"]
        .as_array()
        .unwrap();
    assert!(decode_boundaries.iter().all(|boundary| !boundary
        .as_str()
        .unwrap()
        .contains("predicate_null_or_bool")));
}

#[test]
fn kernel_bool_path_predicate_matches_materialized_output() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.where(active).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true}), json!({"active": true})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);
}

#[test]
fn kernel_identity_bool_predicate_matches_materialized_output() {
    let bytes =
        object_file_with_bool_records_and_function_registry(&[true, false, true], &["identity"]);
    let query = "Thing.where(identity(active)).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        &vec![json!({"active": true}), json!({"active": true})]
    );
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let explain = kernel.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:identity"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_coalesce_bool_predicate_matches_materialized_output() {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = "Thing.where(coalesce(active, true)).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        &vec![json!({"active": true}), json!({"active": null})]
    );
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let explain = kernel.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:coalesce"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_coalesce_bool_compare_predicate_preserves_unknown_nulls() {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = "Thing.where(coalesce(active, null) == false).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"active": false})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let explain = kernel.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_bool_compare"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_identity_safe_cast_compare_matches_materialized_output() {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = r#"Thing.where(cast(active, "bool") == true).select(active)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"active": true})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);

    let contracts = kernel.explain_json()["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap()
        .clone();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:cast"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_identity_safe_cast_bool_predicate_matches_materialized_output() {
    let bytes = object_file_with_nullable_bool_records(&[Some(true), None, Some(false)]);
    let query = r#"Thing.where(cast(active, "bool")).select(active)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(ref rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"active": true})]);
    assert!(kernel.kernel_report.compared_with_materialized);

    let contracts = kernel.explain_json()["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap()
        .clone();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:cast"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_path_literal_type_mismatch_falls_back_for_not_equal() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = r#"Thing.where(active != "true").select(active)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"active": true}), json!({"active": false})]
    );
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Fallback
    );
    assert_eq!(
        kernel.kernel_report.fallback_reason,
        Some(KernelFallbackReason::UnsafeCodedPredicate)
    );

    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("fallback explain includes coded operator contracts");
    let compare_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "predicate_compare")
        .expect("predicate compare contract is present");
    assert_eq!(compare_contract["exact"], false);
    assert_eq!(compare_contract["residual_required"], true);
    assert!(compare_contract["reason"]
        .as_str()
        .unwrap()
        .contains("compatible literal type"));
}

#[test]
fn kernel_identity_cast_literal_type_mismatch_falls_back_for_not_equal() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = r#"Thing.where(cast(active, "bool") != "true").select(active)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
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
        KernelDecisionKind::Fallback
    );
    assert_eq!(
        kernel.kernel_report.fallback_reason,
        Some(KernelFallbackReason::UnsafeCodedPredicate)
    );

    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("fallback explain includes coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["representation_class"] == "code_pure"
            && contract["exact"] == false
            && contract["residual_required"] == true
    }));
    assert!(!contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:cast"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn kernel_in_list_literal_type_mismatch_falls_back_under_negation() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = r#"Thing.where(!(active in ["true"])).select(active)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"active": true}), json!({"active": false})]
    );
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Fallback
    );
    assert_eq!(
        kernel.kernel_report.fallback_reason,
        Some(KernelFallbackReason::UnsafeCodedPredicate)
    );

    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("fallback explain includes coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_in"
            && contract["exact"] == false
            && contract["residual_required"] == true
            && contract["reason"]
                .as_str()
                .unwrap()
                .contains("path value domain")
    }));
}

#[test]
fn kernel_not_predicate_preserves_three_valued_logic_for_nulls() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(!(name == "Ada")).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Bob"})]);
    assert!(kernel.kernel_report.compared_with_materialized);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_not"
            && contract["exact"] == json!(true)
            && contract["residual_required"] == json!(false)
    }));
}

#[test]
fn kernel_or_predicate_preserves_three_valued_logic_for_nulls() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(name == "Ada" || name.isNull()).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Ada"}), json!({"name": null})]);
    assert!(kernel.kernel_report.compared_with_materialized);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_or"
            && contract["exact"] == json!(true)
            && contract["residual_required"] == json!(false)
    }));
}

#[test]
fn kernel_in_predicate_preserves_null_membership_semantics() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None, Some("Bob")]);
    let query = r#"Person.where(name in ["Ada", null]).select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
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
        &bytes,
        query,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "Ada"})]);
    assert!(kernel.kernel_report.compared_with_materialized);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_in"
            && contract["exact"] == json!(true)
            && contract["residual_required"] == json!(false)
    }));
}
