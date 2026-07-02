use super::*;

#[test]
fn materialized_object_execution_returns_selected_json_rows() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
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
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("active").is_some());
    assert_eq!(executed.row_counts.output_rows, 1);
    assert_eq!(executed.output_fingerprint.len(), 64);
}

#[test]
fn completed_execution_explain_reports_materialized_boundaries() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = executed.explain_json();
    assert_eq!(explain["execution"]["completed"], true);
    assert_eq!(explain["execution"]["kind"], "materialized_execution");
    assert_eq!(
        explain["execution"]["output_fingerprint"],
        executed.output_fingerprint
    );
    assert_eq!(explain["execution"]["row_counts"], "<redacted>");
    assert!(explain["execution"]["materialization_boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "cove_o_materialized_readback"));
    assert!(executed.explain_text().contains("completed=true"));
}

#[test]
fn completed_execution_explain_can_disclose_row_counts_when_allowed() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = executed.explain_json();
    assert_eq!(explain["execution"]["row_counts"]["output_rows"], 1);
}

#[test]
fn explain_query_execution_is_marked_plan_only() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ExplainJson(explain) = executed.result else {
        panic!("expected explain JSON");
    };
    assert_eq!(explain["execution"]["completed"], false);
    assert_eq!(explain["execution"]["kind"], "plan_explanation");
    assert!(explain["execution"]["row_counts"].is_null());
}

#[test]
fn materialized_as_of_execution_uses_csn_cut() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let latest = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let as_of = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.asOf(csn: 1).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(latest.row_counts.output_rows, 1);
    assert_eq!(as_of.row_counts.output_rows, 1);
}

#[test]
fn conservative_pushdown_is_enabled_by_default_and_reports_candidates() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(executed.pushdown_report.enabled);
    assert!(matches!(
        executed.pushdown_report.outcome,
        PushdownOutcome::Applied | PushdownOutcome::NoCandidates
    ));
    assert!(executed
        .pushdown_report
        .decisions
        .iter()
        .any(|decision| decision.kind == PushdownDecisionKind::PropertyCandidateSeed));
    assert!(
        executed
            .pushdown_report
            .counters
            .property_predicate_candidates
            >= 1
    );
}

#[test]
fn association_endpoint_pushdown_reports_residual_candidate() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(executed
        .pushdown_report
        .decisions
        .iter()
        .any(
            |decision| decision.kind == PushdownDecisionKind::AssociationEndpointCandidate
                && decision.outcome == PushdownOutcome::Residual
        ));
}

#[test]
fn disabled_pushdown_preserves_visible_json_rows() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let enabled = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut execution_options = ExecutionOptions::default();
    execution_options.pushdown = PushdownOptions {
        enabled: false,
        ..PushdownOptions::default()
    };
    let disabled = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        execution_options,
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(enabled_rows) = enabled.result else {
        panic!("expected JSON rows");
    };
    let CoveQlExecutionResult::JsonRows(disabled_rows) = disabled.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(enabled_rows, disabled_rows);
    assert_eq!(enabled.output_fingerprint, disabled.output_fingerprint);
    assert_eq!(disabled.pushdown_report.outcome, PushdownOutcome::Disabled);
}

#[test]
fn exact_bool_property_candidate_prunes_rows_but_preserves_result() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let enabled = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut disabled_options = ExecutionOptions::default();
    disabled_options.pushdown = PushdownOptions {
        enabled: false,
        ..PushdownOptions::default()
    };
    let disabled = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.where(active == true).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        disabled_options,
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(enabled_rows) = enabled.result else {
        panic!("expected JSON rows");
    };
    let CoveQlExecutionResult::JsonRows(disabled_rows) = disabled.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(enabled_rows, disabled_rows);
    assert_eq!(enabled_rows.len(), 2);
    assert_eq!(
        enabled
            .pushdown_report
            .counters
            .rows_skipped_by_property_candidates,
        1
    );
    assert!(enabled
        .pushdown_report
        .decisions
        .iter()
        .any(
            |decision| decision.kind == PushdownDecisionKind::PropertyCandidateSeed
                && decision.outcome == PushdownOutcome::Applied
        ));
}

#[test]
fn goid_in_list_pushdown_prunes_candidates_but_preserves_result() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = concat!(
        r#"Thing.where(goid in ["#,
        r#"uuid"00000000-0000-0000-0000-000000000000", "#,
        r#"uuid"02020202-0202-0202-0202-020202020202"]"#,
        r#").select(goid, active)"#,
    );
    let enabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut disabled_options = ExecutionOptions::default();
    disabled_options.pushdown = PushdownOptions {
        enabled: false,
        ..PushdownOptions::default()
    };
    let disabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        disabled_options,
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(enabled_rows) = enabled.result else {
        panic!("expected JSON rows");
    };
    let CoveQlExecutionResult::JsonRows(disabled_rows) = disabled.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(enabled_rows, disabled_rows);
    assert_eq!(enabled_rows.len(), 2);
    assert!(enabled.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::GoidRowCandidate
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["candidate_goids"] == json!(2)
            && decision.reason.contains("GOID in-list")
    }));
}

