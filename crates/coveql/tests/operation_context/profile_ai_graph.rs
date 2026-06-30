use super::*;

#[test]
fn conformance_profile_declares_full_coveql_surface_defaults() {
    let profile = coveql::conformance_profile();

    assert_eq!(profile.language_version, coveql::COVEQL_LANGUAGE_VERSION);
    assert_eq!(profile.core_version, coveql::COVEQL_CORE_VERSION);
    assert_eq!(profile.grammar_version, coveql::COVEQL_GRAMMAR_VERSION);
    assert_eq!(
        profile.profile_contract_version,
        coveql::COVEQL_PROFILE_CONTRACT_VERSION
    );
    assert_eq!(
        profile.bridge_contract_version,
        coveql::COVEQL_BRIDGE_CONTRACT_VERSION
    );
    assert_eq!(
        profile.object_profile_version,
        coveql::COVEQL_OBJECT_PROFILE_VERSION
    );
    assert_eq!(
        profile.table_profile_version,
        coveql::COVEQL_TABLE_PROFILE_VERSION
    );
    assert_eq!(
        profile.graph_profile_version,
        coveql::COVEQL_GRAPH_PROFILE_VERSION
    );
    assert_eq!(profile.resolved_ast_version, coveql::RESOLVED_AST_VERSION);
    assert_eq!(profile.logical_plan_version, coveql::LOGICAL_PLAN_VERSION);
    assert_eq!(profile.physical_plan_version, coveql::PHYSICAL_PLAN_VERSION);
    assert_eq!(
        profile.projection_dependency_contract_version,
        coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION
    );
    assert_eq!(
        profile.predicate_normal_form_version,
        coveql::PREDICATE_NORMAL_FORM_VERSION
    );
    assert_eq!(
        profile.coded_operator_contract_version,
        coveql::CODED_OPERATOR_CONTRACT_VERSION
    );
    assert_eq!(
        profile.predicate_representation_contract_version,
        coveql::PREDICATE_REPRESENTATION_CONTRACT_VERSION
    );
    assert_eq!(
        profile.physical_operator_contract_version,
        coveql::PHYSICAL_OPERATOR_CONTRACT_VERSION
    );
    assert_eq!(
        profile.physical_sidecar_validation_version,
        coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION
    );
    assert_eq!(
        profile.datafusion_coveql_report_version,
        coveql::DATAFUSION_COVEQL_REPORT_VERSION
    );
    assert!(profile
        .mandatory_history_modes
        .contains(&"records_and_states"));
    assert!(profile.mandatory_change_modes.contains(&"final_rows"));
    assert!(profile.mandatory_functions.contains(&"coalesce"));
    assert!(profile.mandatory_functions.contains(&"length"));
    assert!(profile
        .projection_default_order
        .contains(&"manifest_or_file_ordinal_then_source_row_ordinal"));
    assert!(profile
        .evidence_shorthands
        .contains(&"evidence(projection(...))"));
    assert!(profile
        .evidence_shorthands
        .contains(&"evidence(root as binding)"));
    assert!(profile
        .required_fingerprint_fields
        .contains(&"predicate_ast"));
    assert!(profile
        .required_fingerprint_fields
        .contains(&"projection_dependency"));
    assert!(profile
        .required_coded_explain_fields
        .contains(&"pushed_filters"));
    assert!(profile
        .required_coded_explain_fields
        .contains(&"pushed_columns"));
    assert!(profile
        .required_coded_operator_contract_fields
        .contains(&"contract_version"));
    assert!(profile
        .required_coded_operator_contract_fields
        .contains(&"proof_obligation"));
    assert!(profile
        .required_coded_operator_contract_fields
        .contains(&"fallback_boundary"));
    assert!(profile
        .required_physical_sidecar_validation_fields
        .contains(&"report_version"));
    assert!(profile
        .required_physical_sidecar_validation_fields
        .contains(&"fallback_reason"));
    assert!(profile
        .required_physical_plan_sidecar_fields
        .contains(&"runtime_compatibility"));
    assert!(profile
        .required_physical_plan_sidecar_fields
        .contains(&"cache_compatibility"));
    assert!(profile
        .required_physical_plan_sidecar_fields
        .contains(&"codec_compatibility"));
    assert!(profile
        .required_datafusion_scan_negotiation_fields
        .contains(&"projection_pushed_to_coveql"));
    assert!(profile
        .required_datafusion_scan_negotiation_fields
        .contains(&"limit_pushed_to_coveql"));
    assert!(profile
        .required_datafusion_scan_negotiation_fields
        .contains(&"unhandled_residuals"));
    assert!(profile.required_diagnostic_fields.contains(&"safe_details"));
    assert!(profile.required_diagnostic_fields.contains(&"redacted"));
    assert!(profile.required_diagnostic_codes.contains(&"E_PARSE"));
    assert!(profile
        .required_diagnostic_codes
        .contains(&"E_DATAFUSION_PUSH_FILTER_UNSAFE"));
    assert!(profile
        .required_diagnostic_codes
        .contains(&"E_UNSUPPORTED_PROFILE_METHOD"));
    assert!(profile
        .required_diagnostic_codes
        .contains(&"E_UNKNOWN_TABLE_SURFACE"));
    assert_eq!(profile.conformance_tiers.len(), 3);
    assert_eq!(profile.conformance_tiers[0].name, "semantic_correctness");
    assert_eq!(
        profile.conformance_tiers[0].authority,
        "materialized_coveql"
    );
    assert!(profile.conformance_tiers[0]
        .required_surfaces
        .contains(&"datafusion"));
    assert_eq!(profile.conformance_tiers[1].name, "fallback_invariance");
    assert!(profile.conformance_tiers[1]
        .required_surfaces
        .contains(&"security_blocked_optional_metadata"));
    assert_eq!(profile.conformance_tiers[2].name, "acceleration_proof");
    assert!(profile.conformance_tiers[2]
        .required_surfaces
        .contains(&"manifest_bridges"));
    assert_unique_contract_fields("mandatory_history_modes", profile.mandatory_history_modes);
    assert_unique_contract_fields("mandatory_change_modes", profile.mandatory_change_modes);
    assert_unique_contract_fields("mandatory_functions", profile.mandatory_functions);
    assert_unique_contract_fields(
        "required_fingerprint_fields",
        profile.required_fingerprint_fields,
    );
    assert_unique_contract_fields(
        "required_coded_explain_fields",
        profile.required_coded_explain_fields,
    );
    assert_unique_contract_fields(
        "required_coded_operator_contract_fields",
        profile.required_coded_operator_contract_fields,
    );
    assert_unique_contract_fields(
        "required_physical_sidecar_validation_fields",
        profile.required_physical_sidecar_validation_fields,
    );
    assert_unique_contract_fields(
        "required_physical_plan_sidecar_fields",
        profile.required_physical_plan_sidecar_fields,
    );
    assert_unique_contract_fields(
        "required_datafusion_scan_negotiation_fields",
        profile.required_datafusion_scan_negotiation_fields,
    );
    assert_unique_contract_fields(
        "required_diagnostic_fields",
        profile.required_diagnostic_fields,
    );
    assert_unique_contract_fields(
        "required_diagnostic_codes",
        profile.required_diagnostic_codes,
    );

    let core_contract = coveql::coveql_core_contract();
    assert_eq!(core_contract.core_version, coveql::COVEQL_CORE_VERSION);
    assert!(core_contract
        .primary_profiles
        .contains(&coveql::CoveQlProfileId::Object));
    assert!(core_contract
        .primary_profiles
        .contains(&coveql::CoveQlProfileId::Table));
    assert!(core_contract
        .primary_profiles
        .contains(&coveql::CoveQlProfileId::Graph));
    assert!(core_contract
        .primary_profiles
        .contains(&coveql::CoveQlProfileId::Ai));

    let contracts = coveql::builtin_coveql_profile_contracts();
    assert_eq!(contracts.len(), 4);
    assert!(coveql::coveql_profile_contract(coveql::CoveQlProfileId::Object).implemented);
    let table_contract = coveql::coveql_profile_contract(coveql::CoveQlProfileId::Table);
    assert!(table_contract.implemented);
    assert!(table_contract
        .materialization_boundaries
        .contains(&"raw_table_surface_requires_table_contract"));
    let graph_contract = coveql::coveql_profile_contract(coveql::CoveQlProfileId::Graph);
    assert!(graph_contract.implemented);
    assert!(graph_contract
        .relationship_capabilities
        .contains(&"chained_traverse"));
    assert!(graph_contract
        .relationship_capabilities
        .contains(&"multi_hop_path_binding"));
    assert!(graph_contract
        .materialization_boundaries
        .contains(&"variable_length_traversal_requires_explicit_contract"));
    let ai_contract = coveql::coveql_profile_contract(coveql::CoveQlProfileId::Ai);
    assert!(ai_contract.implemented);
    assert!(ai_contract.profile_methods.contains(&"similar"));
    assert!(ai_contract.profile_methods.contains(&"asPromptContext"));
    assert!(ai_contract.profile_methods.contains(&"generatorAudit"));
    assert!(ai_contract.security_barriers.contains(&"payload_integrity"));
    let bridges = coveql::builtin_coveql_bridge_contracts()
        .iter()
        .map(|bridge| {
            (
                bridge.source_profile.as_str().to_string(),
                bridge.target_profile.as_str().to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        bridges,
        BTreeSet::from([
            ("object".to_string(), "table".to_string()),
            ("table".to_string(), "object".to_string()),
            ("object".to_string(), "graph".to_string()),
            ("graph".to_string(), "object".to_string()),
            ("table".to_string(), "graph".to_string()),
            ("graph".to_string(), "table".to_string()),
            ("table".to_string(), "table".to_string()),
        ])
    );
}

#[test]
fn query_builder_matches_handwritten_query_fingerprints() {
    let built = coveql::CoveQlQueryBuilder::object("Person")
        .where_predicate("active == true")
        .select(["active"])
        .as_of_csn(7)
        .order_by(
            "active",
            coveql::AstOrderDirection::Desc,
            coveql::AstNullOrdering::Default,
        )
        .take(10)
        .explain(ExplainMode::Coded);
    let parsed_built = built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "Person.where(active == true).select(active).asOf(csn: 7).orderBy(active, desc).take(10).explain(\"coded\")",
        ParseOptions::default(),
    )
    .unwrap();

    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let recursive_built = coveql::CoveQlQueryBuilder::table("people")
        .with_recursive_table_step(
            "reach",
            "people",
            None::<&str>,
            "people_step",
            None::<&str>,
            "row-key",
            4,
        )
        .select(["id"]);
    let parsed_built = recursive_built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "table(people).withRecursive(name: reach, seed: table(people), step: table(people_step), key: `row-key`, maxIterations: 4).select(id)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );
}

#[test]
fn query_builder_matches_table_and_graph_profile_syntax() {
    let table_built = coveql::CoveQlQueryBuilder::table("thing_projection")
        .alias("l")
        .lookup_many("thing_projection", "r", "l.active == r.active")
        .select(["left_active: l.active", "right_active: r.active"]);
    let parsed_built = table_built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active, cardinality: many, unmatched: nulls, duplicate: many, nulls_match: false).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let cte_built = coveql::CoveQlQueryBuilder::table("people")
        .with_table("right", "people_right", None::<&str>)
        .join_table(
            "right",
            Some("r"),
            "people.id == r.id",
            coveql::TableJoinKind::Inner,
        )
        .select(["people.id"]);
    let parsed_built = cte_built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "table(people).with(right: table(people_right)).join(table(right) as r, on: people.id == r.id, kind: inner).select(people.id)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let relationship = coveql::coveql_relationship_expr_to_node(
        coveql::AstAssociationDirection::Out,
        "CustomerPlacedOrder",
        Some("placed"),
        "Order",
        Some("o"),
    );
    let graph_built = coveql::CoveQlQueryBuilder::node("Customer")
        .alias("c")
        .traverse(relationship)
        .select(["customer: c.goid", "order: o.goid"]);
    let parsed_built = graph_built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "node(Customer) as c.traverse(out(edge(CustomerPlacedOrder) as placed).to(node(Order) as o)).select(customer: c.goid, order: o.goid)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let algorithm_built = coveql::CoveQlQueryBuilder::node("Customer")
        .alias("c")
        .degree(Some("out(edge(CustomerPlacedOrder))"))
        .select(["c.goid", "degree"]);
    let parsed_built = algorithm_built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "node(Customer) as c.degree(out(edge(CustomerPlacedOrder))).select(c.goid, degree)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let algorithm_variant_built = coveql::CoveQlQueryBuilder::node("Customer")
        .alias("c")
        .degree_kind(Some("out(edge(CustomerPlacedOrder))"), "total")
        .centrality_kind(Some("out(edge(CustomerPlacedOrder))"), "degree")
        .spanning_tree_kind(Some("out(edge(CustomerPlacedOrder))"), "dfs")
        .select(["c.goid", "degree", "centrality", "tree_depth"]);
    let parsed_built = algorithm_variant_built
        .parse(ParseOptions::default())
        .unwrap();
    let parsed_handwritten = coveql::parse_query(
        "node(Customer) as c.degree(out(edge(CustomerPlacedOrder)), kind: total).centrality(out(edge(CustomerPlacedOrder)), kind: degree).spanningTree(out(edge(CustomerPlacedOrder)), kind: dfs).select(c.goid, degree, centrality, tree_depth)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );
}

