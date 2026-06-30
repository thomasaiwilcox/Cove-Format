use super::*;

#[test]
fn kernel_force_mode_executes_aggregate_shape_with_residual_contract() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.select(n: count(*))";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"n": 3})]);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel aggregate reports coded operator contracts");
    let aggregate_contract = contracts
        .iter()
        .find(|contract| contract["operator"].as_str().unwrap().contains("aggregate"))
        .expect("aggregate operator contract is present");
    assert_eq!(aggregate_contract["residual_required"], true);
    assert_eq!(
        aggregate_contract["representation_class"],
        json!("materialized_residual")
    );
    assert_eq!(
        aggregate_contract["row_grain"],
        json!("groups_over_reconstructed_visible_rows")
    );
    assert_eq!(
        aggregate_contract["fallback_boundary"],
        json!("materialized_aggregate_evaluation")
    );
    assert!(aggregate_contract["proof_obligation"]
        .as_str()
        .unwrap()
        .contains("raw temporal records"));
    assert!(aggregate_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "state_grain_contract"));
}

#[test]
fn kernel_force_mode_executes_direct_aggregates_with_exact_native_contract() {
    let bytes =
        object_file_with_nullable_bool_records(&[Some(true), None, Some(false), Some(true)]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected, aggregate, counted_path, strategy) in [
        (
            "Thing.select(n: count(*))",
            json!({"n": 4}),
            "count",
            None,
            "row_count",
        ),
        (
            "Thing.select(n: count(active))",
            json!({"n": 3}),
            "count",
            Some("active"),
            "single_pass_value_ref",
        ),
        (
            "Thing.select(e: exists(active))",
            json!({"e": true}),
            "exists",
            Some("active"),
            "single_pass_value_ref",
        ),
        (
            "Thing.select(d: distinct_count(active))",
            json!({"d": 2}),
            "distinct_count",
            Some("active"),
            "single_pass_value_ref",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        assert_eq!(rows, vec![expected]);
        assert!(!kernel.executed.authority.residual_required);
        assert_eq!(
            serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
            json!("authoritative")
        );
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel aggregate reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native direct aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("code_pure")
        );
        assert_eq!(
            kernel.kernel_report.decision.safe_details["residual_verification"],
            json!(false)
        );
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["counted_path"]
                    == counted_path.map_or(serde_json::Value::Null, |path| json!(path))
                && diagnostic.safe_details["aggregate_strategy"] == json!(strategy)
        }));
        assert!(kernel.kernel_report.decisions.iter().any(|decision| {
            decision.reason.contains("native direct aggregate")
                && decision.safe_details["aggregate"] == json!(aggregate)
                && decision.safe_details["aggregate_strategy"] == json!(strategy)
        }));
    }
}

#[test]
fn kernel_force_mode_executes_single_file_filecode_direct_aggregates_with_exact_native_contract() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected, aggregate) in [
        ("Person.select(n: count(name))", json!({"n": 4}), "count"),
        (
            "Person.select(e: exists(name))",
            json!({"e": true}),
            "exists",
        ),
        (
            "Person.select(d: distinct_count(name))",
            json!({"d": 3}),
            "distinct_count",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        assert_eq!(rows, vec![expected]);
        assert!(!kernel.executed.authority.residual_required);
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel FileCode aggregate reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native direct aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("code_pure")
        );
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["counted_path"] == json!("name")
        }));
    }
}

#[test]
fn kernel_force_mode_executes_min_max_numcode_aggregates_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 30, 10]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected, aggregate) in [
        ("MetricThing.select(m: min(metric))", json!({"m": 5}), "min"),
        (
            "MetricThing.select(m: max(metric))",
            json!({"m": 30}),
            "max",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        assert_eq!(rows, vec![expected]);
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel min/max reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native min/max aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("typed_numeric_coded")
        );
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["counted_path"] == json!("metric")
                && diagnostic.safe_details["aggregate_strategy"] == json!("single_pass_typed_order")
        }));
    }
}

