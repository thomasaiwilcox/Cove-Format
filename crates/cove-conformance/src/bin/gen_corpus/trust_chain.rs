use super::*;

pub(super) fn write_trust_chain_reject_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    let valid_temporal_rows = valid_temporal_rows();
    let mut bad_trust_manifest = trust_manifest_payload(5, &valid_temporal_rows);
    *bad_trust_manifest.last_mut().unwrap() ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_trust_manifest_bad_digest.cove",
            "cove",
            "reject",
            Some("COVE_E_DIGEST_MISMATCH"),
            &["§63", "§73", "§76"],
        ),
        semantic_profile_cove_file(
            PrimaryProfile::ObjectTemporal,
            FEATURE_OBJECT_PROFILE | FEATURE_TRUST_CHAIN,
            0,
            vec![
                cove_o_object_catalog_section(),
                cove_o_temporal_segment_index_section(&[(5, &valid_temporal_rows)]),
                cove_o_temporal_segment_data_section(5, &valid_temporal_rows),
                SectionPayload {
                    section_kind: SectionKind::TrustManifest as u16,
                    profile: PrimaryProfile::ObjectTemporal as u8,
                    flags: 0,
                    item_count: valid_temporal_rows.len() as u64,
                    row_count: valid_temporal_rows.len() as u64,
                    compression: 0,
                    alignment_log2: 0,
                    required_features: FEATURE_TRUST_CHAIN,
                    optional_features: 0,
                    data: bad_trust_manifest,
                },
            ],
        ),
    );
}
