use super::*;

pub(super) fn write_feature_scope_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_unknown_required_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§74", "§77", "§76"],
        ),
        cove_with_unknown_required_feature(),
    );

    write_fixture(
        root,
        entries,
        feature_scope_use_fixture(
            "feature-scope/section_entry_unknown_unneeded_accept.cove",
            "accept",
            None,
            None,
            None,
            &[],
            &[],
        ),
        cove_with_unknown_section_required_feature(),
    );
    write_fixture(
        root,
        entries,
        feature_scope_use_fixture(
            "feature-scope/section_entry_unknown_needed_reject.cove",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            None,
            None,
            &[1],
            &[],
        ),
        cove_with_unknown_section_required_feature(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_required_bad_descriptor.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§40", "§74", "§77", "§76"],
        ),
        profile_cove_file(
            FEATURE_ENGINE_PROFILE,
            0,
            SectionKind::ExecutionCodeDescriptor,
            PrimaryProfile::EngineExecution,
            FEATURE_ENGINE_PROFILE,
            0,
            invalid_execution_descriptor_payload(),
        ),
    );

    let mut lz4_missing_feature = compressed_profile_cove_file(
        FEATURE_ENGINE_PROFILE,
        FEATURE_CODEC_LZ4,
        SectionKind::ExecutionCodeDescriptor,
        PrimaryProfile::EngineExecution,
        FEATURE_ENGINE_PROFILE,
        FEATURE_CODEC_LZ4,
        CompressionCodec::Lz4,
        valid_execution_descriptor().serialize().to_vec(),
    );
    rewrite_cove_feature_bits(&mut lz4_missing_feature, FEATURE_ENGINE_PROFILE, 0);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_lz4_missing_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§66", "§73", "§76"],
        ),
        lz4_missing_feature,
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_required_bad_refs.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§39", "§40", "§41", "§42", "§43", "§73", "§76"],
        ),
        cove_e_profile_bundle_file(true, true),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_h_required_bad_hints.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§44", "§74", "§77", "§76"],
        ),
        profile_cove_file(
            FEATURE_HARBOR_PROFILE,
            0,
            SectionKind::HarborMountHints,
            PrimaryProfile::HarborExecution,
            FEATURE_HARBOR_PROFILE,
            0,
            invalid_harbor_mount_hints_payload(),
        ),
    );
}
