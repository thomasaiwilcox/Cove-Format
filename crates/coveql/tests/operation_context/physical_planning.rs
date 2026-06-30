use super::*;

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
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
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
    let bytes = include_bytes!("../../../../conformance/accept/cove_o_temporal_valid.cove");
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
