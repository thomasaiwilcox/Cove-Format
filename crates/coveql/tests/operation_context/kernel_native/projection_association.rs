use super::*;

#[test]
fn kernel_auto_mode_executes_projection_roots_with_exact_provider_authority() {
    for mode in [KernelExecutionMode::Auto, KernelExecutionMode::ForceKernel] {
        let kernel = parse_resolve_plan_build_physical_and_execute_query(
            &minimal_object_with_projection_file(),
            "projection(people_projection).select(active)",
            ParseOptions::default(),
            json_resolve_options(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            ExecutionOptions::default(),
            KernelExecutionOptions {
                mode,
                ..KernelExecutionOptions::default()
            },
            validation_options(),
        )
        .unwrap();

        assert_eq!(
            kernel.kernel_report.decision.kind,
            KernelDecisionKind::Applied
        );
        assert_eq!(kernel.kernel_report.fallback_reason, None);
        assert!(kernel.kernel_report.optimization_authority.authoritative);
        assert!(
            !kernel
                .kernel_report
                .optimization_authority
                .materialized_fallback
        );
        assert_eq!(
            kernel.kernel_report.decision.safe_details["kernel_shape"]["root_kind"],
            json!("projection")
        );
        assert_eq!(
            kernel.kernel_report.decision.safe_details["residual_verification"],
            json!(false)
        );
        assert!(kernel.kernel_report.decision.safe_details["fallback_boundary"].is_null());
        assert!(!kernel.executed.authority.materialized_fallback);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_PROJECTION_READBACK_EXECUTED"
                && diagnostic.safe_details["projection_id"] == json!("people_projection")
        }));
        assert_eq!(
            kernel.executed.pushdown_report.outcome,
            PushdownOutcome::Applied
        );
        assert!(kernel
            .executed
            .pushdown_report
            .decisions
            .iter()
            .any(|decision| {
                decision.kind == PushdownDecisionKind::ProjectionColumnPrune
                    && decision.outcome == PushdownOutcome::Applied
                    && decision.safe_details["columns"] == json!(["active"])
            }));
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert!(rows.is_empty());
    }
}

#[test]
fn kernel_auto_mode_executes_projection_rows_with_exact_provider_authority() {
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.optimization_authority.authoritative);
    assert!(
        !kernel
            .kernel_report
            .optimization_authority
            .materialized_fallback
    );
    assert!(!kernel.executed.authority.materialized_fallback);
    assert!(!kernel.executed.authority.residual_required);
    let CoveQlExecutionResult::ProjectionRows(rows) = kernel.executed.result else {
        panic!("expected projection rows");
    };
    assert!(rows.is_empty());
}

#[test]
fn kernel_projection_root_pagination_keeps_explicit_materialized_boundary() {
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active).take(1)",
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

    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(
        kernel
            .kernel_report
            .optimization_authority
            .materialized_fallback
    );
    assert_eq!(
        kernel.kernel_report.decision.safe_details["fallback_boundary"],
        json!("projection_materialized_readback")
    );
    assert!(kernel.executed.authority.materialized_fallback);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_PROJECTION_MATERIALIZED_READBACK_EXECUTED"
            && diagnostic.safe_details["projection_id"] == json!("people_projection")
    }));
}

#[test]
fn kernel_projection_root_declared_order_pagination_executes_with_exact_provider_authority() {
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_with_ordered_projection_file(),
        "projection(people_projection).select(active).take(1)",
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

    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(
        !kernel
            .kernel_report
            .optimization_authority
            .materialized_fallback
    );
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel.executed.authority.authoritative);
    assert!(!kernel.executed.authority.materialized_fallback);
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_PROJECTION_READBACK_EXECUTED"
            && diagnostic.safe_details["projection_id"] == json!("people_projection")
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let limit_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "limit_offset")
        .expect("limit/offset contract is present");
    assert_eq!(limit_contract["exact"], json!(true));
    assert_eq!(limit_contract["residual_required"], json!(false));
    assert_eq!(limit_contract["fallback_boundary"], json!(null));
}

