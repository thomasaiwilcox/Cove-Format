use super::*;

#[test]
fn covi_row_range_prune_matches_materialized_kernel_output() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = "Thing.where(active == true).select(active)";
    let covi = object_property_bool_lookup_covi(&bytes, "Thing", "active", true);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions {
            sidecars: PhysicalSidecarInputs {
                covi_artifact_bytes: Some(covi),
                ..PhysicalSidecarInputs::default()
            },
            ..PhysicalPlanOptions::default()
        },
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
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.authority.compared_with_materialized);
    assert_eq!(
        serde_json::to_value(kernel.executed.authority.source).unwrap(),
        json!("exact_optimized_kernel")
    );
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_COVI_ROW_RANGE_PRUNE_EXECUTED"));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("row ranges")
            && decision.safe_details["residual_verification"] == json!(true)
    }));
}

#[test]
fn coverage_row_range_prune_matches_materialized_kernel_output() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let query = "Thing.where(active == true).select(active)";
    let (coverage_set, proof_records) =
        object_property_bool_coverage(&bytes, "Thing", "active", true, 0);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions {
            sidecars: PhysicalSidecarInputs {
                coverage_set_bytes: Some(coverage_set),
                coverage_proof_record_bytes: Some(proof_records),
                ..PhysicalSidecarInputs::default()
            },
            ..PhysicalPlanOptions::default()
        },
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.authority.compared_with_materialized);
    assert_eq!(
        serde_json::to_value(kernel.executed.authority.source).unwrap(),
        json!("exact_optimized_kernel")
    );
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_COVERAGE_ROW_RANGE_PRUNE_EXECUTED"));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("COVE-COVERAGE row ranges")
            && decision.safe_details["residual_verification"] == json!(true)
    }));
}

#[test]
fn execution_code_filecode_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) =
        object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let query = r#"Person.where(name == "red").select(name)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["name"] == json!("red")));
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("execution-code remap was used")
            && decision.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn execution_code_filecode_bool_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) =
        object_file_with_bool_filecode_records(&[true, false, true, false]);
    let query = r#"FlagThing.where(active == true).select(active)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let predicate_debug = format!(
        "{:#?}",
        kernel
            .physical
            .planned
            .resolved
            .method_chain
            .where_predicate
    );
    let execution_domains_debug = format!(
        "{:#?}",
        kernel.physical.physical_plan.execution_code_domains
    );
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
        panic!("expected json rows");
    };
    assert_eq!(rows, vec![json!({"active": true}), json!({"active": true})]);
    assert!(kernel.kernel_report.compared_with_materialized);
    let remap_diagnostic = kernel
        .executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED");
    assert_eq!(
        remap_diagnostic.map(|diagnostic| diagnostic.safe_details["matched_rows"].clone()),
        Some(json!(2)),
        "{}\n{}\n{:?}",
        predicate_debug,
        execution_domains_debug,
        kernel.executed.diagnostics
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn execution_code_filecode_int64_in_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) = object_file_with_int64_filecode_records(&[-5, 7, -5, 12]);
    let query = r#"MetricThing.where(metric in [-5, 12]).select(metric)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"metric": -5}),
            json!({"metric": -5}),
            json!({"metric": 12})
        ]
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["matched_rows"] == json!(3)
    }));
}

#[test]
fn execution_code_filecode_uuid_prune_matches_materialized_kernel_output() {
    let first = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let second = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let (bytes, execution_map) = object_file_with_uuid_filecode_records(&[first, second, first]);
    let query = r#"UuidThing.where(uid == uuid"00000000-0000-0000-0000-000000000001").select(uid)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row["uid"]
            .as_str()
            .is_some_and(|value| value.ends_with("000000000001"))
    }));
    let remap_diagnostic = kernel
        .executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED");
    assert_eq!(
        remap_diagnostic.map(|diagnostic| diagnostic.safe_details["matched_rows"].clone()),
        Some(json!(2)),
        "{:#?}\n{:#?}\n{:?}",
        kernel
            .physical
            .planned
            .resolved
            .method_chain
            .where_predicate,
        kernel.physical.physical_plan.execution_code_domains,
        kernel.executed.diagnostics
    );
}

#[test]
fn execution_code_filecode_timestamp_prune_matches_materialized_kernel_output() {
    let timestamp = 1_767_225_600_000_000;
    let (bytes, execution_map) =
        object_file_with_timestamp_filecode_records(&[timestamp, timestamp + 1, timestamp]);
    let query = r#"EventThing.where(event_time == "2026-01-01T00:00:00Z").select(event_time)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["event_time"] == json!(timestamp)));
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["matched_rows"] == json!(2)
    }));
}

#[test]
fn execution_code_filecode_or_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) =
        object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let query = r#"Person.where(name == "red" || name in ["blue"]).select(name)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["red", "blue", "red"]
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(3)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("execution-code remap was used")
            && decision.safe_details["matched_rows"] == json!(3)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn execution_code_filecode_not_equal_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) =
        object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let query = r#"Person.where(name != "red").select(name)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["blue", "green"]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(2)
    }));
}

#[test]
fn execution_code_filecode_negated_or_prune_matches_materialized_kernel_output() {
    let (bytes, execution_map) =
        object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let query = r#"Person.where(!(name == "red" || name in ["blue"])).select(name)"#;
    let mut resolve_options = json_resolve_options();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let execution_options = ExecutionOptions {
        execution_code_filecode_map: execution_map,
        ..ExecutionOptions::default()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        execution_options,
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
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected json rows");
    };
    assert_eq!(
        rows.iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["green"]
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_EXECUTION_CODE_REMAP_EXECUTED"
            && diagnostic.safe_details["predicate_count"] == json!(1)
            && diagnostic.safe_details["matched_rows"] == json!(1)
    }));
}

