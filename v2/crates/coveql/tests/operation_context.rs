use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow_array::{Array, BooleanArray, Int64Array, StringArray};
use arrow_schema::DataType;
use cove_core::{
    artifact::covm::{CovmFile, CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1},
    canonical::CanonicalValue,
    checksum,
    collation::CollationKind,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        PrimaryProfile, SectionKind, StorageClass, FEATURE_ENGINE_PROFILE, FEATURE_FILE_DICTIONARY,
        FEATURE_OBJECT_PROFILE, FEATURE_REDACTIONS, FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        FEATURE_SEMANTIC_MAP,
    },
    dictionary::{FileDictionaryEncoding, FileDictionaryKey},
    digest::compute_digest,
    page::ColumnPageIndexEntryV1,
    page_payload::{
        ColumnPagePayloadHeaderV1, ColumnPagePayloadV1, CoveEncodingNodeV1, PageBufferDescriptorV1,
        PageBufferKind, COLUMN_PAGE_PAYLOAD_HEADER_LEN, COLUMN_PAGE_PAYLOAD_MAGIC,
        COLUMN_PAGE_PAYLOAD_VERSION_MAJOR, COVE_ENCODING_NODE_LEN, PAGE_BUFFER_DESCRIPTOR_LEN,
    },
    profile::cove_e::{
        CodeSpaceDescriptorV1, EngineMountPolicyV1, ExecutionCodeCanonicality,
        ExecutionCodeComparisonScope, ExecutionCodeDescriptorV1, ExecutionCodeKind,
        ExecutionCodeLifetime, ExecutionScopeDescriptorV1, ExecutionScopeKind, FileCodeMappingKind,
        MissingValuePolicy, NullCodePolicy, ReverseLookupPolicy, StaleMappingPolicy,
    },
    profile::cove_o::{
        read_object_surface_from_bytes, read_retained_object_temporal_segments, CoveRecordRefV1,
        ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1,
        TemporalSegmentHeaderV1, TemporalSegmentIndex, TemporalSegmentIndexEntryV1,
        OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    },
    reader::{validate_bytes, OptionalPushdownPolicy, ValidationOptions},
    redaction::{RedactionEntry, RedactionManifest},
    retained_bytes::RetainedBytes,
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    wire,
    writer::{MinimalCoveWriter, SectionPayload},
};
use cove_coverage::{
    coverage_set_payload_checksum, CoverageExactnessV2, CoverageGranularityV2, CoverageProofKindV2,
    CoverageProofRecordV2, CoverageProofStrengthV2, CoverageSetEntryV2, CoverageSetHeaderV2,
    CoverageSetV2,
};
use cove_index::{
    execution::CoviAggregateKindV2, CoviAggregateAnswerBlockHeaderV2, CoviAggregateAnswerBlockV2,
    CoviAggregateAnswerV2, CoviArtifactV2, CoviComparatorKindV2, CoviEntryBlockHeaderV2,
    CoviEntryBlockV2, CoviIndexEntryV2, CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2,
    CoviKeyBlockHeaderV2, CoviKeyBlockV2, CoviKeyEncodingKindV2, CoviPostingsBlockHeaderV2,
    CoviPostingsBlockV2, CoviPostingsHeaderV2, CoviReferencedFileV2, CoviRowRangePostingV2,
    CoviSectionKindV2, CoviSectionPayloadV2, CoviSnapshotValidityV2, IndexCapabilityExactnessV2,
    IndexCapabilityV2, IndexOnlyCapabilityV2,
};
use cove_layout::{
    ZeroCopyBufferMapEntryV2, ZeroCopyBufferMapHeaderV2, ZeroCopyBufferMapV2,
    ZeroCopyDictionarySemanticsV2, ZeroCopyLifetimeScopeV2, ZeroCopyNestedLayoutKindV2,
    ZeroCopyNullBitmapPolarityV2, ZeroCopySourceBufferRoleV2, ZeroCopyTargetBufferRoleV2,
    ZeroCopyTargetV2,
};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeHintKindV2};
use coveql::{
    build_operation_context, build_physical_plan, execute_manifest_physical_planned_query,
    execute_planned_query, execute_planned_query_stream, parse_and_resolve_query,
    parse_resolve_and_plan_query, parse_resolve_plan_and_build_physical_plan,
    parse_resolve_plan_and_execute_query, parse_resolve_plan_build_physical_and_execute_query,
    parse_resolve_plan_build_physical_and_execute_query_retained, render_explain_text,
    AggregateDisclosurePolicy, AssociationEndpointRole, AstCompareOp, CoveQlExecutionResult,
    CoveQlOperationRequest, CoveQlOutputMode, CoveQlRetainedInput, CoveQlSelectedOperation,
    ExecutionOptions, ExplainDisclosurePolicy, ExplainMode, FallbackPolicy, FilterClassification,
    KernelDecisionKind, KernelExecutionMode, KernelExecutionOptions, KernelFallbackReason,
    LogicalPlanNodeKind, LogicalPredicateKind, MaterializedChangeDiffKind,
    MetadataDisclosurePolicy, OptionalMetadataKind, OptionalMetadataStatus, OutputGrain,
    ParseOptions, PhysicalPlanNodeKind, PhysicalPlanOptions, PhysicalPredicateFormKind,
    PhysicalRepresentationClass, PhysicalSidecarInputs, PhysicalSidecarStatus, PlanOptions,
    PredicatePlacement, PredicateProofState, PushdownDecisionKind, PushdownOptions,
    PushdownOutcome, RedactionPolicy, RepresentationClass, ResolveOptions, ResolvedExpr,
    ResolvedLiteralValue, ResolvedPredicate, ResolvedRoot, SecurityContext, TemporalRole,
    VisibilityOverlay, VisibilityPolicy,
};
use serde_json::{json, Value};

fn validation_options() -> ValidationOptions {
    ValidationOptions {
        semantic: true,
        verify_digests: false,
        allow_unknown_optional_extensions: true,
        optional_pushdown_policy: OptionalPushdownPolicy::Strict,
    }
}

fn projection_backed_thing_table_contract() -> coveql::TableSurfaceContract {
    coveql::TableSurfaceContract {
        table_id: "projection:thing_projection".into(),
        table_name: "thing_projection".into(),
        contract_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
        authority_kind: coveql::TableSurfaceAuthorityKind::DeterministicProjection,
        authority_fingerprint: "thing_projection".into(),
        schema_fingerprint: "thing_projection".into(),
        logical_column_map: vec![coveql::TableSurfaceColumnContract {
            name: "active".into(),
            logical_type: Some("bool".into()),
            nullable: true,
            source_path: Some("active".into()),
            code_domain: None,
            collation: None,
        }],
        row_grain: "one_row_per_object".into(),
        row_identity: vec!["projection_row_identity".into()],
        canonical_order: vec!["projection_row_identity".into()],
        visibility_authority: "cove_o_visibility".into(),
        redaction_authority: "cove_o_redaction".into(),
        temporal_authority: coveql::TableTemporalAuthority::MaterializedSnapshotOnly,
        evidence_capabilities: vec![
            coveql::AstEvidenceGrain::Row,
            coveql::AstEvidenceGrain::Column,
            coveql::AstEvidenceGrain::Projection,
            coveql::AstEvidenceGrain::Source,
        ],
        null_missing_nan_policy: "projection_contract".into(),
        collation_policy: "projection_contract".into(),
        code_domain_contexts: Vec::new(),
        code_domain_bridges: Vec::new(),
        projection_dependency_contract_id: Some("thing_projection".into()),
        datafusion_interop_contract: Some("coveql_table_projection_provider".into()),
    }
}

fn graph_traversal_contract() -> coveql::GraphTraversalContract {
    coveql::GraphTraversalContract {
        contract_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
        allow_variable_length: true,
        supported_modes: vec![
            coveql::GraphTraversalMode::Walk,
            coveql::GraphTraversalMode::Trail,
            coveql::GraphTraversalMode::SimplePath,
        ],
        supported_distinct_policies: vec![
            coveql::GraphTraversalDistinctPolicy::None,
            coveql::GraphTraversalDistinctPolicy::Path,
            coveql::GraphTraversalDistinctPolicy::EndNode,
        ],
        max_depth: 4,
        max_fanout_per_node: 16,
        max_paths: 64,
        max_frontier: 64,
        path_identity: vec!["start_goid".into(), "edge_goids".into(), "end_goid".into()],
        hidden_endpoint_policy: "suppress_hidden_endpoints".into(),
        ordering_policy: "breadth_first_canonical_association_order".into(),
        execution_authority: "visible_materialized_graph_oracle".into(),
    }
}

fn assert_unique_contract_fields(label: &str, fields: &[&str]) {
    let mut seen = BTreeSet::new();
    for field in fields {
        assert!(
            seen.insert(*field),
            "{label} contains duplicate field {field}"
        );
    }
}

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

    let contracts = coveql::builtin_coveql_profile_contracts();
    assert_eq!(contracts.len(), 3);
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

    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_TABLE_SURFACE");
    assert!(err.diagnostics[0].message.contains("row_identity"));
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
}

fn minimal_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_file_with_runtime_hints(hints: &[RuntimeCompatibilityHintV2]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut runtime_payload = Vec::new();
    for hint in hints {
        runtime_payload.extend_from_slice(&hint.serialize().unwrap());
    }

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.optional_features = FEATURE_RUNTIME_COMPATIBILITY_HINTS;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::RuntimeCompatibilityHints as u16,
        profile: PrimaryProfile::RuntimeCompatibility as u8,
        flags: 0,
        item_count: hints.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        data: runtime_payload,
    });
    writer.write().unwrap()
}

fn minimal_object_file_with_id(file_id: [u8; 16]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_incompatible_object_file_with_id(file_id: [u8; 16]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_projection_file_with_id(file_id: [u8; 16], column_name: &str) -> Vec<u8> {
    minimal_object_projection_file_with_id_and_mapping(
        file_id,
        column_name,
        "people-map",
        "2026.05",
    )
}

fn minimal_object_projection_file_with_id_and_mapping(
    file_id: [u8; 16],
    column_name: &str,
    mapping_id: &str,
    mapping_version: &str,
) -> Vec<u8> {
    minimal_object_projection_file_with_id_mapping_and_ordering(
        file_id,
        column_name,
        mapping_id,
        mapping_version,
        &[],
    )
}

fn minimal_object_projection_file_with_id_mapping_and_ordering(
    file_id: [u8; 16],
    column_name: &str,
    mapping_id: &str,
    mapping_version: &str,
    ordering: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    let mut projection = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": column_name,
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });
    if !ordering.is_empty() {
        projection["projections"][0]["ordering"] = json!(ordering);
    }
    push_embedded_map_section(&mut writer, SectionKind::MapProjectionCatalog, projection);
    writer.write().unwrap()
}

fn minimal_object_with_ordered_projection_file() -> Vec<u8> {
    minimal_object_projection_file_with_id_mapping_and_ordering(
        [0xC3; 16],
        "active",
        "people-map",
        "2026.05",
        &["active"],
    )
}