#[test]
fn goid_or_pushdown_unions_candidates_but_preserves_result() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let query = concat!(
        r#"Thing.where("#,
        r#"goid == uuid"00000000-0000-0000-0000-000000000000" || "#,
        r#"goid == uuid"02020202-0202-0202-0202-020202020202""#,
        r#").select(goid, active)"#,
    );
    let enabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut disabled_options = ExecutionOptions::default();
    disabled_options.pushdown = PushdownOptions {
        enabled: false,
        ..PushdownOptions::default()
    };
    let disabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        disabled_options,
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(enabled_rows) = enabled.result else {
        panic!("expected JSON rows");
    };
    let CoveQlExecutionResult::JsonRows(disabled_rows) = disabled.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(enabled_rows, disabled_rows);
    assert_eq!(enabled_rows.len(), 2);
    assert!(enabled.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::GoidRowCandidate
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["candidate_goids"] == json!(2)
            && decision.safe_details["or_terms"] == json!(2)
            && decision.reason.contains("GOID OR")
    }));
}

#[test]
fn as_of_pushdown_reports_temporal_candidate_without_changing_rows() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.asOf(csn: 1).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(executed.row_counts.output_rows, 1);
    assert!(executed
        .pushdown_report
        .decisions
        .iter()
        .any(|decision| decision.kind == PushdownDecisionKind::TemporalSegmentPrune));
}

#[test]
fn role_bound_as_of_does_not_apply_commit_time_pushdown() {
    let bytes = object_file_with_numcode_records(&[1, 1, 1]);
    let query =
        r#"MetricThing.asOf(source_event_time: "1970-01-01T00:00:00.000005Z").select(metric)"#;
    let resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        temporal_role_bindings: BTreeMap::from([(TemporalRole::SourceEventTime, "metric".into())]),
        ..ResolveOptions::default()
    };
    let enabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut disabled_options = ExecutionOptions::default();
    disabled_options.pushdown = PushdownOptions {
        enabled: false,
        ..PushdownOptions::default()
    };
    let disabled = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        disabled_options,
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(enabled_rows) = enabled.result else {
        panic!("expected JSON rows");
    };
    let CoveQlExecutionResult::JsonRows(disabled_rows) = disabled.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(enabled_rows, disabled_rows);
    assert_eq!(enabled_rows.len(), 3);
    assert!(!enabled
        .pushdown_report
        .decisions
        .iter()
        .any(|decision| decision.kind == PushdownDecisionKind::TemporalSegmentPrune));
}

#[test]
fn completed_execution_explain_includes_redacted_pushdown_report() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.asOf(csn: 1).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = executed.explain_json();
    assert_eq!(
        explain["execution"]["pushdown_report"]["outcome"],
        "applied"
    );
    assert_eq!(
        explain["execution"]["pushdown_report"]["counters"]["rows_seen"],
        "<redacted>"
    );
    assert!(executed.explain_text().contains("pushdown.outcome"));
}

#[test]
fn null_check_function_filters_nullable_properties_and_reports_validity_candidate() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.where(name.isNotNull()).select(name)",
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
    assert_eq!(rows, vec![json!({"name": "Ada"})]);
    assert!(executed
        .pushdown_report
        .decisions
        .iter()
        .any(|decision| decision.kind == PushdownDecisionKind::ValidityNullCheckCandidate));
}

#[test]
fn coded_safe_function_predicates_do_not_report_materialized_function_pushdown_residuals() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), Some("Bob"), None],
        &["startsWith", "length", "lower"],
    );
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        r#"Person.where(startsWith(name, "A") && length(name) == 3 && lower(name) == "ada").select(name)"#,
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
    assert_eq!(rows, vec![json!({"name": "Ada"})]);
    assert!(executed.pushdown_report.residual_predicates.is_empty());
    assert!(executed
        .pushdown_report
        .decisions
        .iter()
        .any(|decision| decision.kind == PushdownDecisionKind::PropertyCandidateSeed));
    assert!(!executed.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::ResidualMaterialized
            && decision.reason.contains("function")
    }));
}

#[test]
fn null_check_functions_project_two_valued_booleans() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), None]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.select(is_null: isNull(name), present: name.isNotNull())",
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
        vec![
            json!({"is_null": false, "present": true}),
            json!({"is_null": true, "present": false})
        ]
    );
}

