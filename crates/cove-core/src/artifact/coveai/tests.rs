use super::fixtures::*;
use super::*;

#[test]
fn writes_and_parses_empty_coveai_artifact() {
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 123, &[]).unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.artifact_kind, CoveAiArtifactKind::CoveAiBundle);
    assert_eq!(parsed.header.artifact_id, [7u8; 16]);
    assert_eq!(parsed.sections.len(), 0);
}

#[test]
fn rejects_duplicate_section_id() {
    let sections = [
        CoveAiWritableSection {
            section_id: 1,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: 16,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![1, 2, 3],
        },
        CoveAiWritableSection {
            section_id: 1,
            section_kind: SectionKind::AiPayloadBytes as u32,
            profile_kind: 16,
            payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
            requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
            source_binding_ref: 0,
            required_ai_features: 0,
            optional_ai_features: 0,
            feature_binding_ref: 0,
            payload: vec![4, 5, 6],
        },
    ];
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [0; 16], 0, &sections).unwrap();
    assert!(CoveAiFile::parse(&bytes).is_err());
}

#[test]
fn descriptor_bundle_serializes_digest_bound_companion_ref() {
    let uri = b"vectors.covev";
    let digest = [0xAB; 32];
    let mut payload = Vec::new();
    payload.extend_from_slice(uri);
    payload.extend_from_slice(&digest);

    let mut tables = AiDescriptorTablesV1::default();
    tables.strings.push(AiStringEntryV1 {
        string_ref: 1,
        utf8_byte_length: uri.len() as u32,
        payload_ref: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: digest.len() as u16,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: uri.len() as u64,
        decoded_length: uri.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: uri.len() as u64,
        payload_length: digest.len() as u64,
        decoded_length: digest.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.companion_artifacts.push(AiCompanionArtifactRefV1 {
        artifact_ref: 1,
        artifact_kind: AI_COMPANION_ARTIFACT_KIND_CVV2,
        artifact_id: [9; 16],
        uri_ref: 1,
        artifact_digest_ref: 1,
        source_binding_ref: 0,
        required_ai_features: AI_FEATURE_VECTOR,
        optional_ai_features: 0,
        flags: 0,
        crc32c: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [21u8; 16],
        created_at_us: 908,
        payload_sections: vec![coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveAiShared,
            payload,
        )],
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.companion_artifacts.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.companion_artifacts[0].artifact_kind,
        AI_COMPANION_ARTIFACT_KIND_CVV2
    );
    assert_eq!(parsed.descriptor_tables.strings.len(), 1);
    assert_eq!(parsed.descriptor_tables.digests.len(), 1);
}

#[test]
fn rejects_companion_ref_missing_artifact_digest() {
    let uri = b"vectors.covev";
    let sections = vec![
        coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveAiShared,
            uri.to_vec(),
        ),
        coveai_binary_section(
            2,
            SectionKind::AiReferenceTables,
            PrimaryProfile::CoveAiShared,
            encode_records([
                encode_string_entry(AiStringEntryV1 {
                    string_ref: 1,
                    utf8_byte_length: uri.len() as u32,
                    payload_ref: 1,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
                encode_payload_ref_entry(AiPayloadRefEntryV1 {
                    payload_ref: 1,
                    storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
                    media_type_ref: 0,
                    section_id: 1,
                    uri_ref: 0,
                    payload_offset: 0,
                    section_payload_offset: 0,
                    payload_length: uri.len() as u64,
                    decoded_length: uri.len() as u64,
                    integrity_ref: 0,
                    flags: 0,
                    crc32c: 0,
                })
                .unwrap(),
            ])
            .unwrap(),
        ),
        coveai_binary_section(
            3,
            SectionKind::AiCompanionArtifactRef,
            PrimaryProfile::CoveAiShared,
            encode_records([encode_companion_artifact_ref(AiCompanionArtifactRefV1 {
                artifact_ref: 1,
                artifact_kind: AI_COMPANION_ARTIFACT_KIND_CVV2,
                artifact_id: [9; 16],
                uri_ref: 1,
                artifact_digest_ref: 99,
                source_binding_ref: 0,
                required_ai_features: AI_FEATURE_VECTOR,
                optional_ai_features: 0,
                flags: 0,
                crc32c: 0,
            })
            .unwrap()])
            .unwrap(),
        ),
    ];
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [22u8; 16], 909, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing artifact_digest_ref")
    ));
}

#[test]
fn descriptor_bundle_serializes_source_binding() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.source_bindings.push(AiSourceBindingV1 {
        source_binding_id: 1,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_artifact_ref: 0,
        source_file_digest_ref: 0,
        covm_snapshot_ref: 0,
        schema_fingerprint_ref: 0,
        dictionary_digest_ref: 0,
        map_fingerprint_ref: 0,
        policy_context_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        branch_ref: 0,
        as_of_csn: 42,
        flags: 0,
        crc32c: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [23u8; 16],
        created_at_us: 910,
        payload_sections: Vec::new(),
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.source_bindings.len(), 1);
    assert_eq!(parsed.descriptor_tables.source_bindings[0].as_of_csn, 42);
}

#[test]
fn descriptor_bundle_rejects_source_binding_missing_digest_ref() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.source_bindings.push(AiSourceBindingV1 {
        source_binding_id: 1,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_artifact_ref: 0,
        source_file_digest_ref: 99,
        covm_snapshot_ref: 0,
        schema_fingerprint_ref: 0,
        dictionary_digest_ref: 0,
        map_fingerprint_ref: 0,
        policy_context_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        branch_ref: 0,
        as_of_csn: 0,
        flags: 0,
        crc32c: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [24u8; 16],
            created_at_us: 911,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("missing source_file_digest_ref")
    ));
}

#[test]
fn descriptor_bundle_serializes_policy_ref_and_source_policy_ref() {
    let authority = b"visibility-policy";
    let mut tables = AiDescriptorTablesV1::default();
    tables.strings.push(AiStringEntryV1 {
        string_ref: 1,
        utf8_byte_length: authority.len() as u32,
        payload_ref: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: authority.len() as u64,
        decoded_length: authority.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.policies.push(AiPolicyRefEntryV1 {
        policy_ref: 1,
        policy_kind: AI_POLICY_KIND_VISIBILITY,
        authority_ref: 1,
        payload_ref: 0,
        digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.source_bindings.push(AiSourceBindingV1 {
        source_binding_id: 1,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_artifact_ref: 0,
        source_file_digest_ref: 0,
        covm_snapshot_ref: 0,
        schema_fingerprint_ref: 0,
        dictionary_digest_ref: 0,
        map_fingerprint_ref: 0,
        policy_context_ref: 1,
        visibility_scope_ref: 1,
        redaction_scope_ref: 0,
        branch_ref: 0,
        as_of_csn: 0,
        flags: 0,
        crc32c: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [25u8; 16],
        created_at_us: 912,
        payload_sections: vec![coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveAiShared,
            authority.to_vec(),
        )],
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.policies.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.source_bindings[0].visibility_scope_ref,
        1
    );
}

#[test]
fn descriptor_bundle_rejects_policy_payload_without_digest() {
    let policy_payload = b"policy";
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: policy_payload.len() as u64,
        decoded_length: policy_payload.len() as u64,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.policies.push(AiPolicyRefEntryV1 {
        policy_ref: 1,
        policy_kind: AI_POLICY_KIND_VISIBILITY,
        authority_ref: 0,
        payload_ref: 1,
        digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [26u8; 16],
            created_at_us: 913,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                policy_payload.to_vec(),
            )],
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("payload_ref requires digest_ref")
    ));
}

#[test]
fn descriptor_bundle_serializes_privacy_summary_refs() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 1,
        decoded_length: 1,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.policies.push(AiPolicyRefEntryV1 {
        policy_ref: 1,
        policy_kind: AI_POLICY_KIND_VISIBILITY,
        authority_ref: 0,
        payload_ref: 0,
        digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 1,
        sensitivity_bits_ref: 1,
        policy_ref: 1,
        visibility_scope_ref: 1,
        redaction_scope_ref: 0,
        retention_state: 1,
        disclosure_state: 1,
        flags: 0,
        crc32c: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [38u8; 16],
        created_at_us: 925,
        payload_sections: vec![coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveAiShared,
            vec![1],
        )],
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.privacy_summaries.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.privacy_summaries[0].visibility_scope_ref,
        1
    );
}

#[test]
fn descriptor_bundle_rejects_privacy_summary_missing_visibility_policy() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 99,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [39u8; 16],
            created_at_us: 926,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("missing visibility_scope_ref 99")
    ));
}