#[test]
fn parsed_query_canonical_query_round_trips_ast_fingerprint() {
    let cases = [
        concat!(
            r#"Person.where((active == true) && "#,
            r#"(goid in [uuid"00000000-0000-0000-0000-000000000001"]))."#,
            r#"select(flag: if(active, "yes", "no"), payload: x"48656c6c6f")."#,
            r#"asOf(csn: 7).orderBy(flag, desc, nulls_last)."#,
            r#"take(10).skip(2).explain(coded)"#
        ),
        concat!(
            r#"association(CustomerPlacedOrder, from: customer)."#,
            r#"where(source_goid.isNotNull())."#,
            r#"history(mode: records_and_states)."#,
            r#"select(source_goid, target_goid)"#
        ),
        concat!(
            r#"evidence(out(association(CustomerPlacedOrder, to: order)), grain: row)."#,
            r#"where(source_id == "crm")."#,
            r#"changes(from: 1, to: 3, mode: property_diffs)."#,
            r#"select(source_id, row_count: count(*))"#
        ),
        concat!(
            r#"projection(`people-projection`).where(name != "Ada")."#,
            r#"groupBy(name).select(name, n: count(*)).explain("proof")"#
        ),
    ];

    for query in cases {
        let parsed = coveql::parse_query(query, ParseOptions::default()).unwrap();
        let rendered = parsed.to_canonical_query();
        let reparsed = coveql::parse_query(
            &rendered,
            ParseOptions {
                allow_implicit_language_version: false,
                required_language_version: Some(coveql::COVEQL_LANGUAGE_VERSION.into()),
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            reparsed.parsed_ast_fingerprint, parsed.parsed_ast_fingerprint,
            "canonical query did not preserve parsed AST fingerprint: {rendered}"
        );
    }
}

#[test]
fn query_builder_matches_default_method_syntax() {
    let built = coveql::CoveQlQueryBuilder::object("Thing")
        .history_default()
        .order_by_default("active")
        .explain_default();
    let parsed_built = built.parse(ParseOptions::default()).unwrap();
    let parsed_handwritten = coveql::parse_query(
        "Thing.history().orderBy(active).explain()",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        parsed_built.parsed_ast_fingerprint,
        parsed_handwritten.parsed_ast_fingerprint
    );

    let changes_built = coveql::CoveQlQueryBuilder::object("Thing").changes_csn_default(1, 3);
    let parsed_changes_built = changes_built.parse(ParseOptions::default()).unwrap();
    let parsed_changes_handwritten =
        coveql::parse_query("Thing.changes(from: 1, to: 3)", ParseOptions::default()).unwrap();
    assert_eq!(
        parsed_changes_built.parsed_ast_fingerprint,
        parsed_changes_handwritten.parsed_ast_fingerprint
    );

    let branch_tombstone_built = coveql::CoveQlQueryBuilder::object("Thing")
        .branch_reject_ambiguous()
        .include_tombstones_enabled()
        .explain_coded()
        .parse(ParseOptions::default())
        .unwrap();
    let branch_tombstone_handwritten = coveql::parse_query(
        "Thing.branch(reject_ambiguous).includeTombstones(true).explain(\"coded\")",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        branch_tombstone_built.parsed_ast_fingerprint,
        branch_tombstone_handwritten.parsed_ast_fingerprint
    );

    let as_of_time_built = coveql::CoveQlQueryBuilder::object("Thing")
        .as_of_time("2026-01-01T00:00:00Z")
        .parse(ParseOptions::default())
        .unwrap();
    let as_of_time_handwritten = coveql::parse_query(
        r#"Thing.asOf(time: "2026-01-01T00:00:00Z")"#,
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        as_of_time_built.parsed_ast_fingerprint,
        as_of_time_handwritten.parsed_ast_fingerprint
    );

    for (built, handwritten) in [
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .history_records()
                .to_query(),
            "Thing.history(mode: records)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .history_states()
                .to_query(),
            "Thing.history(mode: states)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .history_records_and_states()
                .to_query(),
            "Thing.history(mode: records_and_states)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_csn_records(1, 3)
                .to_query(),
            "Thing.changes(from: 1, to: 3, mode: records)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_csn_state_transitions(1, 3)
                .to_query(),
            "Thing.changes(from: 1, to: 3, mode: state_transitions)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_csn_property_diffs(1, 3)
                .to_query(),
            "Thing.changes(from: 1, to: 3, mode: property_diffs)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_csn_final_objects(1, 3)
                .to_query(),
            "Thing.changes(from: 1, to: 3, mode: final_objects)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_timestamp_records(
                    coveql::AstTimeRole::ValidTime,
                    "2026-01-01T00:00:00Z",
                    "2026-01-02T00:00:00Z",
                )
                .to_query(),
            r#"Thing.changes(valid_time: "2026-01-01T00:00:00Z", valid_time: "2026-01-02T00:00:00Z", mode: records)"#,
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_timestamp_state_transitions(
                    coveql::AstTimeRole::ObservedTime,
                    "2026-01-01T00:00:00Z",
                    "2026-01-02T00:00:00Z",
                )
                .to_query(),
            r#"Thing.changes(observed_time: "2026-01-01T00:00:00Z", observed_time: "2026-01-02T00:00:00Z", mode: state_transitions)"#,
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_timestamp_property_diffs(
                    coveql::AstTimeRole::SourceEventTime,
                    "2026-01-01T00:00:00Z",
                    "2026-01-02T00:00:00Z",
                )
                .to_query(),
            r#"Thing.changes(source_event_time: "2026-01-01T00:00:00Z", source_event_time: "2026-01-02T00:00:00Z", mode: property_diffs)"#,
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_time_final_objects("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z")
                .to_query(),
            r#"Thing.changes(time: "2026-01-01T00:00:00Z", time: "2026-01-02T00:00:00Z", mode: final_objects)"#,
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_timestamp_final_objects(
                    coveql::AstTimeRole::AssociationValidTime,
                    "2026-01-01T00:00:00Z",
                    "2026-01-02T00:00:00Z",
                )
                .to_query(),
            r#"Thing.changes(association_valid_time: "2026-01-01T00:00:00Z", association_valid_time: "2026-01-02T00:00:00Z", mode: final_objects)"#,
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_bounds_records(
                    coveql::AstChangeBound::Csn(1),
                    coveql::AstChangeBound::Csn(3),
                )
                .to_query(),
            "Thing.changes(csn: 1, csn: 3, mode: records)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_bounds_state_transitions(
                    coveql::AstChangeBound::Csn(1),
                    coveql::AstChangeBound::Csn(3),
                )
                .to_query(),
            "Thing.changes(csn: 1, csn: 3, mode: state_transitions)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_bounds_property_diffs(
                    coveql::AstChangeBound::Csn(1),
                    coveql::AstChangeBound::Csn(3),
                )
                .to_query(),
            "Thing.changes(csn: 1, csn: 3, mode: property_diffs)",
        ),
        (
            coveql::CoveQlQueryBuilder::object("Thing")
                .changes_bounds_final_objects(
                    coveql::AstChangeBound::Csn(1),
                    coveql::AstChangeBound::Csn(3),
                )
                .to_query(),
            "Thing.changes(csn: 1, csn: 3, mode: final_objects)",
        ),
    ] {
        let parsed_built = coveql::parse_query(&built, ParseOptions::default()).unwrap();
        let parsed_handwritten = coveql::parse_query(handwritten, ParseOptions::default()).unwrap();
        assert_eq!(
            parsed_built.parsed_ast_fingerprint, parsed_handwritten.parsed_ast_fingerprint,
            "{built} should match {handwritten}"
        );
    }
}

#[test]
fn prefix_explain_coded_matches_method_explain_syntax() {
    let prefixed = coveql::parse_query(
        "EXPLAIN CODED Person.where(active == true).select(active)",
        ParseOptions::default(),
    )
    .unwrap();
    let method = coveql::parse_query(
        r#"Person.where(active == true).select(active).explain("coded")"#,
        ParseOptions::default(),
    )
    .unwrap();
    let uppercase_method = coveql::parse_query(
        r#"Person.where(active == true).select(active).explain("CODED")"#,
        ParseOptions::default(),
    )
    .unwrap();

    assert_eq!(
        prefixed.parsed_ast_fingerprint,
        method.parsed_ast_fingerprint
    );
    assert_eq!(
        uppercase_method.parsed_ast_fingerprint,
        method.parsed_ast_fingerprint
    );

    let object_named_coded =
        coveql::parse_query("EXPLAIN coded.select(active)", ParseOptions::default()).unwrap();
    let explicit_public =
        coveql::parse_query("coded.select(active).explain()", ParseOptions::default()).unwrap();
    assert_eq!(
        object_named_coded.parsed_ast_fingerprint,
        explicit_public.parsed_ast_fingerprint
    );
}

#[test]
fn query_builder_matches_grouped_aggregate_syntax() {
    let count_built = coveql::CoveQlQueryBuilder::object("Person")
        .group_by_count_star_as(["active"], "n")
        .parse(ParseOptions::default())
        .unwrap();
    let count_handwritten = coveql::parse_query(
        "Person.groupBy(active).select(active, n: count(*))",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        count_built.parsed_ast_fingerprint,
        count_handwritten.parsed_ast_fingerprint
    );

    let sum_built = coveql::CoveQlQueryBuilder::object("Person")
        .group_by_aggregate_as(
            ["active"],
            "total_score",
            coveql::AstAggregateName::Sum,
            "score",
        )
        .parse(ParseOptions::default())
        .unwrap();
    let sum_handwritten = coveql::parse_query(
        "Person.groupBy(active).select(active, total_score: sum(score))",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        sum_built.parsed_ast_fingerprint,
        sum_handwritten.parsed_ast_fingerprint
    );

    let select_count_built = coveql::CoveQlQueryBuilder::object("Person")
        .select_count_star_as("n")
        .parse(ParseOptions::default())
        .unwrap();
    let select_count_handwritten =
        coveql::parse_query("Person.select(n: count(*))", ParseOptions::default()).unwrap();
    assert_eq!(
        select_count_built.parsed_ast_fingerprint,
        select_count_handwritten.parsed_ast_fingerprint
    );

    let select_exists_built = coveql::CoveQlQueryBuilder::object("Person")
        .select_star_aggregate_as("e", coveql::AstAggregateName::Exists)
        .parse(ParseOptions::default())
        .unwrap();
    let select_exists_handwritten =
        coveql::parse_query("Person.select(e: exists(*))", ParseOptions::default()).unwrap();
    assert_eq!(
        select_exists_built.parsed_ast_fingerprint,
        select_exists_handwritten.parsed_ast_fingerprint
    );

    let distinct_built = coveql::CoveQlQueryBuilder::object("Person")
        .select_aggregate_as("d", coveql::AstAggregateName::DistinctCount, "active")
        .parse(ParseOptions::default())
        .unwrap();
    let distinct_handwritten = coveql::parse_query(
        "Person.select(d: distinct_count(active))",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        distinct_built.parsed_ast_fingerprint,
        distinct_handwritten.parsed_ast_fingerprint
    );

    let grouped_exists_built = coveql::CoveQlQueryBuilder::object("Person")
        .group_by_star_aggregate_as(["active"], "e", coveql::AstAggregateName::Exists)
        .parse(ParseOptions::default())
        .unwrap();
    let grouped_exists_handwritten = coveql::parse_query(
        "Person.groupBy(active).select(active, e: exists(*))",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        grouped_exists_built.parsed_ast_fingerprint,
        grouped_exists_handwritten.parsed_ast_fingerprint
    );

    let grouped_distinct_built = coveql::CoveQlQueryBuilder::object("Person")
        .group_by_aggregate_as(
            ["active"],
            "d",
            coveql::AstAggregateName::DistinctCount,
            "score",
        )
        .parse(ParseOptions::default())
        .unwrap();
    let grouped_distinct_handwritten = coveql::parse_query(
        "Person.groupBy(active).select(active, d: distinct_count(score))",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        grouped_distinct_built.parsed_ast_fingerprint,
        grouped_distinct_handwritten.parsed_ast_fingerprint
    );
}

