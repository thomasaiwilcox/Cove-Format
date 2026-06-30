use super::*;

#[test]
fn materialized_association_root_executes_endpoint_rows() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
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
fn materialized_association_root_can_return_arrow_batches() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ArrowRecordBatches(batches) = executed.result else {
        panic!("expected Arrow batches");
    };
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.schema().field(0).name(), "source_goid");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
    assert_eq!(batch.schema().field(1).name(), "target_goid");
    assert_eq!(batch.schema().field(1).data_type(), &DataType::Utf8);
    let source = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let target = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(source.value(0), "00000000000000000000000000000000");
    assert_eq!(target.value(0), "02020202020202020202020202020202");
}

#[test]
fn association_current_root_aggregates_count_exists_and_distinct_targets() {
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "Person.select(c: count(out(association(CustomerPlacedOrder))), e: exists(out(association(CustomerPlacedOrder))), d: distinct_count(out(association(CustomerPlacedOrder))))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"c": 1, "e": true, "d": 1})]);
}

#[test]
fn association_current_root_exists_filter_respects_endpoint_direction() {
    let outbound = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "Person.where(exists(out(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(outbound_rows) = outbound.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(outbound_rows, vec![json!({"active": true})]);

    let inbound = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "Person.where(exists(in(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(inbound_rows) = inbound.result else {
        panic!("expected JSON rows");
    };
    assert!(inbound_rows.is_empty());
}

#[test]
fn hidden_association_does_not_leak_through_counts_or_negated_exists() {
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    resolve_options.security.visibility_policy =
        VisibilityPolicy::ExternalOverlay("tenant-a".into());
    let execution_options = ExecutionOptions {
        visibility_overlay: Some(VisibilityOverlay {
            overlay_id: "tenant-a".into(),
            visible_goids: BTreeSet::from(["00000000000000000000000000000000".into()]),
            ..VisibilityOverlay::default()
        }),
        ..ExecutionOptions::default()
    };

    let aggregates = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "Person.select(c: count(out(association(CustomerPlacedOrder))), e: exists(out(association(CustomerPlacedOrder))), d: distinct_count(out(association(CustomerPlacedOrder))))",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        execution_options.clone(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = aggregates.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"c": 0, "e": false, "d": 0})]);

    let negated_exists = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "Person.where(!exists(out(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        execution_options,
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = negated_exists.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);
}

#[test]
fn evidence_existence_rejects_without_protected_disclosure_policy() {
    let err = parse_and_resolve_query(
        &minimal_object_with_evidence_index_file(),
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_PROTECTED_EVIDENCE_EXISTENCE");
}

#[test]
fn evidence_helper_requires_map_evidence_metadata_during_resolution() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_MISSING_METADATA");
    assert!(err.diagnostics[0]
        .message
        .contains("evidence roots and helpers require COVE-MAP evidence metadata"));
}

#[test]
fn projection_evidence_helper_requires_map_evidence_metadata_during_resolution() {
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let err = parse_and_resolve_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(c: count(evidence()))",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_MISSING_METADATA");
    assert!(err.diagnostics[0]
        .message
        .contains("evidence roots and helpers require COVE-MAP evidence metadata"));
}

#[test]
fn materialized_evidence_root_without_map_metadata_rejects_before_execution() {
    let err = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "evidence().select(source_id)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_MISSING_METADATA");
    assert!(err.diagnostics[0]
        .message
        .contains("evidence roots and helpers require COVE-MAP evidence metadata"));
}

#[test]
fn materialized_targetless_evidence_root_with_map_metadata_executes_object_grain_rows() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_evidence_index_file(),
        "evidence().select(source_id, source_row_identity, assertion_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["source_id"], json!("crm.customers"));
    assert_eq!(
        rows[0].fields["source_row_identity"],
        json!("customer_id=1")
    );
    assert_eq!(rows[0].fields["assertion_id"], json!("assert_person"));
}