#[test]
fn descriptor_bundle_rejects_payload_integrity_digest_length_mismatch() {
    let mut payload = vec![0xAA; 4];
    payload.extend_from_slice(&[0xBB; 32]);
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 4,
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
        digest_payload_ref: 2,
        domain_hint: 6,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_integrity.push(AiPayloadIntegrityV1 {
        integrity_ref: 1,
        payload_ref: 1,
        digest_domain: 6,
        reserved0: 0,
        digest_algorithm: 1,
        digest_len: 16,
        digest_ref: 1,
        payload_crc32c: 0,
        flags: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [36u8; 16],
            created_at_us: 923,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                payload,
            )],
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("digest algorithm/length mismatch")
    ));
}

#[test]
fn descriptor_bundle_rejects_payload_integrity_digest_payload_cycle() {
    let mut payload = vec![0xAA; 4];
    payload.extend_from_slice(&[0xBB; 32]);
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 1,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 4,
        payload_length: 32,
        decoded_length: 32,
        integrity_ref: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 32,
        digest_payload_ref: 2,
        domain_hint: 6,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_integrity.push(AiPayloadIntegrityV1 {
        integrity_ref: 1,
        payload_ref: 1,
        digest_domain: 6,
        reserved0: 0,
        digest_algorithm: 1,
        digest_len: 32,
        digest_ref: 1,
        payload_crc32c: 0,
        flags: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [37u8; 16],
            created_at_us: 924,
            payload_sections: vec![coveai_payload_section(
                1,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveAiShared,
                payload,
            )],
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("cyclic digest payload integrity")
    ));
}

#[test]
fn descriptor_bundle_serializes_source_span_and_transform() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.source_spans.push(AiSourceSpanEntryV1 {
        source_span_ref: 1,
        source_binding_ref: 0,
        source_kind: AI_SOURCE_KIND_COVE_FILE,
        source_row_ref: 7,
        source_object_ref: 0,
        byte_start: 11,
        byte_length: 5,
        token_start: 0,
        token_count: 0,
        evidence_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.transforms.push(AiTransformEntryV1 {
        transform_ref: 1,
        transform_kind: AI_TRANSFORM_KIND_TEXT_NORMALIZATION,
        function_or_template_ref: 0,
        input_digest_ref: 0,
        output_digest_ref: 0,
        parameter_payload_ref: 0,
        transform_digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [27u8; 16],
        created_at_us: 914,
        payload_sections: Vec::new(),
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.source_spans.len(), 1);
    assert_eq!(parsed.descriptor_tables.transforms.len(), 1);
    assert_eq!(parsed.descriptor_tables.source_spans[0].byte_start, 11);
}

#[test]
fn descriptor_bundle_rejects_transform_missing_parameter_payload() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.transforms.push(AiTransformEntryV1 {
        transform_ref: 1,
        transform_kind: AI_TRANSFORM_KIND_TEXT_NORMALIZATION,
        function_or_template_ref: 0,
        input_digest_ref: 0,
        output_digest_ref: 0,
        parameter_payload_ref: 99,
        transform_digest_ref: 0,
        flags: 0,
        crc32c: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [28u8; 16],
            created_at_us: 915,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("missing parameter_payload_ref")
    ));
}

#[test]
fn descriptor_bundle_serializes_ai_section_feature_binding() {
    let mut tables = AiDescriptorTablesV1::default();
    tables
        .section_feature_bindings
        .push(AiSectionFeatureBindingV1 {
            binding_ref: 1,
            section_id: 1,
            scope: FeatureScopeV2::OperationRequired as u8,
            profile_kind: PrimaryProfile::CoveVec as u8,
            operation_kind: OperationKindV2::AiEmbedding as u16,
            required_ai_features: AI_FEATURE_VECTOR,
            optional_ai_features: 0,
            target_local_ref: 0,
            flags: 0,
            crc32c: 0,
        });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [29u8; 16],
        created_at_us: 916,
        payload_sections: vec![coveai_payload_section(
            1,
            SectionKind::AiPayloadBytes,
            PrimaryProfile::CoveVec,
            vec![0],
        )],
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.section_feature_bindings.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.section_feature_bindings[0].operation_kind,
        OperationKindV2::AiEmbedding as u16
    );
}

#[test]
fn descriptor_bundle_rejects_ai_section_feature_binding_self_target() {
    let mut tables = AiDescriptorTablesV1::default();
    tables
        .section_feature_bindings
        .push(AiSectionFeatureBindingV1 {
            binding_ref: 1,
            section_id: 1_000,
            scope: FeatureScopeV2::OperationRequired as u8,
            profile_kind: PrimaryProfile::CoveVec as u8,
            operation_kind: OperationKindV2::AiEmbedding as u16,
            required_ai_features: AI_FEATURE_VECTOR,
            optional_ai_features: 0,
            target_local_ref: 0,
            flags: 0,
            crc32c: 0,
        });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [30u8; 16],
            created_at_us: 917,
            payload_sections: Vec::new(),
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("must not target AI_SECTION_FEATURE_BINDING")
    ));
}

#[test]
fn descriptor_bundle_serializes_vector_compatibility_and_composition() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables
        .vector_space_compatibilities
        .push(VectorSpaceCompatibilityDescriptorV1 {
            compatibility_id: 1,
            left_vector_space_id: 1,
            right_vector_space_id: 2,
            compatibility_kind: 1,
            compatibility_authority: 0,
            metric: 1,
            normalization_policy: 0,
            transform_ref: 0,
            numeric_transform_error_ppm: 0,
            ranking_eval_ref: 0,
            calibration_dataset_ref: 0,
            evidence_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_arithmetic_profiles
        .push(VectorArithmeticProfileV1 {
            arithmetic_profile_id: 1,
            profile_name_ref: 0,
            arithmetic_kind: 0,
            input_quantization_kind: 0,
            accumulator_kind: 0,
            rounding_mode: 0,
            overflow_policy: 0,
            component_order: 0,
            weight_scale: 1_000_000,
            output_quantization_kind: 0,
            output_element_type: 0,
            normalization_policy: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_composition_components
        .push(VectorCompositionComponentV1 {
            component_id: 1,
            slot_policy_ref: 0,
            source_vector_space_id: 1,
            weight_ppm: 1_000_000,
            required: 1,
            redaction_behavior: 0,
            missing_behavior: 0,
            reserved: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_composition_profiles
        .push(VectorCompositionProfileV1 {
            composition_profile_id: 1,
            composition_name_ref: 0,
            output_vector_space_id: 1,
            arithmetic_profile_ref: 1,
            method: 1,
            missing_policy: 0,
            normalize_inputs: 1,
            normalize_output: 1,
            result_authority: 0,
            reproducibility_class: 0,
            first_component_ref: 1,
            component_count: 1,
            template_ref: 0,
            flags: 0,
            checksum: 0,
        });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [31u8; 16],
        created_at_us: 918,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed.descriptor_tables.vector_space_compatibilities.len(),
        1
    );
    assert_eq!(parsed.descriptor_tables.vector_arithmetic_profiles.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.vector_composition_components.len(),
        1
    );
    assert_eq!(
        parsed.descriptor_tables.vector_composition_profiles.len(),
        1
    );
}

#[test]
fn descriptor_bundle_rejects_vector_compatibility_bad_kind() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    let mut compatibility = test_vector_space_compatibility();
    compatibility.compatibility_kind = 42;
    tables.vector_space_compatibilities.push(compatibility);

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [46u8; 16],
            created_at_us: 933,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("VectorSpaceCompatibility compatibility_kind")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_compatibility_bad_authority() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    let mut compatibility = test_vector_space_compatibility();
    compatibility.compatibility_authority = 42;
    tables.vector_space_compatibilities.push(compatibility);

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [47u8; 16],
            created_at_us: 934,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("VectorSpaceCompatibility compatibility_authority")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_compatibility_excessive_numeric_error() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    let mut compatibility = test_vector_space_compatibility();
    compatibility.numeric_transform_error_ppm = 1_000_001;
    tables.vector_space_compatibilities.push(compatibility);

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [48u8; 16],
            created_at_us: 935,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("numeric_transform_error_ppm exceeds 1_000_000")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_compatibility_missing_evidence_ref() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    let mut compatibility = test_vector_space_compatibility();
    compatibility.evidence_ref = 99;
    tables.vector_space_compatibilities.push(compatibility);

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [49u8; 16],
            created_at_us: 936,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 99")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_composition_bad_method() {
    let (mut tables, payload_sections) = vector_composition_tables();
    tables.vector_composition_profiles[0].method = 42;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [41u8; 16],
            created_at_us: 928,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("VectorCompositionProfile method")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_composition_missing_template_ref() {
    let (mut tables, payload_sections) = vector_composition_tables();
    tables.vector_composition_profiles[0].template_ref = 99;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [42u8; 16],
            created_at_us: 929,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message)) if message.contains("missing template_ref 99")
    ));
}