#[test]
fn materialized_safe_cast_function_executes_scalar_conversions() {
    let bytes = object_file_with_bool_records_and_function_registry(&[true, false], &["cast"]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        r#"Thing.select(active_text: cast(active, "utf8"), one: cast("1", "uint64"), truth: cast("true", "bool"))"#,
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
        vec![
            json!({"active_text": "true", "one": 1, "truth": true}),
            json!({"active_text": "false", "one": 1, "truth": true})
        ]
    );
}

#[test]
fn in_list_null_literals_use_three_valued_logic() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let cases = [
        ("Thing.where(active in [null]).select(active)", json!([])),
        (
            "Thing.where(active in [null, true]).select(active)",
            json!([{"active": true}]),
        ),
        ("Thing.where(!(active in [null])).select(active)", json!([])),
    ];

    for (query, expected) in cases {
        let executed = parse_resolve_plan_and_execute_query(
            &bytes,
            query,
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
        assert_eq!(json!(rows), expected, "query: {query}");
    }
}

#[test]
fn numeric_equality_preserves_large_integer_precision() {
    let first = 9_007_199_254_740_992_i64;
    let second = 9_007_199_254_740_993_i64;
    let bytes = object_file_with_numcode_records(&[first, second]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "MetricThing.where(metric == 9007199254740993).select(metric)",
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
    assert_eq!(rows, vec![json!({"metric": second})]);
}

#[test]
fn distinct_count_dedupes_logical_numeric_values() {
    let bytes = object_file_with_bool_records(&[true, false]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.select(d: distinct_count(if(active, 1, 1.0)))",
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
    assert_eq!(rows, vec![json!({"d": 1})]);
}

#[test]
fn materialized_aggregate_execution_counts_visible_rows() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(n: count(*))",
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
    assert_eq!(rows, vec![json!({"n": 1})]);
}

#[test]
fn numeric_aggregates_keep_integer_precision() {
    let bytes = object_file_with_numcode_records(&[i64::MAX, 1]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "MetricThing.select(total: sum(metric), average: avg(metric), low: min(metric), high: max(metric))",
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
    assert_eq!(rows[0]["total"], json!(9_223_372_036_854_775_808_u64));
    assert_eq!(rows[0]["average"], json!(4_611_686_018_427_387_904_i64));
    assert_eq!(rows[0]["low"], json!(1));
    assert_eq!(rows[0]["high"], json!(i64::MAX));
}

#[test]
fn thresholded_aggregate_policy_suppresses_small_groups() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy =
        AggregateDisclosurePolicy::AllowThresholded;
    resolve_options.security.aggregate_disclosure_threshold = Some(2);
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(n: count(*))",
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
    assert_eq!(
        rows,
        vec![json!({"n": {"policy": "thresholded", "status": "suppressed", "threshold": 2}})]
    );
}

#[test]
fn thresholded_aggregate_policy_returns_exact_when_threshold_passes() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy =
        AggregateDisclosurePolicy::AllowThresholded;
    resolve_options.security.aggregate_disclosure_threshold = Some(1);
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(n: count(*))",
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
    assert_eq!(rows, vec![json!({"n": 1})]);
}

#[test]
fn redacted_aggregate_policy_returns_marker() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowRedacted;
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(n: count(*))",
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
    assert_eq!(
        rows,
        vec![json!({"n": {"policy": "redacted", "status": "redacted"}})]
    );
}

#[test]
fn redacted_filecode_values_return_policy_marker_by_default() {
    let (bytes, _) = object_file_with_filecode_records_and_redactions(&["secret"], &["secret"]);
    let mut validation = validation_options();
    validation.semantic = false;
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation,
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"name": {"policy": "redacted", "status": "redacted"}})]
    );
}

#[test]
fn redacted_filecode_values_are_refused_when_policy_requires() {
    let (bytes, _) = object_file_with_filecode_records_and_redactions(&["secret"], &["secret"]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.redaction_policy = RedactionPolicy::RefuseProtectedValues;
    let mut validation = validation_options();
    validation.semantic = false;

    let err = parse_resolve_plan_and_execute_query(
        &bytes,
        "Person.select(name)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation,
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_SECURITY_DISCLOSURE_FORBIDDEN");
}

#[test]
fn materialized_execution_returns_history_grain() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.history().select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.row_counts.input_rows, 2);
    assert_eq!(executed.row_counts.output_rows, 2);
}