#[test]
fn materialized_evidence_root_with_map_metadata_executes_evidence_rows() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_evidence_index_file(),
        "evidence(Person, grain: object).where(source_id == \"crm.customers\").select(source_id, source_row_identity, rule_id, assertion_id, output_object_id).take(1)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(executed.evidence_authority).unwrap(),
        json!("cove_map_metadata")
    );
    let explain = executed.explain_json();
    assert_eq!(
        explain["execution"]["evidence_authority"],
        json!("cove_map_metadata")
    );
    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["source_id"], json!("crm.customers"));
    assert_eq!(
        rows[0].fields["source_row_identity"],
        json!("customer_id=1")
    );
    assert_eq!(rows[0].fields["rule_id"], json!("upsert_person"));
    assert_eq!(rows[0].fields["assertion_id"], json!("assert_person"));
    assert_eq!(rows[0].fields["output_object_id"], json!("goid:person:1"));
}

#[test]
fn materialized_evidence_root_can_return_arrow_batches() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_evidence_index_file(),
        "evidence(Person, grain: object).where(source_id == \"crm.customers\").select(source_id, source_row_identity).take(1)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ArrowRecordBatches(batches) = executed.result else {
        panic!("expected Arrow batches");
    };
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.schema().field(0).name(), "source_id");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
    assert_eq!(batch.schema().field(1).name(), "source_row_identity");
    assert_eq!(batch.schema().field(1).data_type(), &DataType::Utf8);
    let source_id = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let source_row_identity = batch
        .column(1)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(source_id.value(0), "crm.customers");
    assert_eq!(source_row_identity.value(0), "customer_id=1");
}

#[test]
fn materialized_property_evidence_root_filters_object_property_grain() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true],
        vec![
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_active",
                "output_object_id": "property-active",
                "operation_target": "property",
                "object_type": "Person",
                "property_name": "active",
                "property_id": "active"
            }),
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_other_property",
                "output_object_id": "property-inactive",
                "operation_target": "property",
                "object_type": "Person",
                "property_name": "inactive",
                "property_id": "inactive"
            }),
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_object",
                "output_object_id": "object-person",
                "operation_target": "object",
                "object_type": "Person"
            }),
        ],
    );

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "evidence(Person.active, grain: property).select(source_id, property_name, assertion_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["assertion_id"], json!("assert_active"));
    assert_eq!(rows[0].fields["property_name"], json!("active"));
}

#[test]
fn materialized_association_evidence_root_filters_association_type_grain() {
    let bytes = association_file_with_evidence_entries(vec![
        json!({
            "source_id": "crm.orders",
            "source_row_identity": "order_id=1",
            "rule_id": "link_customer_order",
            "assertion_id": "assert_customer_order",
            "output_object_id": "association:1",
            "operation_target": "association",
            "association_type": "CustomerPlacedOrder"
        }),
        json!({
            "source_id": "crm.orders",
            "source_row_identity": "order_id=2",
            "rule_id": "link_customer_order",
            "assertion_id": "assert_other_association",
            "output_object_id": "association:2",
            "operation_target": "association",
            "association_type": "OtherAssociation"
        }),
    ]);

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "evidence(association(CustomerPlacedOrder), grain: association).select(source_id, association_type, assertion_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].fields["assertion_id"],
        json!("assert_customer_order")
    );
    assert_eq!(
        rows[0].fields["association_type"],
        json!("CustomerPlacedOrder")
    );
}

#[test]
fn materialized_source_evidence_root_filters_source_grain_for_object_target() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true],
        vec![
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_source_row",
                "output_object_id": "source-person",
                "operation_target": "source_record",
                "object_type": "Person"
            }),
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_object",
                "output_object_id": "object-person",
                "operation_target": "object",
                "object_type": "Person"
            }),
        ],
    );

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "evidence(Person, grain: source).select(source_id, source_row_identity, assertion_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["assertion_id"], json!("assert_source_row"));
}

#[test]
fn kernel_evidence_root_row_scan_executes_with_exact_native_authority() {
    let bytes = minimal_object_with_evidence_index_file();
    let query = "evidence(Person, grain: object).select(source_id, source_row_identity, rule_id, assertion_id, output_object_id)";
    let mut resolve_options = ResolveOptions::default();
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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_EVIDENCE_ROOT_SCAN_EXECUTED"
            && diagnostic.safe_details["rows_returned"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision.reason.contains("root scan")
            && decision.safe_details["root_kind"] == json!("evidence")
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    let CoveQlExecutionResult::EvidenceRows(rows) = kernel.executed.result else {
        panic!("expected evidence rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fields["source_id"], json!("crm.customers"));
    assert_eq!(
        rows[0].fields["source_row_identity"],
        json!("customer_id=1")
    );
}

#[test]
fn kernel_direct_evidence_projection_executes_with_exact_native_authority() {
    let bytes = minimal_object_with_evidence_index_file();
    let query = "evidence(Person, grain: object).select(source_id, source_row_identity)";
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
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("evidence")
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
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1"
        })]
    );
}