#[test]
fn query_builder_supports_evidence_projection_shorthand() {
    let parsed = coveql::CoveQlQueryBuilder::evidence_projection_with_grain(
        "people_projection",
        coveql::AstEvidenceGrain::Row,
    )
    .select(["source_id"])
    .parse(ParseOptions::default())
    .unwrap();

    let coveql::AstRoot::Evidence(evidence) = parsed.root.node else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, Some(coveql::AstEvidenceGrain::Row));
    assert!(matches!(
        evidence.target,
        Some(coveql::AstEvidenceTarget::Projection(_))
    ));
}

#[test]
fn parser_supports_evidence_root_binding_targets() {
    let parsed = coveql::parse_query(
        r#"evidence(table("order-history") as o, grain: row).select(source_id)"#,
        ParseOptions::default(),
    )
    .unwrap();
    let coveql::AstRoot::Evidence(evidence) = parsed.root.node else {
        panic!("expected evidence root");
    };
    let Some(coveql::AstEvidenceTarget::RootBinding(binding)) = evidence.target else {
        panic!("expected root-binding evidence target");
    };
    assert!(matches!(binding.root.node, coveql::AstRoot::Table(_)));
    assert_eq!(binding.alias.unwrap().name, "o");
    assert_eq!(evidence.grain, Some(coveql::AstEvidenceGrain::Row));
}

#[test]
fn query_builder_matches_evidence_root_binding_target() {
    let built = coveql::CoveQlQueryBuilder::evidence_root_binding_with_grain(
        "table(thing_projection) as t",
        coveql::AstEvidenceGrain::Row,
    )
    .select(["source_id"])
    .parse(ParseOptions::default())
    .unwrap();
    let handwritten = coveql::parse_query(
        "evidence(table(thing_projection) as t, grain: row).select(source_id)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        built.parsed_ast_fingerprint,
        handwritten.parsed_ast_fingerprint
    );
}

#[test]
fn evidence_root_binding_targets_resolve_through_profile_root_context() {
    let resolved = parse_and_resolve_query(
        &minimal_object_with_evidence_index_file(),
        "evidence(object(Person), grain: object).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedRoot::Evidence(evidence) = &resolved.root else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Object);
    assert!(matches!(
        evidence.target,
        Some(coveql::ResolvedEvidenceTarget::ObjectType { .. })
    ));

    let resolved = parse_and_resolve_query(
        &minimal_object_with_evidence_index_file(),
        "evidence(node(Person), grain: node).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedRoot::Evidence(evidence) = &resolved.root else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Node);
    assert!(matches!(
        evidence.target,
        Some(coveql::ResolvedEvidenceTarget::GraphNode { .. })
    ));
}

#[test]
fn table_profile_evidence_targets_preserve_row_and_column_grain() {
    let resolved = parse_and_resolve_query(
        &minimal_object_with_projection_and_evidence_index_file(),
        "evidence(table(people_projection) as p, grain: row).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedRoot::Evidence(evidence) = &resolved.root else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Row);
    assert!(matches!(
        evidence.target,
        Some(coveql::ResolvedEvidenceTarget::TableRow { .. })
    ));

    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let resolved = parse_and_resolve_query(
        &minimal_object_with_projection_and_evidence_index_file(),
        "table(people_projection) as p.select(evidence_count: count(evidence(p.active)))",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.as_ref().unwrap();
    let coveql::ResolvedExpr::AggregateCall { arg: Some(arg), .. } = &select[0].expr else {
        panic!("expected evidence aggregate");
    };
    let coveql::ResolvedExpr::Evidence(evidence) = arg.as_ref() else {
        panic!("expected evidence helper");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Column);
    assert!(matches!(
        evidence.target,
        Some(coveql::ResolvedEvidenceTarget::TableColumn { .. })
    ));
}

#[test]
fn graph_profile_evidence_targets_preserve_edge_grain() {
    let resolved = parse_and_resolve_query(
        &association_file_with_evidence_entries(Vec::new()),
        "evidence(edge(CustomerPlacedOrder) as e, grain: edge).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedRoot::Evidence(evidence) = &resolved.root else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Edge);
    assert!(matches!(
        evidence.target,
        Some(coveql::ResolvedEvidenceTarget::GraphEdge { .. })
    ));
}

#[test]
fn root_property_evidence_target_resolves_object_property_path() {
    let resolved = parse_and_resolve_query(
        &minimal_object_with_evidence_index_file(),
        "evidence(Person.active, grain: property).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let ResolvedRoot::Evidence(evidence) = &resolved.root else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, coveql::AstEvidenceGrain::Property);
    let Some(coveql::ResolvedEvidenceTarget::Property {
        object_type_id,
        property_id,
        property_name,
    }) = &evidence.target
    else {
        panic!("expected property evidence target");
    };
    assert_eq!(*object_type_id, Some(1));
    assert_eq!(*property_id, 1);
    assert_eq!(property_name, "active");
}

#[test]
fn parser_supports_directed_association_root() {
    let parsed = coveql::parse_query(
        "out(association(CustomerPlacedOrder, from: customer)).select(source_goid)",
        ParseOptions::default(),
    )
    .unwrap();

    let coveql::AstRoot::Association(association) = parsed.root.node else {
        panic!("expected association root");
    };
    assert_eq!(
        association.direction,
        Some(coveql::AstAssociationDirection::Out)
    );
    assert_eq!(association.role, Some(coveql::AstAssociationRole::From));
    assert_eq!(association.role_name.unwrap().name, "customer");
}

#[test]
fn query_builder_supports_directed_evidence_association_shorthand() {
    let parsed = coveql::CoveQlQueryBuilder::evidence_association_with_direction_role_and_grain(
        coveql::AstAssociationDirection::In,
        "CustomerPlacedOrder",
        coveql::AstAssociationRole::To,
        "order",
        coveql::AstEvidenceGrain::Association,
    )
    .select(["source_id"])
    .parse(ParseOptions::default())
    .unwrap();

    let coveql::AstRoot::Evidence(evidence) = parsed.root.node else {
        panic!("expected evidence root");
    };
    assert_eq!(evidence.grain, Some(coveql::AstEvidenceGrain::Association));
    let Some(coveql::AstEvidenceTarget::Association(association)) = evidence.target else {
        panic!("expected association evidence target");
    };
    assert_eq!(
        association.direction,
        Some(coveql::AstAssociationDirection::In)
    );
    assert_eq!(association.role, Some(coveql::AstAssociationRole::To));
    assert_eq!(association.role_name.unwrap().name, "order");
}

#[test]
fn query_builder_matches_directed_evidence_association_without_grain() {
    let built = coveql::CoveQlQueryBuilder::evidence_association_with_direction_and_role(
        coveql::AstAssociationDirection::Out,
        "CustomerPlacedOrder",
        coveql::AstAssociationRole::From,
        "customer",
    )
    .select(["source_id"])
    .parse(ParseOptions::default())
    .unwrap();
    let handwritten = coveql::parse_query(
        "evidence(out(association(CustomerPlacedOrder, from: customer))).select(source_id)",
        ParseOptions::default(),
    )
    .unwrap();

    assert_eq!(
        built.parsed_ast_fingerprint,
        handwritten.parsed_ast_fingerprint
    );
}

#[test]
fn parser_accepts_coveql_profile_directives_aliases_and_explicit_object_root() {
    let parsed = coveql::parse_query(
        "# coveql: 0.1\n# profiles: object, table\nobject(Person) as p.where(p.active == true).select(p.active)",
        ParseOptions::default(),
    )
    .unwrap();

    assert_eq!(
        parsed.profiles,
        vec![
            coveql::CoveQlProfileId::Object,
            coveql::CoveQlProfileId::Table
        ]
    );
    assert_eq!(parsed.root_alias.as_ref().unwrap().name, "p");
    assert!(matches!(parsed.root.node, coveql::AstRoot::Object(_)));
    assert!(parsed.to_canonical_query().contains("object(Person) as p"));

    let explicit = coveql::parse_query(
        "object(Person).where(active == true)",
        ParseOptions::default(),
    )
    .unwrap();
    let legacy =
        coveql::parse_query("Person.where(active == true)", ParseOptions::default()).unwrap();
    assert_eq!(
        explicit.parsed_ast_fingerprint,
        legacy.parsed_ast_fingerprint
    );
}

#[test]
fn parser_accepts_table_and_graph_profile_constructs() {
    let table = coveql::parse_query(
        "table(orders) as o.lookup(table(customers) as c, on: o.customer_id == c.customer_id).select(o.id)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(table.profiles, vec![coveql::CoveQlProfileId::Table]);
    assert!(matches!(table.root.node, coveql::AstRoot::Table(_)));
    assert_eq!(table.root_alias.as_ref().unwrap().name, "o");
    assert!(matches!(
        table.methods[0].node,
        coveql::AstMethod::ProfileCall { .. }
    ));

    let graph = coveql::parse_query(
        "node(Customer) as c.traverse(out(edge(CustomerPlacedOrder) as placed).to(node(Order) as o)).select(c.goid)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(graph.profiles, vec![coveql::CoveQlProfileId::Graph]);
    assert!(matches!(graph.root.node, coveql::AstRoot::Node(_)));
    assert!(matches!(
        graph.methods[0].node,
        coveql::AstMethod::ProfileCall { .. }
    ));

    let path = coveql::parse_query(
        "path(node(Customer) as c.out(edge(CustomerPlacedOrder)).to(node(Order) as o))",
        ParseOptions::default(),
    )
    .unwrap();
    assert!(matches!(path.root.node, coveql::AstRoot::Path(_)));
}

#[test]
fn parser_accepts_coveql_ai_profile_methods_and_explain_mode() {
    let parsed = coveql::parse_query(
        "# coveql: 0.1\n# profiles: table, ai\ntable(people).embedding(id).similar(query: \"alice\", k: 3).explain(ai)",
        ParseOptions::default(),
    )
    .unwrap();

    assert_eq!(
        parsed.profiles,
        vec![coveql::CoveQlProfileId::Table, coveql::CoveQlProfileId::Ai]
    );
    assert!(matches!(parsed.root.node, coveql::AstRoot::Table(_)));
    assert!(matches!(
        parsed.methods[0].node,
        coveql::AstMethod::ProfileCall { .. }
    ));
    assert!(matches!(
        parsed.methods.last().map(|method| &method.node),
        Some(coveql::AstMethod::Explain(ExplainMode::Ai))
    ));
    assert!(parsed
        .to_canonical_query()
        .contains("# profiles: table, ai"));
    assert!(parsed.to_canonical_query().contains("explain(ai)"));
}

#[test]
fn parser_rejects_invalid_path_start_and_canonicalizes_final_rows() {
    let err = coveql::parse_query(
        "path(table(orders).out(edge(CustomerPlacedOrder)))",
        ParseOptions::default(),
    )
    .unwrap_err();
    assert_eq!(err[0].code, "E_PARSE");

    let legacy = coveql::parse_query(
        "object(Person).changes(from: 1, to: 3, mode: final_objects)",
        ParseOptions::default(),
    )
    .unwrap();
    let canonical = coveql::parse_query(
        "object(Person).changes(from: 1, to: 3, mode: final_rows)",
        ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(
        legacy.parsed_ast_fingerprint,
        canonical.parsed_ast_fingerprint
    );
    assert!(legacy.to_canonical_query().contains("mode: final_rows"));
}

#[test]
fn resolver_executes_projection_backed_table_surface() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "table(thing_projection) as t.where(t.active == true).select(t.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        executed.planned.resolved.parsed.profiles,
        vec![coveql::CoveQlProfileId::Table]
    );
    let ResolvedRoot::Table(table) = &executed.planned.resolved.root else {
        panic!("expected resolved table root");
    };
    assert_eq!(table.table_name, "thing_projection");
    assert_eq!(table.binding_name.as_deref(), Some("t"));
    assert_eq!(
        table.authority_kind,
        coveql::TableSurfaceAuthorityKind::DeterministicProjection
    );
    assert_eq!(table.table_id, "projection:thing_projection");
    assert_eq!(table.row_grain, "one_row_per_object");
    assert!(!table.row_identity.is_empty());
    assert_eq!(table.canonical_order, table.row_identity);
    assert_eq!(
        table.temporal_authority,
        coveql::TableTemporalAuthority::MaterializedSnapshotOnly
    );
    assert_eq!(table.table_surface_contract.table_id, table.table_id);
    assert_eq!(
        table
            .table_surface_contract
            .projection_dependency_contract_id
            .as_deref(),
        Some("thing_projection")
    );
    assert!(table
        .table_surface_contract
        .logical_column_map
        .iter()
        .any(|column| column.name == "active"));
    assert_eq!(table.projection.projection_id, "thing_projection");
    assert_eq!(
        executed.planned.resolved.output_mode,
        CoveQlOutputMode::JsonRows
    );
    assert_eq!(executed.pushdown_report.outcome, PushdownOutcome::Applied);
    assert_eq!(
        executed
            .pushdown_report
            .counters
            .property_predicate_candidates,
        1
    );

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected table JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["active"], json!(true));
}

#[test]
fn table_surface_contract_override_must_prove_row_identity() {
    let mut contract = projection_backed_thing_table_contract();
    contract.row_identity.clear();
    let mut resolve_options = ResolveOptions::default();
    resolve_options
        .table_surface_contracts
        .insert("thing_projection".into(), contract);
    let err = parse_and_resolve_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "table(thing_projection).select(active)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_TABLE_ROW_IDENTITY_MISSING");
    assert!(err.diagnostics[0].message.contains("row_identity"));
}

#[test]
fn registered_materialized_table_authority_executes_without_projection_catalog() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false)]),
            },
        ),
    );

    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(people).where(active == true).select(id, active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let ResolvedRoot::Table(table) = &executed.planned.resolved.root else {
        panic!("expected table root");
    };
    assert_eq!(
        table.authority_kind,
        coveql::TableSurfaceAuthorityKind::MaterializedTable
    );
    assert_eq!(table.projection.projection_id, "table:people");
    let explain = executed.explain_json();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected materialized table JSON rows");
    };
    assert_eq!(rows, vec![json!({"id": "a", "active": true})]);
    let table_contract = &explain["profile_contracts"][0]["query_contract"];
    assert_eq!(
        table_contract["authority_kind"],
        json!("materialized_table")
    );
    assert_eq!(table_contract["authority_fingerprint"], json!("people:v1"));
    assert_eq!(
        table_contract["execution_authority"],
        json!({
            "kind": "materialized_rows",
            "row_count": 2,
        })
    );
}