#[test]
fn every_history_and_change_mode_keeps_distinct_temporal_and_scan_grain() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    for (query, temporal_mode, scan_grain) in [
        (
            "Thing.history(mode: records)",
            "history_records",
            "history_record",
        ),
        (
            "Thing.history(mode: states)",
            "history_states",
            "history_state",
        ),
        (
            "Thing.history(mode: records_and_states)",
            "history_records_and_states",
            "history_records_and_states",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: records)",
            "changes_records",
            "change_record",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: state_transitions)",
            "changes_state_transitions",
            "change_state_transition",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: property_diffs)",
            "changes_property_diffs",
            "change_property_diff",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: final_objects)",
            "changes_final_rows",
            "change_final_row",
        ),
    ] {
        let planned = parse_resolve_and_plan_query(
            bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            PlanOptions::default(),
            validation_options(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&planned.resolved.temporal.mode).unwrap(),
            json!(temporal_mode)
        );
        assert_eq!(
            serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap(),
            json!(scan_grain)
        );
        assert_eq!(
            planned.explain_json()["operation_context"]["temporal_mode"],
            json!(temporal_mode)
        );
    }
}

#[test]
fn kernel_force_mode_reports_temporal_history_and_changes_fallback_contracts() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    for (query, operator, boundary, row_grain) in [
        (
            "Thing.history(mode: records).where(active == true).select(active)",
            "temporal_history",
            "materialized_history_reconstruction",
            "history_record",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: property_diffs).where(active == true)",
            "temporal_changes",
            "materialized_changes_reconstruction",
            "change_property_diff",
        ),
    ] {
        let err = parse_resolve_plan_build_physical_and_execute_query(
            bytes,
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
        .unwrap_err();
        assert_eq!(err.diagnostics[0].code, "E_KERNEL_UNSUPPORTED");
        let contracts = err.diagnostics[0].safe_details["kernel_shape"]["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let contract = contracts
            .iter()
            .find(|contract| contract["operator"] == operator)
            .expect("temporal fallback contract is present");

        assert_eq!(contract["exact"], false);
        assert_eq!(contract["residual_required"], true);
        assert_eq!(contract["fallback_boundary"], json!(boundary));
        assert_eq!(contract["row_grain"], json!(row_grain));
    }
}

#[test]
fn kernel_temporal_history_direct_projection_executes_with_exact_native_authority() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let query = "Thing.history(mode: records).select(active)";
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        bytes,
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
        bytes,
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
    assert!(kernel.kernel_report.optimization_authority.authoritative);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    assert_eq!(
        kernel.kernel_report.decision.safe_details["kernel_surface_source"],
        json!("temporal_segment_direct")
    );
    assert_eq!(
        kernel.kernel_report.decision.safe_details["materialized_surface_role"],
        json!("reconstruction_oracle_and_fallback")
    );
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let temporal = contracts
        .iter()
        .find(|contract| contract["operator"] == "temporal_history")
        .expect("temporal history contract is present");
    assert_eq!(temporal["exact"], true);
    assert_eq!(temporal["residual_required"], false);
    assert_eq!(temporal["fallback_boundary"], Value::Null);
    assert_eq!(temporal["row_grain"], json!("history_record"));
}

#[test]
fn kernel_temporal_changes_direct_projection_executes_with_exact_native_authority() {
    let bytes = object_file_with_bool_change();
    let query = "Thing.changes(from: 1, to: 3, mode: records).select(active)";
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
    assert!(kernel.kernel_report.optimization_authority.authoritative);
    assert!(!kernel.executed.authority.residual_required);
    assert_eq!(
        kernel.kernel_report.decision.safe_details["residual_verification"],
        json!(false)
    );
    let CoveQlExecutionResult::JsonRows(rows) = &kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert!(!rows.is_empty());
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let temporal = contracts
        .iter()
        .find(|contract| contract["operator"] == "temporal_changes")
        .expect("temporal changes contract is present");
    assert_eq!(temporal["exact"], true);
    assert_eq!(temporal["residual_required"], false);
    assert_eq!(temporal["fallback_boundary"], Value::Null);
    assert_eq!(temporal["row_grain"], json!("change_record"));
}

#[test]
fn kernel_temporal_object_modes_report_exact_native_contracts() {
    let bytes = object_file_with_bool_change();
    for (query, operator, row_grain) in [
        (
            "Thing.changes(from: 1, to: 3, mode: property_diffs)",
            "temporal_changes",
            "change_property_diff",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: final_objects).select(active)",
            "temporal_changes",
            "change_final_row",
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
        assert!(kernel.kernel_report.compared_with_materialized);
        assert!(kernel.kernel_report.optimization_authority.authoritative);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
                && diagnostic.safe_details["root_kind"] == json!("object")
                && diagnostic.safe_details["residual_verification"] == json!(false)
        }));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let temporal = contracts
            .iter()
            .find(|contract| contract["operator"] == operator)
            .expect("temporal contract is present");
        assert_eq!(temporal["exact"], true);
        assert_eq!(temporal["residual_required"], false);
        assert_eq!(temporal["fallback_boundary"], Value::Null);
        assert_eq!(temporal["row_grain"], json!(row_grain));
    }
}