#[test]
fn kernel_direct_evidence_projection_filters_root_rows_with_exact_native_authority() {
    let bytes = minimal_object_with_evidence_index_file();
    let query = "evidence(Person, grain: object).where(source_id in [\"crm.customers\"]).select(source_id, source_row_identity)";
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
    assert_eq!(kernel.executed.row_counts.filtered_rows, 1);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("evidence")
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1"
        })]
    );
}

#[test]
fn kernel_direct_evidence_projection_orders_and_pages_with_exact_native_authority() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_person_1",
                "output_object_id": "00000000000000000000000000000000",
                "operation_target": "object",
                "object_type": "Person"
            }),
            json!({
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=2",
                "rule_id": "upsert_person",
                "assertion_id": "assert_person_2",
                "output_object_id": "01010101010101010101010101010101",
                "operation_target": "object",
                "object_type": "Person"
            }),
        ],
    );
    let query = "evidence(Person, grain: object).select(source_row_identity).orderBy(source_row_identity, desc).take(1)";
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
    assert_eq!(kernel.executed.row_counts.filtered_rows, 2);
    assert_eq!(kernel.executed.row_counts.output_rows, 1);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("evidence")
            && diagnostic.safe_details["rows_projected"] == json!(1)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("native ordered evidence projection reports coded operator contracts");
    let order_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "order_by")
        .expect("order_by contract is present");
    assert_eq!(order_contract["exact"], true);
    assert_eq!(order_contract["residual_required"], false);
    assert_eq!(
        order_contract["representation_class"],
        json!("ordinal_map_assisted")
    );
    let limit_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "limit_offset")
        .expect("limit/offset contract is present");
    assert_eq!(limit_contract["exact"], true);
    assert!(limit_contract["required_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .any(|metadata| metadata == "stable_order_contract"));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "source_row_identity": "customer_id=2"
        })]
    );
}

#[test]
fn evidence_current_root_aggregates_count_exists_and_distinct_visible_entries() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.select(c: count(evidence()), e: exists(evidence()), d: distinct_count(evidence()))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"c": 1, "e": true, "d": 1})]);
}

#[test]
fn evidence_current_root_exists_filter_uses_matching_output_object_id() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);
}

#[test]
fn hidden_evidence_does_not_leak_through_counts_or_negated_exists() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person",
            "redacted": true
        })],
    );
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    let aggregates = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.select(c: count(evidence()), e: exists(evidence()), d: distinct_count(evidence()))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = aggregates.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"c": 0, "e": false, "d": 0})]);

    let negated_exists = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.where(!exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = negated_exists.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);
}

#[test]
fn hidden_evidence_root_scan_returns_no_rows() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person",
            "suppressed": true
        })],
    );

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "evidence(Person, grain: object).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::EvidenceRows(rows) = executed.result else {
        panic!("expected evidence rows");
    };
    assert!(rows.is_empty());
}

#[test]
fn hidden_evidence_is_not_counted_in_kernel_explain_report() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person",
            "redacted": true
        })],
    );

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel.kernel_report.evidence.enabled);
    assert_eq!(kernel.kernel_report.evidence.evidence_entry_count, 0);
    assert!(kernel.kernel_report.evidence.hidden_entry_filtering_applied);
    assert_eq!(
        kernel.explain_json()["execution"]["kernel_report"]["evidence"]["evidence_entry_count"],
        json!(0)
    );
    assert_eq!(
        kernel.explain_json()["execution"]["kernel_report"]["evidence"]
            ["hidden_entry_filtering_applied"],
        json!(true)
    );
    assert!(kernel
        .kernel_report
        .evidence
        .fallback_reasons
        .iter()
        .any(|reason| reason.contains("aggregate_disclosure_policy")));
}

