use super::*;

pub(super) struct BaseFixtureBytes {
    pub(super) empty_cove: Vec<u8>,
    pub(super) covx: Vec<u8>,
    pub(super) covm: Vec<u8>,
}

pub(super) fn write_base_fixtures(writer: &mut CorpusWriter<'_>) -> BaseFixtureBytes {
    let root = writer.root;
    let entries = &mut *writer.entries;
    // accept/min_empty: structurally valid empty COVE-T file.
    let bytes = MinimalCoveWriter::write_empty_file().unwrap();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/min_empty.cove",
            "cove",
            "accept",
            None,
            &["§9", "§10", "§12", "§13", "§72.1"],
        ),
        bytes.clone(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_scan_table.cove",
            "cove",
            "accept",
            None,
            &["§24", "§25", "§26", "§27", "§72.2", "§72.3", "§73"],
        ),
        cove_t_scan_table_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_registered_stable_codec_valid.cove",
            "cove",
            "accept",
            None,
            &["§20.8", "§20.9", "§72.2", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::SupportedStable),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_registered_stable_codec_no_fallback_valid.cove",
            "cove",
            "accept",
            None,
            &["§20.8", "§20.9", "§72.2", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::SupportedStableNoFallback),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_registered_unsupported_with_fallback.cove",
            "cove",
            "accept",
            None,
            &["§20.9", "§72.2", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::UnsupportedWithFallback),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_registered_required_without_fallback.cove",
            "cove",
            "reject",
            Some("COVE_E_CODEC_UNSUPPORTED"),
            &["§20.9", "§72.2", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::UnsupportedNoFallback),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_registered_fallback_mismatch.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_CODEC_EXTENSION"),
            &["§20.8", "§20.9", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::FallbackMismatch),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_registered_malformed_envelope.cove",
            "cove",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§20.9", "§73"],
        ),
        cove_t_registered_codec_file(RegisteredFixtureKind::MalformedEnvelope),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_bool_numcode_declared.cove",
            "cove",
            "accept",
            None,
            &["§19", "§24", "§25", "§73"],
        ),
        cove_t_bool_numcode_file(true),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_bool_numcode_invalid_value.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§19", "§24", "§27.3", "§73", "§76"],
        ),
        cove_t_bool_numcode_invalid_value_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_constant_numcode_high_bits.cove",
            "cove",
            "accept",
            None,
            &["§20.3", "§73"],
        ),
        cove_t_constant_numcode_high_bits_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_plain_varint_filecode_valid.cove",
            "cove",
            "accept",
            None,
            &["§20.3", "§73"],
        ),
        cove_t_plain_varint_filecode_file(0),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_plain_varint_filecode_bad_code.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_FILECODE"),
            &["§20.3", "§73", "§76"],
        ),
        cove_t_plain_varint_filecode_file(1),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_plain_varint_bool_numcode_invalid_value.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20.3", "§73", "§76"],
        ),
        cove_t_plain_varint_bool_numcode_invalid_value_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_constant_filecode_valid.cove",
            "cove",
            "accept",
            None,
            &["§20.3", "§73"],
        ),
        cove_t_constant_filecode_file(0),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_constant_filecode_bad_code.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_FILECODE"),
            &["§20.3", "§73", "§76"],
        ),
        cove_t_constant_filecode_file(1),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_filecode_missing_dictionary.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_FILECODE"),
            &["§16", "§27.3", "§73", "§76"],
        ),
        cove_t_filecode_missing_dictionary_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_all_non_null_flag_without_elision_feature.cove",
            "cove",
            "accept",
            None,
            &["§27.2", "§72.2", "§73"],
        ),
        cove_t_all_non_null_flag_without_elision_feature_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_payload_elision_stats_only_all_null_valid.cove",
            "cove",
            "accept",
            None,
            &["§27.2", "§72.2", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_null_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_payload_elision_stats_only_all_non_null_valid.cove",
            "cove",
            "accept",
            None,
            &["§27.2", "§28", "§72.2", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_file(Some(valid_constant_page_stats())),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_payload_elision_value_stream_mixed_constant.cove",
            "cove",
            "accept",
            None,
            &["§20.6", "§27.2", "§72.2", "§73"],
        ),
        cove_t_payload_elision_value_stream_mixed_constant_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_payload_elision_value_stream_wrong_root.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20.6", "§27.2", "§73"],
        ),
        cove_t_payload_elision_value_stream_wrong_root_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_payload_elision_value_stream_missing_bitmap.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§20.6", "§27.2", "§73"],
        ),
        cove_t_payload_elision_value_stream_missing_bitmap_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_payload_elision_value_stream_missing_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§20.6", "§27.2", "§72.2"],
        ),
        cove_t_payload_elision_value_stream_missing_feature_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_payload_elision_missing_feature.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§27.2", "§72.2"],
        ),
        cove_t_payload_elision_missing_feature_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_stats_only_all_non_null_missing_stats.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27.2", "§28", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_file(None),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_stats_only_all_non_null_missing_constant_flag.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27.2", "§28", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_file(Some(constant_page_stats_with_flags(
            ZoneStatFlags::HAS_MIN_MAX,
        ))),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_stats_only_all_non_null_wrong_scope.cove",
            "cove",
            "reject",
            Some("COVE_E_BAD_STATS"),
            &["§27.2", "§28", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_file(
            Some(wrong_scope_constant_page_stats()),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_stats_only_all_non_null_float32_stats.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27.2", "§28", "§73"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_float32_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_stats_only_all_non_null_filecode_stats.cove",
            "cove",
            "accept",
            None,
            &["§16", "§27.2", "§28", "§73", "§76"],
        ),
        cove_t_payload_elision_stats_only_all_non_null_filecode_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_t_numcode_page_short_values.cove",
            "cove",
            "reject",
            Some("COVE_E_PAGE_CORRUPT"),
            &["§27.3", "§73"],
        ),
        cove_t_numcode_page_short_values_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_local_codebook_lz4.cove",
            "cove",
            "accept",
            None,
            &["§20", "§25", "§27", "§66", "§72.3"],
        ),
        cove_t_local_codebook_lz4_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_explicit_zero_null_bitmap.cove",
            "cove",
            "accept",
            None,
            &["§25", "§27", "§52", "§72.3"],
        ),
        cove_t_explicit_zero_null_bitmap_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_nested_list_valid.cove",
            "cove",
            "accept",
            None,
            &["§25", "§27", "§52", "§72.3"],
        ),
        cove_t_nested_list_valid_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_nested_struct_valid.cove",
            "cove",
            "accept",
            None,
            &["§25", "§27", "§52", "§72.3"],
        ),
        cove_t_nested_struct_valid_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_nested_map_valid.cove",
            "cove",
            "accept",
            None,
            &["§25", "§27", "§52", "§72.3"],
        ),
        cove_t_nested_map_valid_file(),
    );

    let mut parquet_accept = fixture(
        "accept/parquet_primitives_valid.parquet",
        "parquet_conversion_case",
        "accept",
        None,
        &["§24", "§25", "§27", "§51", "§72.3"],
    );
    parquet_accept["table_name"] = json!("parquet_demo");
    parquet_accept["namespace"] = json!("interop");
    parquet_accept["expected_row_count"] = json!(3u64);
    parquet_accept["expected_columns"] = json!([
        {
            "name": "active",
            "logical": "Bool",
            "physical": "Boolean",
            "values": [true, false, true]
        },
        {
            "name": "id",
            "logical": "Int64",
            "physical": "NumCode",
            "values": [10, 20, 30]
        },
        {
            "name": "score",
            "logical": "Float64",
            "physical": "NumCode",
            "values": [1.5, 2.0, 3.25]
        },
        {
            "name": "city",
            "logical": "Utf8",
            "physical": "VarBytes",
            "values": ["sea", "lon", "par"]
        },
        {
            "name": "blob",
            "logical": "Binary",
            "physical": "VarBytes",
            "values": ["6161", "6262", "6363"]
        },
        {
            "name": "event_date",
            "logical": "DateDays",
            "physical": "NumCode",
            "values": [19000, 19001, 19002]
        },
        {
            "name": "ts_us",
            "logical": "TimestampMicros",
            "physical": "NumCode",
            "values": [1000, 2000, 3000]
        }
    ]);
    write_fixture(
        root,
        entries,
        parquet_accept,
        parquet_primitives_valid_file(),
    );

    let mut parquet_wrong_expectation = fixture(
        "reject/parquet_primitives_wrong_expectation.parquet",
        "parquet_conversion_case",
        "reject",
        Some("COVE_E_BAD_SECTION"),
        &["§51", "§76"],
    );
    parquet_wrong_expectation["table_name"] = json!("parquet_demo");
    parquet_wrong_expectation["namespace"] = json!("interop");
    parquet_wrong_expectation["expected_row_count"] = json!(4u64);
    write_fixture(
        root,
        entries,
        parquet_wrong_expectation,
        parquet_primitives_valid_file(),
    );

    let mut parquet_nullable = fixture(
        "accept/parquet_nullable_valid.parquet",
        "parquet_conversion_case",
        "accept",
        None,
        &["§6.6", "§24", "§25", "§27", "§51", "§72.3"],
    );
    parquet_nullable["table_name"] = json!("parquet_nullable");
    parquet_nullable["namespace"] = json!("interop");
    parquet_nullable["expected_row_count"] = json!(3u64);
    parquet_nullable["expected_columns"] = json!([
        {
            "name": "id",
            "logical": "Int64",
            "physical": "NumCode",
            "values": [1, null, 3]
        }
    ]);
    write_fixture(
        root,
        entries,
        parquet_nullable,
        parquet_nullable_valid_file(),
    );

    let mut parquet_nested = fixture(
        "accept/parquet_nested_json_fallback.parquet",
        "parquet_conversion_case",
        "accept",
        None,
        &["§51", "§52", "§72.3"],
    );
    parquet_nested["table_name"] = json!("parquet_nested_json");
    parquet_nested["namespace"] = json!("interop");
    parquet_nested["expected_row_count"] = json!(2u64);
    parquet_nested["expected_columns"] = json!([
        {
            "name": "times",
            "logical": "Json",
            "physical": "VarBytes",
            "values": [
                [
                    "unsupported Arrow JSON fallback value for Time32(Millisecond): PrimitiveArray<Time32(ms)>\n[\n  00:00:01,\n]",
                    "unsupported Arrow JSON fallback value for Time32(Millisecond): PrimitiveArray<Time32(ms)>\n[\n  00:00:02,\n]"
                ],
                null
            ]
        }
    ]);
    write_fixture(
        root,
        entries,
        parquet_nested,
        parquet_nested_unsupported_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/column_domain_valid.bin",
            "column_domain",
            "accept",
            None,
            &["§23"],
        ),
        valid_column_domain_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/table_catalog_valid.bin",
            "table_catalog",
            "accept",
            None,
            &["§24"],
        ),
        valid_table_catalog().serialize().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/table_segment_index_valid.bin",
            "table_segment_index",
            "accept",
            None,
            &["§25"],
        ),
        valid_table_segment_index().serialize().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/table_segment_header_valid.bin",
            "table_segment_header",
            "accept",
            None,
            &["§25"],
        ),
        valid_table_segment_header().serialize().to_vec(),
    );

    let row_morsel_valid = fixture(
        "accept/row_morsel_directory_valid.bin",
        "row_morsel_directory",
        "accept",
        None,
        &["§26"],
    );
    write_fixture(
        root,
        entries,
        with_morsel_count(row_morsel_valid, 2),
        valid_row_morsel_directory().serialize(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/sort_key_valid.bin",
            "sort_key",
            "accept",
            None,
            &["§53"],
        ),
        valid_sort_key().serialize().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/clustering_key_valid.bin",
            "clustering_key",
            "accept",
            None,
            &["§53"],
        ),
        valid_clustering_key().serialize().to_vec(),
    );

    let mut intermediate_clustering_key = valid_clustering_key();
    intermediate_clustering_key.clustering_strength = ClusteringStrength(9);
    write_fixture(
        root,
        entries,
        fixture(
            "accept/clustering_key_intermediate_strength.bin",
            "clustering_key",
            "accept",
            None,
            &["§53"],
        ),
        intermediate_clustering_key.serialize().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/row_ref_valid.bin",
            "row_ref",
            "accept",
            None,
            &["§54"],
        ),
        RowRef {
            table_id: 1,
            segment_id: 2,
            morsel_id: 3,
            row_in_morsel: 4,
        }
        .encode()
        .to_vec(),
    );

    let covx_bytes = valid_covx_file();
    write_fixture(
        root,
        entries,
        fixture("accept/covx_valid.covx", "covx", "accept", None, &["§68"]),
        covx_bytes.clone(),
    );

    let covm_bytes = valid_covm_file();
    write_fixture(
        root,
        entries,
        fixture("accept/covm_valid.covm", "covm", "accept", None, &["§69"]),
        covm_bytes.clone(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covm_delta_required_non_delta_reader.covm",
            "covm",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§69", "§76"],
        ),
        covm_delta_required_non_delta_reader_file(),
    );

    BaseFixtureBytes {
        empty_cove: bytes,
        covx: covx_bytes,
        covm: covm_bytes,
    }
}