#[test]
fn kernel_temporal_association_modes_report_exact_native_contracts() {
    let bytes = association_file_with_endpoint_change();
    for (query, operator, row_grain) in [
        (
            "association(CustomerPlacedOrder).history(mode: records_and_states).select(source_goid, target_goid)",
            "temporal_history",
            "history_records_and_states",
        ),
        (
            "association(CustomerPlacedOrder).changes(from: 2, to: 3, mode: property_diffs).select(source_goid, target_goid)",
            "temporal_changes",
            "change_property_diff",
        ),
        (
            "association(CustomerPlacedOrder).changes(from: 1, to: 3, mode: final_objects).select(source_goid, target_goid)",
            "temporal_changes",
            "change_final_row",
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
        assert!(kernel.kernel_report.compared_with_materialized);
        assert!(kernel.kernel_report.optimization_authority.authoritative);
        assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
                && diagnostic.safe_details["root_kind"] == json!("association")
                && diagnostic.safe_details["residual_verification"] == json!(false)
        }));
        let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
            ["operator_contracts"]
            .as_array()
            .expect("kernel shape reports coded operator contracts");
        let temporal = contracts
            .iter()
            .find(|contract| contract["operator"] == operator)
            .expect("temporal contract is present");
        assert_eq!(temporal["exact"], true);
        assert_eq!(temporal["residual_required"], false);
        assert_eq!(temporal["fallback_boundary"], Value::Null);
        assert_eq!(temporal["row_grain"], json!(row_grain));
    }
}

#[test]
fn history_records_and_states_tags_output_grain() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.history(mode: records_and_states)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(planned.logical_plan.context.scan_grain).unwrap(),
        json!("history_records_and_states")
    );
    let sort_fields = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort {
                keys,
                defaulted: true,
                ..
            } => Some(
                keys.iter()
                    .filter_map(|key| key.field.as_deref())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        sort_fields,
        vec![
            "object_type_id",
            "branch_key",
            "goid",
            "timestamp_us",
            "csn",
            "record_id",
            "output_grain"
        ]
    );

    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.history(mode: records_and_states)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ObjectRows(rows) = executed.result else {
        panic!("expected object rows");
    };
    assert!(rows
        .iter()
        .any(|row| row.output_grain == OutputGrain::HistoryRecord));
    assert!(rows
        .iter()
        .any(|row| row.output_grain == OutputGrain::HistoryState));
}

#[test]
fn history_records_and_states_default_ordering_pages_mixed_grains() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.history(mode: records_and_states).skip(1).take(1)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ObjectRows(rows) = executed.result else {
        panic!("expected object rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].csn, 1);
    assert_eq!(rows[0].output_grain, OutputGrain::HistoryState);
}

#[test]
fn reject_ambiguous_branch_checks_materialized_history_rows() {
    let bytes = object_file_with_bool_records_on_branches(&[true, false], &[0, 7]);
    let err = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.branch(reject_ambiguous).history(mode: records).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_BRANCH");
}

#[test]
fn reject_ambiguous_branch_allows_single_materialized_state_branch() {
    let bytes = object_file_with_bool_records_on_branches(&[true, false], &[7, 7]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.branch(reject_ambiguous).select(active)",
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
        vec![json!({"active": true}), json!({"active": false})]
    );
}

#[test]
fn reject_ambiguous_branch_rejects_multiple_materialized_state_branches() {
    let bytes = object_file_with_bool_records_on_branches(&[true, false], &[0, 7]);
    let err = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.branch(reject_ambiguous).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_BRANCH");
}

#[test]
fn changes_property_diffs_emit_property_level_rows() {
    let bytes = object_file_with_bool_change();
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.changes(from: 1, to: 3, mode: property_diffs)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ObjectRows(rows) = executed.result else {
        panic!("expected object rows");
    };
    assert!(!rows.is_empty());
    assert!(rows
        .iter()
        .all(|row| row.output_grain == OutputGrain::ChangePropertyDiff));
    assert!(rows.iter().all(|row| row.change.is_some()));
}

#[test]
fn association_history_records_and_states_tags_output_grain() {
    let bytes = association_file_with_endpoint_change();
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "association(CustomerPlacedOrder).history(mode: records_and_states)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::AssociationRows(rows) = executed.result else {
        panic!("expected association rows");
    };
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter()
            .filter(|row| row.output_grain == OutputGrain::HistoryRecord)
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.output_grain == OutputGrain::HistoryState)
            .count(),
        2
    );

    let paged = parse_resolve_plan_and_execute_query(
        &bytes,
        "association(CustomerPlacedOrder).history(mode: records_and_states).skip(1).take(1)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::AssociationRows(rows) = paged.result else {
        panic!("expected paged association rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].csn, 1);
    assert_eq!(rows[0].output_grain, OutputGrain::HistoryState);
}

#[test]
fn association_changes_property_diffs_emit_endpoint_diffs() {
    let bytes = association_file_with_endpoint_change();
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "association(CustomerPlacedOrder).changes(from: 2, to: 3, mode: property_diffs)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::AssociationRows(rows) = executed.result else {
        panic!("expected association rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].output_grain, OutputGrain::ChangePropertyDiff);
    assert_eq!(
        rows[0].target_goid.as_deref(),
        Some("03030303030303030303030303030303")
    );
    let change = rows[0].change.as_ref().expect("endpoint diff is present");
    assert_eq!(change.property_id, 12);
    assert_eq!(change.property_name, "target_goid");
    assert_eq!(change.diff_kind, MaterializedChangeDiffKind::Changed);
    assert_eq!(change.old_value, json!("02020202020202020202020202020202"));
    assert_eq!(change.new_value, json!("03030303030303030303030303030303"));
}

#[test]
fn association_changes_final_rows_reconstruct_final_endpoint_state() {
    let bytes = association_file_with_endpoint_change();
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "association(CustomerPlacedOrder).changes(from: 1, to: 3, mode: final_objects)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::AssociationRows(rows) = executed.result else {
        panic!("expected association rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].output_grain, OutputGrain::FinalObject);
    assert_eq!(
        rows[0].source_goid.as_deref(),
        Some("01010101010101010101010101010101")
    );
    assert_eq!(
        rows[0].target_goid.as_deref(),
        Some("03030303030303030303030303030303")
    );
}

#[test]
fn incompatible_root_output_grains_reject_before_execution() {
    let mut output_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::AssociationRows),
        ..ResolveOptions::default()
    };
    let err = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        output_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_EXECUTION_GRAIN");

    output_options.output_mode = Some(CoveQlOutputMode::ProjectionRows);
    let err = parse_resolve_plan_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).history(mode: records).select(active)",
        ParseOptions::default(),
        output_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_EXECUTION_GRAIN");
}