#[test]
fn kernel_force_mode_executes_sum_avg_numcode_aggregates_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 30, 10]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected, aggregate) in [
        (
            "MetricThing.select(s: sum(metric))",
            json!({"s": 65}),
            "sum",
        ),
        (
            "MetricThing.select(a: avg(metric))",
            json!({"a": "16.25"}),
            "avg",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        assert_eq!(rows, vec![expected]);
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel sum/avg reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native sum/avg aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("typed_numeric_coded")
        );
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["counted_path"] == json!("metric")
                && diagnostic.safe_details["aggregate_strategy"]
                    == json!("single_pass_typed_numeric")
        }));
    }
}

#[test]
fn kernel_force_mode_native_sum_avg_preserve_large_integer_precision() {
    let bytes = object_file_with_numcode_records(&[i64::MAX, 1]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected, aggregate) in [
        (
            "MetricThing.select(total: sum(metric))",
            json!({"total": 9_223_372_036_854_775_808_u64}),
            "sum",
        ),
        (
            "MetricThing.select(average: avg(metric))",
            json!({"average": 4_611_686_018_427_387_904_i64}),
            "avg",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        assert_eq!(rows, vec![expected]);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["counted_path"] == json!("metric")
        }));
    }
}

#[test]
fn kernel_force_mode_executes_group_by_shape_with_residual_contract() {
    let bytes = object_file_with_bool_records(&[true, false, true, false]);
    let query = "Thing.groupBy(active).select(active, n: count(*))";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"active": false, "n": 2}),
            json!({"active": true, "n": 2})
        ]
    );
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel groupBy reports coded operator contracts");
    let group_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "group_by")
        .expect("group_by operator contract is present");
    assert_eq!(group_contract["residual_required"], true);
    assert_eq!(
        group_contract["representation_class"],
        json!("decode_boundary")
    );
    assert!(group_contract["proof_obligation"]
        .as_str()
        .unwrap()
        .contains("reconstructed row grain"));
    assert!(group_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "materialized_value"));
    assert!(contracts.iter().any(|contract| {
        contract["operator"].as_str().unwrap().contains("aggregate")
            && contract["representation_class"] == json!("materialized_residual")
    }));
}

#[test]
fn kernel_force_mode_executes_bool_group_count_with_exact_native_contract() {
    let bytes = object_file_with_bool_records(&[true, false, true, false]);
    let query = "Thing.groupBy(active).select(active, n: count(*))";
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        serde_json::to_value(kernel.executed.authority.source).unwrap(),
        json!("exact_optimized_kernel")
    );
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"active": false, "n": 2}),
            json!({"active": true, "n": 2})
        ]
    );
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel groupBy reports coded operator contracts");
    let group_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "group_by")
        .expect("group_by operator contract is present");
    assert_eq!(group_contract["residual_required"], false);
    assert_eq!(group_contract["exact"], true);
    assert_eq!(group_contract["representation_class"], json!("code_pure"));
    assert_eq!(
        group_contract["row_grain"],
        json!("groups_over_reconstructed_visible_object_states")
    );
    let count_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "aggregate:count")
        .expect("native count aggregate contract is present");
    assert_eq!(count_contract["residual_required"], false);
    assert_eq!(count_contract["exact"], true);
    assert_eq!(count_contract["fallback_boundary"], serde_json::Value::Null);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(
            |diagnostic| diagnostic.code == "W_KERNEL_NATIVE_BOOL_GROUP_COUNT_EXECUTED"
                && diagnostic.safe_details["group_strategy"] == json!("dense_bool_star")
                && diagnostic.safe_details["aggregate_strategy"] == json!("row_count")
        ));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("native direct grouped aggregate")
            && decision.safe_details["group_strategy"] == json!("dense_bool_star")
            && decision.safe_details["aggregate_strategy"] == json!("row_count")
    }));
}

