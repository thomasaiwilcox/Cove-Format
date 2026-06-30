use super::*;

pub(super) fn write_validation_surface_fixtures(
    writer: &mut CorpusWriter<'_>,
    base: &gen_corpus_base::BaseFixtureBytes,
    covemap_bytes: Vec<u8>,
) {
    let root = writer.root;
    let entries = &mut *writer.entries;
    // reject/truncated_magic: clip the trailing magic bytes.
    let mut clipped = base.empty_cove.clone();
    let n = clipped.len();
    clipped.truncate(n - 4);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/truncated_magic.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_MAGIC"),
            &["§12", "§74", "§76"],
        ),
        clipped,
    );

    // reject/short_file: clearly too-short file.
    write_fixture(
        root,
        entries,
        fixture(
            "reject/short_file.cove",
            "cove",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§12", "§74", "§76"],
        ),
        b"COV".to_vec(),
    );

    // reject/header_magic_swapped: header magic bytes corrupted.
    let mut hdr_bad = base.empty_cove.clone();
    hdr_bad[0..4].copy_from_slice(b"XXXX");
    write_fixture(
        root,
        entries,
        fixture(
            "reject/header_magic_swapped.cove",
            "cove",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§9", "§10", "§74", "§76"],
        ),
        hdr_bad,
    );

    // reject/footer_crc_flipped: bit-flip inside the footer payload so the
    // postscript's footer CRC no longer matches the footer bytes.
    let mut crc_bad = base.empty_cove.clone();
    let ps = CovePostscriptV1::parse_from_tail(&base.empty_cove).unwrap();
    let footer_offset = ps.footer.offset as usize;
    crc_bad[footer_offset] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/footer_crc_flipped.cove",
            "cove",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§13", "§74", "§76"],
        ),
        crc_bad,
    );

    // reject/empty_file: zero bytes.
    write_fixture(
        root,
        entries,
        fixture(
            "reject/empty_file.cove",
            "cove",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§12", "§74", "§76"],
        ),
        Vec::new(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_bad_column_domain.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_DOMAIN"),
            &["§23", "§73", "§76"],
        ),
        cove_file_with_section(
            FEATURE_TABLE_PROFILE | FEATURE_COLUMN_DOMAINS,
            SectionKind::ColumnDomain,
            PrimaryProfile::TableScan,
            FEATURE_COLUMN_DOMAINS,
            invalid_column_domain_payload(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_bad_zone_stats.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_STATS"),
            &["§28", "§73", "§76"],
        ),
        semantic_profile_cove_file(
            PrimaryProfile::TableScan,
            FEATURE_TABLE_PROFILE,
            0,
            vec![SectionPayload {
                section_kind: SectionKind::ZoneStats as u16,
                profile: PrimaryProfile::TableScan as u8,
                flags: 0,
                item_count: 1,
                row_count: 10,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_TABLE_PROFILE,
                optional_features: 0,
                data: invalid_zone_stats_payload(),
            }],
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_duplicate_table_id.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§24", "§73", "§76"],
        ),
        cove_file_with_section(
            FEATURE_TABLE_PROFILE,
            SectionKind::TableCatalog,
            PrimaryProfile::TableScan,
            0,
            duplicate_table_catalog().serialize().unwrap(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_bool_numcode_missing_declaration.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_LOGICAL_PHYSICAL_PAIR"),
            &["§19", "§24", "§73", "§76"],
        ),
        cove_t_bool_numcode_file(false),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_bad_segment_gap.cove",
            "cove",
            "reject",
            Some("COVE_E_SEGMENT_CORRUPT"),
            &["§25", "§73", "§76"],
        ),
        cove_file_with_section(
            FEATURE_TABLE_PROFILE,
            SectionKind::TableSegmentIndex,
            PrimaryProfile::TableScan,
            0,
            gap_table_segment_index().serialize().unwrap(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_lz4_page_codec_missing_file_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§27", "§66", "§73", "§76"],
        ),
        cove_t_page_codec_missing_file_feature_file(CompressionCodec::Lz4),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_zstd_page_codec_missing_section_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§27", "§66", "§73", "§76"],
        ),
        cove_t_page_codec_missing_section_feature_file(CompressionCodec::Zstd),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_nested_missing_schema.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§24", "§52", "§73", "§76"],
        ),
        cove_t_nested_missing_schema_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_nested_mismatched_schema.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§24", "§52", "§73", "§76"],
        ),
        cove_t_nested_mismatched_schema_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_nested_list_bad_child_count.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§52", "§73", "§76"],
        ),
        cove_t_nested_list_bad_child_count_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_nested_struct_missing_null_handling.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§52", "§73", "§76"],
        ),
        cove_t_nested_struct_missing_null_handling_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_nested_map_duplicate_keys.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§52", "§73", "§76"],
        ),
        cove_t_nested_map_duplicate_keys_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/column_domain_duplicate.bin",
            "column_domain",
            "reject",
            Some("COVE_E_BAD_DOMAIN"),
            &["§23", "§76"],
        ),
        invalid_column_domain_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/table_catalog_bad_pair.bin",
            "table_catalog",
            "reject",
            Some("COVE_E_BAD_LOGICAL_PHYSICAL_PAIR"),
            &["§24", "§76"],
        ),
        bad_pair_table_catalog().serialize().unwrap(),
    );

    let mut table_catalog_trailing = valid_table_catalog().serialize().unwrap();
    table_catalog_trailing.push(0);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/table_catalog_trailing.bin",
            "table_catalog",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§24", "§76"],
        ),
        table_catalog_trailing,
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/table_segment_index_gap.bin",
            "table_segment_index",
            "reject",
            Some("COVE_E_SEGMENT_CORRUPT"),
            &["§25", "§76"],
        ),
        gap_table_segment_index().serialize().unwrap(),
    );

    let mut table_segment_index_trailing = valid_table_segment_index().serialize().unwrap();
    table_segment_index_trailing.push(0);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/table_segment_index_trailing.bin",
            "table_segment_index",
            "reject",
            Some("COVE_E_SEGMENT_CORRUPT"),
            &["§25", "§76"],
        ),
        table_segment_index_trailing,
    );

    let mut bad_segment_header = valid_table_segment_header().serialize().to_vec();
    bad_segment_header[68] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/table_segment_header_bad_crc.bin",
            "table_segment_header",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§25", "§76"],
        ),
        bad_segment_header,
    );

    let row_morsel_gap = fixture(
        "reject/row_morsel_directory_gap.bin",
        "row_morsel_directory",
        "reject",
        Some("COVE_E_SEGMENT_CORRUPT"),
        &["§26", "§76"],
    );
    write_fixture(
        root,
        entries,
        with_morsel_count(row_morsel_gap, 2),
        gap_row_morsel_directory().serialize(),
    );

    let row_morsel_nonzero_first = fixture(
        "reject/row_morsel_directory_nonzero_first.bin",
        "row_morsel_directory",
        "reject",
        Some("COVE_E_SEGMENT_CORRUPT"),
        &["§26", "§76"],
    );
    write_fixture(
        root,
        entries,
        with_morsel_count(row_morsel_nonzero_first, 1),
        nonzero_first_row_morsel_directory().serialize(),
    );

    let mut bad_sort_key = valid_sort_key().serialize().to_vec();
    bad_sort_key[4] = 9;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/sort_key_bad_direction.bin",
            "sort_key",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§53", "§76"],
        ),
        bad_sort_key,
    );

    let mut covx_bad = base.covx.clone();

    write_fixture(
        root,
        entries,
        fixture(
            "reject/row_ref_truncated.bin",
            "row_ref",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§54"],
        ),
        vec![0u8; 4],
    );
    covx_bad[82] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covx_header_crc_flipped.covx",
            "covx",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§68", "§76"],
        ),
        covx_bad,
    );

    let mut covm_bad = base.covm.clone();
    covm_bad[78] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_header_crc_flipped.covm",
            "covm",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§69", "§76"],
        ),
        covm_bad,
    );

    let mut covemap_bad = covemap_bytes;
    covemap_bad[94] ^= 0xFF;
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covemap_header_crc_flipped.covemap",
            "covemap",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§70", "§76"],
        ),
        covemap_bad,
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/metadata_json_invalid.json",
            "metadata_json",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§15", "§76"],
        ),
        b"{not-json".to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/file_dictionary_bad_utf8_len.bin",
            "file_dictionary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§16", "§17", "§76"],
        ),
        invalid_file_dictionary_bad_utf8_len_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/file_dictionary_bad_map_duplicate.bin",
            "file_dictionary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§16", "§17", "§76"],
        ),
        invalid_file_dictionary_bad_map_duplicate_payload().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/file_dictionary_redacted_null.bin",
            "file_dictionary",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§16", "§76"],
        ),
        invalid_file_dictionary_redacted_null_payload().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/collation_registry_bad_utf8.bin",
            "collation_registry",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§22", "§76"],
        ),
        collation_registry_bad_utf8_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/page_index_bad_null_count.bin",
            "page_index",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§76"],
        ),
        page_index_payload(4, 5, CoveEncodingKind::PlainFixed as u16),
    );

    let mut page_index_with_trailing_bytes =
        page_index_payload(4, 1, CoveEncodingKind::PlainFixed as u16);
    page_index_with_trailing_bytes.push(0);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/page_index_trailing_bytes.bin",
            "page_index",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27", "§76"],
        ),
        page_index_with_trailing_bytes,
    );

    let mut constant_bad_row_count = [0u8; ConstantPayload::ENCODED_LEN];
    constant_bad_row_count[0..8].copy_from_slice(&5i64.to_le_bytes());
    constant_bad_row_count[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_constant_bad_row_count.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "constant",
            "payload": constant_bad_row_count.to_vec(),
            "expect_values": []
        })),
    );

    let mut rle_zero_length = Vec::new();
    rle_zero_length.extend_from_slice(&1u32.to_le_bytes());
    rle_zero_length.extend_from_slice(&0i64.to_le_bytes());
    rle_zero_length.extend_from_slice(&0u32.to_le_bytes());
    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_rle_zero_length.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "rle",
            "payload": rle_zero_length,
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_run_end_bad_order.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "run_end",
            "payload": run_end_payload_bytes(&[1, 2], &[5, 5]),
            "expect_values": []
        })),
    );

    let plain_fixed_valid = PlainFixedPayload {
        values: vec![1, -2, 3, -4],
    }
    .encode();
    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_plain_fixed_truncated.json",
            "encoding_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "plain_fixed",
            "payload": plain_fixed_valid[..plain_fixed_valid.len() - 1].to_vec(),
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_plain_varint_truncated.json",
            "encoding_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "plain_varint",
            "payload": [0x80u8],
            "expect_values": []
        })),
    );

    let mut bit_packed_bad_width = Vec::new();
    bit_packed_bad_width.push(0u8);
    bit_packed_bad_width.extend_from_slice(&1u32.to_le_bytes());
    bit_packed_bad_width.extend_from_slice(&0u32.to_le_bytes());
    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_bit_packed_bad_width.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "bit_packed",
            "payload": bit_packed_bad_width,
            "expect_values": []
        })),
    );

    let mut delta_truncated = Vec::new();
    delta_truncated.extend_from_slice(&5i64.to_le_bytes());
    delta_truncated.extend_from_slice(&1u32.to_le_bytes());

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_delta_truncated.json",
            "encoding_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "delta",
            "payload": delta_truncated,
            "expect_values": []
        })),
    );

    let mut for_truncated = Vec::new();
    for_truncated.extend_from_slice(&7i64.to_le_bytes());
    for_truncated.extend_from_slice(&1u32.to_le_bytes());

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_for_truncated.json",
            "encoding_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "frame_of_reference",
            "payload": for_truncated,
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_patched_base_duplicate_patch.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "patched_base",
            "payload": patched_base_payload_bytes(&[0, 0], &[(1, 1), (1, 2)]),
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_sparse_out_of_range.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "sparse",
            "payload": sparse_payload_bytes(5, 0, &[(5, 1)]),
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/encoding_local_codebook_bad_local_index.json",
            "encoding_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "local_codebook",
            "payload": LocalCodebookPayload {
                values: LocalCodebookValues::FileCode(vec![42]),
                indexes: LocalIndexPayload::BitPacked(
                    BitPackedPayload::pack(&[0, 1], 1).unwrap(),
                ),
            }
            .encode(),
            "expect_values": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/nested_list_bad_child_count.json",
            "nested_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "list",
            "offsets": [0, 2, 2, 5],
            "child_row_count": 4
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/nested_struct_missing_null_handling.json",
            "nested_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "struct",
            "field_row_counts": [3, 3],
            "parent_row_count": 3,
            "parent_null_handling_declared": false
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/nested_map_duplicate_keys.json",
            "nested_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "map",
            "offsets": [0, 2],
            "key_row_count": 2,
            "value_row_count": 2,
            "keys_are_scalar": true,
            "allow_duplicate_keys": false,
            "canonical_keys": ["a", "a"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/nested_map_non_scalar_key.json",
            "nested_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "map",
            "offsets": [0, 1],
            "key_row_count": 1,
            "value_row_count": 1,
            "keys_are_scalar": false,
            "allow_duplicate_keys": false,
            "canonical_keys": ["a"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/nested_map_child_count_mismatch.json",
            "nested_case",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "map",
            "offsets": [0, 2],
            "key_row_count": 2,
            "value_row_count": 1,
            "keys_are_scalar": true,
            "allow_duplicate_keys": false,
            "canonical_keys": ["a", "b"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/digest_manifest_wrong_len.bin",
            "digest_manifest",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§65", "§76"],
        ),
        digest_manifest_wrong_len_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/digest_manifest_bad_checksum.bin",
            "digest_manifest",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§65", "§76"],
        ),
        digest_manifest_bad_checksum_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/redaction_manifest_truncated.bin",
            "redaction_manifest",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§64", "§76"],
        ),
        1u32.to_le_bytes().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/io_hints_truncated.bin",
            "io_hints",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§67", "§76"],
        ),
        vec![0; 8],
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/io_hints_legacy_12_byte_layout.bin",
            "io_hints",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§67", "§76"],
        ),
        vec![0; 12],
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/lakehouse_hints_bad_utf8.bin",
            "lakehouse_hints",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§50", "§76"],
        ),
        lakehouse_hints_bad_utf8_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/kernel_capabilities_unknown_encoding.bin",
            "kernel_capabilities",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§21", "§76"],
        ),
        kernel_capabilities_payload(0xfffe),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/kernel_capabilities_reserved.bin",
            "kernel_capabilities",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§21", "§76"],
        ),
        kernel_capabilities_reserved_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/kernel_capabilities_trailing.bin",
            "kernel_capabilities",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§21", "§76"],
        ),
        kernel_capabilities_trailing_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/kernel_capabilities_truncated.bin",
            "kernel_capabilities",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§21", "§76"],
        ),
        vec![1, 0, 0, 0, CoveEncodingKind::Rle as u8],
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/exact_set_index_unsorted.bin",
            "exact_set_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§30", "§76"],
        ),
        exact_set_index_payload(&[5, 2]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/bloom_index_zero_filter_count.bin",
            "bloom_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§31", "§76"],
        ),
        bloom_index_payload(0, 64),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/inverted_morsel_index_unsorted.bin",
            "inverted_morsel_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§32", "§76"],
        ),
        inverted_index_payload(&[7, 5]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/lookup_index_unsorted.bin",
            "lookup_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§33", "§76"],
        ),
        lookup_index_unsorted_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/aggregate_synopsis_unknown_kind.bin",
            "aggregate_synopsis",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§34", "§76"],
        ),
        aggregate_synopsis_unknown_kind_payload(),
    );

    for (path, payload) in [
        (
            "reject/aggregate_synopsis_bad_payload_bounds.bin",
            aggregate_synopsis_bad_payload_bounds(),
        ),
        (
            "reject/aggregate_synopsis_bad_payload_checksum.bin",
            aggregate_synopsis_bad_payload_checksum(),
        ),
        (
            "reject/aggregate_synopsis_wrong_kind_payload_pairing.bin",
            aggregate_synopsis_wrong_kind_payload_pairing(),
        ),
        (
            "reject/aggregate_synopsis_unsorted_histogram_keys.bin",
            aggregate_synopsis_unsorted_histogram_keys(),
        ),
        (
            "reject/aggregate_synopsis_duplicate_histogram_keys.bin",
            aggregate_synopsis_duplicate_histogram_keys(),
        ),
        (
            "reject/aggregate_synopsis_count_sum_mismatch.bin",
            aggregate_synopsis_count_sum_mismatch(),
        ),
        (
            "reject/aggregate_synopsis_invalid_canonical_value.bin",
            aggregate_synopsis_invalid_canonical_value(),
        ),
        (
            "reject/aggregate_synopsis_approximate_marked_exact.bin",
            aggregate_synopsis_approximate_marked_exact(),
        ),
        (
            "reject/aggregate_synopsis_bad_hll_header.bin",
            aggregate_synopsis_bad_hll_header(),
        ),
        (
            "reject/aggregate_synopsis_bad_kll_header.bin",
            aggregate_synopsis_bad_kll_header(),
        ),
    ] {
        write_fixture(
            root,
            entries,
            fixture(
                path,
                "aggregate_synopsis",
                "reject",
                Some("COVE_E_BAD_INDEX"),
                &["§34", "§76"],
            ),
            payload,
        );
    }

    write_fixture(
        root,
        entries,
        fixture(
            "reject/composite_zone_index_zero_key_columns.bin",
            "composite_zone_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§35", "§76"],
        ),
        composite_index_payload(0),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/topn_summary_bad_direction.bin",
            "topn_summary",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§36", "§76"],
        ),
        topn_summary_bad_direction_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_engine_registry_duplicate_namespace.bin",
            "cove_e_engine_registry",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§39", "§76"],
        ),
        engine_registry_payload(&["org.example", "org.example"]).unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_execution_code_bad_kind.bin",
            "cove_e_execution_code",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§40", "§76"],
        ),
        invalid_execution_descriptor_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_execution_scope_bad_kind.bin",
            "cove_e_execution_scope",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§41", "§76"],
        ),
        invalid_execution_scope_descriptor_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_code_space_bad_utf8.bin",
            "cove_e_code_space",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§42", "§76"],
        ),
        invalid_code_space_descriptor_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_e_mount_policy_bad_mapping.bin",
            "cove_e_mount_policy",
            "reject",
            Some("COVE_E_BAD_ENGINE_PROFILE"),
            &["§43", "§76"],
        ),
        invalid_mount_policy_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_h_mount_hints_reserved.bin",
            "cove_h_mount_hints",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§44", "§76"],
        ),
        invalid_harbor_mount_hints_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_object_catalog_duplicate_property.bin",
            "cove_o_object_catalog",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§56", "§76"],
        ),
        invalid_object_catalog().serialize().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_segment_index_bad_counts.bin",
            "cove_o_temporal_segment_index",
            "reject",
            Some("COVE_E_BAD_SCHEMA"),
            &["§57", "§76"],
        ),
        invalid_temporal_segment_index().serialize().unwrap(),
    );
}