fn covm_manifest_for_members(members: &[(&str, &[u8])]) -> Vec<u8> {
    let files = members
        .iter()
        .map(|(uri, bytes)| {
            let validated = validate_bytes(bytes).unwrap();
            CovmFileEntryV1 {
                file_id: validated.header.file_id,
                uri: (*uri).into(),
                file_len: validated.postscript.file_len,
                footer_crc32c: validated.postscript.footer.crc32c,
                digest_algorithm: DigestAlgorithm::Sha256 as u16,
                digest: compute_digest(DigestAlgorithm::Sha256, bytes).unwrap(),
                row_count: 0,
                segment_count: 0,
                file_stats_ref: 0,
                file_exact_set_ref: 0,
                flags: 0,
            }
        })
        .collect::<Vec<_>>();
    CovmFile {
        header: CovmHeaderV1::new([0xC0; 16], 1, files.len() as u32, 1_700_000_000_000_000),
        files,
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

fn minimal_binary_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "payload".into(),
                logical_type: CoveLogicalType::Binary,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_string_object_with_function_registry(function_id: &str) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let registry = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapFunctionRegistry as u16,
        "mapping_id": "function-map",
        "mapping_version": "2026.06",
        "functions": [{
            "function_id": function_id,
            "version": "1",
            "deterministic": true,
            "dependency": "unicode:15.1"
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapFunctionRegistry as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&registry).unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_nullable_name_records(values: &[Option<&str>]) -> Vec<u8> {
    object_file_with_nullable_name_records_and_function_registry(values, &[])
}

fn string_function_dependency(function_id: &str) -> &'static str {
    match function_id {
        "startsWith" => "unicode:15.1;prefix-accelerator:startsWith:exact",
        "length" => "unicode:15.1;encoded-logical-length:exact",
        _ => "unicode:15.1",
    }
}

fn object_file_with_nullable_name_records_and_function_registry(
    values: &[Option<&str>],
    function_ids: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_utf8_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE
        | if function_ids.is_empty() {
            0
        } else {
            FEATURE_SEMANTIC_MAP
        };
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": string_function_dependency(function_id)
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    writer.write().unwrap()
}

fn object_file_with_bool_records(values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_and_function_registry(values, &[])
}

fn object_file_with_bool_records_and_function_registry(
    values: &[bool],
    function_ids: &[&str],
) -> Vec<u8> {
    object_file_with_bool_records_with_file_id_and_function_registry([0; 16], values, function_ids)
}

fn object_file_with_bool_records_with_file_id(file_id: [u8; 16], values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_with_file_id_and_function_registry(file_id, values, &[])
}

fn object_file_with_bool_records_with_file_id_and_function_registry(
    file_id: [u8; 16],
    values: &[bool],
    function_ids: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE
        | if function_ids.is_empty() {
            0
        } else {
            FEATURE_SEMANTIC_MAP
        };
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": if *function_id == "cast" {
                        "safe-cast:declared"
                    } else {
                        "pure"
                    }
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    writer.write().unwrap()
}

fn object_file_with_nullable_bool_records(values: &[Option<bool>]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_bool_records_and_projection(values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_and_projection_with_file_id([0; 16], values)
}

fn object_file_with_bool_records_and_projection_with_file_id(
    file_id: [u8; 16],
    values: &[bool],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "thing-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "thing_projection",
            "output_table": "thing_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Thing"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": "active",
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_bool_records_on_branches(values: &[bool], branches: &[u64]) -> Vec<u8> {
    assert_eq!(values.len(), branches.len());
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .zip(branches.iter())
        .enumerate()
        .map(|(index, (_, branch))| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: *branch,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_bool_change() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [7; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Delta,
            prev_ref: Some(CoveRecordRefV1 {
                segment_id: 7,
                row_index: 0,
                target_kind: 1,
            }),
        },
    ];
    let segment = temporal_segment_with_bool_property(&rows, &[false, true]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn association_file_with_endpoint_change() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };
    let rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [40; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [7; 16],
            record_id: [41; 16],
            record_kind: RecordKind::Delta,
            prev_ref: Some(CoveRecordRefV1 {
                segment_id: 8,
                row_index: 0,
                target_kind: 1,
            }),
        },
    ];
    let segment = temporal_segment_with_association_endpoints(
        &rows,
        &[[1; 16], [1; 16]],
        &[[2; 16], [3; 16]],
    );
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_object_type_rows(
            8,
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_numcode_records(values: &[i64]) -> Vec<u8> {
    for payload_padding in 0..8 {
        let bytes = object_file_with_numcode_records_with_padding(values, payload_padding);
        if retained_metric_values_are_aligned(&bytes) {
            return bytes;
        }
    }
    panic!("could not build aligned retained NumCode object fixture");
}

fn object_file_with_numcode_records_with_padding(
    values: &[i64],
    payload_padding: usize,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "MetricThing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 3,
                property_name: "metric".into(),
                logical_type: CoveLogicalType::Int64,
                physical_kind: CovePhysicalKind::NumCode,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 96; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_numcode_property(&rows, values, payload_padding);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn retained_metric_values_are_aligned(bytes: &[u8]) -> bool {
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_vec(bytes.to_vec()),
        validation_options(),
    )
    .unwrap();
    let payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let values = payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap();
    (values.as_ptr() as usize).is_multiple_of(std::mem::align_of::<u64>())
}

fn metric_zero_copy_map() -> Vec<u8> {
    ZeroCopyBufferMapV2 {
        header: ZeroCopyBufferMapHeaderV2 {
            map_count: 1,
            target_count: 1,
            flags: 0,
            checksum: 0,
        },
        targets: vec![ZeroCopyTargetV2 {
            target_id: 1,
            namespace: "org.apache.arrow".into(),
            target_name: "arrow".into(),
            version_major: 1,
            version_minor: 0,
            flags: 0,
        }],
        entries: vec![ZeroCopyBufferMapEntryV2 {
            target_id: 1,
            table_id: 1,
            column_id: 3,
            segment_id: 7,
            morsel_id: 0,
            page_ref: 1,
            buffer_id: 0,
            buffer_kind: PageBufferKind::Values as u16,
            logical_type: CoveLogicalType::Int64 as u16,
            physical_kind: CovePhysicalKind::NumCode as u8,
            source_endianness: 0,
            required_alignment_log2: 3,
            null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2::NoNullBitmap,
            source_offset_width_bits: 0,
            target_offset_width_bits: 0,
            dictionary_key_width_bits: 0,
            dictionary_semantics: ZeroCopyDictionarySemanticsV2::NoDictionary,
            lifetime_scope: ZeroCopyLifetimeScopeV2::ReaderSession,
            nested_layout_kind: ZeroCopyNestedLayoutKindV2::NotNested,
            compression_required_none: 1,
            target_buffer_role: ZeroCopyTargetBufferRoleV2::Values,
            source_buffer_role: ZeroCopySourceBufferRoleV2::CoveValues,
            target_type_ref: u32::MAX,
            dictionary_values_ref: u32::MAX,
            child_layout_ref: u32::MAX,
            owner_lifetime_ref: u32::MAX,
            flags: 0,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap()
}

fn object_file_with_filecode_records(values: &[&str]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_records_and_function_registry(values, &[])
}

fn object_file_with_filecode_records_with_collation(
    values: &[&str],
    collation_id: u16,
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
        [0; 16],
        "Person",
        "name",
        CoveLogicalType::Utf8,
        collation_id,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_records_with_file_id(
    file_id: [u8; 16],
    values: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        file_id,
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_records_and_function_registry(
    values: &[&str],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_and_function_registry(
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &[],
        function_ids,
    )
}

fn object_file_with_filecode_records_and_redactions(
    values: &[&str],
    redacted_values: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    let redacted_keys = redacted_values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_and_function_registry(
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &redacted_keys,
        &[],
    )
}

fn object_file_with_bool_filecode_records(values: &[bool]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::Bool(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records("FlagThing", "active", CoveLogicalType::Bool, &keys, &[])
}

fn object_file_with_int64_filecode_records(values: &[i64]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            file_dictionary_key_from_canonical(CanonicalValue::Int {
                width: 8,
                value: i128::from(*value),
            })
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records(
        "MetricThing",
        "metric",
        CoveLogicalType::Int64,
        &keys,
        &[],
    )
}

fn object_file_with_uuid_filecode_records(values: &[[u8; 16]]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::Uuid(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records("UuidThing", "uid", CoveLogicalType::Uuid, &keys, &[])
}

fn object_file_with_timestamp_filecode_records(values: &[i64]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::TimestampMicros(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records(
        "EventThing",
        "event_time",
        CoveLogicalType::TimestampMicros,
        &keys,
        &[],
    )
}

fn object_file_with_timestamp_filecode_records_with_file_id(
    file_id: [u8; 16],
    values: &[i64],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::TimestampMicros(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        file_id,
        "EventThing",
        "event_time",
        CoveLogicalType::TimestampMicros,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_key_records(
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_and_function_registry(
        type_name,
        property_name,
        logical_type,
        keys,
        redacted_keys,
        &[],
    )
}

fn object_file_with_filecode_key_records_and_function_registry(
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        [0; 16],
        type_name,
        property_name,
        logical_type,
        keys,
        redacted_keys,
        function_ids,
    )
}

fn object_file_with_filecode_key_records_with_file_id_and_function_registry(
    file_id: [u8; 16],
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
        file_id,
        type_name,
        property_name,
        logical_type,
        0,
        keys,
        redacted_keys,
        function_ids,
    )
}

fn object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
    file_id: [u8; 16],
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    collation_id: u16,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: type_name.into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 2,
                property_name: property_name.into(),
                logical_type,
                physical_kind: CovePhysicalKind::FileCode,
                nullable: false,
                collation_id,
                flags: 0,
            }],
        }],
    };
    let mut dictionary = FileDictionaryEncoding::from_keys(keys.iter().cloned()).unwrap();
    let file_codes = keys
        .iter()
        .map(|key| dictionary.file_code_for_key(key).unwrap())
        .collect::<Vec<_>>();
    let mut redacted_codes = Vec::new();
    for key in redacted_keys {
        let code = dictionary.file_code_for_key(key).unwrap();
        redacted_codes.push(u64::from(code));
        let entry = &mut dictionary.dictionary.entries[code as usize];
        entry.storage_class = StorageClass::Redacted as u8;
        entry.inline_len = 0;
        entry.inline_data = [0; 16];
        entry.payload_offset = 0;
        entry.payload_length = 0;
        entry.canonical_hash64 = 0;
    }
    let mut execution_map = BTreeMap::new();
    for code in 0..dictionary.dictionary.len() {
        execution_map.insert(code, 10_000 + u64::from(code));
    }
    let rows = keys
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 64; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_filecode_property(logical_type, &rows, &file_codes);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };
    let mut dictionary_index = Vec::new();
    dictionary_index.extend_from_slice(&dictionary.dictionary.header.serialize());
    for entry in &dictionary.dictionary.entries {
        dictionary_index.extend_from_slice(&entry.serialize());
    }

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features =
        FEATURE_OBJECT_PROFILE | FEATURE_FILE_DICTIONARY | FEATURE_ENGINE_PROFILE;
    if !function_ids.is_empty() {
        writer.required_features |= FEATURE_SEMANTIC_MAP;
    }
    if !redacted_codes.is_empty() {
        writer.required_features |= FEATURE_REDACTIONS;
    }
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::FileDictionaryIndex as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count: dictionary.dictionary.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_FILE_DICTIONARY,
        optional_features: 0,
        data: dictionary_index,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": string_function_dependency(function_id)
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    if !redacted_codes.is_empty() {
        let manifest = RedactionManifest {
            entries: redacted_codes
                .iter()
                .enumerate()
                .map(|(index, code)| RedactionEntry {
                    redaction_id: index as u64 + 1,
                    section_id: 2,
                    local_ref: *code,
                    reason_code: 0,
                    policy_id: Vec::new(),
                    audit_ref: Vec::new(),
                    created_at_us: 0,
                })
                .collect(),
        };
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::RedactionManifest as u16,
            profile: PrimaryProfile::Mixed as u8,
            flags: 0,
            item_count: redacted_codes.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_REDACTIONS,
            optional_features: 0,
            data: manifest.serialize().unwrap(),
        });
    }
    if !dictionary.dictionary.payload.is_empty() {
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::FileDictionaryPayload as u16,
            profile: PrimaryProfile::Mixed as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_FILE_DICTIONARY,
            optional_features: 0,
            data: dictionary.dictionary.payload.clone(),
        });
    }
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ExecutionCodeDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: ExecutionCodeDescriptorV1 {
            descriptor_id: 1,
            code_kind: ExecutionCodeKind::UnsignedInteger,
            code_width_bits: 64,
            byte_order: 0,
            lifetime: ExecutionCodeLifetime::Scan,
            comparison_scope: ExecutionCodeComparisonScope::File,
            canonicality: ExecutionCodeCanonicality::CanonicalWithinScope,
            null_code_policy: NullCodePolicy::NullBitmapOnly,
            flags: 0,
            scope_ref: 1,
            code_space_ref: 1,
            checksum: 0,
        }
        .serialize()
        .to_vec(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ExecutionScopeDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: ExecutionScopeDescriptorV1 {
            scope_id: 1,
            scope_kind: ExecutionScopeKind::Dataset,
            flags: 0,
            stable_id: b"people".to_vec(),
            display_name: "people".into(),
            private_payload_ref: u32::MAX,
        }
        .serialize()
        .unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::CodeSpaceDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: CodeSpaceDescriptorV1 {
            code_space_id: 1,
            namespace: "org.example.coveql".into(),
            stable_id: b"exec-codes".to_vec(),
            epoch: 1,
            flags: 0,
            private_payload_ref: u32::MAX,
        }
        .serialize()
        .unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::EngineMountPolicy as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: EngineMountPolicyV1 {
            policy_id: 1,
            filecode_mapping_kind: FileCodeMappingKind::MapToExecutionCode,
            missing_value_policy: MissingValuePolicy::Error,
            stale_mapping_policy: StaleMappingPolicy::Reject,
            reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
            flags: 0,
            dictionary_digest_ref: 0,
            code_space_ref: 1,
            cache_key_ref: u32::MAX,
            private_payload_ref: u32::MAX,
            checksum: 0,
        }
        .serialize()
        .to_vec(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    (writer.write().unwrap(), execution_map)
}

fn file_dictionary_key_from_canonical(value: CanonicalValue<'_>) -> FileDictionaryKey {
    FileDictionaryKey {
        value_tag: value.value_tag() as u16,
        canonical: value.encode().unwrap(),
    }
}

fn temporal_segment_entry_for_rows(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
    length: u64,
) -> TemporalSegmentIndexEntryV1 {
    temporal_segment_entry_for_object_type_rows(segment_id, 1, rows, length)
}

fn temporal_segment_entry_for_object_type_rows(
    segment_id: u32,
    object_type_id: u32,
    rows: &[TemporalRowEntryV1],
    length: u64,
) -> TemporalSegmentIndexEntryV1 {
    let (delta_count, snapshot_count, baseline_count, tombstone_count) =
        temporal_row_kind_counts(rows);
    TemporalSegmentIndexEntryV1 {
        segment_id,
        object_type_id,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        delta_count,
        snapshot_count,
        baseline_count,
        tombstone_count,
        min_goid: rows.iter().map(|row| row.goid).min().unwrap_or([0; 16]),
        max_goid: rows.iter().map(|row| row.goid).max().unwrap_or([0; 16]),
        offset: 0,
        length,
        checksum: 0,
    }
}

fn temporal_row_kind_counts(rows: &[TemporalRowEntryV1]) -> (u32, u32, u32, u32) {
    let mut delta_count = 0;
    let mut snapshot_count = 0;
    let mut baseline_count = 0;
    let mut tombstone_count = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta_count += 1,
            RecordKind::Snapshot => snapshot_count += 1,
            RecordKind::Baseline => baseline_count += 1,
            RecordKind::Tombstone => tombstone_count += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta_count, snapshot_count, baseline_count, tombstone_count)
}

fn temporal_segment_with_bool_property(rows: &[TemporalRowEntryV1], values: &[bool]) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values.iter().map(|value| u8::from(*value)).collect();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 1,
        logical_type: CoveLogicalType::Bool,
        physical_kind: CovePhysicalKind::Boolean,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_nullable_bool_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<bool>],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut null_count = 0u32;
    let value_bytes = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if value.is_none() {
                null_count += 1;
                null_bitmap[index / 8] |= 1 << (index % 8);
            }
            u8::from(value.unwrap_or(false))
        })
        .collect();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 1,
        logical_type: CoveLogicalType::Bool,
        physical_kind: CovePhysicalKind::Boolean,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_association_endpoints(
    rows: &[TemporalRowEntryV1],
    sources: &[[u8; 16]],
    targets: &[[u8; 16]],
) -> Vec<u8> {
    assert_eq!(rows.len(), sources.len());
    assert_eq!(rows.len(), targets.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let column_directory_length = 2 * TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_offset = column_directory_offset + column_directory_length;
    let page_index_length = 2 * cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let source_value_bytes = sources
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let target_value_bytes = targets
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let source_payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        source_value_bytes,
    )
    .unwrap();
    let target_payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        target_value_bytes,
    )
    .unwrap();
    let source_data_offset = data_offset;
    let target_data_offset = source_data_offset + source_payload.len() as u64;
    let header = TemporalSegmentHeaderV1 {
        segment_id: 8,
        object_type_id: 7,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 2,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let source_directory = TableColumnDirectoryEntryV1 {
        column_id: 11,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
        flags: 0,
        page_index_offset,
        page_index_length: cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        data_offset: source_data_offset,
        data_length: source_payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let target_directory = TableColumnDirectoryEntryV1 {
        column_id: 12,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
        flags: 0,
        page_index_offset: page_index_offset + cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        page_index_length: cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        data_offset: target_data_offset,
        data_length: target_payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let source_page = ColumnPageIndexEntryV1 {
        column_id: 11,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: source_data_offset,
        page_length: source_payload.len() as u64,
        uncompressed_length: source_payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&source_payload),
    };
    let target_page = ColumnPageIndexEntryV1 {
        column_id: 12,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: target_data_offset,
        page_length: target_payload.len() as u64,
        uncompressed_length: target_payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&target_payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&source_directory.serialize());
    bytes.extend_from_slice(&target_directory.serialize());
    bytes.extend_from_slice(&source_page.serialize());
    bytes.extend_from_slice(&target_page.serialize());
    bytes.extend_from_slice(&source_payload);
    bytes.extend_from_slice(&target_payload);
    bytes
}

fn temporal_segment_with_nullable_utf8_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<&str>],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut value_bytes = Vec::new();
    let mut null_count = 0u32;
    for (index, value) in values.iter().enumerate() {
        if value.is_none() {
            null_count += 1;
            null_bitmap[index / 8] |= 1 << (index % 8);
        }
        let bytes = value.unwrap_or_default().as_bytes();
        value_bytes.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        value_bytes.extend_from_slice(bytes);
    }
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::VarBytes,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 1,
        logical_type: CoveLogicalType::Utf8,
        physical_kind: CovePhysicalKind::VarBytes,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::VarBytes as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_numcode_property(
    rows: &[TemporalRowEntryV1],
    values: &[i64],
    payload_padding: usize,
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values
        .iter()
        .flat_map(|value| (*value as u64).to_le_bytes())
        .collect::<Vec<_>>();
    let payload = aligned_numcode_page_payload(rows.len() as u32, value_bytes, payload_padding);
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 3,
        logical_type: CoveLogicalType::Int64,
        physical_kind: CovePhysicalKind::NumCode,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 3,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::NumCode as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn aligned_numcode_page_payload(row_count: u32, value_bytes: Vec<u8>, padding: usize) -> Vec<u8> {
    let values_offset = COLUMN_PAGE_PAYLOAD_HEADER_LEN
        + COVE_ENCODING_NODE_LEN
        + PAGE_BUFFER_DESCRIPTOR_LEN
        + padding;
    let header = ColumnPagePayloadHeaderV1 {
        magic: COLUMN_PAGE_PAYLOAD_MAGIC,
        version_major: COLUMN_PAGE_PAYLOAD_VERSION_MAJOR,
        header_len: COLUMN_PAGE_PAYLOAD_HEADER_LEN as u16,
        flags: 0,
        root_node_id: 0,
        node_count: 1,
        buffer_count: 1,
        row_count,
        nodes_offset: COLUMN_PAGE_PAYLOAD_HEADER_LEN as u32,
        buffer_directory_offset: (COLUMN_PAGE_PAYLOAD_HEADER_LEN + COVE_ENCODING_NODE_LEN) as u32,
        buffers_offset: (COLUMN_PAGE_PAYLOAD_HEADER_LEN
            + COVE_ENCODING_NODE_LEN
            + PAGE_BUFFER_DESCRIPTOR_LEN) as u32,
        reserved: 0,
    };
    let node = CoveEncodingNodeV1 {
        node_id: 0,
        encoding_kind: CoveEncodingKind::NumCode,
        logical_type: CoveLogicalType::Int64,
        physical_kind: CovePhysicalKind::NumCode,
        flags: 0,
        logical_len: row_count,
        child_count: 0,
        buffer_count: 1,
        params_offset: 0,
        params_length: 0,
        stats_id: u32::MAX,
        reserved: 0,
    };
    let descriptor = PageBufferDescriptorV1 {
        buffer_id: 0,
        kind: PageBufferKind::Values,
        flags: 0,
        offset: values_offset as u64,
        length: value_bytes.len() as u64,
        checksum: checksum::crc32c(&value_bytes),
        reserved: 0,
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.serialize());
    bytes.extend_from_slice(&node.serialize());
    bytes.extend_from_slice(&descriptor.serialize());
    bytes.extend(std::iter::repeat(0).take(padding));
    bytes.extend_from_slice(&value_bytes);
    bytes
}

fn temporal_segment_with_filecode_property(
    logical_type: CoveLogicalType,
    rows: &[TemporalRowEntryV1],
    values: &[u32],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::FileCode,
        logical_type,
        CovePhysicalKind::FileCode,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 2,
        logical_type,
        physical_kind: CovePhysicalKind::FileCode,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 2,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::FileCode as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn object_property_count_covi(bytes: &[u8], row_count: u64) -> Vec<u8> {
    object_property_index_only_covi(bytes, row_count, &[CoviAggregateKindV2::Count])
}

fn object_property_index_only_covi(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
) -> Vec<u8> {
    object_property_index_only_covi_with_target(
        bytes,
        row_count,
        aggregate_kinds,
        IndexOnlyTestTarget {
            object_type_id: 1,
            property_id: 1,
            logical_type: CoveLogicalType::Bool,
            physical_kind: CovePhysicalKind::Boolean,
            aggregate_payloads: Vec::new(),
        },
    )
}

fn object_metric_index_only_covi(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
    sum: i64,
) -> Vec<u8> {
    let mut aggregate_payloads = Vec::new();
    if aggregate_kinds.contains(&CoviAggregateKindV2::Sum) {
        aggregate_payloads.push((CoviAggregateKindV2::Sum, sum.to_le_bytes().to_vec()));
    }
    if aggregate_kinds.contains(&CoviAggregateKindV2::Avg) {
        aggregate_payloads.push((CoviAggregateKindV2::Avg, sum.to_le_bytes().to_vec()));
    }
    object_property_index_only_covi_with_target(
        bytes,
        row_count,
        aggregate_kinds,
        IndexOnlyTestTarget {
            object_type_id: 1,
            property_id: 3,
            logical_type: CoveLogicalType::Int64,
            physical_kind: CovePhysicalKind::NumCode,
            aggregate_payloads,
        },
    )
}

struct IndexOnlyTestTarget {
    object_type_id: u32,
    property_id: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    aggregate_payloads: Vec<(CoviAggregateKindV2, Vec<u8>)>,
}

fn object_property_index_only_covi_with_target(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
    target: IndexOnlyTestTarget,
) -> Vec<u8> {
    let context = build_operation_context(
        bytes,
        CoveQlOperationRequest::default(),
        validation_options(),
    )
    .unwrap();
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
        index_kind: CoviIndexKindV2::Sorted,
        coverage_granularity: 0,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: 0,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        object_type_id: target.object_type_id,
        property_id: target.property_id,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: target.logical_type as u16,
        physical_kind: target.physical_kind as u8,
        key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes as u8,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: row_count,
        distinct_count: 0,
        null_count: 0,
        min_key_ref: u32::MAX,
        max_key_ref: u32::MAX,
        key_block_section_id: 1,
        entry_block_section_id: 2,
        postings_block_section_id: 3,
        aggregate_block_section_id: 4,
        coverage_set_ref: u32::MAX,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 1,
        supports_range: 0,
        supports_membership: 0,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::Count)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Exists)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Avg),
        ),
        supports_min: 0,
        supports_max: 0,
        supports_sum: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::Sum)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Avg),
        ),
        supports_distinct_count: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::DistinctCount),
        ),
        supports_join_coverage: 0,
        supports_index_only: 1,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let index_only = aggregate_kinds
        .iter()
        .map(|kind| IndexOnlyCapabilityV2 {
            capability_id: 0,
            aggregate_kind: *kind as u16,
            predicate_supported: 0,
            exactness: IndexCapabilityExactnessV2::Exact,
            null_semantics: 0,
            flags: 0,
            snapshot_validity_ref: 0,
            required_visibility_overlay_ref: u32::MAX,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    let key_block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: 1,
            index_root_id: 0,
            key_count: 0,
            encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: 0,
            checksum: 0,
        },
        key_data: Vec::new(),
    };
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: 2,
            index_root_id: 0,
            entry_count: 0,
            key_block_id: 1,
            postings_block_id: 3,
            aggregate_block_id: 4,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: 0,
            flags: 0,
            checksum: 0,
        },
        entries: Vec::new(),
    };
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: 3,
            index_root_id: 0,
            postings_count: 0,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: 0,
            flags: 0,
            checksum: 0,
        },
        postings: Vec::new(),
        row_ordinal_sets: Vec::new(),
        payload: Vec::new(),
    };
    let mut aggregate_payload = Vec::new();
    let aggregate_answers = aggregate_kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let value_ref = match kind {
                CoviAggregateKindV2::DistinctCount => {
                    let offset = u32::try_from(aggregate_payload.len()).unwrap();
                    aggregate_payload.extend_from_slice(&0u64.to_le_bytes());
                    offset
                }
                CoviAggregateKindV2::Sum | CoviAggregateKindV2::Avg => {
                    let payload = target
                        .aggregate_payloads
                        .iter()
                        .find(|(payload_kind, _)| payload_kind == kind)
                        .map(|(_, payload)| payload)
                        .expect("sum/avg test sidecar requires a sum payload");
                    let offset = u32::try_from(aggregate_payload.len()).unwrap();
                    aggregate_payload.extend_from_slice(payload);
                    offset
                }
                _ => u32::MAX,
            };
            CoviAggregateAnswerV2 {
                aggregate_answer_ref: index as u32,
                index_root_id: 0,
                aggregate_kind: *kind as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count,
                null_count: 0,
                non_null_count: row_count,
                value_ref,
                predicate_form_ref: u32::MAX,
                snapshot_validity_ref: 0,
                checksum: 0,
            }
        })
        .collect::<Vec<_>>();
    let aggregate_block = CoviAggregateAnswerBlockV2 {
        header: CoviAggregateAnswerBlockHeaderV2 {
            magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
            aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
            aggregate_block_id: 4,
            index_root_id: 0,
            aggregate_answer_count: aggregate_answers.len() as u32,
            aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
            aggregate_payload_offset: 0,
            aggregate_payload_length: aggregate_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        answers: aggregate_answers,
        payload: aggregate_payload,
    };
    let index_only_payload = index_only
        .iter()
        .flat_map(|capability| capability.serialize().unwrap())
        .collect::<Vec<_>>();
    CoviArtifactV2::serialize_with_sections(
        [1; 16],
        [2; 16],
        &[CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: context.file.file_id,
            file_len: context.file.file_len,
            footer_crc32c: context.file.footer_crc32c,
            digest_algorithm: DigestAlgorithm::None as u16,
            digest_len: 0,
            digest_offset: 0,
            uri_ref: u32::MAX,
            schema_fingerprint_ref: u32::MAX,
            checksum: 0,
        }],
        &[CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id: [1; 16],
            snapshot_id: [2; 16],
            schema_fingerprint_ref: u32::MAX,
            semantic_map_fingerprint_ref: u32::MAX,
            external_visibility_ref: u32::MAX,
            data_checksum_root_ref: u32::MAX,
            valid_from_us: 0,
            valid_until_us: i64::MAX,
            flags: 0,
            checksum: 0,
        }],
        &[root],
        &[capability],
        &[
            CoviSectionPayloadV2 {
                section_id: 1,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: key_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 2,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: entry_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 3,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: postings_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 4,
                section_kind: CoviSectionKindV2::AggregateAnswerBlock,
                payload: aggregate_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 5,
                section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
                payload: index_only_payload,
                item_count: aggregate_kinds.len() as u64,
                required_features: 0,
                optional_features: 0,
            },
        ],
    )
    .unwrap()
}