#[test]
fn kernel_force_mode_dense_bool_group_count_preserves_null_group_order() {
    let bytes =
        object_file_with_nullable_bool_records(&[Some(false), None, Some(true), Some(true)]);
    let query = "Thing.groupBy(active).select(active, n: count(*))";
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"active": false, "n": 1}),
            json!({"active": null, "n": 1}),
            json!({"active": true, "n": 2})
        ]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_BOOL_GROUP_COUNT_EXECUTED"
            && diagnostic.safe_details["group_strategy"] == json!("dense_bool_star")
            && diagnostic.safe_details["aggregate_strategy"] == json!("row_count")
            && diagnostic.safe_details["group_count"] == json!(3)
            && diagnostic.safe_details["rows_counted"] == json!(4)
            && diagnostic.safe_details["values_seen"] == json!(4)
    }));
}

#[test]
fn kernel_force_mode_executes_numcode_group_count_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 20, 10]);
    let query = "MetricThing.groupBy(metric).select(metric, n: count(*))";
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert!(!kernel.executed.authority.residual_required);
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"metric": 10, "n": 1}),
            json!({"metric": 20, "n": 2}),
            json!({"metric": 5, "n": 1})
        ]
    );
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel numcode groupBy reports coded operator contracts");
    let group_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "group_by")
        .expect("group_by operator contract is present");
    assert_eq!(group_contract["residual_required"], false);
    assert_eq!(group_contract["exact"], true);
    assert_eq!(
        group_contract["representation_class"],
        json!("typed_numeric_coded")
    );
    let count_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "aggregate:count")
        .expect("native count aggregate contract is present");
    assert_eq!(count_contract["residual_required"], false);
    assert_eq!(count_contract["exact"], true);
    assert_eq!(count_contract["representation_class"], json!("code_pure"));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED"
            && diagnostic.safe_details["aggregate"] == json!("count")
            && diagnostic.safe_details["group_property"] == json!("metric")
            && diagnostic.safe_details["logical_type"] == json!("int64")
            && diagnostic.safe_details["group_strategy"] == json!("row_index_single_pass")
            && diagnostic.safe_details["aggregate_strategy"] == json!("row_count")
    }));
}

#[test]
fn kernel_force_mode_executes_single_file_filecode_grouped_aggregates_with_exact_native_contract() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected_rows, aggregate, representation_class, aggregate_strategy) in [
        (
            "Person.groupBy(name).select(name, n: count(*))",
            vec![
                json!({"name": "blue", "n": 1}),
                json!({"name": "green", "n": 1}),
                json!({"name": "red", "n": 2}),
            ],
            "count",
            "code_pure",
            "row_count",
        ),
        (
            "Person.groupBy(name).select(name, d: distinct_count(name))",
            vec![
                json!({"name": "blue", "d": 1}),
                json!({"name": "green", "d": 1}),
                json!({"name": "red", "d": 1}),
            ],
            "distinct_count",
            "code_pure",
            "single_pass_value_ref",
        ),
        (
            "Person.groupBy(name).select(name, e: exists(name))",
            vec![
                json!({"name": "blue", "e": true}),
                json!({"name": "green", "e": true}),
                json!({"name": "red", "e": true}),
            ],
            "exists",
            "code_pure",
            "single_pass_value_ref",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, expected_rows);
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel FileCode groupBy reports coded operator contracts");
        let group_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == "group_by")
            .expect("group_by operator contract is present");
        assert_eq!(group_contract["residual_required"], false);
        assert_eq!(group_contract["exact"], true);
        assert_eq!(group_contract["representation_class"], json!("code_pure"));
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native grouped aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!(representation_class)
        );
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["group_property"] == json!("name")
                && diagnostic.safe_details["logical_type"] == json!("utf8")
                && diagnostic.safe_details["group_strategy"] == json!("row_index_single_pass")
                && diagnostic.safe_details["aggregate_strategy"] == json!(aggregate_strategy)
        }));
    }
}