#[test]
fn kernel_association_root_reports_phase8_optimization_candidates() {
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_association_file(),
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert!(kernel.kernel_report.association.enabled);
    assert_eq!(kernel.kernel_report.association.endpoint_plans.len(), 1);
    assert_eq!(
        kernel.kernel_report.association.endpoint_plans[0].endpoint_role,
        AssociationEndpointRole::Either
    );
    assert_eq!(
        kernel.explain_json()["execution"]["kernel_report"]["association"]["endpoint_plan_count"],
        1
    );
}

#[test]
fn kernel_association_root_row_scan_executes_with_exact_native_authority() {
    let bytes = object_file_with_person_and_association_record();
    let query = "association(CustomerPlacedOrder).select(source_goid, target_goid)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        ResolveOptions::default(),
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
        ResolveOptions::default(),
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
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_ASSOCIATION_ROOT_SCAN_EXECUTED"
            && diagnostic.safe_details["rows_returned"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision.reason.contains("root scan")
            && decision.safe_details["root_kind"] == json!("association")
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    let CoveQlExecutionResult::AssociationRows(rows) = kernel.executed.result else {
        panic!("expected association rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].source_goid.as_deref(),
        Some("00000000000000000000000000000000")
    );
    assert_eq!(
        rows[0].target_goid.as_deref(),
        Some("02020202020202020202020202020202")
    );
}

#[test]
fn kernel_direct_association_projection_executes_with_exact_native_authority() {
    let bytes = object_file_with_person_and_association_record();
    let query = "association(CustomerPlacedOrder).select(source_goid, target_goid)";
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
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("association")
            && diagnostic.safe_details["column_count"] == json!(2)
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "source_goid": "00000000000000000000000000000000",
            "target_goid": "02020202020202020202020202020202"
        })]
    );
}

#[test]
fn kernel_direct_association_projection_filters_root_rows_with_exact_native_authority() {
    let bytes = object_file_with_person_and_association_record();
    let query = "association(CustomerPlacedOrder).where(\"ffffffffffffffffffffffffffffffff\" > source_goid).select(source_goid, target_goid)";
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
    assert_eq!(kernel.executed.row_counts.filtered_rows, 1);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("association")
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "source_goid": "00000000000000000000000000000000",
            "target_goid": "02020202020202020202020202020202"
        })]
    );
}

#[test]
fn kernel_association_antijoin_candidate_is_reported_and_redacted() {
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_with_association_file(),
        "Person.where(!exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    assert_eq!(kernel.kernel_report.association.anti_join_candidates, 1);
    let report = &kernel.explain_json()["execution"]["kernel_report"]["association"];
    assert_eq!(report["anti_join_candidates"], 1);
    assert_eq!(report["edge_count"], 0);
}

#[test]
fn kernel_association_exists_prefilter_executes_exact_semijoin_projection() {
    let query = "Person.where(exists(out(association(CustomerPlacedOrder)))).select(active)";
    let bytes = object_file_with_person_and_association_record();
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision.reason.contains("exact authority")
            && decision.safe_details["association_semi_join_terms"] == json!(1)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_HELPER_PREFILTER_EXECUTED"
            && diagnostic.safe_details["association_semi_join_terms"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let exists_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "predicate_exists:association")
        .expect("association exists contract is present");
    assert_eq!(
        exists_contract["representation_class"],
        json!("ordinal_map_assisted")
    );
    assert_eq!(exists_contract["residual_required"], json!(false));
    assert_eq!(
        exists_contract["fallback_boundary"],
        serde_json::Value::Null
    );
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);
}

#[test]
fn association_antijoin_rejects_without_protected_disclosure_policy() {
    let err = parse_resolve_plan_and_execute_query(
        &minimal_object_with_association_file(),
        "Person.where(!exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_PROTECTED_ASSOCIATION_EXISTENCE");
}

#[test]
fn association_count_rejects_without_protected_disclosure_policy() {
    let err = parse_and_resolve_query(
        &minimal_object_with_association_file(),
        "Person.select(order_count: count(either(association(CustomerPlacedOrder))))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_PROTECTED_ASSOCIATION_EXISTENCE");
}
