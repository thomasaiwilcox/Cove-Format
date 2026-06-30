use super::*;

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
        include_bytes!("../../../../conformance/feature-scope/optional_layout_crc_ignored.cove");
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
        "../../../../conformance/feature-scope/operation_scoped_unknown_coverage_reject.cove"
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