fn object_property_bool_lookup_covi(
    bytes: &[u8],
    type_name: &str,
    property_name: &str,
    value: bool,
) -> Vec<u8> {
    let context = build_operation_context(
        bytes,
        CoveQlOperationRequest::default(),
        validation_options(),
    )
    .unwrap();
    let surface = read_object_surface_from_bytes(bytes).unwrap();
    let object_type = surface
        .object_types
        .iter()
        .find(|object_type| object_type.type_name == type_name)
        .unwrap();
    let property = object_type
        .properties
        .iter()
        .find(|property| property.property_name == property_name)
        .unwrap();
    let mut key_data = Vec::new();
    wire::append_u64_leb128(
        &mut key_data,
        CanonicalValue::Bool(value).value_tag() as u64,
    );
    key_data.extend_from_slice(&CanonicalValue::Bool(value).encode().unwrap());
    let row_ranges = surface
        .records
        .iter()
        .filter(|record| record.object_type_id == object_type.object_type_id)
        .filter(|record| {
            record.properties.iter().any(|record_property| {
                record_property.property_id == property.property_id
                    && record_property.value == json!(value)
            })
        })
        .map(|record| CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: u32::MAX,
            segment_id: record.segment_id,
            morsel_id: 0,
            row_start: u64::from(record.row_index),
            row_count: 1,
            flags: 0,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    assert!(!row_ranges.is_empty());
    let postings_payload = row_ranges
        .iter()
        .flat_map(|range| range.serialize().unwrap())
        .collect::<Vec<_>>();
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
        index_kind: CoviIndexKindV2::Sorted,
        coverage_granularity: 0,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: IndexCapabilityExactnessV2::Exact as u8,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        object_type_id: object_type.object_type_id,
        property_id: property.property_id,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: CoveLogicalType::Bool as u16,
        physical_kind: CovePhysicalKind::Boolean as u8,
        key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes as u8,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: row_ranges.len() as u64,
        distinct_count: 1,
        null_count: 0,
        min_key_ref: 0,
        max_key_ref: 0,
        key_block_section_id: 1,
        entry_block_section_id: 2,
        postings_block_section_id: 3,
        aggregate_block_section_id: u32::MAX,
        coverage_set_ref: u32::MAX,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 1,
        supports_range: 0,
        supports_membership: 1,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 0,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let key_block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: 1,
            index_root_id: 0,
            key_count: 1,
            encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: key_data.len() as u64,
            checksum: 0,
        },
        key_data: key_data.clone(),
    };
    let entry = CoviIndexEntryV2 {
        entry_ref: 0,
        index_root_id: 0,
        entry_id: 0,
        key_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
        flags: 0,
        key_offset: 0,
        key_length: key_data.len() as u32,
        key_hash64: 0,
        postings_ref: 0,
        coverage_set_ref: u32::MAX,
        aggregate_answer_ref: u32::MAX,
        next_duplicate_ref: u32::MAX,
        checksum: 0,
    };
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: 2,
            index_root_id: 0,
            entry_count: 1,
            key_block_id: 1,
            postings_block_id: 3,
            aggregate_block_id: u32::MAX,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: CoviIndexEntryV2::LEN as u64,
            flags: 0,
            checksum: 0,
        },
        entries: vec![entry],
    };
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: 3,
            index_root_id: 0,
            postings_count: 1,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: postings_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        postings: vec![CoviPostingsHeaderV2 {
            postings_ref: 0,
            index_root_id: 0,
            representation: cove_index::CoviPostingRepresentationV2::RowRangeList,
            target_granularity: 0,
            flags: 0,
            item_count: row_ranges.len() as u64,
            payload_offset: 0,
            payload_length: postings_payload.len() as u64,
            coverage_set_ref: u32::MAX,
            checksum: 0,
        }],
        row_ordinal_sets: Vec::new(),
        payload: postings_payload,
    };
    CoviArtifactV2::serialize_with_sections(
        [1; 16],
        [2; 16],
        &[CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: context.file.file_id,
            file_len: context.file.file_len,
            footer_crc32c: context.file.footer_crc32c,
            digest_algorithm: DigestAlgorithm::None as u16,
            digest_len: 0,
            digest_offset: 0,
            uri_ref: u32::MAX,
            schema_fingerprint_ref: u32::MAX,
            checksum: 0,
        }],
        &[CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id: [1; 16],
            snapshot_id: [2; 16],
            schema_fingerprint_ref: u32::MAX,
            semantic_map_fingerprint_ref: u32::MAX,
            external_visibility_ref: u32::MAX,
            data_checksum_root_ref: u32::MAX,
            valid_from_us: 0,
            valid_until_us: i64::MAX,
            flags: 0,
            checksum: 0,
        }],
        &[root],
        &[capability],
        &[
            CoviSectionPayloadV2 {
                section_id: 1,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: key_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 2,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: entry_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 3,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: postings_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
        ],
    )
    .unwrap()
}

fn object_property_bool_coverage(
    bytes: &[u8],
    type_name: &str,
    property_name: &str,
    value: bool,
    predicate_form_ref: u32,
) -> (Vec<u8>, Vec<u8>) {
    let surface = read_object_surface_from_bytes(bytes).unwrap();
    let object_type = surface
        .object_types
        .iter()
        .find(|object_type| object_type.type_name == type_name)
        .unwrap();
    let property = object_type
        .properties
        .iter()
        .find(|property| property.property_name == property_name)
        .unwrap();
    let entries = surface
        .records
        .iter()
        .filter(|record| record.object_type_id == object_type.object_type_id)
        .filter(|record| {
            record.properties.iter().any(|record_property| {
                record_property.property_id == property.property_id
                    && record_property.value == json!(value)
            })
        })
        .map(|record| CoverageSetEntryV2 {
            target_kind: CoverageGranularityV2::RowRange,
            flags: 0,
            file_ref: 0,
            table_id: 0,
            segment_id: record.segment_id,
            morsel_id: u32::MAX,
            page_ref: u32::MAX,
            object_type_id: record.object_type_id,
            path_ref: u32::MAX,
            dimensional_bucket_ref: u32::MAX,
            row_start: u64::from(record.row_index),
            row_count: 1,
            row_ordinal_bitmap_ref: u32::MAX,
            byte_range_ref: u32::MAX,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    assert!(!entries.is_empty());
    let set = CoverageSetV2 {
        header: CoverageSetHeaderV2 {
            coverage_set_id: 1,
            provider_id: 1,
            granularity: CoverageGranularityV2::RowRange,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            predicate_form_ref,
            snapshot_validity_ref: 1,
            total_fragment_count: surface.records.len() as u64,
            covered_fragment_count: entries.len() as u64,
            required_fragment_count_estimate: entries.len() as u64,
            coverage_degree_ppm: 1_000_000,
            tightness_degree_ppm: 1_000_000,
            entries_offset: CoverageSetHeaderV2::LEN as u64,
            entries_length: (entries.len() * CoverageSetEntryV2::LEN) as u64,
            checksum: 0,
        },
        entries,
    };
    let set_bytes = set.serialize().unwrap();
    let proof = CoverageProofRecordV2 {
        proof_id: 1,
        provider_id: 1,
        coverage_set_id: 1,
        predicate_form_ref,
        proof_kind: CoverageProofKindV2::ExactSet,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::RowRange,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref: 1,
        coverage_set_checksum: coverage_set_payload_checksum(&set_bytes),
        proof_payload_ref: u32::MAX,
        checksum: 0,
    };
    (set_bytes, proof.serialize().unwrap().to_vec())
}

fn minimal_map_file() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::SemanticMapping as u8;
    writer.required_features = FEATURE_SEMANTIC_MAP;
    writer.write().unwrap()
}

fn minimal_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn association_file_with_evidence_entries(evidence_entries: Vec<Value>) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    let assertions = evidence_entries
        .iter()
        .map(|entry| {
            json!({
                "assertion_id": entry
                    .get("assertion_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has assertion id"),
                "output_object_id": entry
                    .get("output_object_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has output object id"),
            })
        })
        .collect::<Vec<_>>();
    let output_assertion_ids = assertions
        .iter()
        .filter_map(|assertion| assertion.get("assertion_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.orders",
                "schema_fingerprint": "orders-schema-v1",
                "snapshot_digest": "orders-digest-v1",
                "row_identity_rules": ["order_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "customer_order_identity",
                "object_type": "CustomerPlacedOrder",
                "semantic_role": "association",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "order_id",
                    "source_column": "order_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "link_customer_order",
                "source_id": "crm.orders",
                "identity_rule_id": "customer_order_identity",
                "row_semantics_kind": "AssociationOnly",
                "assertion_kinds": ["association", "evidence"],
                "function_ids": [],
                "output_assertion_ids": output_assertion_ids,
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "assertions": assertions,
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "entries": evidence_entries,
        }),
    );
    writer.write().unwrap()
}

fn minimal_object_with_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_person_and_association_record() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0; 16],
        record_id: [32; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let association_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: [7; 16],
        record_id: [40; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true]);
    let association_segment =
        temporal_segment_with_association_endpoints(&association_rows, &[[0; 16]], &[[2; 16]]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 2,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn object_file_with_two_person_association_record() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 11,
            csn: 2,
            branch_key: 0,
            goid: [2; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let association_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 3,
        branch_key: 0,
        goid: [7; 16],
        record_id: [40; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true, false]);
    let association_segment =
        temporal_segment_with_association_endpoints(&association_rows, &[[0; 16]], &[[2; 16]]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 3,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn object_file_with_three_person_two_association_records() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 11,
            csn: 2,
            branch_key: 0,
            goid: [2; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 12,
            csn: 3,
            branch_key: 0,
            goid: [3; 16],
            record_id: [34; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let association_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 4,
            branch_key: 0,
            goid: [7; 16],
            record_id: [40; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 21,
            csn: 5,
            branch_key: 0,
            goid: [8; 16],
            record_id: [41; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true, false, true]);
    let association_segment = temporal_segment_with_association_endpoints(
        &association_rows,
        &[[0; 16], [2; 16]],
        &[[2; 16], [3; 16]],
    );
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 5,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn minimal_filecode_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 2,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_with_projection_file() -> Vec<u8> {
    minimal_object_with_projection_file_with_evidence_index(false)
}

fn minimal_object_with_projection_and_evidence_index_file() -> Vec<u8> {
    minimal_object_with_projection_file_with_evidence_index(true)
}

fn minimal_object_with_projection_file_with_evidence_index(
    include_evidence_index: bool,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "customer-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": "active",
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    if include_evidence_index {
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v1",
                    "snapshot_digest": "digest-v1",
                    "row_identity_rules": ["customer_id"],
                    "replay_claimed": true
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "identity_rules": [{
                    "rule_id": "projection_identity",
                    "object_type": "Person",
                    "semantic_role": "projection_row",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": [],
                    "join_keys": [{
                        "role_id": "customer_id",
                        "source_column": "customer_id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "asc"
                    }]
                }],
                "do_not_merge": []
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "rules": [{
                    "rule_id": "projection_people",
                    "source_id": "crm.customers",
                    "identity_rule_id": "projection_identity",
                    "row_semantics_kind": "ProjectionOnly",
                    "assertion_kinds": ["projection", "evidence"],
                    "function_ids": [],
                    "output_assertion_ids": ["assert_people_projection_row"],
                    "association_endpoints": []
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapAssertionLog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "assertions": [{
                    "assertion_id": "assert_people_projection_row",
                    "output_object_id": "projection:people:1"
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapEvidenceIndex,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "entries": [{
                    "source_id": "crm.customers",
                    "source_row_identity": "customer_id=1",
                    "rule_id": "projection_people",
                    "assertion_id": "assert_people_projection_row",
                    "output_object_id": "projection:people:1",
                    "operation_target": "projection"
                }]
            }),
        );
    }
    writer.write().unwrap()
}

fn minimal_object_with_two_column_projection_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
                PropertyEntryV1 {
                    property_id: 2,
                    property_name: "enabled".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "customer-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [
                {
                    "name": "active",
                    "value": "property.active",
                    "logical_type": "bool"
                },
                {
                    "name": "enabled",
                    "value": "property.enabled",
                    "logical_type": "bool"
                }
            ],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_with_evidence_index_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v1",
                "snapshot_digest": "digest-v1",
                "row_identity_rules": ["customer_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "person_identity",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "upsert_person",
                "source_id": "crm.customers",
                "identity_rule_id": "person_identity",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": [],
                "output_assertion_ids": ["assert_person"],
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "assertions": [{
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1"
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": [{
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1",
                "observed_schema_fingerprint": "schema-v1",
                "observed_snapshot_digest": "digest-v1",
                "operation_target": "object",
                "object_type": "Person"
            }]
        }),
    );
    writer.write().unwrap()
}

fn person_file_with_bool_records_and_evidence_entries(
    values: &[bool],
    evidence_entries: Vec<Value>,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    let assertions = evidence_entries
        .iter()
        .map(|entry| {
            json!({
                "assertion_id": entry
                    .get("assertion_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has assertion id"),
                "output_object_id": entry
                    .get("output_object_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has output object id"),
            })
        })
        .collect::<Vec<_>>();
    let output_assertion_ids = assertions
        .iter()
        .filter_map(|assertion| assertion.get("assertion_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v1",
                "snapshot_digest": "digest-v1",
                "row_identity_rules": ["customer_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "person_identity",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "upsert_person",
                "source_id": "crm.customers",
                "identity_rule_id": "person_identity",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": [],
                "output_assertion_ids": output_assertion_ids,
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "assertions": assertions,
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": evidence_entries,
        }),
    );
    writer.write().unwrap()
}

fn push_embedded_map_section(
    writer: &mut MinimalCoveWriter,
    section_kind: SectionKind,
    mut value: Value,
) {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".into(),
            Value::String("org.coveformat.covemap.v2".into()),
        );
        object.insert(
            "section_id".into(),
            Value::Number((section_kind as u16).into()),
        );
    }
    writer.sections.push(SectionPayload {
        section_kind: section_kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&value).unwrap(),
    });
}

#[test]
fn object_context_validates_minimal_object_file() {
    let context = build_operation_context(
        &minimal_object_file(),
        CoveQlOperationRequest::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        context.file.primary_profile,
        PrimaryProfile::ObjectTemporal as u8
    );
    assert_eq!(context.validation_reports.len(), 2);
    assert_eq!(
        context.explain_json()["operation_context"]["operation"],
        "object_reconstruction"
    );
    assert!(context.snapshot.dataset_id.is_some());
    assert!(context.snapshot.snapshot_id.is_some());
    assert!(context.snapshot.schema_fingerprint.is_some());
    assert!(context.snapshot.file_digest.is_some());
    assert!(context.snapshot.authority.is_some());
    let explain = context.explain_json();
    let operation_context = &explain["operation_context"];
    assert!(operation_context["dataset_id"].is_string());
    assert!(operation_context["snapshot_id"].is_string());
    assert!(operation_context["schema_fingerprint"].is_string());
    assert!(operation_context["file_digest"].is_string());
    assert!(operation_context["authority"].is_string());
    assert_eq!(context.dataset.files.len(), 1);
    assert!(context
        .dataset
        .file_membership_fingerprint
        .starts_with("sha256:"));
    assert_eq!(
        context.dataset.object_schema_fingerprint,
        context.snapshot.schema_fingerprint
    );
    assert_eq!(
        context.dataset.semantic_map_fingerprint,
        context.snapshot.semantic_map_fingerprint
    );
    assert_eq!(context.dataset.projection_catalog_fingerprint, None);
    assert!(operation_context["dataset"].is_object());
    assert!(context
        .optional_metadata
        .iter()
        .any(|outcome| outcome.kind == OptionalMetadataKind::CoveCache
            && outcome.status == OptionalMetadataStatus::Disabled));
}

#[test]
fn manifest_dataset_scope_validates_members_and_keeps_bridges_inexact() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
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
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();

    assert_eq!(scope.scope_version, 1);
    assert_eq!(scope.files.len(), 2);
    assert_eq!(
        scope.cross_file_ordering,
        coveql::CrossFileOrderingPolicy::CanonicalDatasetOrder
    );
    assert!(scope.execution_code_domains.is_empty());
    assert_eq!(
        scope.object_identity,
        coveql::CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid
    );
    assert_eq!(
        scope.association_identity,
        coveql::CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints
    );
    assert!(scope.file_membership_fingerprint.starts_with("sha256:"));
    assert!(scope
        .object_schema_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| fingerprint.starts_with("sha256:")));
    assert_eq!(scope.semantic_map_fingerprint, None);
    assert_eq!(scope.projection_catalog_fingerprint, None);
    assert!(scope.manifest_id.as_deref().unwrap().starts_with("covm:"));
    assert!(scope.snapshot_id.as_deref().unwrap().contains("sha256:"));
    assert_eq!(scope.security_scope.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(
        scope.security_scope.principal_or_session.as_deref(),
        Some("principal-a")
    );
    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert!(!scope.code_domain_bridges[0].exact);
    assert!(scope.code_domain_bridges[0]
        .bridge_kind
        .contains("requires_canonical_remap"));
}

#[test]
fn manifest_dataset_scope_accepts_explicit_exact_code_domain_bridge_proof() {
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

    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert_eq!(
        scope.code_domain_bridges[0].domain_id,
        "cove_e:org.example.coveql:exec-codes"
    );
    assert_eq!(
        scope.code_domain_bridges[0].bridge_kind,
        "manifest_validated_canonical_remap"
    );
    assert_eq!(scope.code_domain_bridges[0].epoch, Some(1));
    assert_eq!(
        scope.code_domain_bridges[0].security_scope_id.as_deref(),
        Some("tenant:tenant-a")
    );
    assert!(scope.code_domain_bridges[0].exact);
    assert!(scope.code_domain_bridges[0].reason.contains("epoch=1"));
    assert_eq!(scope.execution_code_domains.len(), 2);
    assert!(scope
        .execution_code_domains
        .iter()
        .all(|domain| domain.epoch == Some(1)
            && domain.semantic_domain_id.as_deref()
                == Some("cove_e:org.example.coveql:exec-codes")));
}

#[test]
fn manifest_dataset_scope_rejects_exact_bridge_proof_without_epoch() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let err = coveql::build_manifest_dataset_scope_context(
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
                domain_id: "customer_status".into(),
                bridge_kind: "manifest_validated_canonical_remap".into(),
                exact: true,
                epoch: None,
                reason: "manifest member dictionaries remap to the same canonical code domain"
                    .into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("missing"));
    assert!(err.diagnostics[0].message.contains("epoch"));
    assert_eq!(
        err.rejections[0].kind,
        coveql::RejectionKind::UnsupportedDatasetScope
    );
}

#[test]
fn manifest_dataset_scope_rejects_exact_bridge_proof_for_unobserved_epoch() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let err = coveql::build_manifest_dataset_scope_context(
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
                epoch: Some(42),
                reason: "stale remap proof".into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("epoch 42"));
    assert!(err.diagnostics[0].message.contains("observed on 0 of 2"));
}

#[test]
fn manifest_dataset_scope_rejects_exact_raw_local_code_bridge_kind() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let err = coveql::build_manifest_dataset_scope_context(
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
                domain_id: "customer_status".into(),
                bridge_kind: "raw_local_code_equality".into(),
                exact: true,
                epoch: Some(42),
                reason: "unsafe raw local codes happen to match".into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("canonical remap"));
    assert!(err.diagnostics[0].message.contains("raw local-code"));
}

#[test]
fn manifest_dataset_scope_rejects_duplicate_bridge_proofs_for_same_domain() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let err = coveql::build_manifest_dataset_scope_context(
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
            code_domain_bridge_proofs: vec![
                coveql::ManifestCodeDomainBridgeProof {
                    domain_id: "customer_status".into(),
                    bridge_kind: "manifest_validated_canonical_remap".into(),
                    exact: true,
                    epoch: Some(42),
                    reason: "first proof".into(),
                },
                coveql::ManifestCodeDomainBridgeProof {
                    domain_id: "customer_status".into(),
                    bridge_kind: "materialized_canonical_value".into(),
                    exact: false,
                    epoch: None,
                    reason: "conflicting proof".into(),
                },
            ],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("ambiguous"));
    assert!(err.diagnostics[0].message.contains("more than one proof"));
}

#[test]
fn manifest_dataset_scope_redacts_explicit_bridge_proof_when_security_blocks_metadata() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
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
            code_domain_bridge_proofs: vec![coveql::ManifestCodeDomainBridgeProof {
                domain_id: "sensitive_domain".into(),
                bridge_kind: "manifest_validated_canonical_remap".into(),
                exact: true,
                epoch: Some(42),
                reason: "sensitive remap proof".into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();

    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert_eq!(
        scope.code_domain_bridges[0].domain_id,
        "redacted:manifest_code_domains"
    );
    assert_eq!(scope.code_domain_bridges[0].bridge_kind, "security_blocked");
    assert!(!scope.code_domain_bridges[0].exact);
    assert!(!scope.code_domain_bridges[0]
        .reason
        .contains("sensitive_domain"));
}

#[test]
fn manifest_dataset_scope_blocks_code_domain_bridge_details_without_metadata_permission() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
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

    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert_eq!(
        scope.code_domain_bridges[0].domain_id,
        "redacted:manifest_code_domains"
    );
    assert_eq!(scope.code_domain_bridges[0].bridge_kind, "security_blocked");
    assert!(!scope.code_domain_bridges[0].exact);
    assert!(scope.code_domain_bridges[0]
        .reason
        .contains("security policy blocks manifest code-domain bridge exposure"));
}

#[test]
fn manifest_dataset_scope_blocks_code_domain_bridge_details_without_tenant_scope() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
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
            security: SecurityContext {
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();

    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert_eq!(
        scope.code_domain_bridges[0].domain_id,
        "redacted:manifest_code_domains"
    );
    assert_eq!(scope.code_domain_bridges[0].bridge_kind, "security_blocked");
    assert!(scope.code_domain_bridges[0]
        .reason
        .contains("tenant-scoped security context is required"));
}

#[test]
fn manifest_dataset_scope_blocks_code_domain_bridge_details_without_principal_scope() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
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
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            code_domain_bridge_proofs: vec![coveql::ManifestCodeDomainBridgeProof {
                domain_id: "sensitive_domain".into(),
                bridge_kind: "manifest_validated_canonical_remap".into(),
                exact: true,
                epoch: Some(42),
                reason: "sensitive remap proof".into(),
            }],
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();

    assert_eq!(scope.code_domain_bridges.len(), 1);
    assert_eq!(
        scope.code_domain_bridges[0].domain_id,
        "redacted:manifest_code_domains"
    );
    assert_eq!(scope.code_domain_bridges[0].bridge_kind, "security_blocked");
    assert!(!scope.code_domain_bridges[0].exact);
    assert!(scope.code_domain_bridges[0]
        .reason
        .contains("principal or session scope is required"));
    assert!(!scope.code_domain_bridges[0]
        .reason
        .contains("sensitive_domain"));
}