#[test]
fn evidence_report_records_target_indexes_and_exact_fast_path_policy() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel.kernel_report.evidence.enabled);
    assert_eq!(
        kernel.kernel_report.evidence.existence_fast_path_candidates,
        1
    );
    assert!(kernel.kernel_report.evidence.existence_fast_path_exact);
    assert_eq!(kernel.kernel_report.evidence.count_fast_path_candidates, 0);
    assert!(!kernel.kernel_report.evidence.count_fast_path_exact);
    assert!(kernel.kernel_report.evidence.filtered_by_target);
    assert_eq!(
        kernel.kernel_report.evidence.target_index_kinds,
        vec![coveql::EvidenceTargetIndexKind::ObjectType]
    );
    assert!(kernel.kernel_report.evidence.fallback_reasons.is_empty());
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision
                .reason
                .contains("evidence existence semi-join executed with exact authority")
            && decision.safe_details["evidence_semi_join_terms"] == json!(1)
            && decision.safe_details["evidence_fast_path_exact"] == json!(true)
            && decision.safe_details["residual_verification"] == json!(false)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_HELPER_PREFILTER_EXECUTED"
            && diagnostic.safe_details["evidence_semi_join_terms"] == json!(1)
            && diagnostic.safe_details["evidence_fast_path_exact"] == json!(true)
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
        .find(|contract| contract["operator"] == "predicate_exists:evidence")
        .expect("evidence exists contract is present");
    assert_eq!(exists_contract["exact"], json!(true));
    assert_eq!(exists_contract["residual_required"], json!(false));
    assert_eq!(exists_contract["fallback_boundary"], json!(null));
    let explain = kernel.explain_json();
    assert_eq!(
        explain["execution"]["kernel_report"]["evidence"]["target_index_kinds"],
        json!(["object_type"])
    );
    assert_eq!(
        explain["execution"]["kernel_report"]["evidence"]["existence_fast_path_exact"],
        json!(true)
    );
}

#[test]
fn kernel_evidence_exists_prefilter_narrows_candidates_before_residual_verification() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );
    let query = "Person.where(exists(evidence())).select(active)";
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
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision.reason.contains("existence prefilters")
            && decision.safe_details["input_rows"] == json!(2)
            && decision.safe_details["output_rows"] == json!(1)
            && decision.safe_details["evidence_semi_join_terms"] == json!(1)
            && decision.safe_details["residual_verification"] == json!(true)
    }));
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_HELPER_PREFILTER_EXECUTED"
            && diagnostic.safe_details["input_rows"] == json!(2)
            && diagnostic.safe_details["output_rows"] == json!(1)
            && diagnostic.safe_details["evidence_semi_join_terms"] == json!(1)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let exists_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "predicate_exists:evidence")
        .expect("evidence exists contract is present");
    assert_eq!(
        exists_contract["representation_class"],
        json!("ordinal_map_assisted")
    );
    assert_eq!(exists_contract["residual_required"], json!(true));
    assert_eq!(
        exists_contract["fallback_boundary"],
        json!("materialized_helper_residual_verification")
    );
}

#[test]
fn kernel_helper_association_aggregates_execute_with_exact_native_contract() {
    let bytes = object_file_with_person_and_association_record();
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    for (query, expected, aggregate) in [
        (
            "Person.select(c: count(out(association(CustomerPlacedOrder))))",
            json!({"c": 1}),
            "count",
        ),
        (
            "Person.select(e: exists(out(association(CustomerPlacedOrder))))",
            json!({"e": true}),
            "exists",
        ),
        (
            "Person.select(d: distinct_count(out(association(CustomerPlacedOrder))))",
            json!({"d": 1}),
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
        assert!(kernel.kernel_report.compared_with_materialized);
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, vec![expected]);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_HELPER_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["helper_kind"] == json!("association")
        }));
        assert!(kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_EXACT_AUTHORITY"));
        assert!(!kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_BASELINE_AUTHORITY"));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native helper aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], json!(false));
        assert_eq!(aggregate_contract["exact"], json!(true));
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("ordinal_map_assisted")
        );
        assert_eq!(
            aggregate_contract["row_grain"],
            json!("reconstructed_visible_object_states")
        );
    }
}