#[test]
fn coveql_ai_methods_select_operation_scoped_feature_use_and_plan_nodes() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false)]),
            },
        ),
    );

    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "# profiles: table, ai\ntable(people).embedding(id).similar(query: \"alice\", k: 3).select(id)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(matches!(
        physical
            .planned
            .resolved
            .operation_context
            .request
            .selected_operation,
        CoveQlSelectedOperation::Ai {
            operation: coveql::CoveQlAiOperation::SemanticSearch,
            ..
        }
    ));
    assert!(physical
        .planned
        .resolved
        .operation_context
        .selected_feature_uses
        .iter()
        .any(|feature_use| feature_use.requested_operation
            == Some(cove_core::feature_binding::OperationKindV2::AiSemanticSearch)));
    assert_eq!(
        physical.planned.resolved.method_chain.ai_operations.len(),
        2
    );
    assert!(physical.planned.logical_plan.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            LogicalPlanNodeKind::AiOperation {
                operations,
                sidecar_required: true,
                ..
            } if operations
                .iter()
                .any(|operation| operation.operation == coveql::CoveQlAiOperation::SemanticSearch)
        )
    }));
    assert!(physical.physical_plan.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            PhysicalPlanNodeKind::AiSidecarOperation {
                operations,
                sidecar_required: true,
                ..
            } if operations.contains(&coveql::CoveQlAiOperation::SemanticSearch)
        )
    }));
}

#[test]
fn coveql_ai_similar_executes_exact_flat_filecode_sidecar() {
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0, 0.8, 0.2, 0.0, -1.0, 0.0, 0.0] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let covev = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [42u8; 16],
        created_at_us: 1_000,
        dimension_count: 3,
        file_codes: vec![10, 20, 30],
        vector_payload,
    })
    .unwrap();

    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false)]),
            },
        ),
    );
    let mut physical_options = PhysicalPlanOptions::default();
    physical_options.sidecars.cove_ai_artifact_bytes = Some(covev);

    let executed = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_file(),
        "# profiles: table, ai\ntable(people).similar(fileCode: 10, k: 2)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        physical_options,
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        executed.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.executed.result else {
        panic!("expected JSON rows from CoveQL-AI semantic search");
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["file_code"], json!(10));
    assert_eq!(rows[0]["rank"], json!(1));
    assert_eq!(rows[0]["exact"], json!(true));
    assert_eq!(rows[1]["file_code"], json!(20));
    assert!(executed
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_EXACT_FLAT_VECTOR_SCAN_EXECUTED"));
}

#[test]
fn coveql_ai_similar_executes_asset_and_multimodal_vector_targets() {
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: vector_payload.len() as u64,
        decoded_length: vector_payload.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.vector_spaces.push(VectorSpaceDescriptorV1 {
        vector_space_id: 1,
        vector_space_name_ref: 0,
        vector_space_fingerprint_ref: 0,
        embedding_namespace_ref: 0,
        embedding_model_ref: 0,
        embedding_model_version_ref: 0,
        embedding_model_digest_ref: 0,
        embedding_pipeline_ref: 0,
        tokenizer_profile_ref: 0,
        chunk_profile_ref: 0,
        dimension_count: 3,
        element_type: 0,
        metric: 1,
        normalization_policy: 0,
        quantization_policy: 0,
        deterministic: 1,
        approximate: 0,
        reproducibility_class: 1,
        reserved: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .vector_payload_blocks
        .push(VectorPayloadBlockHeaderV1 {
            block_id: 1,
            vector_space_id: 1,
            vector_count: 2,
            dimension_count: 3,
            element_type: 0,
            compression_codec: 0,
            quantization_kind: 0,
            layout_kind: 0,
            tensor_layout_ref: 0,
            memory_alignment_bytes: 0,
            payload_stride_ref: 0,
            device_transfer_hint_ref: 0,
            payload_ref: 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            checksum: 0,
        });
    for vector_ref in 1..=2 {
        tables.vector_entries.push(VectorEntryV1 {
            vector_ref,
            block_id: 1,
            vector_ordinal: vector_ref - 1,
            payload_offset: 0,
            payload_length: 0,
            integrity_ref: 0,
            flags: 0,
            checksum: 0,
        });
    }
    tables.assets.push(AiAssetRefV1 {
        asset_ref_id: 1,
        parent_asset_ref: 0,
        asset_kind: 2,
        uri_ref: 0,
        embedded_section_ref: 0,
        media_type_ref: 0,
        byte_length: 0,
        digest_ref: 0,
        width: 64,
        height: 64,
        duration_us: 0,
        sample_rate_hz: 0,
        channel_count: 0,
        decode_profile_ref: 0,
        preprocessing_profile_ref: 0,
        transform_profile_ref: 0,
        transform_digest_ref: 0,
        tensor_layout_ref: 0,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.asset_vector_bindings.push(AssetVectorBindingV1 {
        binding_id: 1,
        vector_space_id: 1,
        asset_ref: 1,
        transform_ref: 0,
        asset_digest_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .multimodal_sequence_packs
        .push(MultimodalSequencePackV1 {
            sequence_pack_id: 1,
            training_profile_id: 0,
            tokenizer_profile_id: 0,
            sequence_profile_ref: 0,
            element_count: 0,
            first_element_ref: 0,
            split_ref: 0,
            sample_weight_ppm: 1_000_000,
            loss_mask_ref: 0,
            attention_mask_ref: 0,
            position_map_ref: 0,
            label_ref: 0,
            source_snapshot_ref: 0,
            evidence_ref: 0,
            generator_provenance_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .multimodal_sequence_vector_bindings
        .push(MultimodalSequenceVectorBindingV1 {
            binding_id: 2,
            vector_space_id: 1,
            sequence_pack_id: 1,
            sequence_profile_ref: 0,
            source_snapshot_ref: 0,
            vector_ref: 2,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .association_state_vector_bindings
        .push(AssociationStateVectorBindingV1 {
            binding_id: 3,
            vector_space_id: 1,
            composition_profile_ref: 0,
            file_ref: 0,
            association_type_id: 7,
            association_key_ref: 0,
            branch_ref: 0,
            temporal_kind: 0,
            csn: 0,
            timestamp_us: 0,
            property_dependency_fingerprint_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });
    let sidecar = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [47u8; 16],
        created_at_us: 1_004,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 10,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveVec as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vector_payload,
        }],
        descriptor_tables: tables,
    })
    .unwrap();

    for (target, vector_ref, target_kind, ref_column, ref_value) in [
        ("asset", 1, "asset", "asset_ref", json!(1)),
        (
            "multimodal",
            2,
            "multimodal_sequence",
            "multimodal_sequence_pack_id",
            json!(1),
        ),
        (
            "association",
            1,
            "association_state",
            "association_type_id",
            json!(7),
        ),
    ] {
        let mut resolve_options = ResolveOptions::default();
        resolve_options.table_authorities.insert(
            "people".into(),
            registered_people_table_authority(
                coveql::TableSurfaceAuthorityKind::MaterializedTable,
                coveql::TableExecutionAuthority::MaterializedRows {
                    rows: people_rows(&[("a", true)]),
                },
            ),
        );
        let mut physical_options = PhysicalPlanOptions::default();
        physical_options.sidecars.cove_ai_artifact_bytes = Some(sidecar.clone());

        let executed = parse_resolve_plan_build_physical_and_execute_query(
            &minimal_object_file(),
            &format!(
                "# profiles: table, ai\ntable(people).similar(vectorRef: {vector_ref}, k: 1, target: \"{target}\")"
            ),
            ParseOptions::default(),
            resolve_options,
            PlanOptions::default(),
            physical_options,
            ExecutionOptions::default(),
            KernelExecutionOptions::default(),
            validation_options(),
        )
        .unwrap();
        let CoveQlExecutionResult::JsonRows(rows) = executed.executed.result else {
            panic!("expected JSON rows from CoveQL-AI {target} search");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["target_kind"], json!(target_kind));
        assert_eq!(rows[0][ref_column], ref_value);
        assert_eq!(rows[0]["exact"], json!(true));
    }
}

#[test]
fn coveql_ai_hybrid_and_rerank_execute_advisory_vector_scan() {
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0, 0.8, 0.2, 0.0] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let covev = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [45u8; 16],
        created_at_us: 1_003,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload,
    })
    .unwrap();

    for method in ["hybrid", "rerank"] {
        let mut resolve_options = ResolveOptions::default();
        resolve_options.table_authorities.insert(
            "people".into(),
            registered_people_table_authority(
                coveql::TableSurfaceAuthorityKind::MaterializedTable,
                coveql::TableExecutionAuthority::MaterializedRows {
                    rows: people_rows(&[("a", true)]),
                },
            ),
        );
        let mut physical_options = PhysicalPlanOptions::default();
        physical_options.sidecars.cove_ai_artifact_bytes = Some(covev.clone());

        let executed = parse_resolve_plan_build_physical_and_execute_query(
            &minimal_object_file(),
            &format!("# profiles: table, ai\ntable(people).{method}(fileCode: 10, k: 2)"),
            ParseOptions::default(),
            resolve_options,
            PlanOptions::default(),
            physical_options,
            ExecutionOptions::default(),
            KernelExecutionOptions::default(),
            validation_options(),
        )
        .unwrap();

        let CoveQlExecutionResult::JsonRows(rows) = executed.executed.result else {
            panic!("expected JSON rows from CoveQL-AI {method}");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["method"], json!(method));
        assert_eq!(rows[0]["exact"], json!(false));
        assert_eq!(rows[0]["vector_exact"], json!(true));
        assert_eq!(rows[0]["result_authority"], json!("RuntimeAdvisory"));
        assert_eq!(
            rows[0]["advisory_reason"],
            json!("no_persisted_hybrid_or_rerank_authority")
        );
        assert!(executed
            .executed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_AI_ADVISORY_VECTOR_SCAN_EXECUTED"));
    }
}

#[test]
fn coveql_ai_embedding_executes_filecode_vector_lookup() {
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0, 0.25, 0.5, 0.75] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let covev = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [43u8; 16],
        created_at_us: 1_001,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload,
    })
    .unwrap();

    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true)]),
            },
        ),
    );
    let mut physical_options = PhysicalPlanOptions::default();
    physical_options.sidecars.cove_ai_artifact_bytes = Some(covev);

    let executed = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_file(),
        "# profiles: table, ai\ntable(people).embedding(fileCode: 20)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        physical_options,
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        executed.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.executed.result else {
        panic!("expected JSON rows from CoveQL-AI embedding lookup");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["file_code"], json!(20));
    assert_eq!(rows[0]["vector_ref"], json!(2));
    assert_eq!(rows[0]["dimension_count"], json!(3));
    assert_eq!(rows[0]["embedding"], json!([0.25, 0.5, 0.75]));
    assert!(executed
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_FILECODE_EMBEDDING_EXECUTED"));
}