#[test]
fn kernel_force_mode_executes_grouped_numcode_direct_aggregates_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 20, 10]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected_rows, aggregate, representation_class, aggregate_strategy) in [
        (
            "MetricThing.groupBy(metric).select(metric, n: count(metric))",
            vec![
                json!({"metric": 10, "n": 1}),
                json!({"metric": 20, "n": 2}),
                json!({"metric": 5, "n": 1}),
            ],
            "count",
            "code_pure",
            "single_pass_value_ref",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, e: exists(metric))",
            vec![
                json!({"metric": 10, "e": true}),
                json!({"metric": 20, "e": true}),
                json!({"metric": 5, "e": true}),
            ],
            "exists",
            "code_pure",
            "single_pass_value_ref",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, d: distinct_count(metric))",
            vec![
                json!({"metric": 10, "d": 1}),
                json!({"metric": 20, "d": 1}),
                json!({"metric": 5, "d": 1}),
            ],
            "distinct_count",
            "typed_numeric_coded",
            "single_pass_value_ref",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, m: min(metric))",
            vec![
                json!({"metric": 10, "m": 10}),
                json!({"metric": 20, "m": 20}),
                json!({"metric": 5, "m": 5}),
            ],
            "min",
            "typed_numeric_coded",
            "single_pass_typed_order",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, m: max(metric))",
            vec![
                json!({"metric": 10, "m": 10}),
                json!({"metric": 20, "m": 20}),
                json!({"metric": 5, "m": 5}),
            ],
            "max",
            "typed_numeric_coded",
            "single_pass_typed_order",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, s: sum(metric))",
            vec![
                json!({"metric": 10, "s": 10}),
                json!({"metric": 20, "s": 40}),
                json!({"metric": 5, "s": 5}),
            ],
            "sum",
            "typed_numeric_coded",
            "single_pass_typed_numeric",
        ),
        (
            "MetricThing.groupBy(metric).select(metric, a: avg(metric))",
            vec![
                json!({"metric": 10, "a": 10}),
                json!({"metric": 20, "a": 20}),
                json!({"metric": 5, "a": 5}),
            ],
            "avg",
            "typed_numeric_coded",
            "single_pass_typed_numeric",
        ),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();
        let materialized = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
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
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, expected_rows);
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("force-kernel grouped aggregate reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native grouped aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], false);
        assert_eq!(aggregate_contract["exact"], true);
        assert_eq!(
            aggregate_contract["representation_class"],
            json!(representation_class)
        );
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["group_property"] == json!("metric")
                && diagnostic.safe_details["logical_type"] == json!("int64")
                && diagnostic.safe_details["group_strategy"] == json!("row_index_single_pass")
                && diagnostic.safe_details["aggregate_strategy"] == json!(aggregate_strategy)
        }));
    }
}

#[test]
fn kernel_force_mode_executes_bool_order_by_with_exact_native_contract() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = "Thing.select(active).orderBy(active, desc)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"active": true}),
            json!({"active": true}),
            json!({"active": false})
        ]
    );
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel orderBy reports coded operator contracts");
    let order_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "order_by")
        .expect("explicit order_by contract is present");
    assert_eq!(order_contract["residual_required"], false);
    assert_eq!(order_contract["exact"], true);
    assert_eq!(order_contract["representation_class"], json!("code_pure"));
    assert_eq!(order_contract["fallback_boundary"], serde_json::Value::Null);
    assert!(order_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "typed_value_lane"));
    assert!(order_contract["proof_obligation"]
        .as_str()
        .unwrap()
        .contains("ORDER BY"));
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_KERNEL_NATIVE_TYPED_ORDER_EXECUTED"));
}

#[test]
fn kernel_force_mode_executes_numcode_order_by_and_pagination_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 30, 10]);
    let query = "MetricThing.select(metric).orderBy(metric, asc).skip(1).take(2)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel numeric orderBy reports coded operator contracts");
    let order_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "order_by")
        .expect("explicit order_by contract is present");
    assert_eq!(order_contract["residual_required"], false);
    assert_eq!(order_contract["exact"], true);
    assert_eq!(
        order_contract["representation_class"],
        json!("typed_numeric_coded")
    );
    let limit_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "limit_offset")
        .expect("limit/offset contract is present");
    assert_eq!(limit_contract["residual_required"], false);
    assert_eq!(limit_contract["exact"], true);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_KERNEL_NATIVE_TYPED_ORDER_EXECUTED"));
}