#[test]
fn index_only_count_executes_only_after_compare_match() {
    let bytes = minimal_object_file();
    let covi = object_property_count_covi(&bytes, 0);
    let mut resolve_options = json_resolve_options();
    resolve_options.security = SecurityContext {
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        index_only_answer_permission: true,
        ..SecurityContext::default()
    };
    let physical_options = PhysicalPlanOptions::default()
        .with_index_only_answers(true)
        .with_sidecars(PhysicalSidecarInputs {
            covi_artifact_bytes: Some(covi),
            ..PhysicalSidecarInputs::default()
        });
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        "Person.select(n: count(active))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        physical_options,
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
    assert_eq!(rows, vec![json!({"n": 0})]);
    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Disabled
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.authority.compared_with_materialized);
    assert_eq!(
        serde_json::to_value(kernel.executed.authority.source).unwrap(),
        json!("exact_index_only_answer")
    );
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_INDEX_ONLY_ANSWER_EXECUTED"));
}

#[test]
fn disabled_index_only_candidate_planning_suppresses_index_only_answers() {
    let bytes = minimal_object_file();
    let covi = object_property_count_covi(&bytes, 0);
    let mut resolve_options = json_resolve_options();
    resolve_options.security = SecurityContext {
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        index_only_answer_permission: true,
        ..SecurityContext::default()
    };
    let mut physical_options = PhysicalPlanOptions::default()
        .with_index_only_answers(true)
        .with_sidecars(PhysicalSidecarInputs {
            covi_artifact_bytes: Some(covi),
            ..PhysicalSidecarInputs::default()
        });
    physical_options.candidates.enable_index_only_candidates = false;

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        "Person.select(n: count(active))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        physical_options,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows.as_slice(), [json!({"n": 0})]);
    assert_eq!(
        kernel
            .physical
            .index_capability_report
            .index_only_candidates,
        0
    );
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_INDEX_ONLY_ANSWER_EXECUTED"));
}

#[test]
fn index_only_exists_and_distinct_count_use_validated_payloads() {
    let bytes = minimal_object_file();
    let covi = object_property_index_only_covi(
        &bytes,
        0,
        &[
            CoviAggregateKindV2::Exists,
            CoviAggregateKindV2::DistinctCount,
        ],
    );
    let mut resolve_options = json_resolve_options();
    resolve_options.security = SecurityContext {
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        index_only_answer_permission: true,
        ..SecurityContext::default()
    };
    for (query, expected) in [
        ("Person.select(e: exists(active))", json!({"e": false})),
        ("Person.select(d: distinct_count(active))", json!({"d": 0})),
    ] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &bytes,
            query,
            ParseOptions::default(),
            resolve_options.clone(),
            PlanOptions::default(),
            PhysicalPlanOptions::default()
                .with_index_only_answers(true)
                .with_sidecars(PhysicalSidecarInputs {
                    covi_artifact_bytes: Some(covi.clone()),
                    ..PhysicalSidecarInputs::default()
                }),
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
        assert_eq!(rows, vec![expected]);
        assert!(kernel.kernel_report.compared_with_materialized);
        assert!(kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_INDEX_ONLY_ANSWER_EXECUTED"));
    }
}

#[test]
fn index_only_sum_and_avg_use_validated_numeric_payloads() {
    let bytes = object_file_with_numcode_records(&[20, 5, 30, 10]);
    let covi = object_metric_index_only_covi(
        &bytes,
        4,
        &[CoviAggregateKindV2::Sum, CoviAggregateKindV2::Avg],
        65,
    );
    let mut resolve_options = json_resolve_options();
    resolve_options.security = SecurityContext {
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        index_only_answer_permission: true,
        ..SecurityContext::default()
    };
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
            PhysicalPlanOptions::default()
                .with_index_only_answers(true)
                .with_sidecars(PhysicalSidecarInputs {
                    covi_artifact_bytes: Some(covi.clone()),
                    ..PhysicalSidecarInputs::default()
                }),
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
        assert_eq!(rows, vec![expected]);
        assert!(kernel.kernel_report.compared_with_materialized);
        assert_eq!(
            serde_json::to_value(kernel.executed.authority.source).unwrap(),
            json!("exact_index_only_answer")
        );
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_INDEX_ONLY_ANSWER_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
        }));
    }
}

#[test]
fn covi_empty_lookup_short_circuits_only_after_compare_match() {
    let bytes = minimal_object_file();
    let covi = object_property_count_covi(&bytes, 0);
    let mut resolve_options = json_resolve_options();
    resolve_options.security = SecurityContext {
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        index_only_answer_permission: true,
        ..SecurityContext::default()
    };
    let physical_options = PhysicalPlanOptions::default()
        .with_index_only_answers(true)
        .with_sidecars(PhysicalSidecarInputs {
            covi_artifact_bytes: Some(covi),
            ..PhysicalSidecarInputs::default()
        });
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        physical_options,
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
        "Person.where(active == true).select(active)",
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
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Disabled
    );
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_COVI_EMPTY_LOOKUP_EXECUTED"));
    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_COVI_EMPTY_LOOKUP_COMPARE_MATCHED"));
}