#[test]
fn default_ordering_is_applied_before_skip_take() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.history(mode: records).skip(1).take(1)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let CoveQlExecutionResult::ObjectRows(rows) = executed.result else {
        panic!("expected object rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].csn, 2);
}

#[test]
fn include_tombstones_false_matches_default_state_surface() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let default_surface = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explicit_false = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.includeTombstones(false).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        default_surface.output_fingerprint,
        explicit_false.output_fingerprint
    );
}

#[test]
fn planned_query_stream_batches_deterministically() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.history().select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut execution_options = ExecutionOptions::default();
    execution_options.batch_size = Some(1);
    execution_options.allow_partial_results = true;
    let mut stream = execute_planned_query_stream(bytes, planned, execution_options).unwrap();
    assert!(stream.executed().is_none());
    assert!(!stream.is_blocking());

    let first = stream.next_batch().unwrap().unwrap();
    assert!(stream.executed().is_none());
    let second = stream.next_batch().unwrap().unwrap();
    assert!(stream.next_batch().unwrap().is_none());
    let executed = stream.finish().unwrap();

    assert_eq!(executed.row_counts.output_rows, 2);
    assert!(executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_STREAM_BATCHED_EXECUTION"));
    let batches = vec![first, second];
    assert_eq!(batches.len(), 2);
    assert!(matches!(batches[0], CoveQlExecutionResult::JsonRows(_)));
    assert!(matches!(batches[1], CoveQlExecutionResult::JsonRows(_)));
}

#[test]
fn planned_query_stream_cancel_stops_streaming_before_completion() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.history().select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut execution_options = ExecutionOptions::default();
    execution_options.batch_size = Some(1);
    execution_options.allow_partial_results = true;
    let mut stream = execute_planned_query_stream(bytes, planned, execution_options).unwrap();

    assert!(stream.next_batch().unwrap().is_some());
    stream.cancel();
    let next_err = stream.next_batch().unwrap_err();
    assert_eq!(next_err.diagnostics[0].code, "E_STREAM_CANCELLED");
    let finish_err = stream.finish().unwrap_err();
    assert_eq!(finish_err.diagnostics[0].code, "E_STREAM_CANCELLED");
}

#[test]
fn aggregate_stream_is_marked_blocking() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.select(n: count(*))",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut execution_options = ExecutionOptions::default();
    execution_options.batch_size = Some(1);
    let mut stream = execute_planned_query_stream(bytes, planned, execution_options).unwrap();

    assert!(stream.is_blocking());
    assert_eq!(
        stream.blocking_reason(),
        Some("aggregate execution requires complete input")
    );
    let batch = stream.next_batch().unwrap().unwrap();
    assert!(matches!(batch, CoveQlExecutionResult::JsonRows(_)));
    assert!(stream.next_batch().unwrap().is_none());
    let executed = stream.finish().unwrap();
    assert!(executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_STREAM_BLOCKING_PLAN"));
}