#[test]
fn manifest_dataset_scope_rejects_tenant_visibility_scope_mismatch() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let err = coveql::build_manifest_dataset_scope_context(
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
                visibility_policy: VisibilityPolicy::ExternalOverlay("tenant-b".into()),
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0]
        .message
        .contains("tenant/security scope mismatch"));
}

#[test]
fn manifest_dataset_scope_rejects_incompatible_object_schemas() {
    let left = minimal_object_file_with_id([0xA1; 16]);
    let right = minimal_incompatible_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);

    let err = coveql::build_manifest_dataset_scope_context(
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
            security: SecurityContext {
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("object schema"));
    assert_eq!(
        err.rejections[0].kind,
        coveql::RejectionKind::UnsupportedDatasetScope
    );
}

#[test]
fn manifest_dataset_scope_rejects_incompatible_projection_catalogs() {
    let left = minimal_object_projection_file_with_id([0xA1; 16], "active");
    let right = minimal_object_projection_file_with_id([0xB2; 16], "enabled");
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);

    let err = coveql::build_manifest_dataset_scope_context(
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
            security: SecurityContext {
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("projection catalog"));
    assert_eq!(
        err.rejections[0].kind,
        coveql::RejectionKind::UnsupportedDatasetScope
    );
}

#[test]
fn manifest_dataset_scope_rejects_incompatible_semantic_map_identity() {
    let left = minimal_object_projection_file_with_id_and_mapping(
        [0xA1; 16],
        "active",
        "people-map",
        "2026.05",
    );
    let right = minimal_object_projection_file_with_id_and_mapping(
        [0xB2; 16],
        "active",
        "other-map",
        "2026.05",
    );
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);

    let err = coveql::build_manifest_dataset_scope_context(
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
            security: SecurityContext {
                metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
                ..SecurityContext::default()
            },
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0].message.contains("semantic-map identity"));
    assert_eq!(
        err.rejections[0].kind,
        coveql::RejectionKind::UnsupportedDatasetScope
    );
}

#[test]
fn operation_context_reports_execution_code_domain_under_active_security_scope() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue"]);
    let request = CoveQlOperationRequest {
        execution_code_mapping_requested: true,
        security: SecurityContext {
            principal_or_session: Some("principal-a".into()),
            metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
            ..SecurityContext::default()
        },
        ..CoveQlOperationRequest::default()
    };

    let context = build_operation_context(&bytes, request, validation_options()).unwrap();
    assert_eq!(
        context
            .dataset
            .security_scope
            .principal_or_session
            .as_deref(),
        Some("principal-a")
    );
    let execution_domain = context.dataset.execution_code_domains.first().unwrap();
    assert_eq!(
        execution_domain.security_scope_id.as_deref(),
        Some("principal:principal-a")
    );
    assert_eq!(execution_domain.comparison_scope, "File");
    assert_eq!(execution_domain.lifetime, "Scan");
    assert_eq!(execution_domain.null_code_policy, "NullBitmapOnly");
    assert_eq!(execution_domain.epoch, Some(1));
    assert!(!execution_domain.exact);
    assert!(execution_domain.reason.contains("runtime remap proof"));
    assert!(context.optional_metadata.iter().any(|outcome| {
        outcome.kind == OptionalMetadataKind::CoveE
            && outcome.status == OptionalMetadataStatus::Trusted
    }));
}

#[test]
fn manifest_dataset_scope_rejects_stale_member_identity() {
    let original = minimal_object_file_with_id([0xA1; 16]);
    let stale = minimal_object_file_with_id([0xB2; 16]);
    let manifest = covm_manifest_for_members(&[("member.cove", &original)]);

    let err = coveql::build_manifest_dataset_scope_context(
        &manifest,
        &[coveql::ManifestDatasetMember {
            source: "member.cove",
            bytes: &stale,
        }],
        coveql::ManifestDatasetScopeOptions::default(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_STALE_SIDECAR");
    assert_eq!(
        err.rejections[0].kind,
        coveql::RejectionKind::FeatureValidation
    );
}

#[test]
fn single_input_execution_rejects_manifest_scoped_multifile_plan() {
    let bytes = minimal_object_file();
    let mut planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut second = planned.resolved.operation_context.dataset.files[0].clone();
    second.ordinal = 1;
    second.source = "right.cove".into();
    second.file_id[0] ^= 0xff;
    planned
        .resolved
        .operation_context
        .dataset
        .files
        .push(second);
    planned.resolved.operation_context.dataset.manifest_id = Some("covm:test-manifest".into());
    planned
        .resolved
        .operation_context
        .dataset
        .cross_file_ordering = coveql::CrossFileOrderingPolicy::CanonicalDatasetOrder;
    planned.resolved.operation_context.dataset.object_identity =
        coveql::CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid;
    planned
        .resolved
        .operation_context
        .dataset
        .association_identity =
        coveql::CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints;

    let err = coveql::execute_planned_query_retained(
        CoveQlRetainedInput::from_vec(bytes),
        planned,
        ExecutionOptions::default(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_DATASET_SCOPE");
    assert!(err.diagnostics[0]
        .message
        .contains("single-input CoveQL executor refuses"));
    assert_eq!(err.diagnostics[0].safe_details["file_count"], json!(2));
}

#[test]
fn manifest_member_execution_applies_global_order_and_paging() {
    let left = object_file_with_bool_records_with_file_id([0xA1; 16], &[true, true]);
    let right = object_file_with_bool_records_with_file_id([0xB2; 16], &[true, true]);
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
    assert!(scope
        .object_schema_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| fingerprint.starts_with("sha256:")));
    assert_eq!(scope.semantic_map_fingerprint, None);
    assert_eq!(scope.projection_catalog_fingerprint, None);
    let mut planned = parse_resolve_and_plan_query(
        &left,
        "Thing.where(active == true).take(3)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ObjectRows),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;

    let executed = coveql::execute_manifest_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let CoveQlExecutionResult::ObjectRows(rows) = executed.result else {
        panic!("expected object rows");
    };
    assert_eq!(rows.len(), 3);
    assert_eq!(executed.row_counts.input_rows, 4);
    assert_eq!(executed.row_counts.filtered_rows, 4);
    assert_eq!(executed.row_counts.output_rows, 3);
    assert_eq!(rows[0].dataset_file_source.as_deref(), Some("left.cove"));
    assert_eq!(rows[1].dataset_file_source.as_deref(), Some("right.cove"));
    assert_eq!(rows[2].dataset_file_source.as_deref(), Some("left.cove"));
    let manifest_warning = executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_MATERIALIZED_MANIFEST_BASELINE")
        .expect("manifest execution reports materialized authority");
    assert_eq!(
        manifest_warning.safe_details["exact_code_domain_bridge_count"],
        json!(0)
    );
    assert_eq!(
        manifest_warning.safe_details["fallback_boundary"],
        json!("manifest_cross_file_bridge_not_exact")
    );
    assert_eq!(
        executed.pushdown_report.outcome,
        PushdownOutcome::NotApplicable
    );
}

#[test]
fn manifest_member_execution_reports_exact_bridge_materialized_boundary() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        r#"Person.where(name == "red").select(name)"#,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;

    let executed = coveql::execute_manifest_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        planned,
        ExecutionOptions::default(),
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, vec![json!({"name": "red"}), json!({"name": "red"})]);
    let manifest_warning = executed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "W_MATERIALIZED_MANIFEST_BASELINE")
        .expect("manifest execution reports materialized authority");
    assert_eq!(
        manifest_warning.safe_details["exact_code_domain_bridge_count"],
        json!(1)
    );
    assert_eq!(
        manifest_warning.safe_details["fallback_boundary"],
        json!("manifest_physical_kernel_not_selected")
    );
    assert!(manifest_warning
        .message
        .contains("validated exact COVM code-domain bridge proofs"));
}

#[test]
fn manifest_physical_kernel_executes_exact_bridge_direct_projection_with_compare() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        r#"Person.where(name == "red").select(name)"#,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let executed = execute_manifest_physical_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = &executed.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"name": "red"}), json!({"name": "red"})]);
    assert!(executed.kernel_report.optimization_authority.authoritative);
    assert!(
        !executed
            .kernel_report
            .optimization_authority
            .residual_required
    );
    assert!(executed.kernel_report.compared_with_materialized);
    assert!(executed.executed.authority.compared_with_materialized);
    assert!(executed.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_MANIFEST_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["residual_verification"] == json!(false)
            && diagnostic.safe_details["file_count"] == json!(2)
    }));
    assert!(executed
        .executed
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.code == "W_MANIFEST_KERNEL_COMPARE_MATCHED" }));
}

#[test]
fn manifest_physical_kernel_executes_exact_direct_aggregate_after_global_merge() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        "Person.select(n: count(*))",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let executed = execute_manifest_physical_planned_query(
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
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = &executed.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(rows, &vec![json!({"n": 4})]);
    assert!(executed.kernel_report.compared_with_materialized);
    assert!(executed.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_MANIFEST_KERNEL_NATIVE_DIRECT_AGGREGATE_EXECUTED"
            && diagnostic.safe_details["aggregate"] == json!("count")
            && diagnostic.safe_details["rows_counted"] == json!(4)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn manifest_physical_kernel_groups_filecode_values_after_exact_bridge_merge() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowExact,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        "Person.groupBy(name).select(name, n: count(*))",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let executed = execute_manifest_physical_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = &executed.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        &vec![
            json!({"n": 1, "name": "blue"}),
            json!({"n": 1, "name": "green"}),
            json!({"n": 2, "name": "red"}),
        ]
    );
    assert!(executed.kernel_report.compared_with_materialized);
    assert!(executed.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_MANIFEST_KERNEL_NATIVE_DIRECT_GROUP_AGGREGATE_EXECUTED"
            && diagnostic.safe_details["group_property"] == json!("name")
            && diagnostic.safe_details["group_count"] == json!(3)
            && diagnostic.safe_details["rows_counted"] == json!(4)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn manifest_physical_kernel_executes_projection_root_without_code_bridge() {
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
            output_mode: Some(CoveQlOutputMode::JsonRows),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let executed = execute_manifest_physical_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        executed.executed.authority.source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!executed.executed.authority.materialized_fallback);
    assert!(!executed.executed.authority.residual_required);
    assert!(executed.kernel_report.compared_with_materialized);
    let CoveQlExecutionResult::JsonRows(rows) = executed.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![
            json!({"active": false}),
            json!({"active": true}),
            json!({"active": false}),
            json!({"active": true}),
            json!({"active": true}),
        ]
    );
}

#[test]
fn manifest_physical_kernel_executes_projection_rows_without_code_bridge() {
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
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let executed = execute_manifest_physical_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        executed.executed.authority.source,
        coveql::ExecutionAuthoritySource::ExactOptimizedKernel
    );
    assert!(!executed.executed.authority.materialized_fallback);
    assert!(!executed.executed.authority.residual_required);
    assert!(executed.kernel_report.compared_with_materialized);
    let CoveQlExecutionResult::ProjectionRows(rows) = executed.executed.result else {
        panic!("expected projection rows");
    };
    assert_eq!(rows.len(), 5);
    assert!(rows
        .iter()
        .all(|row| row.projection_id == "thing_projection"));
}

#[test]
fn manifest_physical_kernel_executes_role_bound_asof_direct_projection() {
    let (left, _) = object_file_with_timestamp_filecode_records_with_file_id([0xA1; 16], &[1, 3]);
    let (right, _) = object_file_with_timestamp_filecode_records_with_file_id([0xB2; 16], &[2, 4]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let executed = execute_manifest_physical_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &right,
            },
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
        ],
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap();

    let CoveQlExecutionResult::JsonRows(rows) = &executed.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        &vec![json!({"event_time": 1}), json!({"event_time": 2})]
    );
    assert!(executed.kernel_report.compared_with_materialized);
    assert!(executed.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_MANIFEST_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
}

#[test]
fn manifest_physical_kernel_force_rejects_cross_file_direct_projection_without_bridge() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
            ..coveql::ManifestDatasetScopeOptions::default()
        },
    )
    .unwrap();
    let mut planned = parse_resolve_and_plan_query(
        &left,
        r#"Person.select(name)"#,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let err = execute_manifest_physical_planned_query(
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
        physical,
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::ForceKernel,
            ..KernelExecutionOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSAFE_CODE_DOMAIN");
    assert_eq!(
        err.diagnostics[0].safe_details["fallback_boundary"],
        json!("manifest_materialized")
    );
    assert!(
        err.diagnostics[0].safe_details["kernel_shape"]["operator_contracts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|contract| {
                contract["representation_class"] == json!("cross_source_code_bridge")
                    && contract["residual_required"] == json!(true)
                    && contract["reason"].as_str().is_some_and(|text| {
                        text.contains("requires an exact canonical remap bridge")
                    })
            })
    );
}

#[test]
fn manifest_member_execution_rejects_stale_member_bytes() {
    let left = object_file_with_bool_records_with_file_id([0xA1; 16], &[true]);
    let right = object_file_with_bool_records_with_file_id([0xB2; 16], &[true]);
    let stale_right = object_file_with_bool_records_with_file_id([0xC3; 16], &[true]);
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
        "Thing.take(1)",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ObjectRows),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;

    let err = coveql::execute_manifest_planned_query(
        &[
            coveql::ManifestDatasetMember {
                source: "left.cove",
                bytes: &left,
            },
            coveql::ManifestDatasetMember {
                source: "right.cove",
                bytes: &stale_right,
            },
        ],
        planned,
        ExecutionOptions::default(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_DATASET_MEMBER_STALE");
    assert_eq!(
        err.diagnostics[0].safe_details["source"],
        json!("right.cove")
    );
}

#[test]
fn explain_schema_declares_dataset_context_fields() {
    let schema = coveql::explain_json_schema();
    assert_eq!(schema.version, coveql::EXPLAIN_JSON_SCHEMA_VERSION);
    assert!(schema.modes.contains(&"coded"));
    assert!(schema
        .required_top_level_fields
        .contains(&"operation_context"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"language_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"logical_plan_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"physical_plan_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"projection_dependency_contract_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"predicate_normal_form_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"explain_json_schema_version"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"dataset"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"schema_fingerprint"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"semantic_map_fingerprint"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"file_digest"));
    assert!(schema
        .required_operation_context_fields
        .contains(&"authority"));
    assert!(schema
        .required_coded_execution_fields
        .contains(&"coded_suitability"));
    assert!(schema
        .required_coded_execution_fields
        .contains(&"fallback_reasons"));
    assert!(schema
        .required_coded_operator_contract_fields
        .contains(&"contract_version"));
    assert!(schema
        .required_coded_operator_contract_fields
        .contains(&"required_metadata"));
    assert!(schema
        .required_coded_operator_contract_fields
        .contains(&"row_grain"));
    assert!(schema
        .required_physical_sidecar_validation_fields
        .contains(&"report_version"));
    assert!(schema
        .required_physical_sidecar_validation_fields
        .contains(&"candidate_count"));
    assert!(schema
        .required_physical_plan_sidecar_fields
        .contains(&"sidecar_validations"));
    assert!(schema
        .required_physical_plan_sidecar_fields
        .contains(&"runtime_compatibility"));
    assert!(schema
        .required_physical_plan_sidecar_fields
        .contains(&"zero_copy_eligibility"));

    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let context_json = &planned.explain_json()["operation_context"];
    assert_eq!(
        context_json["language_version"],
        coveql::COVEQL_LANGUAGE_VERSION
    );
    assert_eq!(
        context_json["grammar_version"],
        coveql::COVEQL_GRAMMAR_VERSION
    );
    assert_eq!(
        context_json["resolved_ast_version"],
        coveql::RESOLVED_AST_VERSION
    );
    assert_eq!(
        context_json["logical_plan_version"],
        coveql::LOGICAL_PLAN_VERSION
    );
    assert_eq!(
        context_json["physical_plan_version"],
        coveql::PHYSICAL_PLAN_VERSION
    );
    assert_eq!(
        context_json["projection_dependency_contract_version"],
        coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION
    );
    assert_eq!(
        context_json["predicate_normal_form_version"],
        coveql::PREDICATE_NORMAL_FORM_VERSION
    );
    assert_eq!(
        context_json["explain_json_schema_version"],
        coveql::EXPLAIN_JSON_SCHEMA_VERSION
    );
}

#[test]
fn projection_and_evidence_contexts_validate_map_profile() {
    let bytes = minimal_map_file();

    let projection_request = CoveQlOperationRequest {
        selected_operation: CoveQlSelectedOperation::Projection,
        ..CoveQlOperationRequest::default()
    };
    let projection_context =
        build_operation_context(&bytes, projection_request, validation_options()).unwrap();
    assert_eq!(
        projection_context.file.primary_profile,
        PrimaryProfile::SemanticMapping as u8
    );
    assert_eq!(projection_context.validation_reports.len(), 2);

    let evidence_request = CoveQlOperationRequest {
        selected_operation: CoveQlSelectedOperation::Evidence,
        ..CoveQlOperationRequest::default()
    };
    let evidence_context = build_operation_context(
        &minimal_object_with_evidence_index_file(),
        evidence_request,
        validation_options(),
    )
    .unwrap();
    assert!(evidence_context
        .selected_feature_uses
        .iter()
        .any(|feature_use| feature_use.requested_operation
            == Some(cove_core::feature_binding::OperationKindV2::EvidenceReadback)));
    assert!(evidence_context
        .selected_feature_uses
        .iter()
        .any(|feature_use| feature_use.requested_operation
            == Some(cove_core::feature_binding::OperationKindV2::ObjectReconstruction)));
}

#[test]
fn fail_open_optional_metadata_is_reported_as_ignored() {
    let bytes =
        include_bytes!("../../../conformance/feature-scope/optional_layout_crc_ignored.cove");
    let mut options = validation_options();
    options.optional_pushdown_policy = OptionalPushdownPolicy::FailOpen;
    let request = CoveQlOperationRequest {
        selected_operation: CoveQlSelectedOperation::ArrowExport {
            zero_copy_requested: false,
        },
        ..CoveQlOperationRequest::default()
    };

    let context = build_operation_context(bytes, request, options).unwrap();
    assert!(context
        .fallbacks
        .iter()
        .any(|fallback| fallback.reason.contains("ignored optional section")));
}

#[test]
fn operation_required_unknown_feature_rejects_selected_query() {
    let bytes = include_bytes!(
        "../../../conformance/feature-scope/operation_scoped_unknown_coverage_reject.cove"
    );
    let request = CoveQlOperationRequest {
        selected_operation: CoveQlSelectedOperation::Explain {
            target: coveql::CoveQlExplainTarget::Object,
            mode: ExplainMode::Proof,
        },
        ..CoveQlOperationRequest::default()
    };

    let err = build_operation_context(bytes, request, validation_options()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_CONSTRUCT");
}

#[test]
fn proof_explain_json_redacts_without_protected_metadata_permission() {
    let request = CoveQlOperationRequest {
        selected_operation: CoveQlSelectedOperation::Explain {
            target: coveql::CoveQlExplainTarget::Object,
            mode: ExplainMode::Proof,
        },
        security: SecurityContext {
            explain_policy: ExplainDisclosurePolicy::Proof,
            aggregate_disclosure_policy: AggregateDisclosurePolicy::AllowMaterializedOnly,
            metadata_disclosure_policy: MetadataDisclosurePolicy::DenyProtected,
            ..SecurityContext::default()
        },
        ..CoveQlOperationRequest::default()
    };

    let context =
        build_operation_context(&minimal_object_file(), request, validation_options()).unwrap();
    let explain = context.explain_json();
    assert_eq!(explain["mode"], "proof");
    assert!(explain["redactions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["code"] == "E_SECURITY_DISCLOSURE_FORBIDDEN" }));
}

#[test]
fn object_query_resolves_property_ids_and_explain_fingerprint() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.where(active == true).select(goid, active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let ResolvedRoot::Object(root) = &resolved.root else {
        panic!("expected object root");
    };
    assert_eq!(root.object_type_id, 1);
    let select = resolved.method_chain.select.as_ref().unwrap();
    assert_eq!(select.len(), 2);
    let ResolvedExpr::Path(path) = &select[1].expr else {
        panic!("expected active path");
    };
    assert_eq!(path.property_id, Some(1));
    assert_eq!(
        resolved.explain_json()["fingerprints"]["resolved_query"],
        resolved.resolved_query_fingerprint
    );
}

#[test]
fn duplicate_methods_reject_during_resolution() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(active).select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_DUPLICATE_METHOD");

    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(csn: 1).history()",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_METHOD_CONFLICT");
}

#[test]
fn method_placement_conflicts_reject_during_resolution() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.groupBy(active).where(active == true).select(active, n: count(*))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_METHOD_CONFLICT");
    assert!(err.diagnostics[0].message.contains("where after groupBy"));

    let err = parse_and_resolve_query(
        &object_file_with_bool_records_and_projection(&[false, true]),
        "table(thing_projection) as l.groupBy(l.active).lookup(table(thing_projection) as r, on: l.active == r.active).select(l.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_METHOD_CONFLICT");
    assert!(err.diagnostics[0]
        .message
        .contains("lookup cannot appear after groupBy"));

    let err = parse_and_resolve_query(
        &object_file_with_person_and_association_record(),
        "node(Person) as c.traverse(out(edge(CustomerPlacedOrder))).asOf(csn: 1).select(c.goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_METHOD_CONFLICT");
    assert!(err.diagnostics[0].message.contains("asOf must appear"));

    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.explain(public).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_METHOD_CONFLICT");
    assert!(err.diagnostics[0]
        .message
        .contains("explain must be the final method"));
}

#[test]
fn timestamp_literals_require_explicit_rfc3339_offsets() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(time: \"2026-01-01T00:00:00\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_LITERAL");
}

#[test]
fn timestamp_system_predicates_coerce_rfc3339_strings_to_micros() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.where(timestamp_us == \"2026-01-01T00:00:00Z\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedPredicate::Compare { right, .. } =
        resolved.method_chain.where_predicate.as_ref().unwrap()
    else {
        panic!("expected compare predicate");
    };
    let ResolvedExpr::Literal(literal) = right else {
        panic!("expected timestamp literal");
    };
    assert_eq!(literal.logical_type, "timestamp_micros");
    assert_eq!(
        literal.typed_value,
        ResolvedLiteralValue::TimestampMicros {
            micros: 1_767_225_600_000_000,
            canonical_rfc3339: "2026-01-01T00:00:00Z".into(),
        }
    );
}

#[test]
fn timestamp_looking_strings_remain_strings_for_utf8_paths() {
    let resolved = parse_and_resolve_query(
        &minimal_filecode_object_file(),
        "Person.where(name == \"2026-01-01T00:00:00Z\").select(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedPredicate::Compare { right, .. } =
        resolved.method_chain.where_predicate.as_ref().unwrap()
    else {
        panic!("expected compare predicate");
    };
    let ResolvedExpr::Literal(literal) = right else {
        panic!("expected string literal");
    };
    assert_eq!(literal.logical_type, "utf8");
    assert_eq!(
        literal.typed_value,
        ResolvedLiteralValue::String("2026-01-01T00:00:00Z".into())
    );
}

#[test]
fn decimal_literals_preserve_precision_and_scale() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(amount: 12.3400)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::Literal(literal) = &select[0].expr else {
        panic!("expected decimal literal");
    };
    assert_eq!(literal.logical_type, "decimal128");
    assert_eq!(literal.precision, Some(6));
    assert_eq!(literal.scale, Some(4));
    assert_eq!(
        literal.typed_value,
        ResolvedLiteralValue::Decimal {
            canonical: "12.3400".into(),
            precision: 6,
            scale: 4,
        }
    );
}

#[test]
fn uuid_literals_canonicalize_for_uuid_paths() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.where(goid == uuid\"00000000-0000-0000-0000-000000000001\").select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedPredicate::Compare { right, .. } =
        resolved.method_chain.where_predicate.as_ref().unwrap()
    else {
        panic!("expected compare predicate");
    };
    let ResolvedExpr::Literal(literal) = right else {
        panic!("expected literal");
    };
    assert_eq!(literal.logical_type, "uuid");
    assert_eq!(
        literal.typed_value,
        ResolvedLiteralValue::Uuid {
            canonical_hex: "00000000000000000000000000000001".into(),
            bytes: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        }
    );
}

#[test]
fn binary_literals_canonicalize_for_binary_paths() {
    let resolved = parse_and_resolve_query(
        &minimal_binary_object_file(),
        "Person.where(payload == x\"48656C6C6F\").select(payload)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ResolvedPredicate::Compare { right, .. } =
        resolved.method_chain.where_predicate.as_ref().unwrap()
    else {
        panic!("expected compare predicate");
    };
    let ResolvedExpr::Literal(literal) = right else {
        panic!("expected literal");
    };
    assert_eq!(literal.logical_type, "binary");
    assert_eq!(
        literal.typed_value,
        ResolvedLiteralValue::Binary {
            canonical_hex: "48656c6c6f".into(),
            bytes: b"Hello".to_vec(),
        }
    );
}

#[test]
fn malformed_uuid_literals_reject_during_resolution() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.where(goid == uuid\"not-a-uuid\").select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_LITERAL");
}

#[test]
fn unknown_paths_are_redacted_by_default() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(secret_property)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_PROPERTY");
    assert!(err.diagnostics[0].redacted);
    assert!(!err.diagnostics[0].message.contains("secret_property"));
}

#[test]
fn protected_diagnostic_policy_can_reveal_unknown_names() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security = SecurityContext {
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(secret_property)",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_PROPERTY");
    assert!(!err.diagnostics[0].redacted);
    assert!(err.diagnostics[0].message.contains("secret_property"));
}