#[test]
fn descriptor_bundle_rejects_canonical_composition_without_arithmetic_profile() {
    let (mut tables, payload_sections) = vector_composition_tables();
    tables.vector_composition_profiles[0].result_authority = 2;
    tables.vector_composition_profiles[0].arithmetic_profile_ref = 0;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [43u8; 16],
            created_at_us: 930,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("CanonicalFixedPointRecompute requires arithmetic_profile_ref")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_component_bad_redaction_behavior() {
    let (mut tables, payload_sections) = vector_composition_tables();
    tables.vector_composition_components[0].redaction_behavior = 42;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [44u8; 16],
            created_at_us: 931,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("VectorCompositionComponent redaction_behavior")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_arithmetic_bad_rounding_mode() {
    let (mut tables, payload_sections) = vector_composition_tables();
    tables.vector_arithmetic_profiles[0].rounding_mode = 42;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [45u8; 16],
            created_at_us: 932,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("VectorArithmeticProfile rounding_mode")
    ));
}

#[test]
fn descriptor_bundle_serializes_chunk_object_and_training_vector_bindings() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
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
        target_tokens: 0,
        min_tokens: 0,
        max_tokens: 0,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    let mut chunk = test_text_chunk(1, 0, 0, 0, 12, 12, 1);
    chunk.source_ref = 0;
    tables.text_chunks.push(chunk);
    tables.training_profiles.push(test_training_profile());
    tables
        .training_samples
        .push(test_training_sample(1, 1, 0, 0));
    tables
        .vector_composition_components
        .push(VectorCompositionComponentV1 {
            component_id: 1,
            slot_policy_ref: 0,
            source_vector_space_id: 1,
            weight_ppm: 1_000_000,
            required: 1,
            redaction_behavior: 0,
            missing_behavior: 0,
            reserved: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .vector_composition_profiles
        .push(VectorCompositionProfileV1 {
            composition_profile_id: 1,
            composition_name_ref: 0,
            output_vector_space_id: 1,
            arithmetic_profile_ref: 0,
            method: 1,
            missing_policy: 0,
            normalize_inputs: 1,
            normalize_output: 1,
            result_authority: 0,
            reproducibility_class: 0,
            first_component_ref: 1,
            component_count: 1,
            template_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .object_state_vector_bindings
        .push(ObjectStateVectorBindingV1 {
            binding_id: 20,
            vector_space_id: 1,
            composition_profile_ref: 1,
            file_ref: 0,
            object_type_id: 7,
            goid_ref: 0,
            branch_ref: 0,
            temporal_kind: 0,
            csn: 123,
            timestamp_us: 456,
            property_dependency_fingerprint_ref: 0,
            vector_ref: 2,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .training_sample_vector_bindings
        .push(TrainingSampleVectorBindingV1 {
            binding_id: 30,
            vector_space_id: 1,
            training_profile_ref: 1,
            sample_id: 1,
            source_snapshot_ref: 0,
            sample_fingerprint_ref: 0,
            vector_ref: 3,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [32u8; 16],
        created_at_us: 919,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.chunk_vector_bindings.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.object_state_vector_bindings.len(),
        1
    );
    assert_eq!(
        parsed
            .descriptor_tables
            .training_sample_vector_bindings
            .len(),
        1
    );
}

#[test]
fn ai_vector_search_covers_association_asset_and_multimodal_bindings() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
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
    tables
        .association_state_vector_bindings
        .push(AssociationStateVectorBindingV1 {
            binding_id: 40,
            vector_space_id: 1,
            composition_profile_ref: 0,
            file_ref: 0,
            association_type_id: 9,
            association_key_ref: 0,
            branch_ref: 0,
            temporal_kind: 0,
            csn: 77,
            timestamp_us: 88,
            property_dependency_fingerprint_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });
    let mut asset = test_asset_ref();
    asset.tensor_layout_ref = 0;
    tables.assets.push(asset);
    tables.asset_vector_bindings.push(AssetVectorBindingV1 {
        binding_id: 50,
        vector_space_id: 1,
        asset_ref: 1,
        transform_ref: 0,
        asset_digest_ref: 0,
        vector_ref: 2,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .multimodal_sequence_packs
        .push(test_multimodal_pack(1, 0));
    tables
        .multimodal_sequence_vector_bindings
        .push(MultimodalSequenceVectorBindingV1 {
            binding_id: 60,
            vector_space_id: 1,
            sequence_pack_id: 1,
            sequence_profile_ref: 0,
            source_snapshot_ref: 0,
            vector_ref: 3,
            model_input_digest_ref: 0,
            flags: 0,
            checksum: 0,
        });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [34u8; 16],
        created_at_us: 921,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed
            .descriptor_tables
            .association_state_vector_bindings
            .len(),
        1
    );
    assert_eq!(parsed.descriptor_tables.asset_vector_bindings.len(), 1);
    assert_eq!(
        parsed
            .descriptor_tables
            .multimodal_sequence_vector_bindings
            .len(),
        1
    );

    let all_results = ai_vector_search(
        &bytes,
        &AiVectorSearchPlan {
            query_file_code: None,
            query_vector_ref: Some(1),
            query_values: None,
            top_k: 3,
            target_kind: AiVectorSearchTargetKind::All,
            index: AiVectorIndexSelection::ExactFlat,
        },
    )
    .unwrap();
    assert_eq!(all_results.len(), 3);
    let target_kinds = all_results
        .iter()
        .map(|result| result.target_kind.as_str())
        .collect::<BTreeSet<_>>();
    assert!(target_kinds.contains("association_state"));
    assert!(target_kinds.contains("asset"));
    assert!(target_kinds.contains("multimodal_sequence"));
    assert_eq!(all_results[0].association_type_id, Some(9));

    let asset_results = ai_vector_search(
        &bytes,
        &AiVectorSearchPlan {
            query_file_code: None,
            query_vector_ref: Some(2),
            query_values: None,
            top_k: 1,
            target_kind: AiVectorSearchTargetKind::Asset,
            index: AiVectorIndexSelection::ExactFlat,
        },
    )
    .unwrap();
    assert_eq!(asset_results[0].target_kind, "asset");
    assert_eq!(asset_results[0].asset_ref, Some(1));
}

#[test]
fn descriptor_bundle_rejects_chunk_vector_binding_missing_vector_ref() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
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
        target_tokens: 0,
        min_tokens: 0,
        max_tokens: 0,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    let mut chunk = test_text_chunk(1, 0, 0, 0, 12, 12, 1);
    chunk.source_ref = 0;
    tables.text_chunks.push(chunk);
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 99,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [33u8; 16],
            created_at_us: 920,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message)) if message.contains("missing vector_ref 99")
    ));
}

#[test]
fn descriptor_bundle_round_trips_v2_vector_bindings_for_all_binding_kinds() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_model_input_digest(&mut tables, 2, AI_DIGEST_DOMAIN_MODEL_INPUT_BYTES);
    add_chunk_binding_support(&mut tables, 1);
    tables.training_profiles.push(test_training_profile());
    tables
        .training_samples
        .push(test_training_sample(1, 1, 0, 0));
    tables
        .filecode_vector_bindings
        .push(FileCodeVectorBindingV1 {
            binding_id: 1,
            vector_space_id: 1,
            slot_policy_ref: 0,
            file_ref: 0,
            dictionary_digest_ref: 0,
            schema_fingerprint_ref: 0,
            table_id: 0,
            column_id: 0,
            object_type_id: 0,
            property_id: 0,
            association_type_id: 0,
            path_ref: 0,
            file_code: 7,
            reserved0: 0,
            canonical_value_hash_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 2,
            flags: 0,
            checksum: 0,
        });
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 2,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });
    tables
        .object_state_vector_bindings
        .push(ObjectStateVectorBindingV1 {
            binding_id: 3,
            vector_space_id: 1,
            composition_profile_ref: 0,
            file_ref: 0,
            object_type_id: 7,
            goid_ref: 0,
            branch_ref: 0,
            temporal_kind: 0,
            csn: 123,
            timestamp_us: 456,
            property_dependency_fingerprint_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 2,
            flags: 0,
            checksum: 0,
        });
    tables
        .training_sample_vector_bindings
        .push(TrainingSampleVectorBindingV1 {
            binding_id: 4,
            vector_space_id: 1,
            training_profile_ref: 1,
            sample_id: 1,
            source_snapshot_ref: 0,
            sample_fingerprint_ref: 0,
            vector_ref: 1,
            model_input_digest_ref: 2,
            flags: 0,
            checksum: 0,
        });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [50u8; 16],
        created_at_us: 937,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    let vector_binding_section = parsed
        .sections
        .iter()
        .find(|section| section.entry.section_kind == SectionKind::AiVectorBinding as u32)
        .unwrap();
    assert_eq!(vector_binding_section.record_headers.len(), 4);
    assert!(vector_binding_section
        .record_headers
        .iter()
        .all(|header| header.record_version == 2));
    assert_eq!(
        parsed.descriptor_tables.filecode_vector_bindings[0].model_input_digest_ref,
        2
    );
    assert_eq!(
        parsed.descriptor_tables.chunk_vector_bindings[0].model_input_digest_ref,
        2
    );
    assert_eq!(
        parsed.descriptor_tables.object_state_vector_bindings[0].model_input_digest_ref,
        2
    );
    assert_eq!(
        parsed.descriptor_tables.training_sample_vector_bindings[0].model_input_digest_ref,
        2
    );
}