#[test]
fn blocking_stream_cancel_rejects_before_materialized_execution() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let planned = parse_resolve_and_plan_query(
        bytes,
        "Thing.select(n: count(*))",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut execution_options = ExecutionOptions::default();
    execution_options.batch_size = Some(1);
    let mut stream = execute_planned_query_stream(bytes, planned, execution_options).unwrap();

    assert!(stream.is_blocking());
    stream.cancel();
    let err = stream.next_batch().unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_STREAM_CANCELLED");
}

#[test]
fn explicit_order_by_stream_is_marked_blocking_even_with_default_nulls() {
    let bytes = object_file_with_bool_records(&[true, false, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Thing.select(active).orderBy(active, desc)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut execution_options = ExecutionOptions::default();
    execution_options.batch_size = Some(1);
    let stream = execute_planned_query_stream(&bytes, planned, execution_options).unwrap();

    assert!(stream.is_blocking());
    assert_eq!(
        stream.blocking_reason(),
        Some("explicit orderBy requires full materialized sort")
    );
}

#[test]
fn external_visibility_overlay_fails_closed_without_overlay_data() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.visibility_policy =
        VisibilityPolicy::ExternalOverlay("tenant-a".into());
    let err = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_VISIBILITY_OVERLAY_UNAVAILABLE");
}

#[test]
fn external_visibility_overlay_filters_rows_before_counts() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.visibility_policy =
        VisibilityPolicy::ExternalOverlay("tenant-a".into());
    let execution_options = ExecutionOptions {
        visibility_overlay: Some(VisibilityOverlay {
            overlay_id: "tenant-a".into(),
            ..VisibilityOverlay::default()
        }),
        ..ExecutionOptions::default()
    };
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        execution_options,
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.row_counts.input_rows, 0);
    assert_eq!(executed.row_counts.output_rows, 0);
}

#[test]
fn refuse_protected_values_allows_unprotected_materialized_values() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut resolve_options = json_resolve_options();
    resolve_options.security.redaction_policy = RedactionPolicy::RefuseProtectedValues;
    let executed = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.row_counts.output_rows, 1);
}

#[test]
fn materialized_execution_enforces_row_budget_without_take() {
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
    let mut execution_options = ExecutionOptions::default();
    execution_options
        .resource_budget
        .maximum_rows_without_explicit_take = 0;

    let err = parse_resolve_plan_and_execute_query(
        bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        execution_options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_RESOURCE_BUDGET_EXCEEDED");
}

#[test]
fn materialized_projection_execution_can_return_arrow_batches() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active)",
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
    assert!(!batches.is_empty());
}

#[test]
fn arrow_output_preserves_selected_schema_for_empty_grouped_rows() {
    let bytes = object_file_with_bool_records(&[true]);
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.where(active == false).groupBy(active).select(active, n: count(*))",
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
    assert_eq!(batches[0].num_rows(), 0);
    assert_eq!(batches[0].schema().field(0).name(), "active");
    assert_eq!(batches[0].schema().field(0).data_type(), &DataType::Boolean);
    assert_eq!(batches[0].schema().field(1).name(), "n");
    assert_eq!(batches[0].schema().field(1).data_type(), &DataType::UInt64);
}

#[test]
fn zero_copy_arrow_request_warns_when_owned_fallback_is_allowed() {
    let bytes = object_file_with_bool_records(&[true]);
    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(matches!(
        executed.result,
        CoveQlExecutionResult::ArrowRecordBatches(_)
    ));
    let diagnostic = executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_ZERO_COPY_MATERIALIZED_FALLBACK")
        .expect("expected zero-copy fallback warning");
    assert!(diagnostic.safe_details["reason"]
        .as_str()
        .unwrap()
        .contains("retained COVE-L page ownership"));
}

#[test]
fn zero_copy_arrow_request_rejects_when_owned_fallback_is_forbidden() {
    let bytes = object_file_with_bool_records(&[true]);
    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        fallback_policy: FallbackPolicy::RejectOnFallback,
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;

    let err = parse_resolve_plan_and_execute_query(
        &bytes,
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_ZERO_COPY_UNSUPPORTED");
}

#[test]
fn retained_physical_zero_copy_arrow_uses_cove_l_object_page_owner() {
    let bytes = Arc::new(object_file_with_numcode_records(&[10, 20]));
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_arc(Arc::clone(&bytes)),
        validation_options(),
    )
    .unwrap();
    let retained_payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let expected_values_ptr = retained_payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap()
        .as_ptr();

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "MetricThing.select(metric)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(metric_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_MATERIALIZED_FALLBACK"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let batch = &batches[0];
    let values = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(values.value(0), 10);
    assert_eq!(values.value(1), 20);
    assert_eq!(values.to_data().buffers()[0].as_ptr(), expected_values_ptr);
    assert_eq!(
        serde_json::to_value(kernel.executed.authority.source).unwrap(),
        json!("zero_copy_arrow")
    );
    assert_eq!(
        serde_json::to_value(kernel.kernel_report.optimization_authority.state).unwrap(),
        json!("authoritative")
    );
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.kind == KernelDecisionKind::Applied
            && decision.reason.contains("zero-copy Arrow projection")
    }));
}