#[test]
fn harmless_quoting_preserves_resolved_fingerprint() {
    let a = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_and_resolve_query(
        &minimal_object_file(),
        "`Person`.select(`active`)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(a.resolved_query_fingerprint, b.resolved_query_fingerprint);
}

#[test]
fn semantic_changes_alter_resolved_fingerprint() {
    let a = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_ne!(a.resolved_query_fingerprint, b.resolved_query_fingerprint);
}

#[test]
fn association_roots_resolve_endpoint_flags() {
    let resolved = parse_and_resolve_query(
        &minimal_association_file(),
        "association(CustomerPlacedOrder).select(source_goid, target_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let ResolvedRoot::Association(root) = &resolved.root else {
        panic!("expected association root");
    };
    assert_eq!(root.object_type_id, 7);
    assert_eq!(root.source_property_id, Some(11));
    assert_eq!(root.target_property_id, Some(12));
}

#[test]
fn directed_association_roots_resolve_endpoint_roles() {
    for (query, direction, endpoint_role) in [
        (
            "out(association(CustomerPlacedOrder)).select(source_goid)",
            coveql::AstAssociationDirection::Out,
            AssociationEndpointRole::Source,
        ),
        (
            "in(association(CustomerPlacedOrder)).select(target_goid)",
            coveql::AstAssociationDirection::In,
            AssociationEndpointRole::Target,
        ),
        (
            "either(association(CustomerPlacedOrder)).select(source_goid)",
            coveql::AstAssociationDirection::Either,
            AssociationEndpointRole::Either,
        ),
    ] {
        let resolved = parse_and_resolve_query(
            &minimal_association_file(),
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            validation_options(),
        )
        .unwrap();
        let ResolvedRoot::Association(root) = &resolved.root else {
            panic!("expected association root");
        };
        assert_eq!(root.direction, Some(direction));
        assert_eq!(root.endpoint_role, endpoint_role);
        assert!(!root.object_relative);
    }
}

#[test]
fn unregistered_functions_reject_during_resolution() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(custom_normalize(active))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_FUNCTION");
}

#[test]
fn lower_requires_unicode_or_collation_function_contract() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(lower(active))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
}

#[test]
fn builtin_functions_resolve_with_coded_safe_contracts_when_kernel_supported() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(coalesce(active, false))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::FunctionCall {
        function_id,
        contract,
        ..
    } = &select[0].expr
    else {
        panic!("expected function call");
    };
    assert_eq!(function_id, "coalesce");
    assert!(contract.deterministic);
    assert_eq!(
        contract.execution_class,
        coveql::FunctionExecutionClass::CodedSafe
    );
}

#[test]
fn coalesce_ignores_leading_null_for_common_type_and_coded_contract() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(value: coalesce(null, active, false))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::FunctionCall {
        function_id,
        logical_type,
        physical_kind,
        contract,
        ..
    } = &select[0].expr
    else {
        panic!("expected function call");
    };

    assert_eq!(function_id, "coalesce");
    assert_eq!(logical_type, "bool");
    assert_eq!(physical_kind, "boolean");
    assert_eq!(
        contract.execution_class,
        coveql::FunctionExecutionClass::CodedSafe
    );
}

#[test]
fn coalesce_rejects_incompatible_non_null_alternatives() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        r#"Person.select(coalesce(active, "fallback"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
}

#[test]
fn safe_cast_function_resolves_with_materialized_contract() {
    let bytes = object_file_with_bool_records_and_function_registry(&[true], &["cast"]);
    let resolved = parse_and_resolve_query(
        &bytes,
        r#"Thing.select(cast(active, "utf8"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::FunctionCall {
        function_id,
        logical_type,
        physical_kind,
        contract,
        ..
    } = &select[0].expr
    else {
        panic!("expected cast function call");
    };

    assert_eq!(function_id, "cast");
    assert_eq!(contract.version, "1");
    assert_eq!(logical_type, "utf8");
    assert_eq!(physical_kind, "var_bytes");
    assert!(contract.dependency.contains("safe-cast"));
    assert_eq!(
        contract.execution_class,
        coveql::FunctionExecutionClass::MaterializedOnly
    );
}

#[test]
fn safe_cast_function_rejects_non_identity_cast_without_covemap_contract() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        r#"Person.select(cast(active, "utf8"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
    assert_eq!(
        err.diagnostics[0].message,
        "non-identity cast requires deterministic COVE-MAP safe-cast metadata"
    );
}

#[test]
fn identity_safe_cast_function_resolves_with_coded_safe_contract() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        r#"Person.select(cast(active, "bool"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::FunctionCall {
        function_id,
        contract,
        ..
    } = &select[0].expr
    else {
        panic!("expected cast function call");
    };

    assert_eq!(function_id, "cast");
    assert!(contract.dependency.contains("coded-identity"));
    assert_eq!(
        contract.execution_class,
        coveql::FunctionExecutionClass::CodedSafe
    );
}

#[test]
fn safe_cast_function_rejects_unknown_target_type() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        r#"Person.select(cast(active, "not_a_type"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
}

#[test]
fn null_check_functions_resolve_with_builtin_contracts() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(active.isNotNull(), isNull(active))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let function_ids = select
        .iter()
        .map(|item| match &item.expr {
            ResolvedExpr::FunctionCall {
                function_id,
                logical_type,
                physical_kind,
                contract,
                ..
            } => {
                assert_eq!(logical_type, "bool");
                assert_eq!(physical_kind, "boolean");
                assert!(contract.deterministic);
                assert_eq!(
                    contract.execution_class,
                    coveql::FunctionExecutionClass::CodedSafe
                );
                function_id.as_str()
            }
            _ => panic!("expected null-check function call"),
        })
        .collect::<Vec<_>>();
    assert_eq!(function_ids, vec!["isNotNull", "isNull"]);
}

#[test]
fn null_check_functions_reject_invalid_arity() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(isNull(active, false))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_FUNCTION_CONTRACT");
}

#[test]
fn registered_covemap_string_functions_resolve_with_coded_safe_bodies() {
    let resolved = parse_and_resolve_query(
        &minimal_string_object_with_function_registry("upper"),
        "Person.select(upper(name))",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let select = resolved.method_chain.select.unwrap();
    let ResolvedExpr::FunctionCall {
        function_id,
        contract,
        logical_type,
        ..
    } = &select[0].expr
    else {
        panic!("expected function call");
    };
    assert_eq!(function_id, "upper");
    assert_eq!(logical_type, "utf8");
    assert_eq!(contract.version, "1");
    assert_eq!(
        contract.execution_class,
        coveql::FunctionExecutionClass::CodedSafe
    );
}

#[test]
fn string_functions_without_covemap_contract_resolve_materialized_only() {
    let resolved = parse_and_resolve_query(
        &object_file_with_nullable_name_records(&[Some("Ada")]),
        r#"Person.select(name_len: length(name), starts_a: startsWith(name, "A"))"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let select = resolved.method_chain.select.unwrap();
    for item in select {
        let ResolvedExpr::FunctionCall {
            contract,
            function_id,
            ..
        } = item.expr
        else {
            panic!("expected function call");
        };
        assert!(matches!(function_id.as_str(), "length" | "startsWith"));
        assert_eq!(
            contract.execution_class,
            coveql::FunctionExecutionClass::MaterializedOnly
        );
        assert_eq!(contract.dependency, "materialized-string-built-in");
        assert!(contract.unicode_or_collation_contract.is_none());
    }
}

#[test]
fn length_and_starts_with_without_exact_accelerator_contracts_remain_materialized() {
    for (bytes, query, expected_function) in [
        (
            minimal_string_object_with_function_registry("length"),
            "Person.select(name_len: length(name))",
            "length",
        ),
        (
            minimal_string_object_with_function_registry("startsWith"),
            r#"Person.select(starts_a: startsWith(name, "A"))"#,
            "startsWith",
        ),
    ] {
        let resolved = parse_and_resolve_query(
            &bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            validation_options(),
        )
        .unwrap();
        let select = resolved.method_chain.select.unwrap();
        let ResolvedExpr::FunctionCall {
            function_id,
            contract,
            ..
        } = &select[0].expr
        else {
            panic!("expected function call");
        };
        assert_eq!(function_id, expected_function);
        assert_eq!(
            contract.execution_class,
            coveql::FunctionExecutionClass::MaterializedOnly
        );
        assert_eq!(contract.dependency, "materialized-string-built-in");
        assert!(contract.unicode_or_collation_contract.is_none());
    }
}

#[test]
fn length_and_starts_with_with_exact_accelerator_contracts_are_coded_safe() {
    for (bytes, query, expected_function, expected_dependency) in [
        (
            object_file_with_nullable_name_records_and_function_registry(
                &[Some("Ada")],
                &["length"],
            ),
            "Person.select(name_len: length(name))",
            "length",
            "encoded-logical-length:exact",
        ),
        (
            object_file_with_nullable_name_records_and_function_registry(
                &[Some("Ada")],
                &["startsWith"],
            ),
            r#"Person.select(starts_a: startsWith(name, "A"))"#,
            "startsWith",
            "prefix-accelerator:startsWith:exact",
        ),
    ] {
        let resolved = parse_and_resolve_query(
            &bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            validation_options(),
        )
        .unwrap();
        let select = resolved.method_chain.select.unwrap();
        let ResolvedExpr::FunctionCall {
            function_id,
            contract,
            ..
        } = &select[0].expr
        else {
            panic!("expected function call");
        };
        assert_eq!(function_id, expected_function);
        assert_eq!(
            contract.execution_class,
            coveql::FunctionExecutionClass::CodedSafe
        );
        assert!(contract.dependency.contains(expected_dependency));
        assert_eq!(
            contract.unicode_or_collation_contract.as_deref(),
            Some(contract.dependency.as_str())
        );
    }
}

#[test]
fn materialized_only_string_functions_do_not_enter_coded_kernel_predicates() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), Some("Åsa"), Some("Bo")]);
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
        .expect("fallback reports coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "function:length"
            && contract["representation_class"] == "decode_boundary"
            && contract["exact"] == false
            && contract["residual_required"] == true
            && contract["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("missing coded contract"))
    }));
    assert!(!contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:length"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn materialized_only_starts_with_does_not_enter_coded_bool_kernel_predicate() {
    let bytes = object_file_with_nullable_name_records(&[Some("Ada"), Some("Bob"), None]);
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
        .expect("fallback reports coded operator contracts");
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "function:startsWith"
            && contract["representation_class"] == "decode_boundary"
            && contract["exact"] == false
            && contract["residual_required"] == true
            && contract["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("missing coded contract"))
    }));
    assert!(!contracts.iter().any(|contract| {
        contract["operator"] == "predicate_function:startsWith"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
}

#[test]
fn mandatory_scalar_functions_execute_with_declared_materialized_contracts() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some(" Ada "), None],
        &["identity", "trim", "lower", "upper", "length"],
    );
    let executed = parse_resolve_plan_and_execute_query(
        &bytes,
        r#"Person.select(lower_name: lower(name), upper_name: upper(name), trimmed: trim(name), name_len: length(name), id_name: identity(name), fallback: coalesce(name, "missing"))"#,
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
            json!({
                "lower_name": " ada ",
                "upper_name": " ADA ",
                "trimmed": "Ada",
                "name_len": 5,
                "id_name": " Ada ",
                "fallback": " Ada "
            }),
            json!({
                "lower_name": null,
                "upper_name": null,
                "trimmed": null,
                "name_len": null,
                "id_name": null,
                "fallback": "missing"
            })
        ]
    );
}

#[test]
fn logical_plan_uses_canonical_object_node_order_and_default_sort() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let kinds = planned
        .logical_plan
        .nodes
        .iter()
        .map(|node| match &node.kind {
            LogicalPlanNodeKind::RootScan { .. } => "root",
            LogicalPlanNodeKind::BranchScope { .. } => "branch",
            LogicalPlanNodeKind::TombstoneScope { .. } => "tombstone",
            LogicalPlanNodeKind::TemporalScope { .. } => "temporal",
            LogicalPlanNodeKind::ScanGrainSelection { .. } => "grain",
            LogicalPlanNodeKind::PreReconstructionFilter { .. } => "pre_filter",
            LogicalPlanNodeKind::Reconstruct { .. } => "reconstruct",
            LogicalPlanNodeKind::VisibilityBarrier { .. } => "visibility",
            LogicalPlanNodeKind::RedactionBarrier { .. } => "redaction",
            LogicalPlanNodeKind::PostReconstructionFilter { .. } => "post_filter",
            LogicalPlanNodeKind::SelectProjection { .. } => "select",
            LogicalPlanNodeKind::Sort { .. } => "sort",
            LogicalPlanNodeKind::SkipTake { .. } => "skip_take",
            LogicalPlanNodeKind::FallbackBoundary { .. } => "fallback",
            LogicalPlanNodeKind::OutputBoundary { .. } => "output",
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        &kinds[..15],
        &[
            "root",
            "branch",
            "tombstone",
            "temporal",
            "grain",
            "pre_filter",
            "reconstruct",
            "visibility",
            "redaction",
            "post_filter",
            "select",
            "sort",
            "skip_take",
            "fallback",
            "output",
        ]
    );
    assert!(planned.logical_plan.default_ordering_applied);
    let LogicalPlanNodeKind::Sort {
        keys, defaulted, ..
    } = &planned.logical_plan.nodes[11].kind
    else {
        panic!("expected sort node");
    };
    assert!(*defaulted);
    assert_eq!(keys[0].field.as_deref(), Some("object_type_id"));
}

#[test]
fn projection_default_ordering_uses_declared_projection_order_before_identity() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_ordered_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let (keys, stable_tiebreaker, defaulted) = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort {
                keys,
                stable_tiebreaker,
                defaulted,
            } => Some((keys, stable_tiebreaker, defaulted)),
            _ => None,
        })
        .expect("expected sort node");

    assert!(*defaulted);
    assert_eq!(
        keys.iter()
            .filter_map(|key| key.field.as_deref())
            .collect::<Vec<_>>(),
        vec![
            "active",
            "projection_row_identity",
            "source_canonical_row_identity"
        ]
    );
    assert_eq!(
        stable_tiebreaker,
        &vec![
            "projection_row_identity".to_string(),
            "source_canonical_row_identity".to_string()
        ]
    );
}

#[test]
fn logical_plan_extracts_dependencies_and_pre_reconstruction_predicates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned.dependencies.object_type_ids.contains(&1));
    assert!(planned.dependencies.property_ids.contains(&1));
    let LogicalPlanNodeKind::PreReconstructionFilter { predicates } =
        &planned.logical_plan.nodes[5].kind
    else {
        panic!("expected pre-reconstruction filter");
    };
    assert_eq!(
        predicates[0].placement,
        PredicatePlacement::PreReconstruction
    );
    assert_eq!(
        predicates[0].classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert_eq!(
        predicates[0].representation.representation,
        RepresentationClass::CodePure
    );
}

#[test]
fn logical_plan_classifies_filecode_ordered_predicate_with_default_collation_as_exact_decode_boundary(
) {
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Nia", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name < "M").select(name)"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let compare_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| {
            matches!(
                &form.kind,
                LogicalPredicateKind::Compare {
                    op: AstCompareOp::Lt,
                    ..
                }
            )
        })
        .expect("expected ordered FileCode comparison predicate");

    assert_eq!(
        compare_form.placement,
        PredicatePlacement::PreReconstruction
    );
    assert_eq!(
        compare_form.classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert_eq!(
        compare_form.representation.representation,
        RepresentationClass::DecodeBoundary
    );
    assert!(compare_form.representation.exact);
    assert!(compare_form.residual_reason.is_none());
    assert!(planned.logical_plan.residual_predicates.is_empty());
    assert!(planned
        .logical_plan
        .decode_boundaries
        .iter()
        .any(|boundary| boundary.contains("effective UTF-8 bytewise collation")));
}

#[test]
fn logical_plan_classifies_filecode_ordered_predicate_with_collation_as_exact_decode_boundary() {
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Ada", "Nia", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name < "M").select(name)"#,
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let compare_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| {
            matches!(
                &form.kind,
                LogicalPredicateKind::Compare {
                    op: AstCompareOp::Lt,
                    ..
                }
            )
        })
        .expect("expected ordered FileCode comparison predicate");

    assert_eq!(
        compare_form.placement,
        PredicatePlacement::PreReconstruction
    );
    assert_eq!(
        compare_form.classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert_eq!(
        compare_form.representation.representation,
        RepresentationClass::DecodeBoundary
    );
    assert!(compare_form.representation.exact);
    assert!(compare_form.residual_reason.is_none());
    assert!(planned.logical_plan.residual_predicates.is_empty());
    assert!(planned
        .logical_plan
        .decode_boundaries
        .iter()
        .any(|boundary| boundary.contains("effective UTF-8 bytewise collation")));
}