#[test]
fn rejects_v2_vector_binding_with_zero_model_input_digest_ref() {
    let mut payload = vec![0u8; 52];
    put_u64(&mut payload, 0, 10);
    put_u32(&mut payload, 8, 1);
    put_u64(&mut payload, 12, 1);
    put_u32(&mut payload, 20, 1);
    put_u64(&mut payload, 32, 1);
    put_u32(&mut payload, 40, 0);
    put_u32(&mut payload, 44, 0);
    let record = encode_ai_record_with_version(
        2,
        2,
        10,
        AI_FLAG_PAYLOAD_CRC32C_PRESENT,
        with_payload_crc32c(payload, 48).unwrap(),
    )
    .unwrap();
    let sections = vec![coveai_binary_section(
        1,
        SectionKind::AiVectorBinding,
        PrimaryProfile::CoveVec,
        encode_records([record]).unwrap(),
    )];
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [51u8; 16], 938, &sections)
        .unwrap();

    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("ChunkVectorBindingV2 requires non-zero model_input_digest_ref")
    ));
}

#[test]
fn descriptor_bundle_rejects_model_input_digest_missing_or_wrong_domain() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_chunk_binding_support(&mut tables, 1);
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 99,
        flags: 0,
        checksum: 0,
    });
    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [52u8; 16],
            created_at_us: 939,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("missing model_input_digest_ref 99")
    ));

    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_chunk_binding_support(&mut tables, 1);
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 1,
        flags: 0,
        checksum: 0,
    });
    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [53u8; 16],
            created_at_us: 940,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("must use ModelInputBytes digest domain")
    ));
}

#[test]
fn descriptor_bundle_rejects_same_model_input_mapping_to_different_vectors() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_model_input_digest(&mut tables, 2, AI_DIGEST_DOMAIN_MODEL_INPUT_BYTES);
    add_chunk_binding_support(&mut tables, 2);
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 11,
        vector_space_id: 1,
        chunk_id: 2,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 2,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [54u8; 16],
            created_at_us: 941,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("same effective source already maps it to vector_ref 1")
    ));
}

#[test]
fn descriptor_bundle_accepts_distinct_bindings_sharing_model_input_and_vector() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_model_input_digest(&mut tables, 2, AI_DIGEST_DOMAIN_MODEL_INPUT_BYTES);
    add_chunk_binding_support(&mut tables, 2);
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });
    tables.chunk_vector_bindings.push(ChunkVectorBindingV1 {
        binding_id: 11,
        vector_space_id: 1,
        chunk_id: 2,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 2,
        flags: 0,
        checksum: 0,
    });

    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [55u8; 16],
        created_at_us: 942,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.chunk_vector_bindings.len(), 2);
}

#[test]
fn rejects_v1_vector_binding_when_model_input_identity_is_required() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    add_chunk_binding_support(&mut tables, 1);
    let binding = ChunkVectorBindingV1 {
        binding_id: 10,
        vector_space_id: 1,
        chunk_id: 1,
        chunk_profile_id: 1,
        source_value_hash_ref: 0,
        chunk_text_hash_ref: 0,
        vector_ref: 1,
        model_input_digest_ref: 0,
        flags: 0,
        checksum: 0,
    };
    let sections = vector_binding_artifact_sections(
        &tables,
        payload_sections,
        vec![encode_chunk_vector_binding(binding).unwrap()],
        AI_FEATURE_MODEL_INPUT_IDENTITY,
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [56u8; 16], 943, &sections)
        .unwrap();

    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("AI_FEATURE_MODEL_INPUT_IDENTITY requires ChunkVectorBinding 10")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_payload_block_dimension_mismatch() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables.vector_payload_blocks[0].dimension_count = 4;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [34u8; 16],
            created_at_us: 921,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("dimension_count does not match vector_space_id")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_payload_block_missing_stride_ref() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables.vector_payload_blocks[0].payload_stride_ref = 99;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [35u8; 16],
            created_at_us: 922,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("missing payload_stride_ref 99")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_entry_dense_length_mismatch() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables.vector_entries[0].payload_length = 8;

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [41u8; 16],
            created_at_us: 928,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("payload_length does not match dense vector width")
    ));
}

#[test]
fn descriptor_bundle_rejects_vector_entry_derived_range_out_of_block() {
    let (mut tables, payload_sections) = vector_descriptor_tables_with_payload();
    tables.vector_payload_blocks[0].vector_count = 4;
    tables.vector_entries.push(VectorEntryV1 {
        vector_ref: 4,
        block_id: 1,
        vector_ordinal: 3,
        payload_offset: 0,
        payload_length: 0,
        integrity_ref: 0,
        flags: 0,
        checksum: 0,
    });

    assert!(matches!(
        write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
            artifact_id: [42u8; 16],
            created_at_us: 929,
            payload_sections,
            descriptor_tables: tables,
        }),
        Err(CoveError::BadSection(message))
            if message.contains("derived payload range exceeds")
    ));
}