#[test]
fn coveql_ai_chunks_and_context_execute_descriptor_metadata() {
    let coveai = coveql_ai_descriptor_sidecar();

    let chunks = execute_ai_descriptor_query(
        coveai.clone(),
        "# profiles: table, ai\ntable(people).chunks()",
    );
    assert_eq!(
        chunks.kernel_report.decision.kind,
        KernelDecisionKind::Applied
    );
    let CoveQlExecutionResult::JsonRows(rows) = chunks.executed.result else {
        panic!("expected JSON rows from CoveQL-AI chunks");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["record_kind"], json!("text_chunk"));
    assert_eq!(rows[0]["chunk_id"], json!(1));
    assert_eq!(rows[0]["byte_start"], json!(4));
    assert_eq!(rows[0]["byte_length"], json!(12));
    assert_eq!(rows[0]["text"], Value::Null);
    assert_eq!(rows[0]["text_withheld"], json!(true));
    assert!(chunks
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_CHUNK_METADATA_EXECUTED"));

    let context =
        execute_ai_descriptor_query(coveai, "# profiles: table, ai\ntable(people).context()");
    let CoveQlExecutionResult::JsonRows(rows) = context.executed.result else {
        panic!("expected JSON rows from CoveQL-AI context");
    };
    assert_eq!(rows[0]["record_kind"], json!("rag_context_chunk"));
    assert_eq!(rows[0]["prompt_context"], Value::Null);
    assert_eq!(
        rows[0]["redaction_report"]["neighbor_chunks_withheld"],
        json!(true)
    );
    assert!(context
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_RAG_CONTEXT_METADATA_EXECUTED"));
}

#[test]
fn coveql_ai_chunks_and_context_reconstruct_text_from_validated_source() {
    let coveai = coveql_ai_source_bound_chunk_sidecar();
    let source = cove_o_chunk_source_file();

    let chunks = execute_ai_descriptor_query_on_source(
        &source,
        coveai.clone(),
        "# profiles: table, ai\ntable(people).chunks(includePayloads: true)",
    );
    let CoveQlExecutionResult::JsonRows(rows) = chunks.executed.result else {
        panic!("expected JSON rows from CoveQL-AI chunks");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["record_kind"], json!("text_chunk"));
    assert_eq!(rows[0]["text"], json!("chunk text"));
    assert_eq!(rows[0]["text_withheld"], json!(false));
    assert_eq!(
        rows[0]["payload_access"],
        json!("validated_source_value_reconstruction")
    );
    assert_eq!(
        rows[0]["source_value_hash_status"],
        json!("verified_sha256")
    );
    assert_eq!(rows[0]["chunk_text_hash_status"], json!("verified_sha256"));
    assert_eq!(
        rows[0]["result_authority"],
        json!("ValidatedAiSourceValueReconstruction")
    );

    let context = execute_ai_descriptor_query_on_source(
        &source,
        coveai,
        "# profiles: table, ai\ntable(people).context(includePayloads: true)",
    );
    let CoveQlExecutionResult::JsonRows(rows) = context.executed.result else {
        panic!("expected JSON rows from CoveQL-AI context");
    };
    assert_eq!(rows[0]["record_kind"], json!("rag_context_chunk"));
    assert_eq!(rows[0]["prompt_context"], json!("chunk text"));
    assert_eq!(rows[0]["redaction_report"]["text_withheld"], json!(false));
    assert_eq!(
        rows[0]["redaction_report"]["neighbor_chunks_withheld"],
        json!(true)
    );
}

#[test]
fn coveql_ai_token_training_pack_and_multimodal_execute_descriptor_metadata() {
    let coveai = coveql_ai_descriptor_sidecar();

    let tokens = execute_ai_descriptor_query(
        coveai.clone(),
        "# profiles: table, ai\ntable(people).tokens()",
    );
    let CoveQlExecutionResult::JsonRows(rows) = tokens.executed.result else {
        panic!("expected JSON rows from CoveQL-AI tokens");
    };
    let record_kinds = rows
        .iter()
        .map(|row| row["record_kind"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert!(record_kinds.contains("token_block"));
    assert!(record_kinds.contains("tokenized_span"));
    assert!(record_kinds.contains("token_sequence_pack"));
    assert!(rows.iter().any(|row| {
        row["record_kind"] == json!("token_block")
            && row["token_payload"]["payload_access"] != Value::Null
    }));
    assert!(tokens
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_TOKEN_METADATA_EXECUTED"));

    let samples = execute_ai_descriptor_query(
        coveai.clone(),
        "# profiles: table, ai\ntable(people).trainingSamples()",
    );
    let CoveQlExecutionResult::JsonRows(rows) = samples.executed.result else {
        panic!("expected JSON rows from CoveQL-AI training samples");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["record_kind"], json!("training_sample"));
    assert_eq!(rows[0]["sample_id"], json!(1));
    assert_eq!(rows[0]["policy_withheld"], json!(false));
    assert!(rows[0]["input"]["payload_access"] != Value::Null);
    assert!(samples
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_TRAINING_SAMPLE_METADATA_EXECUTED"));

    let pack = execute_ai_descriptor_query(
        coveai.clone(),
        "# profiles: table, ai\ntable(people).trainingSamples().split().pack()",
    );
    let CoveQlExecutionResult::JsonRows(rows) = pack.executed.result else {
        panic!("expected JSON rows from CoveQL-AI pack");
    };
    assert!(rows
        .iter()
        .any(|row| row["record_kind"] == json!("token_sequence_pack")));
    assert!(rows
        .iter()
        .any(|row| row["record_kind"] == json!("multimodal_sequence_pack")));
    assert!(pack
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_TRAINING_PACK_METADATA_EXECUTED"));

    let multimodal = execute_ai_descriptor_query(
        coveai.clone(),
        "# profiles: table, ai\ntable(people).multimodal()",
    );
    let CoveQlExecutionResult::JsonRows(rows) = multimodal.executed.result else {
        panic!("expected JSON rows from CoveQL-AI multimodal");
    };
    assert!(rows.iter().any(
        |row| row["record_kind"] == json!("multimodal_sequence_element")
            && row["asset_ref"] == json!(1)
            && row["tensor_ref"] == json!(1)
    ));
    assert!(multimodal
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_MULTIMODAL_METADATA_EXECUTED"));

    let generator = execute_ai_descriptor_query(
        coveai,
        "# profiles: table, ai\ntable(people).generatorAudit()",
    );
    let CoveQlExecutionResult::JsonRows(rows) = generator.executed.result else {
        panic!("expected JSON rows from CoveQL-AI generator audit");
    };
    let record_kinds = rows
        .iter()
        .map(|row| row["record_kind"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert!(record_kinds.contains("model_actor"));
    assert!(record_kinds.contains("generation_decoding_profile"));
    assert!(record_kinds.contains("human_review"));
    assert!(record_kinds.contains("generator_provenance"));
    assert!(record_kinds.contains("training_label"));
    assert!(record_kinds.contains("preference_pair"));
    assert!(rows.iter().any(|row| {
        row["record_kind"] == json!("generator_provenance")
            && row["external_audit_only_unless_deterministic_regeneration_proven"] == json!(true)
    }));
    assert!(generator
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "W_AI_GENERATOR_AUDIT_METADATA_EXECUTED"));
}

#[test]
fn coveql_ai_generator_audit_filters_descriptor_rows() {
    let matched = execute_ai_descriptor_query(
        coveql_ai_descriptor_sidecar(),
        "# profiles: table, ai\ntable(people).generatorAudit(reproducibilityClass: 2, humanReviewStatus: \"reviewed\")",
    );
    let CoveQlExecutionResult::JsonRows(rows) = matched.executed.result else {
        panic!("expected JSON rows from CoveQL-AI generator audit");
    };
    assert!(rows
        .iter()
        .any(|row| row["record_kind"] == json!("generator_provenance")));
    assert!(rows
        .iter()
        .any(|row| row["record_kind"] == json!("model_actor")));
    assert!(rows
        .iter()
        .any(|row| row["record_kind"] == json!("human_review")));
    assert!(rows
        .iter()
        .all(|row| row["generator_filter_matched"] == json!(true)));

    let unmatched = execute_ai_descriptor_query(
        coveql_ai_descriptor_sidecar(),
        "# profiles: table, ai\ntable(people).generatorAudit(reproducibilityClass: \"externalAuditOnly\")",
    );
    let CoveQlExecutionResult::JsonRows(rows) = unmatched.executed.result else {
        panic!("expected JSON rows from CoveQL-AI generator audit");
    };
    assert!(rows.is_empty());
}

fn execute_ai_descriptor_query(coveai: Vec<u8>, query: &str) -> KernelExecutedQuery {
    let source = minimal_object_file();
    execute_ai_descriptor_query_on_source(&source, coveai, query)
}

fn execute_ai_descriptor_query_on_source(
    source: &[u8],
    coveai: Vec<u8>,
    query: &str,
) -> KernelExecutedQuery {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true)]),
            },
        ),
    );
    let mut physical_options = PhysicalPlanOptions::default();
    physical_options.sidecars.cove_ai_artifact_bytes = Some(coveai);

    parse_resolve_plan_build_physical_and_execute_query(
        source,
        query,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        physical_options,
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap()
}

fn cove_o_chunk_source_file() -> Vec<u8> {
    let object_type = ObjectTypeEntryV1 {
        object_type_id: 1,
        type_name: "Document".into(),
        flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        properties: vec![PropertyEntryV1 {
            property_id: 2,
            property_name: "body".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: 0,
        }],
    };
    let state = CoveObjectState {
        object_type_id: 1,
        object_type_name: "Document".into(),
        object_type_flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        branch_key: 0,
        goid: [0x41; 16],
        latest_record_id: [0x42; 16],
        latest_segment_id: 0,
        latest_row_index: 0,
        timestamp_us: 1_700_000_000_000_020,
        csn: 1,
        record_kind: RecordKind::Snapshot,
        tombstone_status: CoveObjectTombstoneStatus::Live,
        properties: vec![CoveObjectPropertyValue {
            property_id: 2,
            property_name: "body".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            flags: 0,
            value: json!("zero chunk text here"),
            redacted: false,
        }],
        association: None,
    };
    cove_map::compact_cove_o_from_object_states(vec![object_type], &[state]).unwrap()
}

fn coveql_ai_source_bound_chunk_sidecar() -> Vec<u8> {
    let source_text = b"zero chunk text here";
    let chunk_text = b"chunk text";
    let source_digest = compute_digest(DigestAlgorithm::Sha256, source_text).unwrap();
    let chunk_digest = compute_digest(DigestAlgorithm::Sha256, chunk_text).unwrap();
    let mut digest_payload = Vec::new();
    digest_payload.extend_from_slice(&source_digest);
    digest_payload.extend_from_slice(&chunk_digest);

    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 32,
        decoded_length: 32,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 32,
        payload_length: 32,
        decoded_length: 32,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 32,
        digest_payload_ref: 1,
        domain_hint: 4,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 32,
        digest_payload_ref: 2,
        domain_hint: 4,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.chunk_profiles.push(ChunkProfileV1 {
        chunk_profile_id: 1,
        profile_name_ref: 0,
        chunker_namespace_ref: 0,
        chunker_name_ref: 0,
        chunker_version_major: 1,
        chunker_version_minor: 0,
        tokenizer_profile_ref: 0,
        boundary_kind: 1,
        overlap_policy: 0,
        parent_policy: 0,
        normalization_policy: 0,
        target_tokens: 16,
        min_tokens: 1,
        max_tokens: 32,
        overlap_tokens: 0,
        max_bytes: 128,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 0,
        column_id: 0,
        object_type_id: 1,
        property_id: 2,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 0,
        source_object_ref: 0,
        source_value_hash_ref: 1,
        byte_start: 5,
        byte_length: 10,
        unicode_scalar_start: 5,
        unicode_scalar_length: 10,
        token_start: 0,
        token_count: 2,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 2,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });

    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [45u8; 16],
        created_at_us: 1_003,
        payload_sections: vec![CoveAiWritableSection {
            section_id: 10,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: PrimaryProfile::CoveAiShared as u8,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: digest_payload,
        }],
        descriptor_tables: tables,
    })
    .unwrap()
}

fn coveql_ai_descriptor_sidecar() -> Vec<u8> {
    let mut tables = AiDescriptorTablesV1::default();
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 8,
        decoded_length: 8,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.chunk_profiles.push(ChunkProfileV1 {
        chunk_profile_id: 1,
        profile_name_ref: 0,
        chunker_namespace_ref: 0,
        chunker_name_ref: 0,
        chunker_version_major: 1,
        chunker_version_minor: 0,
        tokenizer_profile_ref: 1,
        boundary_kind: 1,
        overlap_policy: 0,
        parent_policy: 0,
        normalization_policy: 0,
        target_tokens: 128,
        min_tokens: 1,
        max_tokens: 256,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 1,
        column_id: 2,
        object_type_id: 3,
        property_id: 4,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 7,
        source_object_ref: 8,
        source_value_hash_ref: 1,
        byte_start: 4,
        byte_length: 12,
        unicode_scalar_start: 4,
        unicode_scalar_length: 12,
        token_start: 0,
        token_count: 4,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 2,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tokenizer_profiles.push(TokenizerProfileV1 {
        tokenizer_profile_id: 1,
        tokenizer_namespace_ref: 0,
        tokenizer_name_ref: 0,
        tokenizer_version_major: 1,
        tokenizer_version_minor: 0,
        vocab_digest_ref: 0,
        merges_digest_ref: 0,
        pre_tokenizer_digest_ref: 0,
        normalizer_digest_ref: 0,
        byte_encoder_digest_ref: 0,
        special_tokens_digest_ref: 0,
        added_tokens_digest_ref: 0,
        chat_template_ref: 0,
        unicode_version_ref: 0,
        truncation_policy_ref: 0,
        padding_policy_ref: 0,
        model_max_sequence_length: 4096,
        token_id_width: 2,
        byte_alignment_available: 0,
        reversible: 1,
        deterministic: 1,
        bos_token_id: 0,
        eos_token_id: 0,
        pad_token_id: 0,
        unk_token_id: 0,
        flags: 0,
        checksum: 0,
    });
    tables.token_blocks.push(TokenBlockHeaderV1 {
        token_block_id: 1,
        tokenizer_profile_id: 1,
        token_count: 4,
        token_id_width: 2,
        compression_codec: CompressionCodec::None as u8,
        layout_kind: 0,
        payload_ref: 1,
        payload_offset: 0,
        payload_length: 0,
        integrity_ref: 0,
        checksum: 0,
    });
    tables.tokenized_spans.push(TokenizedSpanV1 {
        tokenized_span_id: 1,
        chunk_id: 1,
        tokenizer_profile_id: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        byte_alignment_ref: 0,
        source_value_hash_ref: 1,
        flags: 0,
        checksum: 0,
    });
    tables.token_sequence_packs.push(TokenSequencePackV1 {
        sequence_pack_id: 1,
        tokenizer_profile_id: 1,
        training_profile_ref: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        source_span_count: 0,
        first_source_span_ref: 0,
        loss_mask_ref: 0,
        attention_mask_ref: 0,
        position_ids_ref: 0,
        labels_ref: 0,
        split_ref: 1,
        sample_weight_ppm: 1_000_000,
        flags: 0,
        checksum: 0,
    });
    tables.training_profiles.push(TrainingProfileV1 {
        training_profile_id: 1,
        profile_name_ref: 0,
        task_family: 1,
        modality_mask: 3,
        source_snapshot_ref: 0,
        map_profile_ref: 0,
        chunk_profile_ref: 1,
        tokenizer_profile_ref: 1,
        vector_space_ref: 0,
        multimodal_sequence_profile_ref: 0,
        split_policy_ref: 0,
        sampling_policy_ref: 0,
        dedup_policy_ref: 0,
        quality_policy_ref: 0,
        license_policy_ref: 0,
        redaction_policy_ref: 0,
        default_generator_provenance_ref: 0,
        reproducibility_class: 1,
        flags: 0,
        checksum: 0,
    });
    tables.dataset_splits.push(DatasetSplitV1 {
        split_id: 1,
        split_name_ref: 0,
        split_method: 1,
        source_snapshot_ref: 0,
        filter_policy_ref: 0,
        seed: 42,
        hash_function_ref: 0,
        stratification_path_ref: 0,
        grouping_ref: 0,
        ordering_policy_ref: 0,
        dedup_policy_ref: 0,
        sample_count: 1,
        first_sample_ref: 1,
        flags: 0,
        checksum: 0,
    });
    tables.dedup_groups.push(DedupGroupV1 {
        dedup_group_id: 1,
        dedup_policy_ref: 0,
        canonical_member_sample_id: 1,
        similarity_kind: 0,
        dedup_authority: 0,
        confidence_ppm: 1_000_000,
        first_member_ref: 1,
        member_count: 1,
        flags: 0,
        checksum: 0,
    });
    tables.training_epoch_plans.push(TrainingEpochPlanV1 {
        epoch_plan_id: 1,
        training_profile_id: 1,
        split_ref: 1,
        seed: 42,
        permutation_kind: 0,
        rng_algorithm_ref: 0,
        permutation_function_ref: 0,
        shard_count: 0,
        first_shard_ref: 0,
        shard_ref_count: 0,
        flags: 0,
        checksum: 0,
    });
    tables.training_samples.push(TrainingSampleEntryV1 {
        sample_id: 1,
        training_profile_id: 1,
        example_kind: 1,
        split_ref: 1,
        source_ref: 0,
        evidence_ref: 0,
        input_ref: 0,
        target_ref: 0,
        label_ref: 0,
        metadata_ref: 0,
        token_sequence_pack_ref: 1,
        multimodal_sequence_pack_ref: 1,
        vector_ref: 0,
        quality_score_ppm: 900_000,
        sample_weight_ppm: 1_000_000,
        dedup_group_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        teacher_model_ref: 0,
        generator_provenance_ref: 0,
        judge_generator_provenance_ref: 0,
        label_generator_provenance_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.model_actors.push(ModelActorDescriptorV1 {
        model_actor_id: 1,
        model_namespace_ref: 0,
        model_name_ref: 0,
        model_version_ref: 0,
        model_checkpoint_digest_ref: 0,
        provider_ref: 0,
        endpoint_ref: 0,
        endpoint_version_ref: 0,
        model_family_ref: 0,
        modality_mask: 3,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .generation_decoding_profiles
        .push(GenerationDecodingProfileV1 {
            decoding_profile_id: 1,
            temperature_micros: 0,
            top_p_micros: 1_000_000,
            top_k: 0,
            seed: 42,
            max_output_tokens: 128,
            stop_sequence_ref: 0,
            safety_policy_ref: 0,
            deterministic_claim: 0,
            flags: 0,
            checksum: 0,
        });
    tables.human_reviews.push(HumanReviewEntryV1 {
        human_review_id: 1,
        review_kind: 1,
        reviewer_role_ref: 0,
        review_time_us: 1_234,
        rating_ppm: 900_000,
        notes_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.generator_provenance.push(GeneratorProvenanceV1 {
        generator_provenance_id: 1,
        generator_kind: 1,
        model_actor_ref: 1,
        prompt_template_ref: 0,
        decoding_profile_ref: 1,
        toolchain_ref: 0,
        source_input_ref: 0,
        source_context_ref: 0,
        source_sample_ref: 1,
        parent_generator_provenance_ref: 0,
        generation_time_us: 1_235,
        confidence_ppm: 800_000,
        human_review_ref: 1,
        policy_ref: 0,
        reproducibility_class: 2,
        flags: 0,
        checksum: 0,
    });
    tables.training_labels.push(TrainingLabelEntryV1 {
        label_id: 1,
        label_kind: 1,
        label_authority: 3,
        label_payload_ref: 0,
        generator_provenance_ref: 1,
        human_review_ref: 1,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.preference_pairs.push(PreferencePairEntryV1 {
        preference_pair_id: 1,
        prompt_ref: 0,
        chosen_ref: 0,
        rejected_ref: 0,
        judge_generator_provenance_ref: 1,
        human_review_ref: 1,
        preference_strength_ppm: 750_000,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tensor_layouts.push(TensorLayoutDescriptorV1 {
        tensor_layout_id: 1,
        layout_name_ref: 0,
        rank: 2,
        dtype: 0,
        byte_order: 1,
        shape_ref: 0,
        stride_ref: 0,
        storage_offset_elements: 0,
        layout_kind: 0,
        memory_alignment_bytes: 64,
        preferred_page_alignment_bytes: 4096,
        tile_shape_ref: 0,
        block_shape_ref: 0,
        quantization_profile_ref: 0,
        sparsity_profile_ref: 0,
        framework_compatibility_ref: 0,
        device_affinity_hint: 0,
        flags: 0,
        checksum: 0,
    });
    tables.device_transfer_hints.push(DeviceTransferHintV1 {
        transfer_hint_id: 1,
        target_kind: 0,
        preferred_alignment_bytes: 64,
        preferred_chunk_bytes: 4096,
        pinned_memory_required: 0,
        contiguous_required: 1,
        zero_copy_possible: 1,
        runtime_registry_binding_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.assets.push(AiAssetRefV1 {
        asset_ref_id: 1,
        parent_asset_ref: 0,
        asset_kind: 2,
        uri_ref: 0,
        embedded_section_ref: 0,
        media_type_ref: 0,
        byte_length: 0,
        digest_ref: 0,
        width: 64,
        height: 64,
        duration_us: 0,
        sample_rate_hz: 0,
        channel_count: 0,
        decode_profile_ref: 0,
        preprocessing_profile_ref: 0,
        transform_profile_ref: 0,
        transform_digest_ref: 0,
        tensor_layout_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .multimodal_sequence_packs
        .push(MultimodalSequencePackV1 {
            sequence_pack_id: 1,
            training_profile_id: 1,
            tokenizer_profile_id: 1,
            sequence_profile_ref: 0,
            element_count: 1,
            first_element_ref: 1,
            split_ref: 1,
            sample_weight_ppm: 1_000_000,
            loss_mask_ref: 0,
            attention_mask_ref: 0,
            position_map_ref: 0,
            label_ref: 0,
            source_snapshot_ref: 0,
            evidence_ref: 0,
            generator_provenance_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .multimodal_sequence_elements
        .push(MultimodalSequenceElementV1 {
            element_id: 1,
            sequence_pack_id: 1,
            ordinal: 0,
            element_kind: 2,
            modality: 2,
            role: 1,
            tokenized_span_ref: 1,
            token_sequence_pack_ref: 1,
            asset_ref: 1,
            tensor_ref: 1,
            vector_ref: 0,
            byte_start: 0,
            byte_length: 0,
            time_start_us: 0,
            time_duration_us: 0,
            position_stream_ref: 0,
            evidence_ref: 0,
            policy_ref: 0,
            flags: 0,
            checksum: 0,
        });

    let payload_sections = vec![CoveAiWritableSection {
        section_id: 10,
        section_kind: SectionKind::AiPayloadBytes as u32,
        profile_kind: PrimaryProfile::CoveTok as u8,
        payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
        requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
        source_binding_ref: 0,
        required_ai_features: 0,
        optional_ai_features: 0,
        feature_binding_ref: 0,
        payload: vec![1, 0, 2, 0, 3, 0, 4, 0],
    }];
    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [44u8; 16],
        created_at_us: 1_002,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap()
}

#[test]
fn coveql_ai_similar_requires_sidecar_for_physical_execution() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true)]),
            },
        ),
    );

    let err = parse_resolve_plan_build_physical_and_execute_query(
        &minimal_object_file(),
        "# profiles: table, ai\ntable(people).similar(fileCode: 10, k: 2)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert!(err
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E_AI_SIDECAR_REQUIRED"));
}

#[test]
fn registered_table_authority_validates_identity_and_kind_match() {
    let mut authority = registered_people_table_authority(
        coveql::TableSurfaceAuthorityKind::RawTable,
        coveql::TableExecutionAuthority::MaterializedRows {
            rows: people_rows(&[("a", true)]),
        },
    );
    authority.contract.row_identity.clear();
    let mut resolve_options = ResolveOptions::default();
    resolve_options
        .table_authorities
        .insert("people".into(), authority);

    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people).select(id)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_TABLE_ROW_IDENTITY_MISSING");

    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::RawTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true)]),
            },
        ),
    );
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people).select(id)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_TABLE_AUTHORITY_UNSUPPORTED");
}

#[test]
fn table_join_set_and_window_execute_over_registered_authorities() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false)]),
            },
        ),
    );
    resolve_options.table_authorities.insert(
        "people_right".into(),
        coveql::TableSurfaceAuthority {
            contract: coveql::TableSurfaceContract {
                table_id: "table:people_right".into(),
                table_name: "people_right".into(),
                ..registered_people_table_authority(
                    coveql::TableSurfaceAuthorityKind::MaterializedTable,
                    coveql::TableExecutionAuthority::MaterializedRows { rows: Vec::new() },
                )
                .contract
            },
            execution_authority: coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("c", true)]),
            },
        },
    );
    let joined = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(people) as l.join(table(people_right) as r, on: l.id == r.id, kind: inner).window(orderBy: l.id).select(left_id: l.id, right_active: r.active, rn: row_number())",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(joined.planned.resolved.method_chain.joins.len(), 1);
    assert_eq!(joined.planned.resolved.method_chain.windows.len(), 1);
    let CoveQlExecutionResult::JsonRows(rows) = joined.result else {
        panic!("expected join JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"left_id": "a", "right_active": true, "rn": 1})]
    );

    let unioned = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(people).union(table(people_right), all: false).select(id)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(
        unioned.planned.resolved.method_chain.set_operations.len(),
        1
    );
    let CoveQlExecutionResult::JsonRows(rows) = unioned.result else {
        panic!("expected union JSON rows");
    };
    assert_eq!(rows.len(), 3);
}