#[test]
fn logical_plan_classifies_goid_or_as_candidate_union_with_residual_verification() {
    let planned = parse_resolve_and_plan_query(
        &object_file_with_bool_records(&[true, false, true]),
        concat!(
            r#"Thing.where("#,
            r#"goid == uuid"00000000-0000-0000-0000-000000000000" || "#,
            r#"goid in [uuid"02020202-0202-0202-0202-020202020202"]"#,
            r#").select(goid, active)"#,
        ),
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let or_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| matches!(&form.kind, LogicalPredicateKind::Or(parts) if parts.len() == 2))
        .expect("expected OR predicate form");
    assert_eq!(or_form.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(or_form.classification, FilterClassification::System);
    assert_eq!(
        or_form.representation.representation,
        RepresentationClass::CodePure
    );
    assert!(!or_form.representation.exact);
    assert_eq!(
        or_form.representation.proof_state,
        PredicateProofState::CandidateNeedsResidual
    );
    assert!(or_form.representation.reason.contains("GOID OR"));
    assert!(or_form
        .residual_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("materialized CoveQL truth verification")));
}

#[test]
fn logical_plan_classifies_same_path_or_as_exact_in_equivalent() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true || active == false).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let or_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| matches!(&form.kind, LogicalPredicateKind::Or(_)))
        .expect("expected OR predicate form");
    assert_eq!(or_form.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        or_form.classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert_eq!(
        or_form.representation.representation,
        RepresentationClass::CodePure
    );
    assert!(or_form.representation.exact);
    assert_eq!(
        or_form.representation.proof_state,
        PredicateProofState::ProvenExact
    );
    assert!(or_form.residual_reason.is_none());
    assert!(or_form
        .representation
        .reason
        .contains("equivalent to one CoveQL IN"));
    let pre_filter = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::PreReconstructionFilter { predicates } => Some(predicates),
            _ => None,
        })
        .expect("expected pre-reconstruction filter node");
    assert!(pre_filter
        .iter()
        .any(|form| matches!(form.kind, LogicalPredicateKind::Or(_))));
}

#[test]
fn logical_plan_classifies_mixed_exact_path_or_as_encoded_disjunction() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        concat!(
            r#"Person.where(active == true || "#,
            r#"goid == uuid"00000000-0000-0000-0000-000000000000""#,
            r#").select(active)"#,
        ),
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let or_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| matches!(&form.kind, LogicalPredicateKind::Or(_)))
        .expect("expected OR predicate form");
    assert_eq!(or_form.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(or_form.classification, FilterClassification::None);
    assert_eq!(
        or_form.representation.representation,
        RepresentationClass::CodePure
    );
    assert!(or_form.representation.exact);
    assert_eq!(
        or_form.representation.proof_state,
        PredicateProofState::ProvenExact
    );
    assert!(or_form
        .representation
        .reason
        .contains("exact encoded disjunction"));
    assert!(or_form.residual_reason.is_none());
    let pre_filter = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::PreReconstructionFilter { predicates } => Some(predicates),
            _ => None,
        })
        .expect("expected pre-reconstruction filter node");
    assert!(or_form
        .representation
        .reason
        .contains("child contracts carry"));
    assert!(pre_filter
        .iter()
        .any(|form| matches!(form.kind, LogicalPredicateKind::Or(_))));
}

#[test]
fn logical_plan_classifies_not_of_exact_predicate_as_exact_complement() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(!(active == true)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let not_form = planned
        .logical_plan
        .predicate_forms
        .iter()
        .find(|form| matches!(&form.kind, LogicalPredicateKind::Not(_)))
        .expect("expected NOT predicate form");
    assert_eq!(not_form.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        not_form.classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert_eq!(
        not_form.representation.representation,
        RepresentationClass::CodePure
    );
    assert!(not_form.representation.exact);
    assert_eq!(
        not_form.representation.proof_state,
        PredicateProofState::ProvenExact
    );
    assert!(not_form.residual_reason.is_none());
    assert!(not_form.representation.reason.contains("three-valued"));
    let pre_filter = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::PreReconstructionFilter { predicates } => Some(predicates),
            _ => None,
        })
        .expect("expected pre-reconstruction filter node");
    assert!(pre_filter
        .iter()
        .any(|form| matches!(form.kind, LogicalPredicateKind::Not(_))));
}

#[test]
fn logical_plan_lowers_association_exists_to_semi_join() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::AssociationSemiJoin { .. })));
    assert!(planned.dependencies.association_type_ids.contains(&7));
}

#[test]
fn logical_plan_lowers_negated_association_exists_to_anti_join() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_association_file(),
        "Person.where(!exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let anti_join = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::AssociationAntiJoin { predicates } => Some(predicates),
            _ => None,
        })
        .expect("expected association anti-join node");
    assert_eq!(anti_join.len(), 1);
    assert!(planned.dependencies.association_type_ids.contains(&7));
}

#[test]
fn directionless_object_relative_association_rejects_when_endpoint_role_is_ambiguous() {
    let err = parse_and_resolve_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(association(CustomerPlacedOrder))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_ASSOCIATION_ROLE");
}

#[test]
fn association_role_from_and_to_disambiguate_object_relative_associations() {
    for query in [
        "Person.where(exists(association(CustomerPlacedOrder, from: customer))).select(active)",
        "Person.where(exists(association(CustomerPlacedOrder, to: order))).select(active)",
    ] {
        parse_and_resolve_query(
            &minimal_object_with_association_file(),
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            validation_options(),
        )
        .unwrap();
    }
}

#[test]
fn free_form_association_role_rejects_without_unique_metadata_proof() {
    let err = parse_and_resolve_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(association(CustomerPlacedOrder, role: customer))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_ASSOCIATION_ROLE");
}

#[test]
fn association_valid_time_resolves_without_commit_time_cut() {
    let planned = parse_resolve_and_plan_query(
        &minimal_association_file(),
        "association(CustomerPlacedOrder).asOf(association_valid_time: \"2026-01-01T00:00:00Z\").select(source_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        planned.resolved.temporal.role,
        TemporalRole::AssociationValidTime
    );
}

#[test]
fn source_event_time_without_binding_rejects_when_no_unambiguous_property() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(source_event_time: \"2026-01-01T00:00:00Z\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_TEMPORAL_ROLE");
}

#[test]
fn source_event_time_infers_event_time_property_binding() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        planned.resolved.temporal.role,
        TemporalRole::SourceEventTime
    );
    assert_eq!(
        planned.resolved.temporal.role_binding.as_deref(),
        Some("event_time")
    );
    assert!(planned
        .dependencies
        .temporal_role_bindings
        .contains("event_time"));

    let executed = execute_planned_query(&bytes, planned, ExecutionOptions::default()).unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"event_time": 1}), json!({"event_time": 2})]
    );
}

#[test]
fn kernel_source_event_time_as_of_direct_projection_executes_with_exact_native_authority() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let query =
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#;

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
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let temporal_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "temporal_role_bound_as_of")
        .expect("role-bound temporal contract is present");
    assert_eq!(temporal_contract["exact"], json!(true));
    assert_eq!(temporal_contract["residual_required"], json!(false));
    assert_eq!(temporal_contract["fallback_boundary"], json!(null));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"event_time": 1}), json!({"event_time": 2})]
    );
}

#[test]
fn kernel_source_event_time_as_of_direct_projection_can_return_arrow_batches() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let query =
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#;
    let resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: false,
        }),
        ..json_resolve_options()
    };

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
    assert!(!kernel.executed.authority.residual_required);
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.schema().field(0).name(), "event_time");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
    let values = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("event_time is int64");
    assert_eq!(values.value(0), 1);
    assert_eq!(values.value(1), 2);
}

#[test]
fn source_event_time_resolves_with_explicit_binding() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(source_event_time: \"2026-01-01T00:00:00Z\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            temporal_role_bindings: BTreeMap::from([(
                TemporalRole::SourceEventTime,
                "source_event_time".into(),
            )]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap();

    assert_eq!(resolved.temporal.role, TemporalRole::SourceEventTime);
}

#[test]
fn branch_string_selector_resolves_through_aliases() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            branch_aliases: BTreeMap::from([("main".into(), 7)]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        resolved.branch.selector,
        coveql::BranchSelector::BranchKey(7)
    );
}

#[test]
fn missing_branch_alias_rejects_named_selector() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_BRANCH");
}

#[test]
fn ambiguous_branch_alias_rejects_named_selector() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            ambiguous_branch_aliases: BTreeMap::from([("main".into(), vec![1, 2])]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_BRANCH");
}

#[test]
fn logical_plan_grouping_rejects_ungrouped_raw_fields() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.groupBy(active).select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_GROUPING");
}

#[test]
fn logical_plan_accepts_grouped_fields_and_aggregates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.groupBy(active).select(active, n: count(*))",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::Aggregate { .. })));
}

#[test]
fn logical_plan_rejects_aggregates_when_disclosure_policy_rejects() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::Reject;
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(n: count(*))",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap();

    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_AGGREGATE_DISCLOSURE_FORBIDDEN");
}

#[test]
fn logical_plan_rejects_aggregate_filters() {
    let err = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(count(*) > 0).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_AGGREGATE_FILTER");
}

#[test]
fn logical_plan_explicit_ordering_defaults_nulls_by_direction() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).orderBy(active, desc)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();
    assert_eq!(sort[0].nulls, coveql::AstNullOrdering::NullsFirst);
    assert!(!planned.logical_plan.default_ordering_applied);
}

#[test]
fn logical_plan_rejects_non_string_filecode_ordering() {
    let (bytes, _) = object_file_with_bool_filecode_records(&[true, false]);
    let resolved = parse_and_resolve_query(
        &bytes,
        "FlagThing.select(active).orderBy(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNSAFE_CODE_ORDERING");
}

#[test]
fn logical_plan_accepts_filecode_ordering_with_default_string_collation() {
    let (bytes, _) = object_file_with_filecode_records(&["Zoë", "Ada", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name).orderBy(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();

    assert_eq!(sort[0].representation, RepresentationClass::DecodeBoundary);
    assert_eq!(sort[0].collation_id, Some(CollationKind::None.id()));
}

#[test]
fn logical_plan_accepts_filecode_ordering_with_declared_collation_contract() {
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Zoë", "Ada", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name).orderBy(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();

    assert_eq!(sort[0].representation, RepresentationClass::DecodeBoundary);
    assert_eq!(sort[0].collation_id, Some(CollationKind::Utf8Bytewise.id()));
}

#[test]
fn logical_plan_marks_function_predicates_as_residual_boundaries() {
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.where(startsWith(name, \"A\")).select(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(!planned.logical_plan.residual_predicates.is_empty());
}

#[test]
fn logical_plan_marks_coded_safe_function_predicates_as_exact_pre_reconstruction() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), Some("Åsa"), None],
        &["startsWith", "length", "lower"],
    );
    for query in [
        r#"Person.where(startsWith(name, "A")).select(name)"#,
        "Person.where(length(name) == 3).select(name)",
        r#"Person.where(lower(name) == "åsa").select(name)"#,
    ] {
        let planned = parse_resolve_and_plan_query(
            &bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            PlanOptions::default(),
            validation_options(),
        )
        .unwrap();

        assert!(
            planned.logical_plan.residual_predicates.is_empty(),
            "{query} should not leave residual function predicates"
        );
        assert!(
            planned
                .logical_plan
                .predicate_forms
                .iter()
                .any(|form| serde_json::to_value(form.placement).unwrap()
                    == json!("pre_reconstruction")
                    && form.representation.exact
                    && form.residual_reason.is_none()
                    && serde_json::to_value(form.representation.proof_state).unwrap()
                        == json!("proven_exact")),
            "{query} should report a proven-exact pre-reconstruction function predicate"
        );
    }
}

#[test]
fn logical_plan_json_redacts_protected_details_by_default() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json().to_string();
    assert!(explain.contains("<redacted>"));
    assert!(!explain.contains("\"active\""));
    assert_eq!(
        planned.explain_json()["fingerprints"]["logical_plan"],
        planned.logical_plan_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["predicate_ast"],
        planned.predicate_ast_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["predicate_cnf"],
        planned.predicate_cnf_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["projection_dependency"],
        planned.projection_dependency_fingerprint
    );
    assert_eq!(planned.predicate_ast_fingerprint.len(), 64);
    assert_eq!(planned.predicate_cnf_fingerprint.len(), 64);
    assert_eq!(planned.projection_dependency_fingerprint.len(), 64);
}

#[test]
fn logical_plan_json_can_disclose_protected_details_when_allowed() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    assert!(explain.to_string().contains("active"));
    assert!(explain["predicate_forms"]
        .as_array()
        .unwrap()
        .iter()
        .all(|form| form["representation"]["contract_version"]
            == coveql::PREDICATE_REPRESENTATION_CONTRACT_VERSION));
}

#[test]
fn stable_explain_json_has_ordered_top_level_schema() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    let keys = explain
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "schema_version",
            "mode",
            "coveql_version",
            "core_version",
            "primary_profile",
            "profiles",
            "profile_contracts",
            "root",
            "grain",
            "operation",
            "temporal_mode",
            "canonical_order",
            "visibility_applied",
            "redaction_applied",
            "fingerprints",
            "operation_context",
            "logical_plan",
            "physical_plan",
            "resolved_dependencies",
            "predicate_forms",
            "trusted_metadata",
            "ignored_metadata",
            "fallbacks",
            "rejections",
            "decode_boundaries",
            "residual_predicates",
            "visibility",
            "redactions",
            "warnings",
            "diagnostics",
            "execution",
        ]
    );
    assert_eq!(explain["schema_version"], "0.1");
    assert_eq!(explain["coveql_version"], coveql::COVEQL_LANGUAGE_VERSION);
    assert_eq!(explain["core_version"], coveql::COVEQL_CORE_VERSION);
    assert_eq!(explain["primary_profile"], "object");
    assert_eq!(explain["profiles"], json!(["object"]));
    assert_eq!(explain["root"], "object");
    assert_eq!(explain["grain"], "latest_state");
    assert_eq!(explain["operation"], "explain_object");
    assert_eq!(explain["temporal_mode"], "latest");
    assert!(explain["canonical_order"]
        .as_array()
        .unwrap()
        .contains(&json!("goid")));
    assert_eq!(explain["physical_plan"], json!([]));
    assert!(explain["fingerprints"]["physical_plan"].is_null());
    assert!(explain["fingerprints"]["predicate_ast"].is_string());
    assert!(explain["fingerprints"]["predicate_cnf"].is_string());
    assert!(explain["fingerprints"]["projection_dependency"].is_string());
    assert_eq!(explain["execution"]["completed"], false);
    assert_eq!(explain["execution"]["kind"], "plan_explanation");
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reason"],
        "physical_plan_required"
    );
    assert!(explain["execution"]["coded_execution"]["operator_contracts"].is_array());
    assert!(
        explain["execution"]["coded_execution"]["operator_contracts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|contract| contract["contract_version"]
                == coveql::CODED_OPERATOR_CONTRACT_VERSION)
    );
}

#[test]
fn stable_explain_modes_are_policy_clamped() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::PublicOnly;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).explain(\"forensic\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();

    assert_eq!(explain["mode"], "public");
    let diagnostic = explain["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["code"] == "E_SECURITY_DISCLOSURE_FORBIDDEN"
                && item["severity"] == "warning"
                && item["redacted"] == true
        })
        .expect("policy clamp diagnostic is present");
    for field in coveql::conformance_profile().required_diagnostic_fields {
        assert!(
            diagnostic.get(*field).is_some(),
            "diagnostic JSON missing required field {field}: {diagnostic}"
        );
    }
}

#[test]
fn stable_explain_supports_all_policy_modes() {
    for (mode, policy) in [
        ("public", ExplainDisclosurePolicy::PublicOnly),
        ("developer", ExplainDisclosurePolicy::Developer),
        ("proof", ExplainDisclosurePolicy::Proof),
        ("coded", ExplainDisclosurePolicy::Proof),
        ("forensic", ExplainDisclosurePolicy::Forensic),
    ] {
        let mut resolve_options = ResolveOptions::default();
        resolve_options.security.explain_policy = policy;
        resolve_options.security.metadata_disclosure_policy =
            MetadataDisclosurePolicy::AllowProtected;

        let planned = parse_resolve_and_plan_query(
            &minimal_object_file(),
            &format!("Person.select(active).explain(\"{mode}\")"),
            ParseOptions::default(),
            resolve_options,
            PlanOptions::default(),
            validation_options(),
        )
        .unwrap();

        assert_eq!(planned.explain_json()["mode"], mode);
    }
}

#[test]
fn coded_explain_mode_reports_coded_execution_contracts() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();

    assert_eq!(explain["mode"], "coded");
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reason"],
        "physical_plan_required"
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["coded_suitability"],
        "physical_plan_required"
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reasons"],
        json!(["physical_plan_required"])
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["residual_verification"],
        true
    );
    assert!(explain["execution"]["coded_execution"]["pushed_filters"].is_array());
    assert!(explain["execution"]["coded_execution"]["pushed_columns"].is_array());
    assert!(explain["execution"]["coded_execution"]["operator_contracts"].is_array());
}

#[test]
fn coded_explain_reports_projection_pushed_columns() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();
    let pushed_columns = explain["execution"]["coded_execution"]["pushed_columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(pushed_columns, vec!["active"]);
}

#[test]
fn coded_explain_reports_projection_pushed_filters() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let pushed_filters = planned.explain_json()["execution"]["coded_execution"]["pushed_filters"]
        .as_array()
        .unwrap()
        .clone();

    assert!(pushed_filters.iter().any(|filter| {
        filter["kind"] == "projection_filter"
            && filter["placement"] == "projection_readback"
            && filter["projection_id"] == "people_projection"
            && filter["predicate"]
                .as_str()
                .is_some_and(|predicate| predicate.contains("compare:Eq:active"))
    }));
}

#[test]
fn coded_explain_reports_execution_code_domain_bridge_decision() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue"]);
    let mut resolve_options = ResolveOptions::default();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name == "red").select(name).explain("coded")"#,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();
    let decisions = explain["execution"]["coded_execution"]["bridge_decisions"]
        .as_array()
        .unwrap();

    assert!(decisions.iter().any(|decision| {
        decision.as_str().is_some_and(|decision| {
            decision.contains("execution_code_domain")
                && decision.contains("scope=File")
                && decision.contains("lifetime=Scan")
                && decision.contains("runtime remap proof")
        })
    }));
}