#[test]
fn writes_and_parses_filecode_vector_sidecar() {
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.5, 0.25, 0.75, 1.0] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [3u8; 16],
        created_at_us: 456,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload,
    })
    .unwrap();

    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.artifact_kind, CoveAiArtifactKind::CoveVec);
    assert_eq!(
        parsed.payload_access,
        AiPayloadAccessState::StructurallyAllowed
    );
    assert_eq!(parsed.descriptor_tables.payload_refs.len(), 2);
    assert_eq!(parsed.descriptor_tables.payload_integrity.len(), 1);
    assert_eq!(parsed.descriptor_tables.privacy_summaries.len(), 1);
    assert_eq!(parsed.descriptor_tables.vector_spaces.len(), 1);
    assert_eq!(parsed.descriptor_tables.vector_payload_blocks.len(), 1);
    assert_eq!(parsed.descriptor_tables.vector_entries.len(), 2);
    assert_eq!(parsed.descriptor_tables.filecode_vector_bindings.len(), 2);
    assert_eq!(
        parsed.descriptor_tables.filecode_vector_bindings[1].file_code,
        20
    );
}

#[test]
fn exact_flat_filecode_vector_search_returns_nearest_filecodes() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [18u8; 16],
        created_at_us: 905,
        dimension_count: 3,
        file_codes: vec![10, 20, 30],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0]),
    })
    .unwrap();

    let results = exact_flat_filecode_vector_search(&bytes, &[0.9, 0.1, 0.0], 2).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].file_code, 10);
    assert_eq!(results[0].vector_ref, 1);
    assert_eq!(results[1].file_code, 20);
    assert!(results[0].score > results[1].score);
}

#[test]
fn exact_flat_filecode_vector_search_by_file_code_uses_stored_query_vector() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [21u8; 16],
        created_at_us: 908,
        dimension_count: 3,
        file_codes: vec![10, 20, 30],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.8, 0.2, 0.0, -1.0, 0.0, 0.0]),
    })
    .unwrap();

    let results = exact_flat_filecode_vector_search_by_file_code(&bytes, 10, 3).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].file_code, 10);
    assert_eq!(results[1].file_code, 20);
    assert_eq!(results[2].file_code, 30);
    assert!(matches!(
        exact_flat_filecode_vector_search_by_file_code(&bytes, 99, 1),
        Err(CoveError::BadSection(message)) if message.contains("query FileCode 99")
    ));
}

#[test]
fn filecode_embedding_returns_validated_stored_vector() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [22u8; 16],
        created_at_us: 909,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.25, 0.5, 0.75]),
    })
    .unwrap();

    let embedding = filecode_embedding(&bytes, 20).unwrap();
    assert_eq!(embedding.file_code, 20);
    assert_eq!(embedding.vector_ref, 2);
    assert_eq!(embedding.vector_space_id, 1);
    assert_eq!(embedding.dimension_count, 3);
    assert_eq!(embedding.values, vec![0.25, 0.5, 0.75]);
}

#[test]
fn ai_embedding_supports_direct_vector_ref() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [23u8; 16],
        created_at_us: 910,
        dimension_count: 3,
        file_codes: vec![10, 20],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.25, 0.5, 0.75]),
    })
    .unwrap();

    let embedding = ai_embedding(
        &bytes,
        &AiEmbeddingRequest {
            file_code: None,
            vector_ref: Some(2),
        },
    )
    .unwrap();
    assert_eq!(embedding.target_kind, "vector_ref");
    assert_eq!(embedding.vector_ref, 2);
    assert_eq!(embedding.vector_space_id, 1);
    assert_eq!(embedding.values, vec![0.25, 0.5, 0.75]);
}

#[test]
fn ai_vector_search_reports_ann_fallback_to_exact_flat() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [24u8; 16],
        created_at_us: 911,
        dimension_count: 3,
        file_codes: vec![10, 20, 30],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.8, 0.2, 0.0, -1.0, 0.0, 0.0]),
    })
    .unwrap();

    let results = ai_vector_search(
        &bytes,
        &AiVectorSearchPlan {
            query_file_code: Some(10),
            query_vector_ref: None,
            query_values: None,
            top_k: 2,
            target_kind: AiVectorSearchTargetKind::FileCode,
            index: AiVectorIndexSelection::Hnsw,
        },
    )
    .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].file_code, Some(10));
    assert_eq!(
        results[0].selected_index,
        "hnsw_unavailable_exact_flat_fallback"
    );
    assert!(results[0].fallback_used);
    assert!(results[0].exact);
}

#[test]
fn ai_vector_search_uses_persisted_ann_index_descriptor() {
    let bytes = write_covev_filecode_vectors_with_index(
        &CoveVecFileCodeVectorBuild {
            artifact_id: [25u8; 16],
            created_at_us: 912,
            dimension_count: 3,
            file_codes: vec![10, 20, 30],
            vector_payload: f32_payload(&[1.0, 0.0, 0.0, 0.8, 0.2, 0.0, -1.0, 0.0, 0.0]),
        },
        1,
    )
    .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.vector_indexes.len(), 1);
    assert_eq!(parsed.descriptor_tables.vector_indexes[0].index_kind, 1);

    let results = ai_vector_search(
        &bytes,
        &AiVectorSearchPlan {
            query_file_code: Some(10),
            query_vector_ref: None,
            query_values: None,
            top_k: 2,
            target_kind: AiVectorSearchTargetKind::FileCode,
            index: AiVectorIndexSelection::Hnsw,
        },
    )
    .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].selected_index, "hnsw");
    assert!(!results[0].fallback_used);
    assert!(!results[0].exact);
    assert_eq!(results[0].result_authority, "ApproximateInternalAnn");
}

#[test]
fn ai_vector_search_ann_paths_generate_bounded_internal_candidates() {
    let file_codes = (1..=96).collect::<Vec<_>>();
    let mut values = Vec::new();
    for file_code in &file_codes {
        let code = *file_code as f32;
        values.extend_from_slice(&[
            code / 96.0,
            (*file_code % 7) as f32,
            (97 - *file_code) as f32 / 96.0,
            ((*file_code * *file_code) % 11) as f32,
        ]);
    }
    for (index_kind, selection, expected_name) in [
        (1, AiVectorIndexSelection::Hnsw, "hnsw"),
        (2, AiVectorIndexSelection::IvfFlat, "ivf_flat"),
        (3, AiVectorIndexSelection::IvfPq, "ivf_pq"),
        (4, AiVectorIndexSelection::DiskAnn, "diskann"),
        (5, AiVectorIndexSelection::Vamana, "vamana"),
    ] {
        let bytes = write_covev_filecode_vectors_with_index(
            &CoveVecFileCodeVectorBuild {
                artifact_id: [index_kind; 16],
                created_at_us: 1_000 + i64::from(index_kind),
                dimension_count: 4,
                file_codes: file_codes.clone(),
                vector_payload: f32_payload(&values),
            },
            index_kind,
        )
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        let vector_space = parsed.descriptor_tables.vector_spaces.first().unwrap();
        let loaded = load_vector_search_candidates(
            &bytes,
            &parsed,
            vector_space,
            AiVectorSearchTargetKind::FileCode,
        )
        .unwrap();
        let query = loaded[41].values.clone();
        let candidates =
            ann_candidate_indices(&loaded, vector_space.metric, &query, 4, index_kind).unwrap();
        assert!(
            candidates.len() >= 4 && candidates.len() < loaded.len(),
            "{expected_name} candidate_count={} loaded={}",
            candidates.len(),
            loaded.len()
        );
        let unique = candidates.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), candidates.len());

        let results = ai_vector_search(
            &bytes,
            &AiVectorSearchPlan {
                query_file_code: Some(42),
                query_vector_ref: None,
                query_values: None,
                top_k: 4,
                target_kind: AiVectorSearchTargetKind::FileCode,
                index: selection,
            },
        )
        .unwrap();
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].selected_index, expected_name);
        assert!(!results[0].fallback_used);
        assert!(!results[0].exact);
        assert_eq!(results[0].result_authority, "ApproximateInternalAnn");
    }
}