#[test]
fn table_windows_execute_full_materialized_function_set() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options
        .table_authorities
        .insert("scores".into(), score_table_authority());

    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(scores).window(partitionBy: active, orderBy: id).select(id, rn: row_number(), r: rank(), dr: dense_rank(), prev: lag(score), next: lead(score), first: first_value(score), last: last_value(score), c: count(), s: sum(score), a: avg(score), mn: min(score), mx: max(score))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected window JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({
                "id": "a",
                "rn": 1,
                "r": 1,
                "dr": 1,
                "prev": null,
                "next": 3,
                "first": 1,
                "last": 1,
                "c": 1,
                "s": 1,
                "a": 1,
                "mn": 1,
                "mx": 1,
            }),
            json!({
                "id": "b",
                "rn": 1,
                "r": 1,
                "dr": 1,
                "prev": null,
                "next": null,
                "first": 2,
                "last": 2,
                "c": 1,
                "s": 2,
                "a": 2,
                "mn": 2,
                "mx": 2,
            }),
            json!({
                "id": "c",
                "rn": 2,
                "r": 2,
                "dr": 2,
                "prev": 1,
                "next": null,
                "first": 1,
                "last": 3,
                "c": 2,
                "s": 4,
                "a": 2,
                "mn": 1,
                "mx": 3,
            }),
        ]
    );

    let mut resolve_options = ResolveOptions::default();
    resolve_options
        .table_authorities
        .insert("scores".into(), score_table_authority());
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "table(scores).select(rn: row_number())",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
}