#[test]
fn coded_explain_requires_exact_manifest_bridge_for_multifile_filecode_contracts() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue"]);
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let mut planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name == "red").select(name).explain("coded")"#,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut second = planned.resolved.operation_context.dataset.files[0].clone();
    second.ordinal = 1;
    second.source = "right.cove".into();
    second.file_id[0] ^= 0xff;
    planned
        .resolved
        .operation_context
        .dataset
        .files
        .push(second);
    planned
        .resolved
        .operation_context
        .dataset
        .cross_file_ordering = coveql::CrossFileOrderingPolicy::CanonicalDatasetOrder;
    planned.resolved.operation_context.dataset.object_identity =
        coveql::CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid;
    planned
        .resolved
        .operation_context
        .dataset
        .association_identity =
        coveql::CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges = vec![coveql::CodeDomainBridgeContext {
        domain_id: "name_domain".into(),
        bridge_kind: "manifest_candidate_requires_canonical_remap".into(),
        epoch: None,
        security_scope_id: None,
        exact: false,
        reason: "raw local FileCode dictionaries remain file scoped".into(),
    }];

    let explain = planned.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "root_scan"
            && contract["representation_class"] == "cross_source_code_bridge"
            && contract["exact"] == false
            && contract["residual_required"] == true
    }));
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["exact"] == false
            && contract["residual_required"] == true
    }));
    assert!(explain["execution"]["coded_execution"]["decode_boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|boundary| boundary
            .as_str()
            .is_some_and(|boundary| boundary.contains("multi-file FileCode identity requires"))));

    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .bridge_kind = "materialized_canonical_value".into();
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .exact = true;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .reason = "materialized canonical value path validated by manifest".into();

    let materialized_value_explain = planned.explain_json();
    let materialized_value_contracts = materialized_value_explain["execution"]["coded_execution"]
        ["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(
        materialized_value_contracts.iter().any(|contract| {
            contract["operator"] == "predicate_compare"
                && contract["representation_class"] == "cross_source_code_bridge"
                && contract["exact"] == false
                && contract["residual_required"] == true
        }),
        "materialized canonical value proofs must not authorize raw coded comparison"
    );
    assert!(
        materialized_value_explain["execution"]["coded_execution"]["bridge_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision.as_str().is_some_and(|decision| {
                decision.contains("materialized_canonical_value") && decision.contains("exact=true")
            }))
    );

    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .bridge_kind = "manifest_validated_canonical_remap".into();
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .exact = true;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .reason = "epoch-bound canonical remap validated by manifest".into();

    let exact_explain = planned.explain_json();
    let exact_contracts = exact_explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(exact_contracts.iter().any(|contract| {
        contract["operator"] == "root_scan"
            && contract["representation_class"] == "cross_source_code_bridge"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
    assert!(exact_contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
    assert!(
        exact_explain["execution"]["coded_execution"]["bridge_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision
                .as_str()
                .is_some_and(|decision| decision.contains("exact=true")))
    );
}

#[test]
fn stable_explain_text_is_derived_from_json() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    let text = planned.explain_text();
    assert_eq!(text, render_explain_text(&explain));
    assert_eq!(text, planned.explain_text());
    assert!(text.contains("schema_version: 0.1"));
    assert!(text.contains("execution: completed=false kind=plan_explanation"));
}

#[test]
fn logical_plan_fingerprint_is_stable_for_harmless_quoting() {
    let a = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "`Person`.select(`active`)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(a.logical_plan_fingerprint, b.logical_plan_fingerprint);
    assert_eq!(a.predicate_ast_fingerprint, b.predicate_ast_fingerprint);
    assert_eq!(a.predicate_cnf_fingerprint, b.predicate_cnf_fingerprint);
    assert_eq!(
        a.projection_dependency_fingerprint,
        b.projection_dependency_fingerprint
    );
}

#[test]
fn predicate_fingerprints_change_with_predicate_semantics() {
    let a = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == false).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_ne!(a.predicate_ast_fingerprint, b.predicate_ast_fingerprint);
    assert_ne!(a.predicate_cnf_fingerprint, b.predicate_cnf_fingerprint);
}

#[test]
fn projection_dependency_fingerprint_changes_with_projection_contract() {
    let bytes = minimal_object_with_two_column_projection_file();
    let a = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(enabled)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_ne!(
        a.projection_dependency_fingerprint,
        b.projection_dependency_fingerprint
    );
    assert_eq!(
        a.explain_json()["fingerprints"]["projection_dependency"],
        a.projection_dependency_fingerprint
    );
}

#[test]
fn logical_plan_projection_roots_produce_projection_read_contract() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned
        .dependencies
        .projection_ids
        .contains("people_projection"));
    assert!(planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::ProjectionRead { .. })));
    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(
        contract.contract_version,
        coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION
    );
    assert_eq!(contract.projection_id, "people_projection");
    assert_eq!(contract.projection_version.as_deref(), Some("2026.05"));
    assert_eq!(contract.mapping_version.as_deref(), Some("2026.05"));
    assert_eq!(contract.row_grain.as_deref(), Some("one_row_per_object"));
    assert_eq!(contract.anchor_object_type.as_deref(), Some("Person"));
    assert_eq!(contract.map_columns.len(), 1);
    assert_eq!(contract.map_columns[0].name, "active");
    assert_eq!(contract.map_columns[0].value, "property.active");
    assert_eq!(contract.columns.len(), 1);
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.source_properties.contains(&1));
    assert!(contract.pushed_predicates.is_empty());
    assert!(contract.residual_predicates.is_empty());
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::FullyPushdownSafe
    );
    assert_eq!(contract.visibility_policy, "all_rows");
    assert_eq!(contract.redaction_policy, "protected_values_redacted");
    assert_eq!(
        contract.domain_policy,
        "same_projection_contract_or_materialized"
    );
    assert_eq!(
        contract.collation_policy,
        "declared_collation_or_materialized_sort"
    );
    assert_eq!(contract.null_policy, "cove_null_semantics_preserved");
    assert!(contract.residual_required_fields.is_empty());
    assert_eq!(contract.output_compatibility, vec!["json", "arrow"]);
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);

    let explain_contract =
        &planned.explain_json()["resolved_dependencies"]["projection_contracts"][0];
    assert_eq!(
        explain_contract["contract_version"],
        json!(coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION)
    );
    assert_eq!(explain_contract["map_columns"][0]["name"], "active");
    assert_eq!(
        explain_contract["map_columns"][0]["value"],
        "property.active"
    );
}

#[test]
fn projection_contract_records_source_properties_for_pushed_columns() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_two_column_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.source_properties.contains(&1));
    assert!(!contract.source_properties.contains(&2));
}

#[test]
fn projection_contract_treats_omitted_select_as_all_projection_columns() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_two_column_projection_file(),
        "projection(people_projection)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(
        contract.selected_columns,
        BTreeSet::from(["active".into(), "enabled".into()])
    );
    assert_eq!(
        contract.pushed_columns,
        BTreeSet::from(["active".into(), "enabled".into()])
    );
    assert!(contract.source_properties.contains(&1));
    assert!(contract.source_properties.contains(&2));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::FullyPushdownSafe
    );
}

#[test]
fn projection_contract_reports_pushed_filter_predicates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Eq:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_root_execution_reports_provider_column_and_filter_pushdown() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(executed.pushdown_report.outcome, PushdownOutcome::Applied);
    assert_eq!(
        executed.pushdown_report.counters.property_columns_requested,
        1
    );
    assert_eq!(
        executed
            .pushdown_report
            .counters
            .property_predicate_candidates,
        1
    );
    assert_eq!(executed.pushdown_report.counters.residual_predicates, 0);
    assert!(executed.pushdown_report.residual_predicates.is_empty());
    assert!(executed.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::ProjectionColumnPrune
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["columns"] == json!(["active"])
    }));
    assert!(executed.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::ProjectionFilterCandidate
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["predicate"]
                .as_str()
                .is_some_and(|predicate| predicate.contains("compare:Eq:active"))
    }));
}

#[test]
fn projection_contract_reports_not_equal_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active != true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Ne:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_negated_equal_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!(active == true)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Ne:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_bool_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["bool:active"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_negated_bool_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!active).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["not_bool:active"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_same_column_or_filter_as_pushed_in_list() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true || active in [false]).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["in:active:2 literals"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_keeps_negated_in_with_null_residual() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!(active in [true, null])).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_predicates.is_empty());
    assert_eq!(
        contract.residual_predicates,
        vec!["not:in:active:2 literals"]
    );
    assert!(contract.residual_required);
    assert!(!contract.pushdown_safe);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
}

#[test]
fn projection_contract_reports_residual_filter_predicates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true || active.isNull()).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_predicates.is_empty());
    assert_eq!(contract.residual_predicates, vec!["or:2 terms"]);
    assert!(contract.residual_required);
    assert!(!contract.pushdown_safe);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
}

#[test]
fn projection_order_by_default_nulls_marks_dependency_contract_residual() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active).orderBy(active, desc)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
    assert!(contract.residual_required_fields.contains("active"));
    assert!(!contract.pushdown_safe);
    assert!(contract.residual_required);
    assert!(contract
        .residual_reasons
        .iter()
        .any(|reason| reason.contains("projection ordering requires residual")));
}

#[test]
fn projection_contract_pushes_columns_required_by_residual_select_expressions() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(value: coalesce(active, false))",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.deterministic_functions.contains("coalesce"));
    assert_eq!(contract.function_requirements.len(), 1);
    assert_eq!(contract.function_requirements[0].id, "coalesce");
    assert!(contract.function_requirements[0]
        .input_columns
        .contains("active"));
    assert!(contract.function_requirements[0].residual_required);
    assert!(!contract.function_requirements[0].pushdown_safe);
    assert!(contract.function_requirements[0]
        .reason
        .contains("residual verification"));
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
    assert!(contract.residual_required);
    assert!(contract.residual_required_fields.contains("active"));
    assert!(contract
        .residual_reasons
        .iter()
        .any(|reason| reason.contains("residual output evaluation required")));
}

#[test]
fn projection_contract_records_aggregate_requirements() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(n: count(active))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.aggregate_kinds.contains("count"));
    assert_eq!(contract.aggregate_requirements.len(), 1);
    assert_eq!(contract.aggregate_requirements[0].id, "count");
    assert!(contract.aggregate_requirements[0]
        .input_columns
        .contains("active"));
    assert!(contract.aggregate_requirements[0].residual_required);
    assert!(!contract.aggregate_requirements[0].pushdown_safe);
    assert!(contract.aggregate_requirements[0]
        .reason
        .contains("duplicate-row"));
}

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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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

fn json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        ..ResolveOptions::default()
    }
}

fn protected_json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        security: SecurityContext {
            metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
            ..SecurityContext::default()
        },
        ..ResolveOptions::default()
    }
}

#[test]
fn materialized_object_execution_returns_selected_json_rows() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
        PhysicalPlanOptions {
            allow_zero_copy_output: true,
            sidecars: PhysicalSidecarInputs {
                zero_copy_buffer_map_bytes: Some(metric_zero_copy_map()),
                ..PhysicalSidecarInputs::default()
            },
            ..PhysicalPlanOptions::default()
        },
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
fn kernel_compare_mode_matches_materialized_object_json_rows() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
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
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision
            .reason
            .contains("single-file FileCode literal predicate")
            && decision.safe_details["residual_verification"] == json!(true)
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
    assert!(kernel
        .kernel_report
        .decisions
        .iter()
        .any(|decision| decision.reason.contains("execution-code remap was used")));
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
    }));
    assert!(kernel.kernel_report.decisions.iter().any(|decision| {
        decision.reason.contains("execution-code remap was used")
            && decision.safe_details["matched_rows"] == json!(3)
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
    let physical_options = PhysicalPlanOptions {
        allow_index_only_answers: true,
        sidecars: PhysicalSidecarInputs {
            covi_artifact_bytes: Some(covi),
            ..PhysicalSidecarInputs::default()
        },
        ..PhysicalPlanOptions::default()
    };
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
            PhysicalPlanOptions {
                allow_index_only_answers: true,
                sidecars: PhysicalSidecarInputs {
                    covi_artifact_bytes: Some(covi.clone()),
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
            PhysicalPlanOptions {
                allow_index_only_answers: true,
                sidecars: PhysicalSidecarInputs {
                    covi_artifact_bytes: Some(covi.clone()),
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
    let physical_options = PhysicalPlanOptions {
        allow_index_only_answers: true,
        sidecars: PhysicalSidecarInputs {
            covi_artifact_bytes: Some(covi),
            ..PhysicalSidecarInputs::default()
        },
        ..PhysicalPlanOptions::default()
    };
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
    for (query, expected, aggregate, counted_path) in [
        ("Thing.select(n: count(*))", json!({"n": 4}), "count", None),
        (
            "Thing.select(n: count(active))",
            json!({"n": 3}),
            "count",
            Some("active"),
        ),
        (
            "Thing.select(e: exists(active))",
            json!({"e": true}),
            "exists",
            Some("active"),
        ),
        (
            "Thing.select(d: distinct_count(active))",
            json!({"d": 2}),
            "distinct_count",
            Some("active"),
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
        .any(|diagnostic| diagnostic.code == "W_KERNEL_NATIVE_BOOL_GROUP_COUNT_EXECUTED"));
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
    }));
}

#[test]
fn kernel_force_mode_executes_single_file_filecode_grouped_aggregates_with_exact_native_contract() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue", "red", "green"]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected_rows, aggregate, representation_class) in [
        (
            "Person.groupBy(name).select(name, n: count(*))",
            vec![
                json!({"name": "blue", "n": 1}),
                json!({"name": "green", "n": 1}),
                json!({"name": "red", "n": 2}),
            ],
            "count",
            "code_pure",
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
        }));
    }
}

#[test]
fn kernel_force_mode_executes_grouped_numcode_direct_aggregates_with_exact_native_contract() {
    let bytes = object_file_with_numcode_records(&[20, 5, 20, 10]);
    let mut resolve_options = json_resolve_options();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    for (query, expected_rows, aggregate, representation_class) in [
        (
            "MetricThing.groupBy(metric).select(metric, n: count(metric))",
            vec![
                json!({"metric": 10, "n": 1}),
                json!({"metric": 20, "n": 2}),
                json!({"metric": 5, "n": 1}),
            ],
            "count",
            "code_pure",
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

#[cfg(feature = "datafusion")]
#[test]
fn datafusion_projection_helper_rejects_non_projection_plans() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ctx = datafusion::execution::context::SessionContext::new();
    let err = coveql::register_datafusion_projection_for_plan(
        &ctx,
        "people",
        std::path::Path::new("/tmp/nonexistent.cove"),
        None,
        &planned,
    )
    .unwrap_err();
    assert!(err.to_string().contains("projection-backed plan"));
}

#[cfg(feature = "datafusion")]
#[test]
fn datafusion_projection_helper_rejects_relation_aware_table_plans() {
    let bytes = object_file_with_bool_records_and_projection(&[false, true]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "table(thing_projection) as l.lookup(table(thing_projection) as r, on: l.active == r.active).select(left_active: l.active, right_active: r.active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ctx = datafusion::execution::context::SessionContext::new();
    let err = coveql::register_datafusion_projection_for_plan(
        &ctx,
        "thing_lookup",
        std::path::Path::new("/tmp/nonexistent.cove"),
        None,
        &planned,
    )
    .unwrap_err();
    assert!(err.to_string().contains("trivial projection-backed plan"));
    assert!(err
        .to_string()
        .contains("register_datafusion_coveql_provider_for_plan"));
}

#[test]
fn physical_plan_uses_canonical_object_node_order() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let kinds = physical
        .physical_plan
        .nodes
        .iter()
        .map(|node| match &node.kind {
            PhysicalPlanNodeKind::ValidateFeatureScopes { .. } => "feature_scopes",
            PhysicalPlanNodeKind::BuildPredicateNormalForms { .. } => "predicate_forms",
            PhysicalPlanNodeKind::ReadObjectCatalog { .. } => "catalog",
            PhysicalPlanNodeKind::SelectTemporalSegments { .. } => "temporal_segments",
            PhysicalPlanNodeKind::TemporalBloomProbe { .. } => "temporal_bloom",
            PhysicalPlanNodeKind::ValidateCoverageProofs { .. } => "coverage_validate",
            PhysicalPlanNodeKind::CoveragePrune { .. } => "coverage_prune",
            PhysicalPlanNodeKind::ValidateCoviOrCovx { .. } => "index_validate",
            PhysicalPlanNodeKind::CoviLookup { .. } => "index_lookup",
            PhysicalPlanNodeKind::PlanLayoutRanges { .. } => "layout_ranges",
            PhysicalPlanNodeKind::RangeReadCoalesce { .. } => "range_coalesce",
            PhysicalPlanNodeKind::ReadSystemColumns { .. } => "system_columns",
            PhysicalPlanNodeKind::ReadPropertyColumns { .. } => "property_columns",
            PhysicalPlanNodeKind::MorselBitmapEval { .. } => "bitmap_eval",
            PhysicalPlanNodeKind::FileCodePredicate { .. } => "file_code",
            PhysicalPlanNodeKind::ExecutionCodePredicate { .. } => "execution_code",
            PhysicalPlanNodeKind::NumericPredicate { .. } => "numeric",
            PhysicalPlanNodeKind::DictionaryLiftedPredicate { .. } => "dictionary_lifted",
            PhysicalPlanNodeKind::ReconstructObjectState { .. } => "reconstruct",
            PhysicalPlanNodeKind::ApplyVisibilityAndRedaction { .. } => "visibility_redaction",
            PhysicalPlanNodeKind::ZeroCopyArrowProjection { .. } => "zero_copy",
            PhysicalPlanNodeKind::JsonProjection { .. } => "json",
            PhysicalPlanNodeKind::MaterializedFilter { .. } => "materialized_filter",
            PhysicalPlanNodeKind::MaterializedSort { .. } => "materialized_sort",
            PhysicalPlanNodeKind::FallbackBoundary { .. } => "fallback",
            PhysicalPlanNodeKind::OutputBoundary { .. } => "output",
            _ => "other",
        })
        .collect::<Vec<_>>();

    assert_eq!(
        &kinds[..26],
        &[
            "feature_scopes",
            "predicate_forms",
            "catalog",
            "temporal_segments",
            "temporal_bloom",
            "coverage_validate",
            "coverage_prune",
            "index_validate",
            "index_lookup",
            "layout_ranges",
            "range_coalesce",
            "system_columns",
            "property_columns",
            "bitmap_eval",
            "file_code",
            "execution_code",
            "numeric",
            "dictionary_lifted",
            "reconstruct",
            "visibility_redaction",
            "zero_copy",
            "json",
            "materialized_filter",
            "materialized_sort",
            "fallback",
            "output",
        ]
    );
    assert!(physical
        .physical_plan
        .nodes
        .iter()
        .all(|node| node.contract.contract_version == coveql::PHYSICAL_OPERATOR_CONTRACT_VERSION));
    assert_eq!(
        physical
            .physical_plan
            .predicate_normal_forms
            .normal_form_version,
        coveql::PREDICATE_NORMAL_FORM_VERSION
    );
    assert_eq!(
        physical.explain_json()["physical_plan"]["predicate_normal_forms"]["normal_form_version"],
        coveql::PREDICATE_NORMAL_FORM_VERSION
    );
    assert!(physical.explain_json()["physical_plan"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|node| node["contract"]["contract_version"]
            == coveql::PHYSICAL_OPERATOR_CONTRACT_VERSION));
    assert_eq!(
        physical.explain_json()["fingerprints"]["physical_plan"],
        physical.physical_plan_fingerprint
    );
    assert_eq!(
        physical.planned.explain_json()["fingerprints"]["physical_plan"],
        serde_json::Value::Null
    );
}

#[test]
fn physical_plan_records_temporal_history_and_changes_reconstruction_boundary() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
    for (query, expected_mode, expected_grain) in [
        (
            "Thing.history(mode: records)",
            "history_records",
            "history_record",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: property_diffs)",
            "changes_property_diffs",
            "change_property_diff",
        ),
    ] {
        let physical = parse_resolve_plan_and_build_physical_plan(
            bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            validation_options(),
        )
        .unwrap();
        let (node, mode, row_grain, native_exact) = physical
            .physical_plan
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                PhysicalPlanNodeKind::TemporalGrainReconstruction {
                    mode,
                    row_grain,
                    native_exact,
                } => Some((node, mode, row_grain, native_exact)),
                _ => None,
            })
            .expect("temporal grain reconstruction node is present");

        assert_eq!(mode, expected_mode);
        assert_eq!(row_grain, expected_grain);
        assert!(!*native_exact);
        assert!(node
            .contract
            .fallback
            .contains("materialized temporal reconstruction"));
    }
}

#[test]
fn physical_plan_marks_temporal_direct_projection_reconstruction_exact() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
    for (query, expected_mode, expected_grain) in [
        (
            "Thing.history(mode: records).select(active)",
            "history_records",
            "history_record",
        ),
        (
            "Thing.changes(from: 1, to: 3, mode: records).select(active)",
            "changes_records",
            "change_record",
        ),
    ] {
        let physical = parse_resolve_plan_and_build_physical_plan(
            bytes,
            query,
            ParseOptions::default(),
            json_resolve_options(),
            PlanOptions::default(),
            PhysicalPlanOptions::default(),
            validation_options(),
        )
        .unwrap();
        let (node, mode, row_grain, native_exact) = physical
            .physical_plan
            .nodes
            .iter()
            .find_map(|node| match &node.kind {
                PhysicalPlanNodeKind::TemporalGrainReconstruction {
                    mode,
                    row_grain,
                    native_exact,
                } => Some((node, mode, row_grain, native_exact)),
                _ => None,
            })
            .expect("temporal grain reconstruction node is present");

        assert_eq!(mode, expected_mode);
        assert_eq!(row_grain, expected_grain);
        assert!(*native_exact);
        assert!(node
            .contract
            .fallback
            .contains("exact temporal row-grain reconstruction"));
        assert_eq!(
            physical.explain_json()["physical_plan"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| {
                    node["kind"]["kind"] == json!("temporal_grain_reconstruction")
                        || node["kind"]["TemporalGrainReconstruction"].is_object()
                })
                .and_then(|node| node["kind"]["value"]["native_exact"].as_bool())
                .or_else(|| {
                    physical.explain_json()["physical_plan"]["nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find_map(|node| node["kind"]["native_exact"].as_bool())
                }),
            Some(true)
        );
    }
}