#[test]
fn covev_vec_build_options_emit_metric_and_quantized_payloads() {
    let source = f32_payload(&[-1.0, 0.0, 0.5, 0.25, 0.75, 1.0]);
    for (quantization_kind, element_type) in [(1, 3), (2, 4), (3, 4)] {
        let bytes = write_covev_filecode_vectors_with_options(
            &CoveVecFileCodeVectorBuild {
                artifact_id: [30 + quantization_kind; 16],
                created_at_us: 1_100 + i64::from(quantization_kind),
                dimension_count: 2,
                file_codes: vec![10, 20, 30],
                vector_payload: source.clone(),
            },
            CoveVecFileCodeVectorBuildOptions {
                index_kind: Some(2),
                metric: 2,
                quantization_kind,
            },
        )
        .unwrap();
        let parsed = CoveAiFile::parse(&bytes).unwrap();
        let space = &parsed.descriptor_tables.vector_spaces[0];
        let block = &parsed.descriptor_tables.vector_payload_blocks[0];
        assert_eq!(space.metric, 2);
        assert_eq!(space.quantization_policy, quantization_kind);
        assert_eq!(space.element_type, element_type);
        assert_eq!(block.quantization_kind, quantization_kind);
        assert_eq!(block.element_type, element_type);
        assert_eq!(parsed.descriptor_tables.vector_indexes[0].metric, 2);
        assert_eq!(parsed.descriptor_tables.vector_indexes[0].index_kind, 2);

        let embedding = ai_embedding(
            &bytes,
            &AiEmbeddingRequest {
                file_code: Some(10),
                vector_ref: None,
            },
        )
        .unwrap();
        assert_eq!(embedding.dimension_count, 2);
        assert_eq!(embedding.element_type, element_type);
        assert_eq!(embedding.values.len(), 2);

        let results = ai_vector_search(
            &bytes,
            &AiVectorSearchPlan {
                query_file_code: Some(10),
                query_vector_ref: None,
                query_values: None,
                top_k: 2,
                target_kind: AiVectorSearchTargetKind::FileCode,
                index: AiVectorIndexSelection::IvfFlat,
            },
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].selected_index, "ivf_flat");
        assert_eq!(results[0].result_authority, "ApproximateInternalAnn");
    }
}

#[test]
fn rejects_exact_flat_filecode_vector_search_dimension_mismatch() {
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [19u8; 16],
        created_at_us: 906,
        dimension_count: 3,
        file_codes: vec![10],
        vector_payload: f32_payload(&[1.0, 0.0, 0.0]),
    })
    .unwrap();

    assert!(matches!(
        exact_flat_filecode_vector_search(&bytes, &[1.0, 0.0], 1),
        Err(CoveError::BadSection(message))
            if message.contains("no COVE-VEC vector space matches query dimension")
    ));
}

#[test]
fn rejects_exact_flat_filecode_vector_search_when_payload_policy_blocked() {
    let sections = [coveai_payload_section(
        1,
        SectionKind::AiPayloadBytes,
        PrimaryProfile::CoveVec,
        f32_payload(&[1.0]),
    )];
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [20u8; 16], 907, &sections).unwrap();

    assert!(matches!(
        exact_flat_filecode_vector_search(&bytes, &[1.0], 1),
        Err(CoveError::BadSection(message)) if message.contains("policy-blocked")
    ));
}

#[test]
fn parses_exact_flat_vector_index_descriptor() {
    let sections = vector_index_sections(test_vector_index(0, 0));
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [16u8; 16], 904, &sections).unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.vector_spaces.len(), 1);
    assert_eq!(parsed.descriptor_tables.vector_indexes.len(), 1);
}

#[test]
fn rejects_ann_vector_index_claiming_exactness() {
    let sections = vector_index_sections(test_vector_index(1, 0));
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("must not claim exactness")
    ));
}

#[test]
fn rejects_vector_index_unsupported_index_kind() {
    let sections = vector_index_sections(test_vector_index(42, 1));
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("VectorIndex index_kind")
    ));
}

#[test]
fn rejects_vector_index_exact_false_negative_policy() {
    let mut index = test_vector_index(0, 0);
    index.false_negative_policy = 1;
    let sections = vector_index_sections(index);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("exact index must not declare false negatives")
    ));
}

#[test]
fn rejects_vector_index_unsupported_score_space_authority() {
    let mut index = test_vector_index(0, 0);
    index.score_space_authority = 42;
    let sections = vector_index_sections(index);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("VectorIndex score_space_authority")
    ));
}

#[test]
fn rejects_vector_index_missing_visibility_scope_ref() {
    let mut index = test_vector_index(0, 0);
    index.visibility_scope_ref = 99;
    let sections = vector_index_sections(index);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing visibility_scope_ref 99")
    ));
}

#[test]
fn rejects_vector_index_missing_dequantization_profile_ref() {
    let mut index = test_vector_index(0, 0);
    index.dequantization_profile_ref = 99;
    let sections = vector_index_sections(index);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveVec, [17u8; 16], 904, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing dequantization_profile_ref 99")
    ));
}

#[test]
fn parses_chunk_profile_and_text_chunks() {
    let sections = chunk_sections(vec![
        test_text_chunk(1, 0, 2, 0, 5, 5, 1),
        test_text_chunk(2, 1, 0, 5, 7, 7, 2),
    ]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.chunk_profiles.len(), 1);
    assert_eq!(parsed.descriptor_tables.text_chunks.len(), 2);
    assert_eq!(parsed.descriptor_tables.text_chunks[1].previous_chunk_id, 1);
}

#[test]
fn rejects_chunk_profile_missing_profile_name_ref() {
    let mut profile = test_chunk_profile();
    profile.profile_name_ref = 99;
    let sections = chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing profile_name_ref 99")
    ));
}

#[test]
fn rejects_chunk_profile_missing_tokenizer_profile_ref() {
    let mut profile = test_chunk_profile();
    profile.tokenizer_profile_ref = 99;
    let sections = chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing tokenizer_profile_ref 99")
    ));
}

#[test]
fn rejects_chunk_profile_unsupported_boundary_kind() {
    let mut profile = test_chunk_profile();
    profile.boundary_kind = 42;
    let sections = chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("ChunkProfile boundary_kind")
    ));
}

#[test]
fn rejects_chunk_profile_unsupported_normalization_policy() {
    let mut profile = test_chunk_profile();
    profile.normalization_policy = 42;
    let sections = chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("ChunkProfile normalization_policy")
    ));
}

#[test]
fn rejects_chunk_profile_target_tokens_outside_bounds() {
    let mut profile = test_chunk_profile();
    profile.target_tokens = 16;
    profile.max_tokens = 8;
    let sections = chunk_sections_with_profile(profile, vec![test_text_chunk(1, 0, 0, 0, 5, 5, 1)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [4u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("target_tokens exceeds max_tokens")
    ));
}

#[test]
fn rejects_text_chunk_missing_source_value_hash() {
    let sections = chunk_sections(vec![test_text_chunk(1, 0, 0, 0, 5, 5, 0)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("requires source_value_hash_ref")
    ));
}

#[test]
fn rejects_text_chunk_missing_source_ref() {
    let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
    chunk.source_ref = 99;
    let sections = chunk_sections(vec![chunk]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing source_ref 99")
    ));
}

#[test]
fn rejects_text_chunk_missing_source_value_hash_ref() {
    let sections = chunk_sections(vec![test_text_chunk(1, 0, 0, 0, 5, 5, 99)]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing source_value_hash_ref 99")
    ));
}

#[test]
fn rejects_text_chunk_missing_chunk_text_hash_ref() {
    let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
    chunk.chunk_text_hash_ref = 99;
    let sections = chunk_sections(vec![chunk]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing chunk_text_hash_ref 99")
    ));
}

#[test]
fn rejects_text_chunk_missing_evidence_ref() {
    let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
    chunk.evidence_ref = 99;
    let sections = chunk_sections(vec![chunk]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 99")
    ));
}

#[test]
fn rejects_text_chunk_missing_policy_ref() {
    let mut chunk = test_text_chunk(1, 0, 0, 0, 5, 5, 1);
    chunk.policy_ref = 99;
    let sections = chunk_sections(vec![chunk]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [5u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 99")
    ));
}

#[test]
fn rejects_text_chunk_broken_neighbor_ref() {
    let sections = chunk_sections(vec![
        test_text_chunk(1, 0, 3, 0, 5, 5, 1),
        test_text_chunk(2, 0, 0, 5, 7, 7, 2),
    ]);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [6u8; 16], 789, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing next_chunk_id")
    ));
}