#[test]
fn retained_physical_zero_copy_arrow_reuses_numcode_values_with_null_bitmap() {
    let bytes = Arc::new(object_file_with_nullable_numcode_records(&[
        Some(10),
        None,
        Some(30),
    ]));
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_arc(Arc::clone(&bytes)),
        validation_options(),
    )
    .unwrap();
    let retained_payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let expected_values_ptr = retained_payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap()
        .as_ptr();

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "MetricThing.select(metric)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(nullable_metric_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_MATERIALIZED_FALLBACK"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(values.value(0), 10);
    assert!(values.is_null(1));
    assert_eq!(values.value(2), 30);
    assert_eq!(values.to_data().buffers()[0].as_ptr(), expected_values_ptr);
}

#[test]
fn retained_physical_zero_copy_arrow_reuses_fixed_bytes_values() {
    let first = [1u8; 16];
    let second = [2u8; 16];
    let bytes = Arc::new(object_file_with_plain_fixed_uuid_records(&[first, second]));
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_arc(Arc::clone(&bytes)),
        validation_options(),
    )
    .unwrap();
    let retained_payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let expected_values_ptr = retained_payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap()
        .as_ptr();

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "UuidFixedThing.select(uid)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(uid_fixed_bytes_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(values.value(0), first);
    assert_eq!(values.value(1), second);
    assert_eq!(values.to_data().buffers()[0].as_ptr(), expected_values_ptr);
}

#[test]
fn retained_physical_zero_copy_arrow_reuses_nullable_fixed_bytes_values() {
    let first = [1u8; 16];
    let third = [3u8; 16];
    let bytes = Arc::new(object_file_with_nullable_plain_fixed_uuid_records(&[
        Some(first),
        None,
        Some(third),
    ]));
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_arc(Arc::clone(&bytes)),
        validation_options(),
    )
    .unwrap();
    let retained_payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let expected_values_ptr = retained_payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap()
        .as_ptr();

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "UuidFixedThing.select(uid)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(nullable_uid_fixed_bytes_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(values.value(0), first);
    assert!(values.is_null(1));
    assert_eq!(values.value(2), third);
    assert_eq!(values.to_data().buffers()[0].as_ptr(), expected_values_ptr);
}

#[test]
fn retained_physical_zero_copy_arrow_exports_bool_page_without_materialized_fallback() {
    let bytes = Arc::new(object_file_with_bool_records(&[true, false, true]));
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_arc(Arc::clone(&bytes)),
        validation_options(),
    )
    .unwrap();
    let retained_payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let cove_values_ptr = retained_payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap()
        .as_ptr();

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "Thing.select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(active_bool_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_MATERIALIZED_FALLBACK"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(values.value(0));
    assert!(!values.value(1));
    assert!(values.value(2));
    assert_ne!(values.to_data().buffers()[0].as_ptr(), cove_values_ptr);
}

#[test]
fn retained_physical_zero_copy_arrow_exports_filecode_dictionary_page() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "red"]);
    let bytes = Arc::new(bytes);

    let mut resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: true,
        }),
        ..ResolveOptions::default()
    };
    resolve_options.security.zero_copy_permission = true;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let kernel = parse_resolve_plan_build_physical_and_execute_query_retained(
        CoveQlRetainedInput::from_arc(Arc::clone(&bytes)),
        "Person.select(name)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default()
            .with_zero_copy_output(true)
            .with_sidecars(PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(name_filecode_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            }),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_ARROW_EXECUTED"));
    assert!(!kernel
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_ZERO_COPY_MATERIALIZED_FALLBACK"));
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    let values = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    let dictionary = values
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    let decoded = (0..values.len())
        .map(|row| dictionary.value(values.keys().value(row) as usize))
        .collect::<Vec<_>>();
    assert_eq!(decoded, vec!["red", "blue", "red"]);
    assert_eq!(
        batches[0].schema().field(0).data_type(),
        &DataType::Dictionary(Box::new(DataType::UInt32), Box::new(DataType::Utf8))
    );
}