#[test]
fn table_join_requires_exact_bridge_for_distinct_code_domains() {
    let mut left = registered_people_table_authority(
        coveql::TableSurfaceAuthorityKind::MaterializedTable,
        coveql::TableExecutionAuthority::MaterializedRows {
            rows: people_rows(&[("a", true)]),
        },
    );
    left.contract.code_domain_contexts = vec!["domain:left:v1".into()];
    let mut right = registered_people_table_authority(
        coveql::TableSurfaceAuthorityKind::MaterializedTable,
        coveql::TableExecutionAuthority::MaterializedRows {
            rows: people_rows(&[("a", true)]),
        },
    );
    right.contract.table_id = "table:people_right".into();
    right.contract.table_name = "people_right".into();
    right.contract.code_domain_contexts = vec!["domain:right:v1".into()];

    let mut resolve_options = ResolveOptions::default();
    resolve_options
        .table_authorities
        .insert("people".into(), left.clone());
    resolve_options
        .table_authorities
        .insert("people_right".into(), right.clone());

    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people) as l.join(table(people_right) as r, on: l.id == r.id).select(l.id)",
        ParseOptions::default(),
        resolve_options.clone(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_BRIDGE");

    resolve_options
        .bridge_contracts
        .push(coveql::CoveQlBridgeRegistration {
            bridge_id: "bridge:people:left-right".into(),
            bridge_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
            source_profile: coveql::CoveQlProfileId::Table,
            target_profile: coveql::CoveQlProfileId::Table,
            source_grain: left.contract.row_grain.clone(),
            target_grain: right.contract.row_grain.clone(),
            identity_mapping: vec![coveql::CoveQlBridgeIdentityMapping {
                source: "id".into(),
                target: "id".into(),
            }],
            temporal_alignment: "snapshot_equal".into(),
            null_missing_policy: "missing_is_null".into(),
            code_domain_policy: "exact_remap".into(),
            visibility_compatibility: "same_policy".into(),
            redaction_compatibility: "same_policy".into(),
            fallback_behavior: "materialized_canonical_values".into(),
            exact: true,
        });

    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people) as l.join(table(people_right) as r, on: l.id == r.id).select(l.id)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap();
    assert_eq!(
        resolved.method_chain.joins[0]
            .bridge_contract
            .as_ref()
            .map(|bridge| bridge.bridge_id.as_str()),
        Some("bridge:people:left-right")
    );
}

#[test]
fn table_cte_methods_register_scoped_table_bindings() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.table_authorities.insert(
        "people".into(),
        registered_people_table_authority(
            coveql::TableSurfaceAuthorityKind::MaterializedTable,
            coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false)]),
            },
        ),
    );
    resolve_options.table_authorities.insert(
        "people_right".into(),
        coveql::TableSurfaceAuthority {
            contract: coveql::TableSurfaceContract {
                table_id: "table:people_right".into(),
                table_name: "people_right".into(),
                ..registered_people_table_authority(
                    coveql::TableSurfaceAuthorityKind::MaterializedTable,
                    coveql::TableExecutionAuthority::MaterializedRows { rows: Vec::new() },
                )
                .contract
            },
            execution_authority: coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true)]),
            },
        },
    );
    resolve_options.table_authorities.insert(
        "people_step".into(),
        coveql::TableSurfaceAuthority {
            contract: coveql::TableSurfaceContract {
                table_id: "table:people_step".into(),
                table_name: "people_step".into(),
                ..registered_people_table_authority(
                    coveql::TableSurfaceAuthorityKind::MaterializedTable,
                    coveql::TableExecutionAuthority::MaterializedRows { rows: Vec::new() },
                )
                .contract
            },
            execution_authority: coveql::TableExecutionAuthority::MaterializedRows {
                rows: people_rows(&[("a", true), ("b", false), ("c", true)]),
            },
        },
    );

    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(people) as l.with(right: table(people_right)).join(table(right) as r, on: l.id == r.id).select(id: l.id, active: r.active)",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.ctes.len(), 1);
    assert_eq!(
        executed.planned.resolved.method_chain.ctes[0].execution_authority,
        "materialized_cte_table_authority"
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected CTE join JSON rows");
    };
    assert_eq!(rows, vec![json!({"id": "a", "active": true})]);

    let recursive = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people).withRecursive(name: seed, seed: table(people), maxIterations: 4).select(id)",
        ParseOptions::default(),
        resolve_options.clone(),
        validation_options(),
    )
    .unwrap();
    assert!(recursive.method_chain.ctes[0].recursive);
    assert_eq!(recursive.method_chain.ctes[0].max_iterations, Some(4));

    let executed_recursive = parse_resolve_plan_and_execute_query(
        &minimal_object_file(),
        "table(people).withRecursive(name: reach, seed: table(people), step: table(people_step), key: id, maxIterations: 4).join(table(reach) as r, on: people.id == r.id, kind: right, cardinality: many).select(id: r.id)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert!(executed_recursive.planned.resolved.method_chain.ctes[0].recursive);
    assert!(executed_recursive.planned.resolved.method_chain.ctes[0]
        .step_table
        .is_some());
    let CoveQlExecutionResult::JsonRows(rows) = executed_recursive.result else {
        panic!("expected recursive CTE JSON rows");
    };
    assert_eq!(
        rows.iter()
            .map(|row| row["id"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["a".to_string(), "b".to_string(), "c".to_string()])
    );
}

#[test]
fn table_lookup_executes_projection_backed_left_preserving_join() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let ResolvedRoot::Table(root) = &executed.planned.resolved.root else {
        panic!("expected table root");
    };
    assert_eq!(root.binding_name.as_deref(), Some("l"));
    assert_eq!(executed.planned.resolved.method_chain.lookups.len(), 1);
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0]
            .right
            .binding_name
            .as_deref(),
        Some("r")
    );
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0].cardinality,
        coveql::TableLookupCardinality::One
    );
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0].duplicate_policy,
        coveql::TableLookupDuplicatePolicy::Reject
    );
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0].unmatched_policy,
        coveql::TableLookupUnmatchedPolicy::Nulls
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected table lookup JSON rows");
    };
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row["left_active"] == json!(false) && row["right_active"] == json!(false)));
    assert!(rows
        .iter()
        .any(|row| row["left_active"] == json!(true) && row["right_active"] == json!(true)));
}

#[test]
fn table_lookup_enforces_cardinality_and_duplicate_contract() {
    let bytes = object_file_with_bool_records_and_projection(&[true, true]);
    let err = parse_resolve_plan_and_execute_query(
        &bytes,
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_LOOKUP_DUPLICATE_MATCH");

    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active, cardinality: many).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0].cardinality,
        coveql::TableLookupCardinality::Many
    );
    assert_eq!(
        executed.planned.resolved.method_chain.lookups[0].duplicate_policy,
        coveql::TableLookupDuplicatePolicy::EmitAll
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected table lookup JSON rows");
    };
    assert_eq!(rows.len(), 4);
}

#[test]
fn table_exists_executes_projection_backed_semi_and_anti_join() {
    let bytes = object_file_with_bool_records_and_projection(&[false, true]);
    let semi = parse_resolve_plan_and_execute_query(
        &bytes,
        "table(thing_projection) as l.where(exists(table(thing_projection) as r, on: l.active == r.active)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = semi.result else {
        panic!("expected semi-join JSON rows");
    };
    assert_eq!(rows.len(), 2);

    let anti = parse_resolve_plan_and_execute_query(
        &bytes,
        "table(thing_projection) as l.where(!exists(table(thing_projection) as r, on: l.active == r.active)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = anti.result else {
        panic!("expected anti-join JSON rows");
    };
    assert!(rows.is_empty());
}

#[test]
fn projection_backed_table_emits_projection_dependency_contract() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "table(people_projection).where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        planned.logical_plan.context.root_kind,
        coveql::LogicalRootKind::Table
    );
    assert_eq!(
        planned.logical_plan.context.scan_grain,
        coveql::ScanGrain::TableRow
    );
    assert!(planned
        .logical_plan
        .canonical_order
        .contains(&"projection_row_identity".to_string()));
    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.projection_id, "people_projection");
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushed_predicates[0].contains("compare:Eq:active"));
    assert_eq!(planned.explain_json()["primary_profile"], json!("table"));
    assert_eq!(planned.explain_json()["root"], json!("table"));
    assert_eq!(planned.explain_json()["grain"], json!("table_row"));
    let query_contract = &planned.explain_json()["profile_contracts"][0]["query_contract"];
    assert_eq!(
        query_contract["table_id"],
        json!("projection:people_projection")
    );
    assert_eq!(
        query_contract["row_identity"],
        json!(["projection_row_identity"])
    );
    assert_eq!(
        query_contract["projection_dependency_contract_id"],
        json!("people_projection")
    );
}

#[test]
fn projection_backed_table_direct_scan_supports_alias_qualified_columns() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "table(thing_projection) as p.where(p.active == true).select(value: p.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected table JSON rows");
    };
    assert_eq!(rows, vec![json!({"value": true})]);
    let contract = executed
        .planned
        .dependencies
        .projection_contracts
        .first()
        .unwrap();
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushed_predicates[0].contains("compare:Eq:active"));
}

#[test]
fn graph_node_and_edge_roots_execute_through_object_authority() {
    let node = parse_resolve_plan_and_execute_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "node(Thing) as t.where(t.active == true).select(t.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(
        node.planned.resolved.parsed.profiles,
        vec![coveql::CoveQlProfileId::Graph]
    );
    assert!(matches!(node.planned.resolved.root, ResolvedRoot::Node(_)));
    assert_eq!(
        node.planned.logical_plan.context.root_kind,
        coveql::LogicalRootKind::Node
    );
    assert_eq!(
        node.planned.logical_plan.context.scan_grain,
        coveql::ScanGrain::NodeState
    );
    let CoveQlExecutionResult::JsonRows(rows) = node.result else {
        panic!("expected graph node JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["active"], json!(true));

    let edge = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "edge(CustomerPlacedOrder) as e.select(e.source_goid, e.target_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert!(matches!(edge.planned.resolved.root, ResolvedRoot::Edge(_)));
    assert_eq!(
        edge.planned.logical_plan.context.root_kind,
        coveql::LogicalRootKind::Edge
    );
    assert_eq!(
        edge.planned.logical_plan.context.scan_grain,
        coveql::ScanGrain::EdgeState
    );
    let CoveQlExecutionResult::JsonRows(rows) = edge.result else {
        panic!("expected graph edge JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["source_goid"].is_string());
    assert!(rows[0]["target_goid"].is_string());
}

#[test]
fn graph_traverse_executes_one_hop_edge_expansion() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.traverse(out(edge(CustomerPlacedOrder) as placed)).select(customer: c.goid, target: placed.target_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.traversals.len(), 1);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph traversal JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["customer"].is_string());
    assert!(rows[0]["target"].is_string());
    assert_ne!(rows[0]["customer"], rows[0]["target"]);
}

#[test]
fn graph_relationship_exists_and_aggregates_use_association_authority() {
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.where(exists(out(edge(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph relationship JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);

    let aggregates = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.select(c: count(out(edge(CustomerPlacedOrder))), e: exists(out(edge(CustomerPlacedOrder))), d: distinct_count(out(edge(CustomerPlacedOrder))))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = aggregates.result else {
        panic!("expected graph relationship aggregate JSON rows");
    };
    assert_eq!(rows, vec![json!({"c": 1, "e": true, "d": 1})]);
}

#[test]
fn graph_relationship_target_node_filter_uses_visible_target_node_rows() {
    let mut resolve_options = protected_json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let bytes = object_file_with_two_person_association_record();
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        "node(Person) as c.where(exists(out(edge(CustomerPlacedOrder)).to(node(Person)))).select(active)",
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph target relationship JSON rows");
    };
    assert_eq!(rows, vec![json!({"active": true})]);

    let aggregate = parse_resolve_plan_and_execute_query(
        &bytes,
        "node(Person) as c.select(c: count(out(edge(CustomerPlacedOrder)).to(node(Person))), e: exists(out(edge(CustomerPlacedOrder)).to(node(Person))), d: distinct_count(out(edge(CustomerPlacedOrder)).to(node(Person))))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = aggregate.result else {
        panic!("expected graph target relationship aggregate rows");
    };
    assert_eq!(rows, vec![json!({"c": 1, "e": true, "d": 1})]);
}

#[test]
fn graph_path_root_executes_as_implicit_one_hop_traversal() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "path(node(Person) as c.out(edge(CustomerPlacedOrder) as placed)).select(customer: c.goid, target: placed.target_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.traversals.len(), 1);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph path JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["customer"].is_string());
    assert!(rows[0]["target"].is_string());
}

#[test]
fn graph_path_root_executes_multi_hop_traversal() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "path(node(Person) as a.out(edge(CustomerPlacedOrder) as first).to(node(Person) as b).out(edge(CustomerPlacedOrder) as second).to(node(Person) as c)).select(start: a.goid, mid: b.goid, end: c.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.traversals.len(), 2);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph path JSON rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["start"], json!("00000000000000000000000000000000"));
    assert_eq!(rows[0]["mid"], json!("02020202020202020202020202020202"));
    assert_eq!(rows[0]["end"], json!("03030303030303030303030303030303"));
}

#[test]
fn graph_traverse_method_chains_use_previous_hop_target() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as a.traverse(out(edge(CustomerPlacedOrder) as first).to(node(Person) as b)).traverse(out(edge(CustomerPlacedOrder) as second).to(node(Person) as c)).select(start: a.goid, mid: b.goid, end: c.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.traversals.len(), 2);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected chained graph traverse JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "start": "00000000000000000000000000000000",
            "mid": "02020202020202020202020202020202",
            "end": "03030303030303030303030303030303"
        })]
    );
}

