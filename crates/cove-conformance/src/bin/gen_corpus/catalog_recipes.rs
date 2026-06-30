use super::*;

pub(super) fn write_catalog_and_runtime_fixtures(writer: &mut CorpusWriter<'_>) -> Vec<u8> {
    let root = writer.root;
    let entries = &mut *writer.entries;
    let covemap_bytes = valid_covemap_file();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/covemap_valid.covemap",
            "covemap",
            "accept",
            None,
            &["§70"],
        ),
        covemap_bytes.clone(),
    );

    let mut covemap_unknown_required = covemap_bytes.clone();
    rewrite_covemap_feature_bits(
        &mut covemap_unknown_required,
        FEATURE_SEMANTIC_MAP | (1u64 << 63),
        0,
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covemap_unknown_required_feature.covemap",
            "covemap",
            "reject",
            Some("COVE_E_UNKNOWN_REQUIRED_FEATURE"),
            &["§70", "§74", "§77", "§76"],
        ),
        covemap_unknown_required,
    );

    let mut covemap_missing_semantic_map = covemap_bytes.clone();
    rewrite_covemap_feature_bits(&mut covemap_missing_semantic_map, 0, 0);
    write_fixture(
        root,
        entries,
        fixture(
            "reject/covemap_missing_semantic_map_feature.covemap",
            "covemap",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§11", "§70", "§76"],
        ),
        covemap_missing_semantic_map,
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/covemap_lz4_missing_feature.covemap",
            "covemap",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§66", "§70", "§76"],
        ),
        covemap_lz4_missing_feature_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/metadata_json_valid.json",
            "metadata_json",
            "accept",
            None,
            &["§15"],
        ),
        br#"{"producer":"cove-conformance","purpose":"metadata fixture"}"#.to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_constant_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "constant",
            "payload": ConstantPayload { value: -42, row_count: 5 }.encode().to_vec(),
            "expect_values": [-42, -42, -42, -42, -42]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_rle_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "rle",
            "payload": RlePayload { runs: vec![(1, 3), (2, 1), (1, 2)] }.encode(),
            "expect_values": [1, 1, 1, 2, 1, 1]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_run_end_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "run_end",
            "payload": run_end_payload_bytes(&[10, 20, 30], &[2, 5, 6]),
            "expect_values": [10, 10, 20, 20, 20, 30]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_plain_fixed_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "plain_fixed",
            "payload": PlainFixedPayload { values: vec![1, -2, 3, -4] }.encode(),
            "expect_values": [1, -2, 3, -4]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_plain_varint_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "plain_varint",
            "payload": PlainVarintPayload { values: vec![0, 1, 2, 127, 128] }.encode(),
            "expect_values": [0, 1, 2, 127, 128]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_bit_packed_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "bit_packed",
            "payload": BitPackedPayload::pack(&[0, 1, 2, 3, 4, 5, 6, 7, 0, 7, 4], 3).unwrap().encode(),
            "expect_values": [0, 1, 2, 3, 4, 5, 6, 7, 0, 7, 4]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_delta_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "delta",
            "payload": DeltaPayload { base: 100, deltas: vec![1, 2, -3, 5] }.encode(),
            "expect_values": [100, 101, 103, 100, 105]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_frame_of_reference_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "frame_of_reference",
            "payload": ForPayload { reference: 1_000_000, offsets: vec![0, 1, -2, 3, 4] }.encode(),
            "expect_values": [1_000_000, 1_000_001, 999_998, 1_000_003, 1_000_004]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_patched_base_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "patched_base",
            "payload": patched_base_payload_bytes(&[0, 0, 0, 0], &[(1, 10), (3, 20)]),
            "expect_values": [0, 10, 0, 20]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_sparse_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "sparse",
            "payload": sparse_payload_bytes(5, 0, &[(1, 42), (4, -7)]),
            "expect_values": [0, 42, 0, 0, -7]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_local_codebook_bit_packed_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "local_codebook",
            "payload": LocalCodebookPayload {
                values: LocalCodebookValues::FileCode(vec![100, 200, 300]),
                indexes: LocalIndexPayload::BitPacked(
                    BitPackedPayload::pack(&[0, 1, 2, 1, 0], 2).unwrap(),
                ),
            }
            .encode(),
            "expect_values": [100, 200, 300, 200, 100]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoding_local_codebook_rle_valid.json",
            "encoding_case",
            "accept",
            None,
            &["§20"],
        ),
        encoding_fixture_bytes(json!({
            "encoding": "local_codebook",
            "payload": LocalCodebookPayload {
                values: LocalCodebookValues::NumCode(vec![7, 9]),
                indexes: LocalIndexPayload::Rle(RlePayload {
                    runs: vec![(0, 3), (1, 1), (0, 2)],
                }),
            }
            .encode(),
            "expect_values": [7, 7, 7, 9, 7, 7]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoded_array_decode_rle_valid.json",
            "encoded_array_decode_case",
            "accept",
            None,
            &["§20", "§72.3"],
        ),
        encoding_fixture_bytes(json!({
            "logical": "Int64",
            "physical": "FixedBytes",
            "encoding": "Rle",
            "row_count": 4,
            "payload": RlePayload { runs: vec![(-2, 2), (9, 2)] }.encode(),
            "expect": [-2, -2, 9, 9]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/encoded_array_decode_local_codebook_varbytes_valid.json",
            "encoded_array_decode_case",
            "accept",
            None,
            &["§20", "§72.3"],
        ),
        encoding_fixture_bytes(json!({
            "logical": "Utf8",
            "physical": "VarBytes",
            "encoding": "LocalCodebook",
            "row_count": 3,
            "payload": LocalCodebookPayload {
                values: LocalCodebookValues::VarBytes(vec![b"red".to_vec(), b"blue".to_vec()]),
                indexes: LocalIndexPayload::Rle(RlePayload {
                    runs: vec![(0, 1), (1, 2)],
                }),
            }
            .encode(),
            "expect": ["red", "blue", "blue"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/arrow_export_utf8_valid.json",
            "arrow_export_case",
            "accept",
            None,
            &["§49", "§20", "§72.3"],
        ),
        encoding_fixture_bytes(json!({
            "logical": "Utf8",
            "physical": "VarBytes",
            "encoding": "VarBytes",
            "row_count": 2,
            "payload": varbytes_payload(&[b"hi".as_ref(), b"there".as_ref()]),
            "expect_type": "Utf8",
            "expect": ["hi", "there"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/arrow_export_json_requires_report.json",
            "arrow_export_case",
            "reject",
            None,
            &["§49", "§20", "§76"],
        ),
        encoding_fixture_bytes(json!({
            "logical": "Json",
            "physical": "VarBytes",
            "encoding": "VarBytes",
            "row_count": 1,
            "payload": varbytes_payload(&[br#"{"a":1}"#.as_ref()]),
            "expect_type": "Utf8",
            "expect": ["{\"a\":1}"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/nested_list_valid.json",
            "nested_case",
            "accept",
            None,
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "list",
            "offsets": [0, 2, 2, 5],
            "child_row_count": 5
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/nested_struct_valid.json",
            "nested_case",
            "accept",
            None,
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "struct",
            "field_row_counts": [3, 3, 3],
            "parent_row_count": 3,
            "parent_null_handling_declared": true
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/nested_map_valid.json",
            "nested_case",
            "accept",
            None,
            &["§52"],
        ),
        nested_fixture_bytes(json!({
            "layout": "map",
            "offsets": [0, 2, 3],
            "key_row_count": 3,
            "value_row_count": 3,
            "keys_are_scalar": true,
            "allow_duplicate_keys": false,
            "canonical_keys": ["a", "b", "a"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/file_dictionary_valid.bin",
            "file_dictionary",
            "accept",
            None,
            &["§16", "§17"],
        ),
        valid_file_dictionary_fixture_payload().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/collation_registry_valid.bin",
            "collation_registry",
            "accept",
            None,
            &["§22"],
        ),
        collation_registry_payload(&[(1, "utf8-bytewise", ""), (3, "signed-numeric", "")]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_index_valid.bin",
            "page_index",
            "accept",
            None,
            &["§27"],
        ),
        page_index_payload(4, 1, CoveEncodingKind::PlainFixed as u16),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_t_zone_stats_valid.cove",
            "cove",
            "accept",
            None,
            &["§28", "§73"],
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
                data: valid_zone_stats_payload(),
            }],
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_null_is_null_all.json",
            "pruning_case",
            "accept",
            None,
            &["§37.4"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "zone_stats": {
                        "row_count": 10,
                        "null_count": 10
                    }
                }
            ],
            "predicate": {
                "op": "is_null",
                "column_id": 7
            },
            "expect_outcome": "all_match",
            "expect_evidence": ["ZoneStats"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_file_code_eq_exact_set_no.json",
            "pruning_case",
            "accept",
            None,
            &["§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "exact_set": {
                        "keys": [1, 4, 7]
                    }
                }
            ],
            "predicate": {
                "op": "file_code_eq",
                "column_id": 7,
                "file_code": 3
            },
            "expect_outcome": "no_match",
            "expect_evidence": ["ExactSet"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_file_code_eq_constant_yes.json",
            "pruning_case",
            "accept",
            None,
            &["§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "zone_stats": {
                        "row_count": 5,
                        "null_count": 0,
                        "flags": ["has_domain_range", "constant"],
                        "min_domain_rank": 1,
                        "max_domain_rank": 1
                    },
                    "column_domain": {
                        "sorted_file_codes": [1, 3, 4, 7],
                        "dictionary_entry_count": 8
                    }
                }
            ],
            "predicate": {
                "op": "file_code_eq",
                "column_id": 7,
                "file_code": 3
            },
            "expect_outcome": "all_match",
            "expect_evidence": ["ColumnDomain"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_domain_rank_range_overlap.json",
            "pruning_case",
            "accept",
            None,
            &["§37.2"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_domain_range"],
                        "min_domain_rank": 1,
                        "max_domain_rank": 2
                    },
                    "column_domain": {
                        "sorted_file_codes": [1, 3, 4, 7],
                        "dictionary_entry_count": 8
                    }
                }
            ],
            "predicate": {
                "op": "domain_rank_range",
                "column_id": 7,
                "min_rank": 2,
                "max_rank": 3
            },
            "expect_outcome": "some_match",
            "expect_evidence": ["ColumnDomain"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_domain_rank_range_unsafe_domain.json",
            "pruning_case",
            "accept",
            None,
            &["§37.2", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_domain_range"],
                        "min_domain_rank": 1,
                        "max_domain_rank": 2
                    },
                    "column_domain": {
                        "sorted_file_codes": [1, 3, 4, 7],
                        "dictionary_entry_count": 8,
                        "safe": false
                    }
                }
            ],
            "predicate": {
                "op": "domain_rank_range",
                "column_id": 7,
                "min_rank": 1,
                "max_rank": 2
            },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_truth_table_and.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.2", "§37.4"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "zone_stats": {
                        "row_count": 10,
                        "null_count": 0
                    }
                },
                {
                    "column_id": 8,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_domain_range"],
                        "min_domain_rank": 1,
                        "max_domain_rank": 2
                    },
                    "column_domain": {
                        "sorted_file_codes": [1, 3, 4, 7],
                        "dictionary_entry_count": 8
                    }
                }
            ],
            "predicate": {
                "op": "and",
                "operands": [
                    {
                        "op": "is_not_null",
                        "column_id": 7
                    },
                    {
                        "op": "domain_rank_range",
                        "column_id": 8,
                        "min_rank": 2,
                        "max_rank": 3
                    }
                ]
            },
            "expect_outcome": "some_match",
            "expect_evidence": ["ZoneStats", "ColumnDomain"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_truth_table_or.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.1", "§37.4"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "exact_set": {
                        "keys": [1, 4, 7]
                    }
                },
                {
                    "column_id": 8,
                    "zone_stats": {
                        "row_count": 6,
                        "null_count": 2
                    }
                }
            ],
            "predicate": {
                "op": "or",
                "operands": [
                    {
                        "op": "file_code_eq",
                        "column_id": 7,
                        "file_code": 3
                    },
                    {
                        "op": "is_null",
                        "column_id": 8
                    }
                ]
            },
            "expect_outcome": "some_match",
            "expect_evidence": ["ExactSet", "ZoneStats"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_truth_table_not.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 7,
                    "exact_set": {
                        "keys": [1, 4, 7]
                    }
                }
            ],
            "predicate": {
                "op": "not",
                "operand": {
                    "op": "file_code_eq",
                    "column_id": 7,
                    "file_code": 3
                }
            },
            "expect_outcome": "all_match",
            "expect_evidence": ["ExactSet"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_numcode_range_all.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.3"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 9,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_min_max"],
                        "min": { "kind": "int64", "value": 22 },
                        "max": { "kind": "int64", "value": 51 }
                    }
                }
            ],
            "predicate": {
                "op": "numcode_range",
                "column_id": 9,
                "lower": { "kind": "int64", "value": 18 },
                "upper": { "kind": "int64", "value": 65 }
            },
            "expect_outcome": "all_match",
            "expect_evidence": ["ZoneStats"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_numcode_range_no.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.3"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 9,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_min_max"],
                        "min": { "kind": "int64", "value": 22 },
                        "max": { "kind": "int64", "value": 51 }
                    }
                }
            ],
            "predicate": {
                "op": "numcode_range",
                "column_id": 9,
                "lower": { "kind": "int64", "value": 70 },
                "upper": { "kind": "int64", "value": 90 }
            },
            "expect_outcome": "no_match",
            "expect_evidence": ["ZoneStats"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_numcode_range_overlap.json",
            "pruning_case",
            "accept",
            None,
            &["§29", "§37.3"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 9,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_min_max"],
                        "min": { "kind": "int64", "value": 22 },
                        "max": { "kind": "int64", "value": 51 }
                    }
                }
            ],
            "predicate": {
                "op": "numcode_range",
                "column_id": 9,
                "lower": { "kind": "int64", "value": 40 },
                "upper": { "kind": "int64", "value": 90 }
            },
            "expect_outcome": "some_match",
            "expect_evidence": ["ZoneStats"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_numcode_range_nan_unknown.json",
            "pruning_case",
            "accept",
            None,
            &["§28", "§37.3"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 9,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_min_max", "has_nan"],
                        "min": { "kind": "float64", "value": 1.0 },
                        "max": { "kind": "float64", "value": 2.0 }
                    }
                }
            ],
            "predicate": {
                "op": "numcode_range",
                "column_id": 9,
                "lower": { "kind": "float64", "value": 0.0 },
                "upper": { "kind": "float64", "value": 3.0 }
            },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_numcode_range_truncated_unknown.json",
            "pruning_case",
            "accept",
            None,
            &["§28", "§37.3"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 9,
                    "zone_stats": {
                        "row_count": 8,
                        "null_count": 0,
                        "flags": ["has_min_max", "minmax_truncated"],
                        "min": { "kind": "int64", "value": 1 },
                        "max": { "kind": "int64", "value": 2 }
                    }
                }
            ],
            "predicate": {
                "op": "numcode_range",
                "column_id": 9,
                "lower": { "kind": "int64", "value": 0 },
                "upper": { "kind": "int64", "value": 3 }
            },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_bloom_membership_no.json",
            "pruning_case",
            "accept",
            None,
            &["§31", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 11,
                    "bloom": { "values": ["alpha", "beta", "gamma"], "bit_count": 64 }
                }
            ],
            "predicate": { "op": "bloom_membership", "column_id": 11, "value": "delta" },
            "expect_outcome": "no_match",
            "expect_evidence": ["BloomFilter"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_bloom_membership_fallback.json",
            "pruning_case",
            "accept",
            None,
            &["§31", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 11,
                    "bloom": { "values": ["alpha"], "bit_count": 64, "fail_open": true }
                }
            ],
            "predicate": { "op": "bloom_membership", "column_id": 11, "value": "alpha" },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_inverted_lookup_no.json",
            "pruning_case",
            "accept",
            None,
            &["§32", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 12, "inverted": { "keys": [3, 5, 7] } }
            ],
            "predicate": { "op": "inverted_lookup", "column_id": 12, "key": 4 },
            "expect_outcome": "no_match",
            "expect_evidence": ["InvertedIndex"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_inverted_lookup_fallback.json",
            "pruning_case",
            "accept",
            None,
            &["§32", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 12, "inverted": { "keys": [3], "fail_open": true } }
            ],
            "predicate": { "op": "inverted_lookup", "column_id": 12, "key": 3 },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_lookup_point_no.json",
            "pruning_case",
            "accept",
            None,
            &["§33", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 13, "lookup": { "keys": [10, 20, 30] } }
            ],
            "predicate": { "op": "lookup_point", "column_id": 13, "key": 15 },
            "expect_outcome": "no_match",
            "expect_evidence": ["InvertedIndex"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_lookup_point_fallback.json",
            "pruning_case",
            "accept",
            None,
            &["§33", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 13, "lookup": { "keys": [10], "fail_open": true } }
            ],
            "predicate": { "op": "lookup_point", "column_id": 13, "key": 10 },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_aggregate_synopsis_no.json",
            "pruning_case",
            "accept",
            None,
            &["§34", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 14, "aggregate": { "proves_no_match": true } }
            ],
            "predicate": { "op": "aggregate_synopsis", "column_id": 14 },
            "expect_outcome": "no_match",
            "expect_evidence": ["AggregateSynopsis"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_aggregate_synopsis_fallback.json",
            "pruning_case",
            "accept",
            None,
            &["§34", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 14, "aggregate": { "proves_no_match": true, "fail_open": true } }
            ],
            "predicate": { "op": "aggregate_synopsis", "column_id": 14 },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_composite_zone_no.json",
            "pruning_case",
            "accept",
            None,
            &["§35", "§37.1"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 15, "composite": { "matches_bindings": false } }
            ],
            "predicate": { "op": "composite_zone", "column_id": 15 },
            "expect_outcome": "no_match",
            "expect_evidence": ["CompositeIndex"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_composite_zone_fallback.json",
            "pruning_case",
            "accept",
            None,
            &["§35", "§73"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                { "column_id": 15, "composite": { "matches_bindings": true, "fail_open": true } }
            ],
            "predicate": { "op": "composite_zone", "column_id": 15 },
            "expect_outcome": "unknown",
            "expect_evidence": ["FallbackToScan"]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_reorder_invariant_and.json",
            "pruning_case",
            "accept",
            None,
            &["§37.5"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 21,
                    "zone_stats": { "row_count": 4, "null_count": 0, "flags": [] }
                },
                {
                    "column_id": 22,
                    "zone_stats": {
                        "row_count": 4,
                        "null_count": 0,
                        "flags": ["has_min_max"],
                        "min": { "kind": "int64", "value": 10 },
                        "max": { "kind": "int64", "value": 20 }
                    }
                },
                {
                    "column_id": 23,
                    "exact_set": { "keys": [1, 2, 3] }
                }
            ],
            "predicate": {
                "op": "reorder_invariant_and",
                "operands": [
                    { "op": "is_not_null", "column_id": 21 },
                    {
                        "op": "numcode_range",
                        "column_id": 22,
                        "lower": { "kind": "int64", "value": 8 },
                        "upper": { "kind": "int64", "value": 25 }
                    },
                    { "op": "file_code_eq", "column_id": 23, "file_code": 7 }
                ]
            },
            "expect_outcome": "no_match"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/pruning_reorder_invariant_or.json",
            "pruning_case",
            "accept",
            None,
            &["§37.5"],
        ),
        pruning_fixture_bytes(json!({
            "columns": [
                {
                    "column_id": 31,
                    "zone_stats": { "row_count": 6, "null_count": 0, "flags": [] }
                },
                {
                    "column_id": 32,
                    "zone_stats": { "row_count": 6, "null_count": 6, "flags": [] }
                },
                {
                    "column_id": 33,
                    "exact_set": { "keys": [1, 2, 3] }
                }
            ],
            "predicate": {
                "op": "reorder_invariant_or",
                "operands": [
                    { "op": "is_not_null", "column_id": 31 },
                    { "op": "is_null", "column_id": 32 },
                    { "op": "file_code_eq", "column_id": 33, "file_code": 99 }
                ]
            },
            "expect_outcome": "all_match"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_none_round_trip.json",
            "page_codec_case",
            "accept",
            None,
            &["§27", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "uncompressed page bytes",
            "expect": "round_trip"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_lz4_round_trip.json",
            "page_codec_case",
            "accept",
            None,
            &["§27", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "lz4",
            "payload": "Cove page-level LZ4 round trip Cove page-level LZ4 round trip",
            "expect": "round_trip"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_zstd_round_trip.json",
            "page_codec_case",
            "accept",
            None,
            &["§27", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "zstd",
            "payload": "Cove page-level Zstd round trip Cove page-level Zstd round trip",
            "expect": "round_trip"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_none_length_mismatch_rejected.json",
            "page_codec_case",
            "accept",
            None,
            &["§13.2", "§27.2", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "abcdef",
            // codec=None requires uncompressed_length == page_length.
            "uncompressed_length_override": 99,
            "expect": "parse_reject"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_unknown_codec_rejected.json",
            "page_codec_case",
            "accept",
            None,
            &["§27.2", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "abcdef",
            // 0xFF is not a known CompressionCodec value.
            "flags_override": 0xFFu64,
            "expect": "parse_reject"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_reserved_flag_bits_rejected.json",
            "page_codec_case",
            "accept",
            None,
            &["§27.2", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "abcdef",
            // Reserved bits above the codec byte must be zero in the v2 page format.
            "flags_override": 0x0000_1000u64,
            "expect": "parse_reject"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_stats_only_constant_all_null_round_trip.json",
            "page_codec_case",
            "accept",
            None,
            &["§27.2", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "",
            "flags_override": 0x0000_0300u64,
            "row_count_override": 1u64,
            "non_null_count_override": 0u64,
            "null_count_override": 1u64,
            "encoding_root_override": 0xFFFF_FFFFu64,
            "page_offset_override": 0u64,
            "page_length_override": 0u64,
            "uncompressed_length_override": 0u64,
            "expect": "round_trip"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_stats_only_constant_requires_empty_payload.json",
            "page_codec_case",
            "accept",
            None,
            &["§27.2", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "none",
            "payload": "abcdef",
            "flags_override": 0x0000_0300u64,
            "row_count_override": 1u64,
            "non_null_count_override": 0u64,
            "null_count_override": 1u64,
            "encoding_root_override": 0xFFFF_FFFFu64,
            "page_offset_override": 0u64,
            "expect": "parse_reject"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/page_codec_lz4_truncated_rejected.json",
            "page_codec_case",
            "accept",
            None,
            &["§27", "§66"],
        ),
        page_codec_fixture_bytes(json!({
            "codec": "lz4",
            "payload": "Cove page-level LZ4 corruption sentinel sentinel sentinel",
            "truncate_wire_bytes": 4,
            "expect": "decode_reject"
        })),
    );

    // §8 — wire-format primitives (varint LEB128, ZigZag, strict bool).
    let wire_fixtures: Vec<(&str, Value, Vec<&str>)> = vec![
        (
            "accept/wire_varint_zero.json",
            json!({ "op": "varint_round_trip", "value": 0u64, "expect_bytes": [0u8] }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_127.json",
            json!({ "op": "varint_round_trip", "value": 127u64, "expect_bytes": [0x7fu8] }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_128.json",
            json!({ "op": "varint_round_trip", "value": 128u64, "expect_bytes": [0x80u8, 0x01u8] }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_u32_max.json",
            json!({
                "op": "varint_round_trip",
                "value": 0xFFFF_FFFFu64,
                "expect_bytes": [0xffu8, 0xffu8, 0xffu8, 0xffu8, 0x0fu8]
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_u64_max.json",
            json!({
                "op": "varint_round_trip",
                "value": u64::MAX,
                "expect_bytes": [0xffu8, 0xffu8, 0xffu8, 0xffu8, 0xffu8, 0xffu8, 0xffu8, 0xffu8, 0xffu8, 0x01u8]
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_truncated_rejected.json",
            json!({
                "op": "varint_decode_reject",
                "input": [0x80u8],
                "reason": "continuation bit set but no following byte"
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_overlong_rejected.json",
            json!({
                "op": "varint_decode_reject",
                "input": [0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x01u8],
                "reason": "11-byte varint overflows u64"
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_varint_10byte_overflow_rejected.json",
            json!({
                "op": "varint_decode_reject",
                "input": [0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x80u8, 0x02u8],
                "reason": "10th-byte high bits would shift past bit 63"
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_zigzag_zero.json",
            json!({ "op": "zigzag_round_trip", "value": 0i64, "expect_zigzag": 0u64 }),
            vec!["§8"],
        ),
        (
            "accept/wire_zigzag_negative_one.json",
            json!({ "op": "zigzag_round_trip", "value": -1i64, "expect_zigzag": 1u64 }),
            vec!["§8"],
        ),
        (
            "accept/wire_zigzag_positive_one.json",
            json!({ "op": "zigzag_round_trip", "value": 1i64, "expect_zigzag": 2u64 }),
            vec!["§8"],
        ),
        (
            "accept/wire_zigzag_i64_min.json",
            json!({ "op": "zigzag_round_trip", "value": i64::MIN, "expect_zigzag": u64::MAX }),
            vec!["§8"],
        ),
        (
            "accept/wire_zigzag_i64_max.json",
            json!({
                "op": "zigzag_round_trip",
                "value": i64::MAX,
                "expect_zigzag": (u64::MAX - 1)
            }),
            vec!["§8"],
        ),
        (
            "accept/wire_bool_strict_false.json",
            json!({ "op": "bool_strict", "byte": 0u8, "expect": false }),
            vec!["§8"],
        ),
        (
            "accept/wire_bool_strict_true.json",
            json!({ "op": "bool_strict", "byte": 1u8, "expect": true }),
            vec!["§8"],
        ),
        (
            "accept/wire_bool_strict_two_rejected.json",
            json!({ "op": "bool_strict_reject", "byte": 2u8 }),
            vec!["§8"],
        ),
        (
            "accept/wire_bool_strict_high_bit_rejected.json",
            json!({ "op": "bool_strict_reject", "byte": 0xffu8 }),
            vec!["§8"],
        ),
    ];
    for (path, body, sections) in wire_fixtures {
        let section_refs: Vec<&str> = sections;
        write_fixture(
            root,
            entries,
            fixture(path, "wire_primitive_case", "accept", None, &section_refs),
            page_codec_fixture_bytes(body),
        );
    }

    write_fixture(
        root,
        entries,
        fixture(
            "accept/digest_manifest_valid.bin",
            "digest_manifest",
            "accept",
            None,
            &["§65"],
        ),
        digest_manifest_payload(7, DigestAlgorithm::Sha256, b"payload").unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/redaction_manifest_valid.bin",
            "redaction_manifest",
            "accept",
            None,
            &["§64"],
        ),
        redaction_manifest_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/io_hints_valid.bin",
            "io_hints",
            "accept",
            None,
            &["§67"],
        ),
        defaults_object_store().encode().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/lakehouse_hints_valid.bin",
            "lakehouse_hints",
            "accept",
            None,
            &["§50"],
        ),
        lakehouse_hints_payload("catalog://cove", "generated"),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/lakehouse_overlay_guard_valid.bin",
            "lakehouse_overlay_guard_case",
            "accept",
            None,
            &["§50"],
        ),
        lakehouse_overlay_guard_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/arrow_bitmap_cove_to_arrow_valid.json",
            "arrow_bitmap_case",
            "accept",
            None,
            &["§49"],
        ),
        arrow_bitmap_fixture_bytes(json!({
            "op": "cove_to_arrow",
            "row_count": 8,
            "input": [10],
            "expect": [245]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/arrow_bitmap_arrow_to_cove_partial_valid.json",
            "arrow_bitmap_case",
            "accept",
            None,
            &["§49"],
        ),
        arrow_bitmap_fixture_bytes(json!({
            "op": "arrow_to_cove",
            "row_count": 4,
            "input": [5],
            "expect": [10]
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/arrow_bitmap_cove_short.json",
            "arrow_bitmap_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§49"],
        ),
        arrow_bitmap_fixture_bytes(json!({
            "op": "cove_to_arrow",
            "row_count": 1,
            "input": [],
            "expect": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/arrow_bitmap_arrow_short.json",
            "arrow_bitmap_case",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§49"],
        ),
        arrow_bitmap_fixture_bytes(json!({
            "op": "arrow_to_cove",
            "row_count": 1,
            "input": [],
            "expect": []
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/kernel_capabilities_valid.bin",
            "kernel_capabilities",
            "accept",
            None,
            &["§21"],
        ),
        kernel_capabilities_payload(CoveEncodingKind::Rle as u16),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/exact_set_index_valid.bin",
            "exact_set_index",
            "accept",
            None,
            &["§30"],
        ),
        exact_set_index_payload(&[2, 5, 9]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/bloom_index_valid.bin",
            "bloom_index",
            "accept",
            None,
            &["§31"],
        ),
        bloom_index_payload(1, 64),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/inverted_morsel_index_valid.bin",
            "inverted_morsel_index",
            "accept",
            None,
            &["§32"],
        ),
        inverted_index_payload(&[5]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/lookup_index_valid.bin",
            "lookup_index",
            "accept",
            None,
            &["§33", "§54"],
        ),
        lookup_index_payload(&[RowRef {
            table_id: 1,
            segment_id: 0,
            morsel_id: 0,
            row_in_morsel: 2,
        }]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/aggregate_synopsis_valid.bin",
            "aggregate_synopsis",
            "accept",
            None,
            &["§34"],
        ),
        aggregate_synopsis_payload(123),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/aggregate_synopsis_all_payloads_valid.bin",
            "aggregate_synopsis",
            "accept",
            None,
            &["§34"],
        ),
        aggregate_synopsis_all_payloads(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/composite_zone_index_valid.bin",
            "composite_zone_index",
            "accept",
            None,
            &["§35"],
        ),
        composite_index_payload(1),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/topn_summary_valid.bin",
            "topn_summary",
            "accept",
            None,
            &["§36"],
        ),
        topn_summary_payload(&[(1, 100), (2, 50)]),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_engine_registry_valid.bin",
            "cove_e_engine_registry",
            "accept",
            None,
            &["§39"],
        ),
        engine_registry_payload(&["org.example"]).unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_execution_code_valid.bin",
            "cove_e_execution_code",
            "accept",
            None,
            &["§40"],
        ),
        valid_execution_descriptor().serialize().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_execution_scope_valid.bin",
            "cove_e_execution_scope",
            "accept",
            None,
            &["§41"],
        ),
        valid_execution_scope_descriptor().serialize().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_code_space_valid.bin",
            "cove_e_code_space",
            "accept",
            None,
            &["§42"],
        ),
        valid_code_space_descriptor().serialize().unwrap(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_mount_policy_valid.bin",
            "cove_e_mount_policy",
            "accept",
            None,
            &["§43"],
        ),
        valid_mount_policy().serialize().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_h_mount_hints_valid.bin",
            "cove_h_mount_hints",
            "accept",
            None,
            &["§44"],
        ),
        valid_harbor_mount_hints().serialize().to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_h_mount_rebuild_reuse.cove",
            "cove_h_mount_case",
            "accept",
            None,
            &["§44", "§48", "§73"],
        ),
        cove_h_mount_case_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_object_catalog_valid.bin",
            "cove_o_object_catalog",
            "accept",
            None,
            &["§56", "§61"],
        ),
        valid_object_catalog().serialize().unwrap(),
    );
    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_object_catalog_old_layout.bin",
            "cove_o_object_catalog",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§56", "§76"],
        ),
        old_layout_object_catalog_bytes(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_temporal_segment_index_valid.bin",
            "cove_o_temporal_segment_index",
            "accept",
            None,
            &["§57"],
        ),
        valid_temporal_segment_index().serialize().unwrap(),
    );

    let valid_temporal_rows = valid_temporal_rows();
    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_temporal_valid.cove",
            "cove",
            "accept",
            None,
            &["§58", "§60", "§73"],
        ),
        semantic_profile_cove_file(
            PrimaryProfile::ObjectTemporal,
            FEATURE_OBJECT_PROFILE,
            0,
            vec![
                cove_o_object_catalog_section(),
                cove_o_temporal_segment_index_section(&[(5, &valid_temporal_rows)]),
                cove_o_temporal_segment_data_section(5, &valid_temporal_rows),
            ],
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_trust_manifest_valid.cove",
            "cove",
            "accept",
            None,
            &["§63", "§73"],
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
                    data: trust_manifest_payload(5, &valid_temporal_rows),
                },
            ],
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_trust_manifest_filecode_reassignment_valid.cove",
            "cove",
            "accept",
            None,
            &["§63", "§73"],
        ),
        cove_o_trust_manifest_filecode_reassignment_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/extension_registry_valid.bin",
            "extension_registry",
            "accept",
            None,
            &["§45"],
        ),
        extension_registry_valid_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_bad_crc.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§45"],
        ),
        extension_registry_bad_crc_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_reserved.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§45"],
        ),
        extension_registry_reserved_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_trailing.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§45"],
        ),
        extension_registry_trailing_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_required_unknown.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§45", "§77"],
        ),
        extension_registry_required_unknown_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_physical_no_fallback.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§45", "§76"],
        ),
        extension_registry_optional_no_fallback_payload(ExtensionKind::PhysicalKind),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_encoding_no_fallback.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§45", "§76"],
        ),
        extension_registry_optional_no_fallback_payload(ExtensionKind::Encoding),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_registry_compression_no_fallback.bin",
            "extension_registry",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§45", "§76"],
        ),
        extension_registry_optional_no_fallback_payload(ExtensionKind::CompressionCodec),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/extension_logical_type_patient_id.bin",
            "extension_logical_type",
            "accept",
            None,
            &["§46"],
        ),
        extension_logical_type_payload(0),
    );

    write_fixture(
        root,
        entries,
        with_collation_count(
            fixture(
                "reject/extension_logical_type_bad_collation.bin",
                "extension_logical_type",
                "reject",
                Some("COVE_E_BAD_EXTENSION"),
                &["§46"],
            ),
            1,
        ),
        extension_logical_type_payload(2),
    );

    write_fixture(
        root,
        entries,
        with_expect_can_skip(
            fixture(
                "accept/extension_index_false_negative_non_skipping.bin",
                "extension_index_descriptor",
                "accept",
                None,
                &["§47"],
            ),
            false,
        ),
        extension_index_descriptor_payload(
            ExtensionProofCapability::None,
            ExtensionFalseNegativePolicy::MayHaveFalseNegatives,
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/extension_index_false_negative_proof_claim.bin",
            "extension_index_descriptor",
            "reject",
            Some("COVE_E_BAD_EXTENSION"),
            &["§47"],
        ),
        extension_index_descriptor_payload(
            ExtensionProofCapability::DefinitelyNo,
            ExtensionFalseNegativePolicy::MayHaveFalseNegatives,
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_temporal_bloom_valid.bin",
            "cove_o_temporal_bloom_index",
            "accept",
            None,
            &["§62"],
        ),
        temporal_bloom_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_bloom_bad_crc.bin",
            "cove_o_temporal_bloom_index",
            "reject",
            Some("COVE_E_CHECKSUM_MISMATCH"),
            &["§62"],
        ),
        temporal_bloom_bad_crc_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_bloom_filter_oob.bin",
            "cove_o_temporal_bloom_index",
            "reject",
            Some("COVE_E_OFFSET_RANGE"),
            &["§62"],
        ),
        temporal_bloom_filter_oob_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_o_temporal_bloom_inverted_bucket.bin",
            "cove_o_temporal_bloom_index",
            "reject",
            Some("COVE_E_BAD_INDEX"),
            &["§62"],
        ),
        temporal_bloom_inverted_bucket_payload(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/durable_publish_replace.json",
            "durable_publish_case",
            "accept",
            None,
            &["§75"],
        ),
        suite_contract_fixture_bytes(json!({
            "case_id": "replace",
            "payload": "durable-cove-candidate"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/durable_publish_delta_manifest_order.json",
            "durable_publish_case",
            "accept",
            None,
            &["§75", "§63.1", "§69"],
        ),
        suite_contract_fixture_bytes(json!({
            "case_id": "delta-manifest-order",
            "payload": "durable-covedelta-candidate",
            "manifest_payload": "durable-covm-candidate"
        })),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/durable_publish_invalid_json.json",
            "durable_publish_case",
            "reject",
            Some("COVE_E_BAD_SECTION"),
            &["§75", "§76"],
        ),
        b"not json".to_vec(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_unknown_optional_feature.cove",
            "cove",
            "accept",
            None,
            &["§74", "§77"],
        ),
        cove_with_unknown_optional_feature(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_optional_bad_descriptor.cove",
            "cove",
            "accept",
            None,
            &["§40", "§74", "§77"],
        ),
        profile_cove_file(
            0,
            FEATURE_ENGINE_PROFILE,
            SectionKind::ExecutionCodeDescriptor,
            PrimaryProfile::EngineExecution,
            0,
            FEATURE_ENGINE_PROFILE,
            invalid_execution_descriptor_payload(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_lz4_valid.cove",
            "cove",
            "accept",
            None,
            &["§40", "§66", "§73"],
        ),
        compressed_profile_cove_file(
            FEATURE_ENGINE_PROFILE,
            FEATURE_CODEC_LZ4,
            SectionKind::ExecutionCodeDescriptor,
            PrimaryProfile::EngineExecution,
            FEATURE_ENGINE_PROFILE,
            FEATURE_CODEC_LZ4,
            CompressionCodec::Lz4,
            valid_execution_descriptor().serialize().to_vec(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_zstd_valid.cove",
            "cove",
            "accept",
            None,
            &["§40", "§66", "§73"],
        ),
        compressed_profile_cove_file(
            FEATURE_ENGINE_PROFILE,
            FEATURE_CODEC_ZSTD,
            SectionKind::ExecutionCodeDescriptor,
            PrimaryProfile::EngineExecution,
            FEATURE_ENGINE_PROFILE,
            FEATURE_CODEC_ZSTD,
            CompressionCodec::Zstd,
            valid_execution_descriptor().serialize().to_vec(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_profile_bundle_valid.cove",
            "cove",
            "accept",
            None,
            &["§39", "§40", "§41", "§42", "§43", "§73"],
        ),
        cove_e_profile_bundle_file(true, false),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_e_optional_bad_refs.cove",
            "cove",
            "accept",
            None,
            &["§39", "§40", "§41", "§42", "§43", "§74", "§77"],
        ),
        cove_e_profile_bundle_file(false, true),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_h_optional_bad_hints.cove",
            "cove",
            "accept",
            None,
            &["§44", "§74", "§77"],
        ),
        profile_cove_file(
            0,
            FEATURE_HARBOR_PROFILE,
            SectionKind::HarborMountHints,
            PrimaryProfile::HarborExecution,
            0,
            FEATURE_HARBOR_PROFILE,
            invalid_harbor_mount_hints_payload(),
        ),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_map_valid.cove",
            "cove",
            "accept",
            None,
            &["§70", "§73.6"],
        ),
        cove_map_valid_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_invalid.cove",
            "cove",
            "reject",
            Some("COVE_E_MAP_INVALID"),
            &["§70", "§73.6"],
        ),
        cove_map_invalid_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_function_undeclared.cove",
            "cove",
            "reject",
            Some("COVE_E_MAP_FUNCTION_UNDECLARED"),
            &["§70.5", "§70.13", "§73.6", "§76"],
        ),
        cove_map_function_undeclared_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_identity_conflict.cove",
            "cove",
            "reject",
            Some("COVE_E_MAP_IDENTITY_CONFLICT"),
            &["§70.6", "§73.6", "§76"],
        ),
        cove_map_identity_conflict_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_source_stale.cove",
            "cove",
            "reject",
            Some("COVE_E_MAP_SOURCE_STALE"),
            &["§70", "§73.6"],
        ),
        cove_map_source_stale_file(),
    );

    write_fixture(
        root,
        entries,
        fixture(
            "reject/cove_map_evidence_invalid.cove",
            "cove",
            "reject",
            Some("COVE_E_MAP_EVIDENCE_INVALID"),
            &["§70.12", "§73.6", "§76"],
        ),
        cove_map_evidence_invalid_file(),
    );

    write_cove_map_execution_cases(root, entries);

    write_fixture(
        root,
        entries,
        fixture(
            "accept/cove_o_optional_bad_catalog.cove",
            "cove",
            "accept",
            None,
            &["§56", "§74", "§77"],
        ),
        profile_cove_file(
            0,
            FEATURE_OBJECT_PROFILE,
            SectionKind::ObjectTypeCatalog,
            PrimaryProfile::ObjectTemporal,
            0,
            FEATURE_OBJECT_PROFILE,
            invalid_object_catalog().serialize().unwrap(),
        ),
    );

    covemap_bytes
}