#[test]
fn parses_tokenizer_span_and_sequence_pack() {
    let sections = token_sections(
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed.payload_access,
        AiPayloadAccessState::StructurallyAllowed
    );
    assert_eq!(parsed.descriptor_tables.tokenizer_profiles.len(), 1);
    assert_eq!(parsed.descriptor_tables.token_blocks.len(), 1);
    assert_eq!(parsed.descriptor_tables.tokenized_spans.len(), 1);
    assert_eq!(parsed.descriptor_tables.token_sequence_packs.len(), 1);
}

#[test]
fn rejects_tokenizer_profile_missing_namespace_ref() {
    let mut profile = test_tokenizer_profile();
    profile.tokenizer_namespace_ref = 99;
    let sections = token_sections_with_profile(
        profile,
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing tokenizer_namespace_ref 99")
    ));
}

#[test]
fn rejects_tokenizer_profile_missing_vocab_digest_ref() {
    let mut profile = test_tokenizer_profile();
    profile.vocab_digest_ref = 99;
    let sections = token_sections_with_profile(
        profile,
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing vocab_digest_ref 99")
    ));
}

#[test]
fn rejects_tokenizer_profile_missing_chat_template_ref() {
    let mut profile = test_tokenizer_profile();
    profile.chat_template_ref = 99;
    let sections = token_sections_with_profile(
        profile,
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing chat_template_ref 99")
    ));
}

#[test]
fn rejects_tokenizer_profile_missing_truncation_policy_ref() {
    let mut profile = test_tokenizer_profile();
    profile.truncation_policy_ref = 99;
    let sections = token_sections_with_profile(
        profile,
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing truncation_policy_ref 99")
    ));
}

#[test]
fn rejects_tokenizer_profile_special_token_outside_width() {
    let mut profile = test_tokenizer_profile();
    profile.eos_token_id = u32::from(u16::MAX) + 1;
    let sections = token_sections_with_profile(
        profile,
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [7u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("eos_token_id 65536 exceeds token_id_width 2")
    ));
}

#[test]
fn rejects_tokenized_span_range_out_of_block() {
    let sections = token_sections(
        test_tokenized_span(1, 0, 3, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [8u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("token range exceeds")
    ));
}

#[test]
fn rejects_token_sequence_pack_missing_mask_payload_ref() {
    let sections = token_sections(
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 99),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing loss_mask_ref")
    ));
}

#[test]
fn rejects_tokenized_span_missing_source_hash_ref() {
    let sections = token_sections(
        test_tokenized_span(1, 0, 1, 2, 99, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing source_value_hash_ref 99")
    ));
}

#[test]
fn rejects_token_sequence_pack_missing_first_source_span_ref() {
    let mut pack = test_token_sequence_pack(1, 0, 4, 0);
    pack.source_span_count = 1;
    pack.first_source_span_ref = 99;
    let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing first_source_span_ref 99")
    ));
}

#[test]
fn rejects_token_sequence_pack_missing_split_ref() {
    let mut pack = test_token_sequence_pack(1, 0, 4, 0);
    pack.split_ref = 77;
    let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing split_ref 77")
    ));
}

#[test]
fn rejects_token_sequence_pack_position_payload_too_short() {
    let mut pack = test_token_sequence_pack(1, 0, 2, 0);
    pack.position_ids_ref = 2;
    let sections = token_sections(test_tokenized_span(1, 0, 1, 2, 1, 0), pack);
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 890, &sections).unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("position_ids_ref decoded_length is shorter")
    ));
}

#[test]
fn rejects_token_block_payload_too_short() {
    let mut sections = token_sections(
        test_tokenized_span(1, 0, 1, 2, 1, 0),
        test_token_sequence_pack(1, 0, 4, 0),
    );
    sections
        .iter_mut()
        .find(|section| section.section_id == 10)
        .unwrap()
        .payload
        .truncate(6);
    sections
        .iter_mut()
        .find(|section| section.section_id == 11)
        .unwrap()
        .payload = encode_records([
        encode_digest_entry(AiDigestEntryV1 {
            digest_ref: 1,
            digest_algorithm: 1,
            digest_len: 4,
            digest_payload_ref: 2,
            domain_hint: 1,
            flags: 0,
            crc32c: 0,
        })
        .unwrap(),
        encode_payload_ref_entry(AiPayloadRefEntryV1 {
            payload_ref: 1,
            storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
            media_type_ref: 0,
            section_id: 10,
            uri_ref: 0,
            payload_offset: 0,
            section_payload_offset: 0,
            payload_length: 6,
            decoded_length: 6,
            integrity_ref: 0,
            flags: 0,
            crc32c: 0,
        })
        .unwrap(),
        encode_payload_ref_entry(AiPayloadRefEntryV1 {
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
        })
        .unwrap(),
    ])
    .unwrap();

    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [40u8; 16], 927, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("payload decoded_length is shorter")
    ));
}

#[test]
fn parses_training_profile_sample_split_and_epoch() {
    let sections = training_sections(true, test_training_sample(1, 1, 1, 1));
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [10u8; 16], 901, &sections)
        .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.training_profiles.len(), 1);
    assert_eq!(parsed.descriptor_tables.dataset_splits.len(), 1);
    assert_eq!(parsed.descriptor_tables.dedup_groups.len(), 1);
    assert_eq!(parsed.descriptor_tables.training_epoch_plans.len(), 1);
    assert_eq!(parsed.descriptor_tables.training_samples.len(), 1);
}

#[test]
fn rejects_training_sample_missing_profile() {
    let sections = training_sections(false, test_training_sample(1, 99, 1, 1));
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing training_profile_id")
    ));
}

#[test]
fn rejects_training_profile_missing_split_policy_ref() {
    let mut profile = test_training_profile();
    profile.split_policy_ref = 77;
    let sections = training_sections_with_records(
        Some(profile),
        test_dataset_split(),
        test_dedup_group(),
        test_training_epoch_plan(),
        test_training_sample(1, 1, 1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing split_policy_ref 77")
    ));
}

#[test]
fn rejects_training_sample_missing_input_ref() {
    let mut sample = test_training_sample(1, 1, 1, 1);
    sample.input_ref = 77;
    let sections = training_sections(true, sample);
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing input_ref 77")
    ));
}

#[test]
fn rejects_dataset_split_missing_filter_policy_ref() {
    let mut split = test_dataset_split();
    split.filter_policy_ref = 77;
    let sections = training_sections_with_records(
        Some(test_training_profile()),
        split,
        test_dedup_group(),
        test_training_epoch_plan(),
        test_training_sample(1, 1, 1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing filter_policy_ref 77")
    ));
}

#[test]
fn rejects_dedup_group_bad_authority() {
    let mut dedup = test_dedup_group();
    dedup.dedup_authority = 42;
    let sections = training_sections_with_records(
        Some(test_training_profile()),
        test_dataset_split(),
        dedup,
        test_training_epoch_plan(),
        test_training_sample(1, 1, 1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [11u8; 16], 901, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("DedupGroup dedup_authority")
    ));
}

#[test]
fn rejects_training_label_missing_policy_ref() {
    let mut label = test_training_label();
    label.policy_ref = 77;
    let sections = provenance_sections_with_label_pair(
        Some(test_model_actor()),
        test_decoding_profile(),
        test_human_review(),
        test_generator_provenance(1, 1),
        label,
        test_preference_pair(),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
    ));
}

#[test]
fn rejects_preference_pair_missing_evidence_ref() {
    let mut pair = test_preference_pair();
    pair.evidence_ref = 77;
    let sections = provenance_sections_with_label_pair(
        Some(test_model_actor()),
        test_decoding_profile(),
        test_human_review(),
        test_generator_provenance(1, 1),
        test_training_label(),
        pair,
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 77")
    ));
}

#[test]
fn parses_generator_provenance_labels_and_preferences() {
    let sections = provenance_sections(true, test_generator_provenance(1, 1));
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [12u8; 16], 902, &sections)
        .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.model_actors.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.generation_decoding_profiles.len(),
        1
    );
    assert_eq!(parsed.descriptor_tables.human_reviews.len(), 1);
    assert_eq!(parsed.descriptor_tables.generator_provenance.len(), 1);
    assert_eq!(parsed.descriptor_tables.training_labels.len(), 1);
    assert_eq!(parsed.descriptor_tables.preference_pairs.len(), 1);
}

