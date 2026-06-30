use super::*;

pub(super) fn write_object_temporal_reject_fixtures(writer: &mut CorpusWriter<'_>) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    let valid_temporal_rows = valid_temporal_rows();
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_required_bad_catalog.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§56", "§74", "§77", "§76"],
        ),
        profile_cove_file(
            FEATURE_OBJECT_PROFILE,
            0,
            SectionKind::ObjectTypeCatalog,
            PrimaryProfile::ObjectTemporal,
            FEATURE_OBJECT_PROFILE,
            0,
            invalid_object_catalog().serialize().unwrap(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_bad_order.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§58", "§73", "§76"],
        ),
        semantic_profile_cove_file(PrimaryProfile::ObjectTemporal, FEATURE_OBJECT_PROFILE, 0, {
            let bad_order_rows = [
                valid_temporal_rows[1].clone(),
                valid_temporal_rows[0].clone(),
            ];
            vec![
                cove_o_object_catalog_section(),
                cove_o_temporal_segment_index_section(&[(5, &bad_order_rows)]),
                cove_o_temporal_segment_data_section(5, &bad_order_rows),
            ]
        }),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_csn_decreases.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§58", "§73", "§76"],
        ),
        semantic_profile_cove_file(PrimaryProfile::ObjectTemporal, FEATURE_OBJECT_PROFILE, 0, {
            let mut bad_csn_rows = valid_temporal_rows.clone();
            bad_csn_rows[0].timestamp_us = 10;
            bad_csn_rows[0].csn = 100;
            bad_csn_rows[1].timestamp_us = 20;
            bad_csn_rows[1].csn = 50;
            vec![
                cove_o_object_catalog_section(),
                cove_o_temporal_segment_index_section(&[(5, &bad_csn_rows)]),
                cove_o_temporal_segment_data_section(5, &bad_csn_rows),
            ]
        }),
    );

    let mut bad_prev_rows = valid_temporal_rows.clone();
    bad_prev_rows[0].prev_ref = Some(CoveRecordRefV1 {
        segment_id: 5,
        row_index: 1,
        target_kind: 0,
    });
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_bad_prev_ref.cove",
            "cove",
            "reject",
            Some("COVE_E_REF_INVALID"),
            &["§60", "§73", "§76"],
        ),
        semantic_profile_cove_file(
            PrimaryProfile::ObjectTemporal,
            FEATURE_OBJECT_PROFILE,
            0,
            vec![
                cove_o_object_catalog_section(),
                cove_o_temporal_segment_index_section(&[(5, &bad_prev_rows)]),
                cove_o_temporal_segment_data_section(5, &bad_prev_rows),
            ],
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_property_stats_only_all_non_null_valid.cove",
            "cove",
            "accept",
            None,
            &["§27", "§61", "§76"],
        ),
        cove_o_property_stats_only_file_with_property(
            FEATURE_OBJECT_PROFILE | FEATURE_TABLE_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
            PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL,
            valid_temporal_rows.len() as u32,
            0,
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            Some(cove_o_zone_stats_payload(vec![
                cove_o_property_constant_stats(),
            ])),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_property_elision_missing_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§61", "§66", "§74", "§76"],
        ),
        cove_o_property_stats_only_file(
            FEATURE_OBJECT_PROFILE,
            PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL,
            0,
            valid_temporal_rows.len() as u32,
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_property_stats_only_all_non_null_missing_stats.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§61", "§66", "§76"],
        ),
        cove_o_property_stats_only_file(
            FEATURE_OBJECT_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
            PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL,
            valid_temporal_rows.len() as u32,
            0,
        ),
    );

    let mut wrong_scope_stats = cove_o_property_constant_stats();
    wrong_scope_stats.morsel_id = u32::MAX;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_property_stats_only_all_non_null_wrong_scope.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§28", "§61", "§76"],
        ),
        cove_o_property_stats_only_file_with_property(
            FEATURE_OBJECT_PROFILE | FEATURE_TABLE_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
            PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL,
            valid_temporal_rows.len() as u32,
            0,
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            Some(cove_o_zone_stats_payload(vec![wrong_scope_stats])),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_property_stats_only_float64_nan_stats.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_STATS"),
            &["§27", "§28", "§61", "§76"],
        ),
        cove_o_property_stats_only_file_with_property(
            FEATURE_OBJECT_PROFILE | FEATURE_TABLE_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
            PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL,
            valid_temporal_rows.len() as u32,
            0,
            CoveLogicalType::Float64,
            CovePhysicalKind::NumCode,
            Some(cove_o_float64_nan_stats_payload()),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_property_stats_only_filecode_stats.cove",
            "cove",
            "accept",
            None,
            &["§16", "§27", "§28", "§61", "§76"],
        ),
        cove_o_property_filecode_stats_only_file(),
    );
}