#[test]
fn graph_traverse_accepts_explicit_one_hop_contract_arguments() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.traverse(out(edge(CustomerPlacedOrder) as placed), min: 1, max: 1, mode: walk).select(customer: c.goid, target: placed.target_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    let traversal = &executed.planned.resolved.method_chain.traversals[0];
    assert_eq!(traversal.min_depth, 1);
    assert_eq!(traversal.max_depth, 1);
    assert_eq!(traversal.mode, coveql::GraphTraversalMode::Walk);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph traversal JSON rows");
    };
    assert_eq!(rows.len(), 1);
}

#[test]
fn graph_traverse_rejects_variable_length_without_contract() {
    let err = parse_and_resolve_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.traverse(out(edge(CustomerPlacedOrder)), min: 1, max: 3, mode: walk).select(c.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_PROFILE_METHOD");
    assert!(err.diagnostics[0]
        .message
        .contains("variable-length traversal"));
}

#[test]
fn graph_traverse_executes_variable_length_with_contract() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.graph_traversal_contract = Some(graph_traversal_contract());
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as a.traverse(out(edge(CustomerPlacedOrder) as placed).to(node(Person) as p), min: 1, max: 2, mode: walk, distinct: path).select(start: a.goid, end: p.goid)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let traversal = &executed.planned.resolved.method_chain.traversals[0];
    assert_eq!(traversal.min_depth, 1);
    assert_eq!(traversal.max_depth, 2);
    assert_eq!(traversal.mode, coveql::GraphTraversalMode::Walk);
    assert_eq!(
        traversal.distinct,
        coveql::GraphTraversalDistinctPolicy::Path
    );
    assert!(traversal.contract.is_some());
    let graph_contract =
        &executed.explain_json()["profile_contracts"][0]["query_contract"]["traversals"][0];
    assert_eq!(graph_contract["min_depth"], json!(1));
    assert_eq!(graph_contract["max_depth"], json!(2));
    assert_eq!(graph_contract["mode"], json!("walk"));
    assert_eq!(graph_contract["distinct"], json!("path"));
    assert_eq!(graph_contract["contract_present"], json!(true));
    assert_eq!(graph_contract["contract"]["max_depth"], json!(4));
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected variable graph traversal JSON rows");
    };
    assert_eq!(
        rows.iter()
            .map(|row| format!(
                "{}>{}",
                row["start"].as_str().unwrap(),
                row["end"].as_str().unwrap()
            ))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "00000000000000000000000000000000>02020202020202020202020202020202".to_string(),
            "02020202020202020202020202020202>03030303030303030303030303030303".to_string(),
            "00000000000000000000000000000000>03030303030303030303030303030303".to_string(),
        ])
    );
}

#[test]
fn graph_algorithm_executes_materialized_degree_and_reports_contract() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as p.degree(out(edge(CustomerPlacedOrder))).select(id: p.goid, degree)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        executed
            .planned
            .resolved
            .method_chain
            .graph_algorithms
            .len(),
        1
    );
    let contract =
        &executed.explain_json()["profile_contracts"][0]["query_contract"]["algorithms"][0];
    assert_eq!(contract["algorithm"], json!("degree"));
    assert_eq!(
        contract["contract"]["disclosure_policy"],
        json!("redaction_safe_no_partial_results")
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph algorithm JSON rows");
    };
    let degrees = rows
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_string(),
                row["degree"].as_u64().unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(degrees["00000000000000000000000000000000"], 1);
    assert_eq!(degrees["02020202020202020202020202020202"], 1);
    assert_eq!(degrees["03030303030303030303030303030303"], 0);
}

#[test]
fn graph_algorithms_execute_iterative_materialized_scores() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as p.pageRank(out(edge(CustomerPlacedOrder)), maxIterations: 20).hits(out(edge(CustomerPlacedOrder)), maxIterations: 20).centrality(out(edge(CustomerPlacedOrder))).select(id: p.goid, pagerank, authority, hub, centrality)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        executed
            .planned
            .resolved
            .method_chain
            .graph_algorithms
            .len(),
        3
    );
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph algorithm JSON rows");
    };
    let by_id = rows
        .iter()
        .map(|row| (row["id"].as_str().unwrap().to_string(), row.clone()))
        .collect::<BTreeMap<_, _>>();
    let start = &by_id["00000000000000000000000000000000"];
    let middle = &by_id["02020202020202020202020202020202"];
    let sink = &by_id["03030303030303030303030303030303"];
    assert!(start["pagerank"].as_f64().unwrap() < middle["pagerank"].as_f64().unwrap());
    assert!(middle["pagerank"].as_f64().unwrap() < sink["pagerank"].as_f64().unwrap());
    assert!(start["hub"].as_f64().unwrap().is_finite());
    assert!(sink["authority"].as_f64().unwrap().is_finite());
    assert_eq!(sink["centrality"], json!(0.0));
}

#[test]
fn graph_algorithm_variants_execute_materialized_oracle() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as p.connectedComponents(out(edge(CustomerPlacedOrder)), kind: strong).degree(out(edge(CustomerPlacedOrder)), kind: total).centrality(out(edge(CustomerPlacedOrder)), kind: degree).community(out(edge(CustomerPlacedOrder)), kind: label_propagation).spanningTree(out(edge(CustomerPlacedOrder)), kind: bfs).select(id: p.goid, component_id, degree, centrality, community_id, tree_parent, tree_depth)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    let algorithms =
        &executed.explain_json()["profile_contracts"][0]["query_contract"]["algorithms"];
    assert_eq!(algorithms[0]["variant"], json!("strong"));
    assert_eq!(algorithms[1]["variant"], json!("total"));
    assert_eq!(algorithms[4]["variant"], json!("bfs"));

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected graph algorithm JSON rows");
    };
    let by_id = rows
        .iter()
        .map(|row| (row["id"].as_str().unwrap().to_string(), row.clone()))
        .collect::<BTreeMap<_, _>>();
    let start = &by_id["00000000000000000000000000000000"];
    let middle = &by_id["02020202020202020202020202020202"];
    let sink = &by_id["03030303030303030303030303030303"];

    assert_eq!(start["degree"], json!(1));
    assert_eq!(middle["degree"], json!(2));
    assert_eq!(sink["degree"], json!(1));
    assert_eq!(middle["centrality"], json!(1.0));
    assert!(start["community_id"].is_u64());
    assert_eq!(start["tree_parent"], Value::Null);
    assert_eq!(
        middle["tree_parent"],
        json!("00000000000000000000000000000000")
    );
    assert_eq!(
        sink["tree_parent"],
        json!("02020202020202020202020202020202")
    );
    assert_eq!(start["tree_depth"], json!(0));
    assert_eq!(middle["tree_depth"], json!(1));
    assert_eq!(sink["tree_depth"], json!(2));

    let err = parse_and_resolve_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as p.spanningTree(out(edge(CustomerPlacedOrder)), kind: min_weight).select(p.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .expect_err("min_weight requires an explicit weight expression");
    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_PROFILE_METHOD");
}

#[test]
fn graph_traverse_contract_rejects_unsupported_distinct_policy() {
    let mut contract = graph_traversal_contract();
    contract.supported_distinct_policies = vec![coveql::GraphTraversalDistinctPolicy::None];
    let mut resolve_options = ResolveOptions::default();
    resolve_options.graph_traversal_contract = Some(contract);
    let err = parse_and_resolve_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as a.traverse(out(edge(CustomerPlacedOrder)), min: 1, max: 2, mode: walk, distinct: path).select(a.goid)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_PROFILE_METHOD");
    assert!(err.diagnostics[0].message.contains("distinct policy path"));
}

#[test]
fn graph_traverse_contract_respects_resource_depth_budget() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.graph_traversal_contract = Some(graph_traversal_contract());
    resolve_options
        .resource_budget
        .maximum_graph_traversal_depth = 1;
    let err = parse_and_resolve_query(
        &object_file_with_three_person_two_association_records(),
        "node(Person) as a.traverse(out(edge(CustomerPlacedOrder)), min: 1, max: 2, mode: walk).select(a.goid)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_RESOURCE_BUDGET_EXCEEDED");
    assert!(err.diagnostics[0]
        .message
        .contains("maximum_graph_traversal_depth"));
}

#[test]
fn graph_path_root_can_continue_with_traverse_method() {
    let executed = parse_resolve_plan_and_execute_query(
        &object_file_with_three_person_two_association_records(),
        "path(node(Person) as a.out(edge(CustomerPlacedOrder) as first).to(node(Person) as b)).traverse(out(edge(CustomerPlacedOrder) as second).to(node(Person) as c)).select(start: a.goid, mid: b.goid, end: c.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(executed.planned.resolved.method_chain.traversals.len(), 2);
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected continued graph path JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({
            "start": "00000000000000000000000000000000",
            "mid": "02020202020202020202020202020202",
            "end": "03030303030303030303030303030303"
        })]
    );
}

#[test]
fn resolver_rejects_unknown_or_undeclared_profile_constructs_with_stable_diagnostics() {
    let table = parse_and_resolve_query(
        &minimal_object_file(),
        "table(orders).select(order_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(table.diagnostics[0].code, "E_UNKNOWN_TABLE_SURFACE");

    let graph = parse_and_resolve_query(
        &minimal_object_file(),
        "node(Customer).select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(graph.diagnostics[0].code, "E_UNKNOWN_GRAPH_LABEL");

    let path = parse_and_resolve_query(
        &minimal_object_file(),
        "path(node(Person) as p.out(edge(CustomerPlacedOrder)))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(path.diagnostics[0].code, "E_UNKNOWN_GRAPH_LABEL");

    let method = parse_and_resolve_query(
        &minimal_object_file(),
        "object(Person).lookup(table(orders), on: active == true)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(method.diagnostics[0].code, "E_UNKNOWN_BRIDGE");

    let exists_bridge = parse_and_resolve_query(
        &object_file_with_bool_records_and_projection(&[true]),
        "object(Thing).where(exists(table(thing_projection) as t, on: active == t.active)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(exists_bridge.diagnostics[0].code, "E_UNKNOWN_BRIDGE");

    let ambiguous = parse_and_resolve_query(
        &minimal_object_file(),
        "# profiles: object, table\nprojection(people_projection)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(ambiguous.diagnostics[0].code, "E_AMBIGUOUS_PROFILE");

    let ai_without_profile = parse_and_resolve_query(
        &minimal_object_file(),
        "table(people).embedding(id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(
        ai_without_profile.diagnostics[0].code,
        "E_UNSUPPORTED_PROFILE_METHOD"
    );
    assert!(ai_without_profile.diagnostics[0]
        .message
        .contains("ai profile"));
}