#[test]
fn rejects_model_generator_missing_actor() {
    let sections = provenance_sections(false, test_generator_provenance(1, 99));
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing model_actor_ref")
    ));
}

#[test]
fn rejects_model_actor_missing_checkpoint_digest_ref() {
    let mut actor = test_model_actor();
    actor.model_checkpoint_digest_ref = 77;
    let sections = provenance_sections_with_records(
        Some(actor),
        test_decoding_profile(),
        test_human_review(),
        test_generator_provenance(1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing model_checkpoint_digest_ref 77")
    ));
}

#[test]
fn rejects_decoding_profile_missing_safety_policy_ref() {
    let mut decoding = test_decoding_profile();
    decoding.safety_policy_ref = 77;
    let sections = provenance_sections_with_records(
        Some(test_model_actor()),
        decoding,
        test_human_review(),
        test_generator_provenance(1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing safety_policy_ref 77")
    ));
}

#[test]
fn rejects_human_review_missing_notes_ref() {
    let mut review = test_human_review();
    review.notes_ref = 77;
    let sections = provenance_sections_with_records(
        Some(test_model_actor()),
        test_decoding_profile(),
        review,
        test_generator_provenance(1, 1),
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing notes_ref 77")
    ));
}

#[test]
fn rejects_generator_provenance_missing_prompt_template_ref() {
    let mut generator = test_generator_provenance(1, 1);
    generator.prompt_template_ref = 77;
    let sections = provenance_sections_with_records(
        Some(test_model_actor()),
        test_decoding_profile(),
        test_human_review(),
        generator,
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("missing prompt_template_ref 77")
    ));
}

#[test]
fn rejects_generator_provenance_bad_reproducibility_class() {
    let mut generator = test_generator_provenance(1, 1);
    generator.reproducibility_class = 42;
    let sections = provenance_sections_with_records(
        Some(test_model_actor()),
        test_decoding_profile(),
        test_human_review(),
        generator,
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [13u8; 16], 902, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("GeneratorProvenance reproducibility_class")
    ));
}

#[test]
fn parses_tensor_asset_and_multimodal_sequence() {
    let sections = phase7_sections(
        true,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [14u8; 16], 903, &sections)
        .unwrap();
    let parsed = CoveAiFile::parse(&bytes).unwrap();
    assert_eq!(parsed.descriptor_tables.tensor_layouts.len(), 1);
    assert_eq!(parsed.descriptor_tables.device_transfer_hints.len(), 1);
    assert_eq!(parsed.descriptor_tables.assets.len(), 1);
    assert_eq!(parsed.descriptor_tables.multimodal_sequence_packs.len(), 1);
    assert_eq!(
        parsed.descriptor_tables.multimodal_sequence_elements.len(),
        1
    );
}

#[test]
fn tensor_zero_copy_view_validates_layout_and_payload_lease() {
    let mut tables = AiDescriptorTablesV1::default();
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 16,
        decoded_length: 16,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 11,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 16,
        decoded_length: 16,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 3,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 12,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 16,
        decoded_length: 16,
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
    tables.tensor_layouts.push(TensorLayoutDescriptorV1 {
        tensor_layout_id: 1,
        layout_name_ref: 0,
        rank: 2,
        dtype: 0,
        byte_order: 1,
        shape_ref: 2,
        stride_ref: 3,
        storage_offset_elements: 0,
        layout_kind: 0,
        memory_alignment_bytes: 0,
        preferred_page_alignment_bytes: 0,
        tile_shape_ref: 0,
        block_shape_ref: 0,
        quantization_profile_ref: 0,
        sparsity_profile_ref: 0,
        framework_compatibility_ref: 0,
        device_affinity_hint: 0,
        flags: 0,
        checksum: 0,
    });
    let bytes = write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [21u8; 16],
        created_at_us: 904,
        payload_sections: vec![
            coveai_payload_section(
                10,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveMmseq,
                f32_payload(&[1.0, 2.0, 3.0, 4.0]),
            ),
            coveai_payload_section(
                11,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveMmseq,
                i64_payload(&[2, 2]),
            ),
            coveai_payload_section(
                12,
                SectionKind::AiPayloadBytes,
                PrimaryProfile::CoveMmseq,
                i64_payload(&[2, 1]),
            ),
        ],
        descriptor_tables: tables,
    })
    .unwrap();

    let view = ai_tensor_zero_copy_view(&bytes, 1, 1).unwrap();
    assert_eq!(view.tensor_layout_id, 1);
    assert_eq!(view.payload_ref, 1);
    assert_eq!(view.dlpack_dtype.code, 2);
    assert_eq!(view.dlpack_dtype.bits, 32);
    assert_eq!(view.shape, vec![2, 2]);
    assert_eq!(view.strides, Some(vec![2, 1]));
    assert_eq!(view.byte_offset, 0);
    assert_eq!(view.data.len(), 16);
    assert_eq!(
        view.result_authority,
        "ValidatedAiPayloadLeaseTensorZeroCopy"
    );
}

#[test]
fn rejects_tensor_layout_missing_shape_ref() {
    let mut tensor_layout = test_tensor_layout();
    tensor_layout.shape_ref = 99;
    let sections = phase7_sections_with_tensor(
        tensor_layout,
        test_device_transfer_hint(),
        true,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing shape_ref 99")
    ));
}

#[test]
fn rejects_tensor_layout_unsupported_dtype() {
    let mut tensor_layout = test_tensor_layout();
    tensor_layout.dtype = 42;
    let sections = phase7_sections_with_tensor(
        tensor_layout,
        test_device_transfer_hint(),
        true,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("TensorLayout dtype")
    ));
}

#[test]
fn rejects_device_transfer_missing_runtime_binding() {
    let mut transfer_hint = test_device_transfer_hint();
    transfer_hint.runtime_registry_binding_ref = 77;
    let sections = phase7_sections_with_tensor(
        test_tensor_layout(),
        transfer_hint,
        true,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing runtime_registry_binding_ref 77")
    ));
}

#[test]
fn rejects_uri_asset_without_uri_ref() {
    let mut asset = test_asset_ref();
    asset.asset_kind = 0;
    asset.uri_ref = 0;
    let sections = phase7_sections_with_asset(
        asset,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("URI asset requires uri_ref")
    ));
}

#[test]
fn rejects_asset_missing_policy_ref() {
    let mut asset = test_asset_ref();
    asset.policy_ref = 77;
    let sections = phase7_sections_with_asset(
        asset,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 1, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
    ));
}

#[test]
fn rejects_multimodal_element_unsupported_role() {
    let mut element = test_multimodal_element(1, 1, 0, 1, 1);
    element.role = 42;
    let sections = phase7_sections(true, test_multimodal_pack(1, 1), vec![element]);
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("MultimodalSequenceElement role")
    ));
}

#[test]
fn rejects_multimodal_element_missing_policy_ref() {
    let mut element = test_multimodal_element(1, 1, 0, 1, 1);
    element.policy_ref = 77;
    let sections = phase7_sections(true, test_multimodal_pack(1, 1), vec![element]);
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing policy_ref 77")
    ));
}

#[test]
fn rejects_multimodal_pack_missing_evidence_ref() {
    let mut pack = test_multimodal_pack(1, 1);
    pack.evidence_ref = 88;
    let sections = phase7_sections(true, pack, vec![test_multimodal_element(1, 1, 0, 1, 1)]);
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing evidence_ref 88")
    ));
}

#[test]
fn rejects_multimodal_element_missing_asset_ref() {
    let sections = phase7_sections(
        false,
        test_multimodal_pack(1, 1),
        vec![test_multimodal_element(1, 1, 0, 99, 1)],
    );
    let bytes = write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [15u8; 16], 903, &sections)
        .unwrap();
    assert!(matches!(
        CoveAiFile::parse(&bytes),
        Err(CoveError::BadSection(message)) if message.contains("missing asset_ref")
    ));
}