#[test]
fn kernel_helper_evidence_aggregates_execute_with_exact_native_contract() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    for (query, expected, aggregate) in [
        (
            "Person.select(c: count(evidence()))",
            json!({"c": 1}),
            "count",
        ),
        (
            "Person.select(e: exists(evidence()))",
            json!({"e": true}),
            "exists",
        ),
        (
            "Person.select(d: distinct_count(evidence()))",
            json!({"d": 1}),
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
        assert!(kernel.kernel_report.compared_with_materialized);
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, vec![expected]);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_HELPER_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["helper_kind"] == json!("evidence")
        }));
        assert!(kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_EXACT_AUTHORITY"));
        assert!(!kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_BASELINE_AUTHORITY"));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native helper aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], json!(false));
        assert_eq!(aggregate_contract["exact"], json!(true));
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("ordinal_map_assisted")
        );
    }
}

#[test]
fn kernel_grouped_helper_association_aggregates_execute_with_exact_native_contract() {
    let bytes = object_file_with_person_and_association_record();
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    for (query, expected_rows, aggregate) in [
        (
            "Person.groupBy(active).select(active, c: count(out(association(CustomerPlacedOrder))))",
            vec![json!({"active": true, "c": 1})],
            "count",
        ),
        (
            "Person.groupBy(active).select(active, e: exists(out(association(CustomerPlacedOrder))))",
            vec![json!({"active": true, "e": true})],
            "exists",
        ),
        (
            "Person.groupBy(active).select(active, d: distinct_count(out(association(CustomerPlacedOrder))))",
            vec![json!({"active": true, "d": 1})],
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
        assert!(kernel.kernel_report.compared_with_materialized);
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, expected_rows);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_GROUPED_HELPER_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["helper_kind"] == json!("association")
                && diagnostic.safe_details["group_property"] == json!("active")
        }));
        assert!(kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_EXACT_AUTHORITY"));
        assert!(!kernel
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_KERNEL_BASELINE_AUTHORITY"));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let group_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == "group_by")
            .expect("native grouped helper group_by contract is present");
        assert_eq!(group_contract["residual_required"], json!(false));
        assert_eq!(group_contract["exact"], json!(true));
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native grouped helper aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], json!(false));
        assert_eq!(aggregate_contract["exact"], json!(true));
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("ordinal_map_assisted")
        );
        assert_eq!(
            aggregate_contract["row_grain"],
            json!("groups_over_reconstructed_visible_object_states")
        );
    }
}

#[test]
fn kernel_grouped_helper_evidence_aggregates_execute_with_exact_native_contract() {
    let bytes = person_file_with_bool_records_and_evidence_entries(
        &[true, false],
        vec![json!({
            "source_id": "crm.customers",
            "source_row_identity": "customer_id=1",
            "rule_id": "upsert_person",
            "assertion_id": "assert_person",
            "output_object_id": "00000000000000000000000000000000",
            "operation_target": "object",
            "object_type": "Person"
        })],
    );
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;

    for (query, expected_rows, aggregate) in [
        (
            "Person.groupBy(active).select(active, c: count(evidence()))",
            vec![
                json!({"active": false, "c": 0}),
                json!({"active": true, "c": 1}),
            ],
            "count",
        ),
        (
            "Person.groupBy(active).select(active, e: exists(evidence()))",
            vec![
                json!({"active": false, "e": false}),
                json!({"active": true, "e": true}),
            ],
            "exists",
        ),
        (
            "Person.groupBy(active).select(active, d: distinct_count(evidence()))",
            vec![
                json!({"active": false, "d": 0}),
                json!({"active": true, "d": 1}),
            ],
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
        assert!(kernel.kernel_report.compared_with_materialized);
        let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
            panic!("expected JSON rows");
        };
        assert_eq!(rows, expected_rows);
        assert!(!kernel.executed.authority.residual_required);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_GROUPED_HELPER_AGGREGATE_EXECUTED"
                && diagnostic.safe_details["aggregate"] == json!(aggregate)
                && diagnostic.safe_details["helper_kind"] == json!("evidence")
                && diagnostic.safe_details["group_property"] == json!("active")
        }));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let aggregate_contract = contracts
            .iter()
            .find(|contract| contract["operator"] == format!("aggregate:{aggregate}"))
            .expect("native grouped helper aggregate contract is present");
        assert_eq!(aggregate_contract["residual_required"], json!(false));
        assert_eq!(aggregate_contract["exact"], json!(true));
        assert_eq!(
            aggregate_contract["representation_class"],
            json!("ordinal_map_assisted")
        );
        assert_eq!(
            kernel.kernel_report.decision.safe_details["residual_verification"],
            json!(false)
        );
    }
}