#[test]
fn physical_plan_records_association_endpoint_and_evidence_grain_candidates() {
    let association = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_with_association_file(),
        "Person.where(exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let PhysicalPlanNodeKind::AssociationSemiJoin {
        direction_plans,
        anti_join_candidate,
        endpoint_fast_path_candidates,
        endpoint_fast_path_exact,
        ..
    } = association
        .physical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            PhysicalPlanNodeKind::AssociationSemiJoin { .. } => Some(&node.kind),
            _ => None,
        })
        .expect("expected association semi-join node")
    else {
        panic!("expected association semi-join node");
    };
    assert!(!anti_join_candidate);
    assert_eq!(*endpoint_fast_path_candidates, 1);
    assert!(*endpoint_fast_path_exact);
    assert_eq!(direction_plans.len(), 1);
    assert_eq!(
        direction_plans[0].endpoint_role,
        AssociationEndpointRole::Either
    );

    let evidence = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_with_projection_and_evidence_index_file(),
        "evidence(projection(people_projection), grain: row).select(source_id)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let PhysicalPlanNodeKind::EvidenceRead {
        grains,
        target_index_kinds,
        target_filtered,
        index_candidate,
        existence_fast_path_candidates,
        count_fast_path_candidates,
        hidden_entry_filtering_required,
        ..
    } = evidence
        .physical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            PhysicalPlanNodeKind::EvidenceRead { .. } => Some(&node.kind),
            _ => None,
        })
        .expect("expected evidence read node")
    else {
        panic!("expected evidence read node");
    };
    assert!(index_candidate);
    assert!(*target_filtered);
    assert!(grains.contains(&coveql::EvidenceGrainKind::Row));
    assert!(target_index_kinds.contains(&coveql::EvidenceTargetIndexKind::Projection));
    assert_eq!(*existence_fast_path_candidates, 0);
    assert_eq!(*count_fast_path_candidates, 0);
    assert!(!*hidden_entry_filtering_required);
}

#[test]
fn physical_plan_records_association_antijoin_node() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_with_association_file(),
        "Person.where(!exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let PhysicalPlanNodeKind::AssociationAntiJoin {
        predicate_count,
        endpoint_fast_path_candidates,
        endpoint_fast_path_exact,
        direction_plans,
    } = physical
        .physical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            PhysicalPlanNodeKind::AssociationAntiJoin { .. } => Some(&node.kind),
            _ => None,
        })
        .expect("expected association anti-join node")
    else {
        panic!("expected association anti-join node");
    };
    assert_eq!(*predicate_count, 1);
    assert_eq!(*endpoint_fast_path_candidates, 1);
    assert!(*endpoint_fast_path_exact);
    assert_eq!(direction_plans.len(), 1);
    assert_eq!(
        direction_plans[0].endpoint_role,
        AssociationEndpointRole::Either
    );
}

#[test]
fn physical_plan_records_object_root_association_aggregates() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_with_association_file(),
        "Person.select(order_count: count(either(association(CustomerPlacedOrder))))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let aggregate = physical
        .physical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            PhysicalPlanNodeKind::AssociationAggregate { .. } => Some(node),
            _ => None,
        })
        .expect("expected association aggregate node");
    let PhysicalPlanNodeKind::AssociationAggregate {
        aggregate_count,
        count_fast_path_candidates,
        distinct_target_fast_path_candidates,
        aggregate_fast_path_exact,
        ..
    } = &aggregate.kind
    else {
        panic!("expected association aggregate node");
    };
    assert!(*aggregate_count >= 1);
    assert_eq!(*count_fast_path_candidates, 1);
    assert_eq!(*distinct_target_fast_path_candidates, 0);
    assert!(*aggregate_fast_path_exact);
    assert!(aggregate
        .contract
        .fallback
        .contains("exact endpoint edge-table fast-path authority"));
}

#[test]
fn physical_predicates_classify_codepure_and_filecode_literals() {
    let code_pure = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert!(code_pure
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .any(
            |form| form.representation == PhysicalRepresentationClass::CodePure
                && form.proof_state == PredicateProofState::ProvenExact
        ));

    let file_code = parse_resolve_plan_and_build_physical_plan(
        &minimal_filecode_object_file(),
        "Person.where(name == \"Ada\").select(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let PhysicalPlanNodeKind::FileCodePredicate { forms } = &file_code
        .physical_plan
        .nodes
        .iter()
        .find(|node| matches!(node.kind, PhysicalPlanNodeKind::FileCodePredicate { .. }))
        .unwrap()
        .kind
    else {
        panic!("expected FileCodePredicate");
    };
    assert!(forms.iter().any(|form| form.representation
        == PhysicalRepresentationClass::FileCodeLiteral
        && form.exact
        && form.proof_state == PredicateProofState::ProvenExact));
    assert!(file_code
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .is_empty());
    assert!(file_code
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .any(
            |form| form.representation == PhysicalRepresentationClass::FileCodeLiteral
                && form.proof_state == PredicateProofState::ProvenExact
        ));
}

#[test]
fn physical_predicates_emit_coverage_forms_separate_from_residuals() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let forms = &physical.physical_plan.predicate_normal_forms;
    let coverage = forms
        .coverage_forms
        .iter()
        .find(|form| form.operator.as_deref() == Some("Eq"))
        .expect("expected coverage-compatible predicate form");
    assert_eq!(coverage.kind, PhysicalPredicateFormKind::CoverageCompatible);
    assert_eq!(
        coverage.representation,
        PhysicalRepresentationClass::CoverageOnly
    );
    assert_eq!(coverage.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        coverage.classification,
        FilterClassification::PropertyCodedCandidate
    );
    assert!(!coverage.exact);
    assert!(coverage.proof_required);
    assert_eq!(
        coverage.proof_state,
        PredicateProofState::CandidateNeedsResidual
    );
    assert!(coverage
        .residual_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("no-false-negative proof")));
    assert!(forms.residual_forms.is_empty());

    let PhysicalPlanNodeKind::BuildPredicateNormalForms {
        coverage_forms,
        residual_forms,
        ..
    } = physical
        .physical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            PhysicalPlanNodeKind::BuildPredicateNormalForms { .. } => Some(&node.kind),
            _ => None,
        })
        .expect("expected predicate normal-form node")
    else {
        panic!("expected predicate normal-form node");
    };
    assert_eq!(*coverage_forms, forms.coverage_forms.len());
    assert_eq!(*residual_forms, 0);
}

#[test]
fn physical_predicates_encode_single_file_filecode_literal_without_residual_boundary() {
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Bob", "Ada"]);
    let physical = parse_resolve_plan_and_build_physical_plan(
        &bytes,
        "Person.where(name == \"Ada\").select(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(physical
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .any(|form| {
            form.representation == PhysicalRepresentationClass::FileCodeLiteral
                && form.exact
                && form.proof_state == PredicateProofState::ProvenExact
        }));
    let encoded_form = physical
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .find(|form| form.representation == PhysicalRepresentationClass::FileCodeLiteral)
        .expect("expected FileCode encoded predicate form");
    let domain = encoded_form
        .code_domain
        .as_ref()
        .expect("FileCode predicate form reports code-domain descriptor");
    let expected_dictionary_id = format!("file:{}:dictionary", "00".repeat(16));
    assert_eq!(
        domain.dictionary_id.as_deref(),
        Some(expected_dictionary_id.as_str())
    );
    assert!(!domain
        .dictionary_id
        .as_deref()
        .unwrap()
        .starts_with("placeholder:"));
    assert_eq!(domain.semantic_domain_id, None);
    assert_eq!(domain.dictionary_epoch, None);
    assert!(!physical
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .iter()
        .any(|form| form.physical_kind.as_deref() == Some("file_code")));
    let PhysicalPlanNodeKind::FileCodePredicate { forms } = &physical
        .physical_plan
        .nodes
        .iter()
        .find(|node| matches!(node.kind, PhysicalPlanNodeKind::FileCodePredicate { .. }))
        .unwrap()
        .kind
    else {
        panic!("expected FileCodePredicate");
    };
    assert!(forms.iter().any(|form| {
        form.representation == PhysicalRepresentationClass::FileCodeLiteral
            && form.exact
            && form.proof_state == PredicateProofState::ProvenExact
    }));
}

#[test]
fn physical_predicate_code_domains_report_dataset_tenant_scope() {
    let bytes = minimal_object_file();
    let mut planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned
        .resolved
        .operation_context
        .dataset
        .security_scope
        .tenant_id = Some("tenant-a".into());

    let physical = build_physical_plan(
        &bytes,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let domain = physical
        .physical_plan
        .predicate_normal_forms
        .code_domains
        .first()
        .expect("expected predicate code-domain descriptor");
    assert_eq!(domain.security_scope.tenant_id.as_deref(), Some("tenant-a"));
}

#[test]
fn physical_predicate_code_domains_report_exact_manifest_bridge_scope() {
    let (left, _) = object_file_with_filecode_records_with_file_id([0xA1; 16], &["red", "blue"]);
    let (right, _) = object_file_with_filecode_records_with_file_id([0xB2; 16], &["red", "green"]);
    let manifest = covm_manifest_for_members(&[("left.cove", &left), ("right.cove", &right)]);
    let security = SecurityContext {
        principal_or_session: Some("principal-a".into()),
        metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
        ..SecurityContext::default()
    };
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
            security: security.clone(),
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
        r#"Person.where(name == "red").select(name)"#,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::JsonRows),
            security,
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    planned.resolved.operation_context.dataset = scope;
    let physical = build_physical_plan(
        &left,
        planned,
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let domain = physical
        .physical_plan
        .predicate_normal_forms
        .code_domains
        .iter()
        .find(|domain| domain.physical_kind.as_deref() == Some("file_code"))
        .expect("expected manifest FileCode code-domain descriptor");

    assert!(domain.file_id.starts_with("dataset:sha256:"));
    assert!(domain
        .dictionary_id
        .as_deref()
        .is_some_and(|dictionary_id| dictionary_id
            .contains("bridged_dictionary_domain:cove_e_org.example.coveql_exec-codes")));
    assert_eq!(
        domain.semantic_domain_id.as_deref(),
        Some("cove_e:org.example.coveql:exec-codes")
    );
    assert_eq!(domain.dictionary_epoch, Some(1));
    assert_eq!(domain.security_scope.tenant_id.as_deref(), Some("tenant-a"));
}

#[test]
fn physical_predicates_report_goid_or_candidate_boundary() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &object_file_with_bool_records(&[true, false, true]),
        concat!(
            r#"Thing.where("#,
            r#"goid == uuid"00000000-0000-0000-0000-000000000000" || "#,
            r#"goid in [uuid"02020202-0202-0202-0202-020202020202"]"#,
            r#").select(goid, active)"#,
        ),
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let or_form = physical
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .iter()
        .find(|form| form.operator.as_deref() == Some("or"))
        .expect("expected OR residual verification form");
    assert_eq!(or_form.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        or_form.representation,
        PhysicalRepresentationClass::CodePure
    );
    assert_eq!(
        or_form.proof_state,
        PredicateProofState::CandidateNeedsResidual
    );
    assert!(or_form
        .residual_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("GOID OR")
            && reason.contains("materialized CoveQL truth verification")));
}

#[test]
fn physical_predicates_encode_same_path_or_without_residual_boundary() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true || active == false).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let encoded_or = physical
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .find(|form| form.operator.as_deref() == Some("or"))
        .expect("expected exact OR encoded predicate form");
    assert_eq!(encoded_or.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        encoded_or.representation,
        PhysicalRepresentationClass::CodePure
    );
    assert!(encoded_or.exact);
    assert_eq!(encoded_or.proof_state, PredicateProofState::ProvenExact);
    assert!(!physical
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .iter()
        .any(|form| form.operator.as_deref() == Some("or")));
}

#[test]
fn physical_predicates_encode_mixed_exact_or_without_residual_boundary() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        concat!(
            r#"Person.where(active == true || "#,
            r#"goid == uuid"00000000-0000-0000-0000-000000000000""#,
            r#").select(active)"#,
        ),
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let encoded_or = physical
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .find(|form| form.operator.as_deref() == Some("or"))
        .expect("expected exact OR encoded predicate form");
    assert_eq!(encoded_or.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        encoded_or.representation,
        PhysicalRepresentationClass::CodePure
    );
    assert!(encoded_or.exact);
    assert_eq!(encoded_or.proof_state, PredicateProofState::ProvenExact);
    assert!(!physical
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .iter()
        .any(|form| form.operator.as_deref() == Some("or")));
}

#[test]
fn physical_predicates_encode_not_of_exact_predicate_without_residual_boundary() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(!(active == true)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let encoded_not = physical
        .physical_plan
        .predicate_normal_forms
        .encoded_forms
        .iter()
        .find(|form| form.operator.as_deref() == Some("not"))
        .expect("expected exact NOT encoded predicate form");
    assert_eq!(encoded_not.placement, PredicatePlacement::PreReconstruction);
    assert_eq!(
        encoded_not.representation,
        PhysicalRepresentationClass::CodePure
    );
    assert!(encoded_not.exact);
    assert_eq!(encoded_not.proof_state, PredicateProofState::ProvenExact);
    assert!(!physical
        .physical_plan
        .predicate_normal_forms
        .residual_forms
        .iter()
        .any(|form| form.operator.as_deref() == Some("not")));
}

#[test]
fn physical_optional_metadata_fail_open_and_strict_modes() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let mut options = PhysicalPlanOptions::default();
    options.sidecars.coverage_proof_record_bytes = Some(vec![1, 2, 3]);

    let physical = build_physical_plan(
        &minimal_object_file(),
        planned.clone(),
        options.clone(),
        validation_options(),
    )
    .unwrap();
    assert!(physical.sidecar_validations.iter().any(|validation| {
        validation.name == "coverage_proof_records"
            && validation.status == PhysicalSidecarStatus::Ignored
    }));
    assert!(physical.sidecar_validations.iter().all(|validation| {
        validation.report_version == coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION
    }));
    let sidecar_validation_json = physical.explain_json()["physical_plan"]["sidecar_validations"]
        .as_array()
        .unwrap()
        .clone();
    for validation in &sidecar_validation_json {
        for field in coveql::explain_json_schema().required_physical_sidecar_validation_fields {
            assert!(
                validation.get(*field).is_some(),
                "physical sidecar validation explain JSON missing field {field}: {validation}"
            );
        }
        assert_eq!(
            validation["report_version"],
            coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION
        );
    }
    for field in coveql::explain_json_schema().required_physical_plan_sidecar_fields {
        assert!(
            physical.explain_json()["physical_plan"]
                .get(*field)
                .is_some(),
            "physical plan explain JSON missing sidecar report group {field}"
        );
    }
    assert!(physical
        .runtime_compatibility_report
        .iter()
        .any(|validation| {
            validation.name == "cove_r" && validation.status == PhysicalSidecarStatus::Missing
        }));
    assert!(
        physical
            .runtime_compatibility_report
            .iter()
            .all(|validation| validation.report_version
                == coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION)
    );
    assert!(physical
        .physical_plan
        .runtime_compatibility
        .iter()
        .any(|validation| {
            validation.name == "cove_r" && validation.status == PhysicalSidecarStatus::Missing
        }));
    assert!(
        physical.explain_json()["physical_plan"]["runtime_compatibility"]
            .as_array()
            .unwrap()
            .iter()
            .any(|validation| validation["name"] == json!("cove_r")
                && validation["status"] == json!("missing")
                && validation["report_version"]
                    == json!(coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION))
    );
    assert!(physical
        .cache_compatibility_report
        .iter()
        .any(|validation| {
            validation.name == "cove_cache" && validation.status == PhysicalSidecarStatus::Missing
        }));
    assert!(
        physical
            .cache_compatibility_report
            .iter()
            .all(|validation| validation.report_version
                == coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION)
    );
    assert!(physical
        .physical_plan
        .cache_compatibility
        .iter()
        .any(|validation| {
            validation.name == "cove_cache" && validation.status == PhysicalSidecarStatus::Missing
        }));
    assert!(
        physical.explain_json()["physical_plan"]["cache_compatibility"]
            .as_array()
            .unwrap()
            .iter()
            .any(|validation| validation["name"] == json!("cove_cache")
                && validation["status"] == json!("missing")
                && validation["report_version"]
                    == json!(coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION))
    );
    assert!(physical
        .codec_compatibility_report
        .iter()
        .any(|validation| {
            validation.name == "cove_cx"
                && validation.status == PhysicalSidecarStatus::TrustedCandidate
                && validation.safe_details["compatibility"] == json!("core_codec_only")
        }));
    assert!(
        physical
            .codec_compatibility_report
            .iter()
            .all(|validation| validation.report_version
                == coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION)
    );
    assert!(physical
        .physical_plan
        .codec_compatibility
        .iter()
        .any(|validation| {
            validation.name == "cove_cx"
                && validation.status == PhysicalSidecarStatus::TrustedCandidate
                && validation.safe_details["compatibility"] == json!("core_codec_only")
        }));
    assert!(
        physical.explain_json()["physical_plan"]["codec_compatibility"]
            .as_array()
            .unwrap()
            .iter()
            .any(|validation| validation["name"] == json!("cove_cx")
                && validation["status"] == json!("trusted_candidate")
                && validation["report_version"]
                    == json!(coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION))
    );
    assert!(physical
        .physical_plan
        .fallbacks
        .iter()
        .any(|fallback| fallback.source == "cove_coverage"));

    options.optional_metadata_fail_open = false;
    let err = build_physical_plan(
        &minimal_object_file(),
        planned,
        options,
        validation_options(),
    )
    .unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_PHYSICAL_METADATA_VALIDATION");
}

#[test]
fn physical_runtime_compatibility_hints_are_reported_as_trusted_or_ignored() {
    let supported_runtime_file =
        minimal_object_file_with_runtime_hints(&[RuntimeCompatibilityHintV2 {
            hint_id: 1,
            hint_kind: RuntimeHintKindV2::EngineAdapter,
            required: true,
            flags: 0,
            namespace: "org.cove".into(),
            name: "datafusion".into(),
            version_major: 1,
            version_minor: 0,
            payload_ref: u32::MAX,
            checksum: 0,
        }]);

    let supported = parse_resolve_plan_and_build_physical_plan(
        &supported_runtime_file,
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let trusted = supported
        .runtime_compatibility_report
        .iter()
        .find(|validation| validation.name == "cove_r")
        .expect("expected COVE-R runtime compatibility report");
    assert_eq!(trusted.status, PhysicalSidecarStatus::TrustedCandidate);
    assert_eq!(trusted.candidate_count, 1);
    assert_eq!(trusted.safe_details["hint_count"], json!(1));
    assert_eq!(trusted.safe_details["required_hint_count"], json!(1));
    assert_eq!(
        trusted.safe_details["unsupported_required_hint_count"],
        json!(0)
    );
    assert!(
        supported.explain_json()["physical_plan"]["runtime_compatibility"]
            .as_array()
            .unwrap()
            .iter()
            .any(|validation| validation["name"] == json!("cove_r")
                && validation["status"] == json!("trusted_candidate")
                && validation["candidate_count"] == json!(1)
                && validation["report_version"]
                    == json!(coveql::PHYSICAL_SIDECAR_VALIDATION_VERSION)
                && validation["redacted"] == json!(true))
    );

    let unsupported_runtime_file =
        minimal_object_file_with_runtime_hints(&[RuntimeCompatibilityHintV2 {
            hint_id: 2,
            hint_kind: RuntimeHintKindV2::PredicateKernel,
            required: true,
            flags: 0,
            namespace: "org.cove".into(),
            name: "missing-kernel".into(),
            version_major: 1,
            version_minor: 0,
            payload_ref: u32::MAX,
            checksum: 0,
        }]);
    let unsupported = parse_resolve_plan_and_build_physical_plan(
        &unsupported_runtime_file,
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let ignored = unsupported
        .runtime_compatibility_report
        .iter()
        .find(|validation| validation.name == "cove_r")
        .expect("expected COVE-R runtime compatibility report");
    assert_eq!(ignored.status, PhysicalSidecarStatus::Ignored);
    assert_eq!(
        ignored.safe_details["unsupported_required_hint_count"],
        json!(1)
    );
    assert!(ignored
        .fallback_reason
        .as_deref()
        .unwrap()
        .contains("unsupported by the default CoveQL runtime"));
    assert!(
        unsupported.explain_json()["physical_plan"]["runtime_compatibility"]
            .as_array()
            .unwrap()
            .iter()
            .any(|validation| validation["name"] == json!("cove_r")
                && validation["status"] == json!("ignored")
                && validation["fallback_reason"]
                    .as_str()
                    .unwrap()
                    .contains("unsupported by the default CoveQL runtime"))
    );
}

#[test]
fn physical_explain_text_is_deterministic_and_redacted() {
    let physical = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = physical.explain_json();
    let text = physical.explain_text();
    assert_eq!(text, render_explain_text(&explain));
    assert_eq!(text, physical.explain_text());
    assert_eq!(explain["execution"]["kind"], "physical_plan_explanation");
    assert_eq!(explain["execution"]["coded_execution"]["eligible"], true);
    assert!(explain["execution"]["coded_execution"]["operator_contracts"].is_array());
    assert_eq!(
        explain["execution"]["coded_execution"]["residual_verification_required"],
        true
    );
    assert!(text.contains("physical_plan.nodes:"));
    assert!(explain.to_string().contains("<redacted>"));
}

#[test]
fn physical_plan_fingerprint_is_stable_for_harmless_quoting() {
    let a = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_plan_and_build_physical_plan(
        &minimal_object_file(),
        "`Person`.where(`active` == true).select(`active`)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(a.physical_plan_fingerprint, b.physical_plan_fingerprint);
}
