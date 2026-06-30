use super::*;

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