#[test]
fn kernel_force_mode_executes_filecode_string_order_by_with_default_collation_boundary() {
    let (bytes, _) = object_file_with_filecode_records(&["Zoë", "Ada", "Bob"]);
    let query = "Person.select(name).orderBy(name, asc)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        rows,
        vec![
            json!({"name": "Ada"}),
            json!({"name": "Bob"}),
            json!({"name": "Zoë"})
        ]
    );
    assert!(!kernel.executed.authority.residual_required);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel FileCode string orderBy reports coded operator contracts");
    let order_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "order_by")
        .expect("explicit order_by contract is present");
    assert_eq!(order_contract["residual_required"], false);
    assert_eq!(order_contract["exact"], true);
    assert_eq!(
        order_contract["representation_class"],
        json!("decode_boundary")
    );
    assert!(order_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "materialized_sort_key"));
    assert!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| boundary
                .as_str()
                .is_some_and(|boundary| boundary.contains("decoded value sort key")))
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_TYPED_ORDER_EXECUTED"
            && diagnostic.safe_details["collation_id"] == json!(CollationKind::None.id())
            && diagnostic.safe_details["sort_key_boundary"]
                == json!("decoded_filecode_sort_key_under_default_collation")
    }));
}

#[test]
fn kernel_force_mode_executes_filecode_string_order_by_with_declared_collation_boundary() {
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Zoë", "Ada", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let query = "Person.select(name).orderBy(name, asc)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(
        rows,
        vec![
            json!({"name": "Ada"}),
            json!({"name": "Bob"}),
            json!({"name": "Zoë"})
        ]
    );
    assert!(!kernel.executed.authority.residual_required);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel FileCode string orderBy reports coded operator contracts");
    let order_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "order_by")
        .expect("explicit order_by contract is present");
    assert_eq!(order_contract["residual_required"], false);
    assert_eq!(order_contract["exact"], true);
    assert_eq!(
        order_contract["representation_class"],
        json!("decode_boundary")
    );
    assert!(order_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "materialized_sort_key"));
    assert!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| boundary
                .as_str()
                .is_some_and(|boundary| boundary.contains("decoded value sort key")))
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_TYPED_ORDER_EXECUTED"
            && diagnostic.safe_details["collation_id"] == json!(CollationKind::Utf8Bytewise.id())
            && diagnostic.safe_details["sort_key_boundary"]
                == json!("decoded_filecode_sort_key_under_declared_collation")
    }));
}

#[test]
fn kernel_force_mode_executes_filecode_string_range_predicate_with_default_collation_boundary() {
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Nia", "Bob"]);
    let query = r#"Person.where(name < "M").select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(rows, vec![json!({"name": "Ada"}), json!({"name": "Bob"})]);
    assert!(!kernel.executed.authority.residual_required);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel FileCode range predicate reports coded operator contracts");
    let compare_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "predicate_compare")
        .expect("predicate_compare contract is present");
    assert_eq!(compare_contract["exact"], true);
    assert_eq!(compare_contract["residual_required"], false);
    assert_eq!(
        compare_contract["representation_class"],
        json!("decode_boundary")
    );
    assert!(compare_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "materialized_compare_key"));
    assert!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| boundary
                .as_str()
                .is_some_and(|boundary| boundary.contains("ordered FileCode comparison")))
    );
}

#[test]
fn kernel_force_mode_executes_filecode_string_range_predicate_with_declared_collation_boundary() {
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Ada", "Nia", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let query = r#"Person.where(name < "M").select(name)"#;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
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
    assert_eq!(rows, vec![json!({"name": "Ada"}), json!({"name": "Bob"})]);
    assert!(!kernel.executed.authority.residual_required);
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("force-kernel FileCode range predicate reports coded operator contracts");
    let compare_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "predicate_compare")
        .expect("predicate_compare contract is present");
    assert_eq!(compare_contract["exact"], true);
    assert_eq!(compare_contract["residual_required"], false);
    assert_eq!(
        compare_contract["representation_class"],
        json!("decode_boundary")
    );
    assert!(compare_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "materialized_compare_key"));
    assert!(
        kernel.kernel_report.decision.safe_details["kernel_shape"]["decode_boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| boundary
                .as_str()
                .is_some_and(|boundary| boundary.contains("ordered FileCode comparison")))
    );
}
