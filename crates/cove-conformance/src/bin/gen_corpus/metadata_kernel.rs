use super::*;

pub(super) fn collation_registry_payload(entries: &[(u16, &str, &str)]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    for (collation_id, name, version) in entries {
        out.extend_from_slice(&collation_id.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(version.len() as u16).to_le_bytes());
        out.extend_from_slice(version.as_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out
}

pub(super) fn collation_registry_bad_utf8_payload() -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.push(0xff);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

pub(super) fn page_index_payload(row_count: u32, null_count: u32, encoding: u16) -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&null_count.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&encoding.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

pub(super) fn zone_stat_scalar(value: &[u8]) -> [u8; STAT_SCALAR_ENCODED_LEN] {
    let mut out = [0u8; STAT_SCALAR_ENCODED_LEN];
    out[0] = 1;
    out[2..4].copy_from_slice(&(value.len() as u16).to_le_bytes());
    out[4..4 + value.len()].copy_from_slice(value);
    out
}

pub(super) fn zone_stats_payload(row_count: u32, null_count: u32, non_null_count: u32) -> Vec<u8> {
    let mut out = [0u8; ZONE_STATS_ENTRY_LEN];
    out[0..4].copy_from_slice(&1u32.to_le_bytes());
    out[4..8].copy_from_slice(&2u32.to_le_bytes());
    out[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    out[12..16].copy_from_slice(&3u32.to_le_bytes());
    out[16..20].copy_from_slice(&row_count.to_le_bytes());
    out[20..24].copy_from_slice(&null_count.to_le_bytes());
    out[24..28].copy_from_slice(&non_null_count.to_le_bytes());
    out[28..32].copy_from_slice(&5u32.to_le_bytes());
    out[32..36].copy_from_slice(&2u32.to_le_bytes());
    out[36..40].copy_from_slice(&ZoneStatFlags::HAS_MIN_MAX.bits().to_le_bytes());
    out[40..60].copy_from_slice(&zone_stat_scalar(&1i64.to_le_bytes()));
    out[60..80].copy_from_slice(&zone_stat_scalar(&9i64.to_le_bytes()));
    out.to_vec()
}

pub(super) fn valid_zone_stats_payload() -> Vec<u8> {
    zone_stats_payload(10, 2, 8)
}

pub(super) fn invalid_zone_stats_payload() -> Vec<u8> {
    zone_stats_payload(10, 2, 7)
}

pub(super) fn digest_manifest_payload(
    section_id: u32,
    algorithm: DigestAlgorithm,
    payload: &[u8],
) -> Result<Vec<u8>, cove_core::CoveError> {
    let digest = compute_digest(algorithm, payload)?;
    DigestManifest {
        algorithm,
        scope: DigestScope::Section,
        root_digest: [0; 32],
        entries: vec![DigestEntry {
            target_kind: DigestTargetKind::Section,
            section_id,
            local_id: 0,
            offset: 0,
            length: payload.len() as u64,
            digest,
        }],
    }
    .serialize()
}

pub(super) fn digest_manifest_wrong_len_payload() -> Vec<u8> {
    let mut out = DigestManifest {
        algorithm: DigestAlgorithm::Sha256,
        scope: DigestScope::Section,
        root_digest: [0; 32],
        entries: vec![DigestEntry {
            target_kind: DigestTargetKind::Section,
            section_id: 7,
            local_id: 0,
            offset: 0,
            length: 4,
            digest: vec![0u8; 32],
        }],
    }
    .serialize()
    .unwrap();
    let digest_len_pos = cove_core::digest::DIGEST_MANIFEST_HEADER_LEN + 2;
    out[digest_len_pos..digest_len_pos + 2].copy_from_slice(&4u16.to_le_bytes());
    out.truncate(cove_core::digest::DIGEST_MANIFEST_HEADER_LEN + 32 + 4);
    out[16..24].copy_from_slice(&(36u64).to_le_bytes());
    out[56..60].fill(0);
    let crc = checksum::crc32c(&out[..cove_core::digest::DIGEST_MANIFEST_HEADER_LEN]);
    out[56..60].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn digest_manifest_bad_checksum_payload() -> Vec<u8> {
    let mut out =
        digest_manifest_payload(7, DigestAlgorithm::Sha256, b"payload").expect("digest manifest");
    out[0] ^= 0xFF;
    out
}

pub(super) fn redaction_manifest_payload() -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    out.extend_from_slice(&7u64.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&42u64.to_le_bytes());
    out.extend_from_slice(&17u16.to_le_bytes());
    out.extend_from_slice(&11u16.to_le_bytes());
    out.extend_from_slice(b"policy/gdpr");
    out.extend_from_slice(&9u16.to_le_bytes());
    out.extend_from_slice(b"ticket-42");
    out.extend_from_slice(&1_700_000_000_000_000i64.to_le_bytes());
    out
}

pub(super) fn lakehouse_hints_payload(catalog: &str, provenance: &str) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out.extend_from_slice(&1u32.to_le_bytes());
    write_len_prefixed(&mut out, b"date");
    write_len_prefixed(&mut out, b"2026-05-04");
    out.push(0);
    write_len_prefixed(&mut out, catalog.as_bytes());
    write_len_prefixed(&mut out, provenance.as_bytes());
    out.extend_from_slice(&[0u8; 32]);
    out
}

pub(super) fn lakehouse_overlay_guard_payload() -> Vec<u8> {
    LakehouseHints {
        schema_fingerprint: [0x11; 32],
        partition_values: vec![("date".into(), "2026-05-04".into())],
        source_snapshot: Some(123),
        sequence_number: Some(456),
        catalog_identifier: "catalog://cove".into(),
        provenance: "generated".into(),
        conversion_digest: [0x22; 32],
        visibility_overlay: Some(LakehouseVisibilityOverlayRef {
            overlay_kind: 1,
            file_id: Some([0x33; 16]),
            file_len: Some(4096),
            footer_crc32c: Some(0x1234_5678),
            digest: Some([0x44; 32]),
            reference: "s3://bucket/deletes.dv".into(),
        }),
    }
    .serialize()
    .unwrap()
}

pub(super) fn lakehouse_hints_bad_utf8_payload() -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(0);
    write_len_prefixed(&mut out, &[0xff]);
    write_len_prefixed(&mut out, b"");
    out.extend_from_slice(&[0u8; 32]);
    out
}

pub(super) struct DictionaryFixtureEntry {
    pub(super) value_tag: ValueTag,
    pub(super) storage_class: StorageClass,
    pub(super) canonical_bytes: Vec<u8>,
}

pub(super) fn valid_file_dictionary_fixture_payload() -> Result<Vec<u8>, cove_core::CoveError> {
    dictionary_fixture_payload(&[
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Utf8("active").encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::DateDays,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::DateDays(12).encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::List,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::List(vec![
                CanonicalValue::Bool(true),
                CanonicalValue::Utf8("x"),
            ])
            .encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Struct,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Struct(vec![
                CanonicalField {
                    field_id: 7,
                    value: CanonicalValue::Bool(false),
                },
                CanonicalField {
                    field_id: 1,
                    value: CanonicalValue::Int { width: 8, value: 9 },
                },
            ])
            .encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Map,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Map(vec![
                (CanonicalValue::Utf8("a"), CanonicalValue::Utf8("1")),
                (CanonicalValue::Utf8("b"), CanonicalValue::Utf8("2")),
            ])
            .encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Payload,
            canonical_bytes: CanonicalValue::Utf8("this is a payload-only dictionary value")
                .encode()?,
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Redacted,
            canonical_bytes: Vec::new(),
        },
    ])
}

pub(super) fn invalid_file_dictionary_bad_utf8_len_payload() -> Vec<u8> {
    dictionary_fixture_payload_unchecked(&[DictionaryFixtureEntry {
        value_tag: ValueTag::Utf8,
        storage_class: StorageClass::Inline,
        canonical_bytes: vec![5, b'a', b'b', b'c'],
    }])
}

pub(super) fn invalid_file_dictionary_bad_map_duplicate_payload(
) -> Result<Vec<u8>, cove_core::CoveError> {
    let key = tagged_canonical_bytes(&CanonicalValue::Utf8("dup"))?;
    let value1 = tagged_canonical_bytes(&CanonicalValue::Utf8("v1"))?;
    let value2 = tagged_canonical_bytes(&CanonicalValue::Utf8("v2"))?;
    let mut map = cove_core::wire::encode_u64_leb128(2);
    map.extend_from_slice(&key);
    map.extend_from_slice(&value1);
    map.extend_from_slice(&key);
    map.extend_from_slice(&value2);
    Ok(dictionary_fixture_payload_unchecked(&[
        DictionaryFixtureEntry {
            value_tag: ValueTag::Map,
            storage_class: StorageClass::Payload,
            canonical_bytes: map,
        },
    ]))
}

pub(super) fn invalid_file_dictionary_redacted_null_payload(
) -> Result<Vec<u8>, cove_core::CoveError> {
    dictionary_fixture_payload(&[DictionaryFixtureEntry {
        value_tag: ValueTag::Null,
        storage_class: StorageClass::Redacted,
        canonical_bytes: Vec::new(),
    }])
}

pub(super) fn tagged_canonical_bytes(
    value: &CanonicalValue<'_>,
) -> Result<Vec<u8>, cove_core::CoveError> {
    let mut out = cove_core::wire::encode_u64_leb128(value.value_tag() as u64);
    out.extend_from_slice(&value.encode()?);
    Ok(out)
}

pub(super) fn dictionary_fixture_payload(
    entries: &[DictionaryFixtureEntry],
) -> Result<Vec<u8>, cove_core::CoveError> {
    Ok(dictionary_fixture_payload_unchecked(entries))
}

pub(super) fn dictionary_fixture_payload_unchecked(entries: &[DictionaryFixtureEntry]) -> Vec<u8> {
    let (index, payload) = dictionary_fixture_index_and_payload(entries);
    let mut out = Vec::with_capacity(4 + index.len() + payload.len());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&payload);
    out
}

pub(super) fn dictionary_fixture_index_and_payload(
    entries: &[DictionaryFixtureEntry],
) -> (Vec<u8>, Vec<u8>) {
    let mut index_entries = Vec::with_capacity(entries.len());
    let mut payload = Vec::new();
    for entry in entries {
        let mut inline_data = [0u8; 16];
        let (inline_len, payload_offset, payload_length) = match entry.storage_class {
            StorageClass::Inline => {
                assert!(entry.canonical_bytes.len() <= inline_data.len());
                inline_data[..entry.canonical_bytes.len()].copy_from_slice(&entry.canonical_bytes);
                (entry.canonical_bytes.len() as u8, 0, 0)
            }
            StorageClass::Payload => {
                let payload_offset = payload.len() as u64;
                payload.extend_from_slice(&entry.canonical_bytes);
                (0, payload_offset, entry.canonical_bytes.len() as u32)
            }
            StorageClass::Redacted => (0, 0, 0),
            _ => panic!("future storage class is not supported by conformance fixtures"),
        };
        index_entries.push(FileDictionaryIndexEntryV1 {
            value_tag: entry.value_tag as u16,
            storage_class: entry.storage_class as u8,
            flags: 0,
            inline_len,
            reserved0: [0; 3],
            inline_data,
            payload_offset,
            payload_length,
            canonical_hash64: 0,
            reserved1: 0,
        });
    }

    let header = FileDictionaryHeaderV1 {
        entry_count: entries.len() as u32,
        flags: 0,
        index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
        value_hash_algorithm: 0,
        payload_length: payload.len() as u64,
        reserved: [0; 24],
    };
    let mut index = header.serialize().to_vec();
    for entry in index_entries {
        index.extend_from_slice(&entry.serialize());
    }
    (index, payload)
}

pub(super) fn write_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(bytes);
}

pub(super) fn kernel_capabilities_payload(encoding: u16) -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    out.extend_from_slice(&encoding.to_le_bytes());
    out.extend_from_slice(&[
        1, // supports_eq
        1, // supports_in
        1, // supports_range
        1, // supports_is_null
        1, // supports_count
        1, // supports_min_max
        1, // supports_selection_decode
        0, // supports_direct_executioncode_remap
        2, // decode_cost_class
        3, // predicate_cost_class
        0, 0, 0, 0, 0, 0, // reserved
    ]);
    out
}

pub(super) fn kernel_capabilities_payload_from_entry(encoding: CoveEncodingKind) -> Vec<u8> {
    KernelCapabilities {
        entries: vec![KernelCapabilityEntry {
            encoding,
            supports_eq: 1,
            supports_in: 1,
            supports_range: 1,
            supports_is_null: 1,
            supports_count: 1,
            supports_min_max: 1,
            supports_selection_decode: 1,
            supports_direct_executioncode_remap: 0,
            decode_cost_class: 2,
            predicate_cost_class: 3,
            reserved: [0; 6],
        }],
    }
    .serialize()
}

pub(super) fn kernel_capabilities_reserved_payload() -> Vec<u8> {
    let mut bytes = kernel_capabilities_payload_from_entry(CoveEncodingKind::Rle);
    *bytes.last_mut().unwrap() = 1;
    bytes
}

pub(super) fn kernel_capabilities_trailing_payload() -> Vec<u8> {
    let mut bytes = kernel_capabilities_payload_from_entry(CoveEncodingKind::Rle);
    bytes.push(0);
    bytes
}

pub(super) fn exact_set_index_payload(codes: &[u64]) -> Vec<u8> {
    let mut data = Vec::new();
    for code in codes {
        data.extend_from_slice(&code.to_le_bytes());
    }
    let header = ExactSetIndexHeaderV1 {
        table_id: 1,
        column_id: 1,
        granularity: ExactSetGranularity::Morsel,
        key_kind: ExactSetKeyKind::FileCode,
        representation: ExactSetRepresentation::SortedList,
        flags: 0,
        entry_count: codes.len() as u32,
        data_offset: EXACT_SET_HEADER_LEN as u64,
        data_length: data.len() as u64,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&data);
    out
}

pub(super) fn bloom_index_payload(filter_count: u32, byte_len: u32) -> Vec<u8> {
    let header = BloomIndexHeaderV1 {
        table_id: 1,
        column_id: 1,
        granularity: BloomGranularity::Morsel,
        hash_domain: BloomHashDomain::FileCode,
        algorithm: BloomAlgorithm::SplitBlock,
        flags: 0,
        target_fpr_ppm: 10_000,
        filter_count,
        data_offset: BLOOM_INDEX_HEADER_LEN as u64,
        data_length: byte_len as u64,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    out.extend(std::iter::repeat_n(0u8, byte_len as usize));
    out
}

pub(super) fn inverted_index_payload(keys: &[u64]) -> Vec<u8> {
    let bitmap_offset = INVERTED_MORSEL_INDEX_HEADER_LEN + keys.len() * INVERTED_MORSEL_ENTRY_LEN;
    let header = InvertedMorselIndexHeaderV1 {
        table_id: 1,
        column_id: 1,
        key_kind: InvertedKeyKind::FileCode,
        flags: 0,
        representation: 0,
        reserved: 0,
        entry_count: keys.len() as u32,
        entries_offset: INVERTED_MORSEL_INDEX_HEADER_LEN as u64,
        bitmap_data_offset: bitmap_offset as u64,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    for (idx, key) in keys.iter().enumerate() {
        let entry = InvertedEntry {
            key: *key,
            morsel_bitmap_offset: idx as u64,
            morsel_bitmap_length: 1,
            row_bitmap_offset: 0,
            row_bitmap_length: 0,
        };
        out.extend_from_slice(&entry.serialize());
    }
    out.extend(std::iter::repeat_n(0xff, keys.len().max(1)));
    out
}

pub(super) fn lookup_index_payload(rows: &[RowRef]) -> Vec<u8> {
    lookup_index_payload_for_entries(&[(10, rows)])
}

pub(super) fn lookup_index_unsorted_payload() -> Vec<u8> {
    let row = RowRef {
        table_id: 1,
        segment_id: 0,
        morsel_id: 0,
        row_in_morsel: 0,
    };
    lookup_index_payload_for_entries(&[(10, &[row]), (5, &[row])])
}

pub(super) fn lookup_index_payload_for_entries(entries: &[(u64, &[RowRef])]) -> Vec<u8> {
    let mut entry_bytes = Vec::new();
    let mut rowref_bytes = Vec::new();
    let mut rowref_start = 0u32;
    for (key, rows) in entries {
        entry_bytes.extend_from_slice(&key.to_le_bytes());
        entry_bytes.extend_from_slice(&rowref_start.to_le_bytes());
        entry_bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        for row in *rows {
            rowref_bytes.extend_from_slice(&row.encode());
        }
        rowref_start += rows.len() as u32;
    }
    let rowref_offset = LOOKUP_INDEX_HEADER_LEN + entry_bytes.len();
    let header = LookupIndexHeaderV1 {
        table_id: 1,
        column_id: 1,
        key_kind: LookupKeyKind::FileCode,
        index_kind: LookupIndexKind::SparseSorted,
        uniqueness: LookupUniqueness::NonUnique,
        flags: 0,
        entry_count: entries.len() as u64,
        entries_offset: LOOKUP_INDEX_HEADER_LEN as u64,
        entries_length: entry_bytes.len() as u64,
        rowref_offset: rowref_offset as u64,
        rowref_length: rowref_bytes.len() as u64,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&entry_bytes);
    out.extend_from_slice(&rowref_bytes);
    out
}

pub(super) fn aggregate_synopsis_payload(count: u64) -> Vec<u8> {
    aggregate_count_entry(SynopsisKind::Count, count as u32, 0)
        .serialize()
        .to_vec()
}

pub(super) fn aggregate_count_entry(
    kind: SynopsisKind,
    row_count: u32,
    null_count: u32,
) -> AggregateEntry {
    AggregateEntry {
        table_id: 1,
        segment_id: 0,
        morsel_id: u32::MAX,
        column_id: 1,
        synopsis_kind: kind,
        key_kind: 0,
        accuracy: if matches!(
            kind,
            SynopsisKind::DistinctSketch | SynopsisKind::QuantileSketch
        ) {
            SynopsisAccuracy::Approximate
        } else {
            SynopsisAccuracy::Exact
        },
        flags: 0,
        row_count,
        null_count,
        payload_offset: 0,
        payload_length: 0,
        checksum: 0,
    }
}

pub(super) fn aggregate_synopsis_unknown_kind_payload() -> Vec<u8> {
    let mut out = aggregate_synopsis_payload(1);
    out[16] = 99;
    out[44..48].fill(0);
    let crc = checksum::crc32c(&out);
    out[44..48].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn aggregate_synopsis_all_payloads() -> Vec<u8> {
    let entries = vec![
        aggregate_count_entry(SynopsisKind::Count, 3, 0),
        aggregate_count_entry(SynopsisKind::MinMax, 3, 0),
        aggregate_count_entry(SynopsisKind::Sum, 3, 0),
        aggregate_count_entry(SynopsisKind::SumAndCount, 3, 0),
        aggregate_count_entry(SynopsisKind::BoolTrueFalseCounts, 3, 0),
        aggregate_count_entry(SynopsisKind::FileCodeHistogram, 3, 0),
        aggregate_count_entry(SynopsisKind::NumCodeHistogram, 3, 0),
        aggregate_count_entry(SynopsisKind::DistinctSketch, 3, 0),
        aggregate_count_entry(SynopsisKind::QuantileSketch, 3, 0),
        aggregate_count_entry(SynopsisKind::TopK, 3, 0),
    ];
    let payloads = vec![
        AggregatePayloadV2::None,
        AggregatePayloadV2::MinMax {
            min: Some(aggregate_i64_value(1)),
            max: Some(aggregate_i64_value(3)),
        },
        AggregatePayloadV2::Sum {
            overflow_policy: NumericAggregateOverflowPolicy::CheckedExact,
            sum: aggregate_i64_value(6),
        },
        AggregatePayloadV2::SumAndCount {
            overflow_policy: NumericAggregateOverflowPolicy::CheckedExact,
            non_null_count: 3,
            sum: aggregate_i64_value(6),
        },
        AggregatePayloadV2::BoolTrueFalseCounts {
            true_count: 2,
            false_count: 1,
        },
        AggregatePayloadV2::FileCodeHistogram {
            buckets: vec![
                HistogramBucket { key: 1, count: 1 },
                HistogramBucket { key: 2, count: 2 },
            ],
        },
        AggregatePayloadV2::NumCodeHistogram {
            buckets: vec![
                HistogramBucket { key: 10, count: 1 },
                HistogramBucket { key: 20, count: 2 },
            ],
        },
        AggregatePayloadV2::DistinctSketch {
            precision: DEFAULT_HLL_PRECISION,
            registers: vec![0; 1usize << DEFAULT_HLL_PRECISION],
        },
        AggregatePayloadV2::QuantileSketch {
            k: DEFAULT_KLL_K,
            value_tag: ValueTag::Int64,
            level_offsets: vec![0, 3],
            values: vec![
                1i64.to_le_bytes().to_vec(),
                2i64.to_le_bytes().to_vec(),
                3i64.to_le_bytes().to_vec(),
            ],
        },
        AggregatePayloadV2::TopK {
            k: 2,
            entries: vec![
                HistogramBucket { key: 2, count: 2 },
                HistogramBucket { key: 1, count: 1 },
            ],
        },
    ];
    AggregateSynopsis::from_parts(entries, payloads)
        .unwrap()
        .serialize()
}

pub(super) fn aggregate_i64_value(value: i64) -> TaggedCanonicalValue {
    TaggedCanonicalValue {
        value_tag: ValueTag::Int64,
        payload: value.to_le_bytes().to_vec(),
    }
}

pub(super) fn aggregate_synopsis_bad_payload_bounds() -> Vec<u8> {
    let mut out = aggregate_bool_synopsis();
    out.pop();
    out
}

pub(super) fn aggregate_synopsis_bad_payload_checksum() -> Vec<u8> {
    let mut out = aggregate_bool_synopsis();
    let last = out.len() - 1;
    out[last] ^= 0x40;
    out
}

pub(super) fn aggregate_synopsis_wrong_kind_payload_pairing() -> Vec<u8> {
    let mut out = aggregate_bool_synopsis();
    out[52] = SynopsisKind::TopK as u8;
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_unsorted_histogram_keys() -> Vec<u8> {
    let mut out = aggregate_filecode_histogram_synopsis();
    let data = 48 + 28;
    out[data..data + 8].copy_from_slice(&2u64.to_le_bytes());
    out[data + 16..data + 24].copy_from_slice(&1u64.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_duplicate_histogram_keys() -> Vec<u8> {
    let mut out = aggregate_filecode_histogram_synopsis();
    let data = 48 + 28;
    out[data + 16..data + 24].copy_from_slice(&1u64.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_count_sum_mismatch() -> Vec<u8> {
    let mut out = aggregate_filecode_histogram_synopsis();
    let data = 48 + 28;
    out[data + 24..data + 32].copy_from_slice(&1u64.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_invalid_canonical_value() -> Vec<u8> {
    let mut out = aggregate_minmax_synopsis();
    let data = 48 + 28;
    out[data + 4..data + 8].copy_from_slice(&7u32.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_approximate_marked_exact() -> Vec<u8> {
    let mut out = aggregate_hll_synopsis();
    out[18] = SynopsisAccuracy::Exact as u8;
    fix_entry_checksum(&mut out, 0);
    out
}

pub(super) fn aggregate_synopsis_bad_hll_header() -> Vec<u8> {
    let mut out = aggregate_hll_synopsis();
    out[48 + 12..48 + 16].copy_from_slice(&3u32.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_synopsis_bad_kll_header() -> Vec<u8> {
    let mut out = aggregate_kll_synopsis();
    let data = 48 + 28;
    out[data + 8 + 4..data + 8 + 8].copy_from_slice(&2u32.to_le_bytes());
    fix_payload_checksum(&mut out, 48);
    out
}

pub(super) fn aggregate_bool_synopsis() -> Vec<u8> {
    AggregateSynopsis::from_parts(
        vec![aggregate_count_entry(
            SynopsisKind::BoolTrueFalseCounts,
            3,
            0,
        )],
        vec![AggregatePayloadV2::BoolTrueFalseCounts {
            true_count: 2,
            false_count: 1,
        }],
    )
    .unwrap()
    .serialize()
}

pub(super) fn aggregate_minmax_synopsis() -> Vec<u8> {
    AggregateSynopsis::from_parts(
        vec![aggregate_count_entry(SynopsisKind::MinMax, 3, 0)],
        vec![AggregatePayloadV2::MinMax {
            min: Some(aggregate_i64_value(1)),
            max: Some(aggregate_i64_value(3)),
        }],
    )
    .unwrap()
    .serialize()
}

pub(super) fn aggregate_filecode_histogram_synopsis() -> Vec<u8> {
    AggregateSynopsis::from_parts(
        vec![aggregate_count_entry(SynopsisKind::FileCodeHistogram, 3, 0)],
        vec![AggregatePayloadV2::FileCodeHistogram {
            buckets: vec![
                HistogramBucket { key: 1, count: 1 },
                HistogramBucket { key: 2, count: 2 },
            ],
        }],
    )
    .unwrap()
    .serialize()
}

pub(super) fn aggregate_hll_synopsis() -> Vec<u8> {
    AggregateSynopsis::from_parts(
        vec![aggregate_count_entry(SynopsisKind::DistinctSketch, 3, 0)],
        vec![AggregatePayloadV2::DistinctSketch {
            precision: DEFAULT_HLL_PRECISION,
            registers: vec![0; 1usize << DEFAULT_HLL_PRECISION],
        }],
    )
    .unwrap()
    .serialize()
}

pub(super) fn aggregate_kll_synopsis() -> Vec<u8> {
    AggregateSynopsis::from_parts(
        vec![aggregate_count_entry(SynopsisKind::QuantileSketch, 3, 0)],
        vec![AggregatePayloadV2::QuantileSketch {
            k: DEFAULT_KLL_K,
            value_tag: ValueTag::Int64,
            level_offsets: vec![0, 3],
            values: vec![
                1i64.to_le_bytes().to_vec(),
                2i64.to_le_bytes().to_vec(),
                3i64.to_le_bytes().to_vec(),
            ],
        }],
    )
    .unwrap()
    .serialize()
}

pub(super) fn fix_entry_checksum(bytes: &mut [u8], entry_offset: usize) {
    bytes[entry_offset + 44..entry_offset + 48].fill(0);
    let crc = checksum::crc32c(&bytes[entry_offset..entry_offset + 48]);
    bytes[entry_offset + 44..entry_offset + 48].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn fix_payload_checksum(bytes: &mut [u8], payload_offset: usize) {
    bytes[payload_offset + 24..payload_offset + 28].fill(0);
    let data_len = u32::from_le_bytes(
        bytes[payload_offset + 20..payload_offset + 24]
            .try_into()
            .unwrap(),
    ) as usize;
    let payload_len = 28 + data_len;
    let crc = checksum::crc32c(&bytes[payload_offset..payload_offset + payload_len]);
    bytes[payload_offset + 24..payload_offset + 28].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn composite_index_payload(key_column_count: u8) -> Vec<u8> {
    let mut key_column_bytes = Vec::new();
    for column_id in 0..key_column_count {
        key_column_bytes.extend_from_slice(&(column_id as u32 + 1).to_le_bytes());
    }
    let entry_bytes = if key_column_count == 0 {
        Vec::new()
    } else {
        vec![0xA5; 8]
    };
    let entries_offset = COMPOSITE_ZONE_INDEX_HEADER_LEN + key_column_bytes.len();
    let header = CompositeZoneIndexHeaderV1 {
        table_id: 1,
        key_column_count: key_column_count as u16,
        transform_kind: CompositeTransformKind::Tuple,
        flags: 0,
        zone_count: if key_column_count == 0 { 0 } else { 1 },
        key_columns_offset: COMPOSITE_ZONE_INDEX_HEADER_LEN as u64,
        entries_offset: entries_offset as u64,
        entries_length: entry_bytes.len() as u64,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&key_column_bytes);
    out.extend_from_slice(&entry_bytes);
    out
}

pub(super) fn topn_summary_payload(entries: &[(u64, u64)]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (code, frequency) in entries {
        payload.extend_from_slice(&code.to_le_bytes());
        payload.extend_from_slice(&frequency.to_le_bytes());
    }
    let summary = TopNSummary {
        table_id: 1,
        column_id: 1,
        segment_id: 0,
        morsel_id: 0,
        direction: TopNDirection::Largest,
        value_count: entries.len() as u16,
        flags: 0,
        payload_offset: TOPN_ZONE_SUMMARY_LEN as u64,
        payload_length: payload.len() as u64,
        checksum: 0,
        payload,
    };
    let mut out = summary.serialize_header().to_vec();
    out.extend_from_slice(&summary.payload);
    out
}

pub(super) fn topn_summary_bad_direction_payload() -> Vec<u8> {
    let mut out = topn_summary_payload(&[(1, 100)]);
    out[16] = 99;
    out[36..40].fill(0);
    let crc = checksum::crc32c(&out[..TOPN_ZONE_SUMMARY_LEN]);
    out[36..40].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn engine_registry_payload(
    namespaces: &[&str],
) -> Result<Vec<u8>, cove_core::CoveError> {
    let profiles = namespaces
        .iter()
        .enumerate()
        .map(|(idx, namespace)| EngineProfileEntryV1 {
            profile_id: idx as u32 + 1,
            namespace: (*namespace).into(),
            profile_name: "engine-dictionary-code".into(),
            version_major: 1,
            version_minor: 0,
            required_features: 0,
            optional_features: 0,
            execution_descriptor_ref: 2,
            mount_policy_ref: 3,
            private_payload_ref: 0,
            checksum: 0,
        })
        .collect();
    EngineProfileRegistry { flags: 0, profiles }.serialize()
}

pub(super) fn valid_execution_descriptor() -> ExecutionCodeDescriptorV1 {
    ExecutionCodeDescriptorV1 {
        descriptor_id: 1,
        code_kind: ExecutionCodeKind::DictionaryKey,
        code_width_bits: 32,
        byte_order: 0,
        lifetime: ExecutionCodeLifetime::Scan,
        comparison_scope: ExecutionCodeComparisonScope::File,
        canonicality: ExecutionCodeCanonicality::Transient,
        null_code_policy: NullCodePolicy::NullBitmapOnly,
        flags: 0,
        scope_ref: 0,
        code_space_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_execution_scope_descriptor() -> ExecutionScopeDescriptorV1 {
    ExecutionScopeDescriptorV1 {
        scope_id: 2,
        scope_kind: ExecutionScopeKind::Catalog,
        flags: 0,
        stable_id: b"catalog/main".to_vec(),
        display_name: "main catalog".into(),
        private_payload_ref: 0,
    }
}

pub(super) fn invalid_execution_scope_descriptor_payload() -> Vec<u8> {
    let mut bytes = valid_execution_scope_descriptor().serialize().unwrap();
    bytes[4..6].copy_from_slice(&99u16.to_le_bytes());
    bytes
}

pub(super) fn valid_code_space_descriptor() -> CodeSpaceDescriptorV1 {
    CodeSpaceDescriptorV1 {
        code_space_id: 3,
        namespace: "org.example.engine".into(),
        stable_id: b"space-1".to_vec(),
        epoch: 7,
        flags: 0,
        private_payload_ref: 0,
    }
}

pub(super) fn invalid_code_space_descriptor_payload() -> Vec<u8> {
    let mut bytes = valid_code_space_descriptor().serialize().unwrap();
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[6] = 0xff;
    bytes
}

pub(super) fn invalid_execution_descriptor_payload() -> Vec<u8> {
    let mut bytes = valid_execution_descriptor().serialize().to_vec();
    bytes[4] = 42;
    bytes[24..28].fill(0);
    let crc = checksum::crc32c(&bytes);
    bytes[24..28].copy_from_slice(&crc.to_le_bytes());
    bytes
}

pub(super) fn valid_mount_policy() -> EngineMountPolicyV1 {
    EngineMountPolicyV1 {
        policy_id: 1,
        filecode_mapping_kind: FileCodeMappingKind::MapToExecutionCode,
        missing_value_policy: MissingValuePolicy::DecodeValueOnly,
        stale_mapping_policy: StaleMappingPolicy::IgnoreIfOptional,
        reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
        flags: 0,
        dictionary_digest_ref: 0,
        code_space_ref: 2,
        cache_key_ref: 0,
        private_payload_ref: 0,
        checksum: 0,
    }
}

pub(super) fn engine_registry_payload_with_refs(
    execution_descriptor_ref: u32,
    mount_policy_ref: u32,
) -> Result<Vec<u8>, cove_core::CoveError> {
    EngineProfileRegistry {
        flags: 0,
        profiles: vec![EngineProfileEntryV1 {
            profile_id: 1,
            namespace: "org.example".into(),
            profile_name: "engine-dictionary-code".into(),
            version_major: 1,
            version_minor: 0,
            required_features: 0,
            optional_features: 0,
            execution_descriptor_ref,
            mount_policy_ref,
            private_payload_ref: 0,
            checksum: 0,
        }],
    }
    .serialize()
}

pub(super) fn valid_execution_descriptor_with_refs(
    descriptor_id: u32,
    scope_ref: u32,
    code_space_ref: u32,
) -> ExecutionCodeDescriptorV1 {
    ExecutionCodeDescriptorV1 {
        descriptor_id,
        code_kind: ExecutionCodeKind::DictionaryKey,
        code_width_bits: 32,
        byte_order: 0,
        lifetime: ExecutionCodeLifetime::Scan,
        comparison_scope: ExecutionCodeComparisonScope::File,
        canonicality: ExecutionCodeCanonicality::Transient,
        null_code_policy: NullCodePolicy::NullBitmapOnly,
        flags: 0,
        scope_ref,
        code_space_ref,
        checksum: 0,
    }
}

pub(super) fn valid_mount_policy_with_refs(
    policy_id: u32,
    code_space_ref: u32,
) -> EngineMountPolicyV1 {
    EngineMountPolicyV1 {
        policy_id,
        filecode_mapping_kind: FileCodeMappingKind::MapToExecutionCode,
        missing_value_policy: MissingValuePolicy::DecodeValueOnly,
        stale_mapping_policy: StaleMappingPolicy::IgnoreIfOptional,
        reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
        flags: 0,
        dictionary_digest_ref: 0,
        code_space_ref,
        cache_key_ref: 0,
        private_payload_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_execution_scope_descriptor_with_id(
    scope_id: u32,
) -> ExecutionScopeDescriptorV1 {
    ExecutionScopeDescriptorV1 {
        scope_id,
        scope_kind: ExecutionScopeKind::Catalog,
        flags: 0,
        stable_id: b"catalog/main".to_vec(),
        display_name: "main catalog".into(),
        private_payload_ref: 0,
    }
}

pub(super) fn valid_code_space_descriptor_with_id(code_space_id: u32) -> CodeSpaceDescriptorV1 {
    CodeSpaceDescriptorV1 {
        code_space_id,
        namespace: "org.example.engine".into(),
        stable_id: b"space-1".to_vec(),
        epoch: 7,
        flags: 0,
        private_payload_ref: 0,
    }
}

pub(super) fn cove_e_profile_bundle_file(required: bool, dangling_scope_ref: bool) -> Vec<u8> {
    let file_required_features = if required { FEATURE_ENGINE_PROFILE } else { 0 };
    let file_optional_features = if required { 0 } else { FEATURE_ENGINE_PROFILE };
    let section_required_features = if required { FEATURE_ENGINE_PROFILE } else { 0 };
    let section_optional_features = if required { 0 } else { FEATURE_ENGINE_PROFILE };
    let scope_id = 31;
    let code_space_id = 41;
    let scope_ref = if dangling_scope_ref { 99 } else { scope_id };
    semantic_profile_cove_file(
        PrimaryProfile::Mixed,
        file_required_features,
        file_optional_features,
        vec![
            SectionPayload {
                section_kind: SectionKind::EngineProfileRegistry as u16,
                profile: PrimaryProfile::EngineExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: section_required_features,
                optional_features: section_optional_features,
                data: engine_registry_payload_with_refs(11, 21).unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::ExecutionCodeDescriptor as u16,
                profile: PrimaryProfile::EngineExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: section_required_features,
                optional_features: section_optional_features,
                data: valid_execution_descriptor_with_refs(11, scope_ref, code_space_id)
                    .serialize()
                    .to_vec(),
            },
            SectionPayload {
                section_kind: SectionKind::ExecutionScopeDescriptor as u16,
                profile: PrimaryProfile::EngineExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: section_required_features,
                optional_features: section_optional_features,
                data: valid_execution_scope_descriptor_with_id(scope_id)
                    .serialize()
                    .unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::CodeSpaceDescriptor as u16,
                profile: PrimaryProfile::EngineExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: section_required_features,
                optional_features: section_optional_features,
                data: valid_code_space_descriptor_with_id(code_space_id)
                    .serialize()
                    .unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::EngineMountPolicy as u16,
                profile: PrimaryProfile::EngineExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: section_required_features,
                optional_features: section_optional_features,
                data: valid_mount_policy_with_refs(21, code_space_id)
                    .serialize()
                    .to_vec(),
            },
        ],
    )
}

pub(super) fn invalid_mount_policy_payload() -> Vec<u8> {
    let mut bytes = valid_mount_policy().serialize().to_vec();
    bytes[4] = 42;
    bytes[28..32].fill(0);
    let crc = checksum::crc32c(&bytes);
    bytes[28..32].copy_from_slice(&crc.to_le_bytes());
    bytes
}

pub(super) fn valid_harbor_mount_hints() -> HarborMountHintsV1 {
    HarborMountHintsV1 {
        harbor_profile_version_major: 1,
        harbor_profile_version_minor: 0,
        tenant_scope_ref: 1,
        code_space_ref: 2,
        lease_epoch: 3,
        dictionary_digest_ref: 0,
        catalog_digest_ref: 0,
        mount_cache_policy: 0,
        reserved: [0; 7],
        private_payload_ref: 0,
        checksum: 0,
    }
}

pub(super) fn cove_h_mount_case_file() -> Vec<u8> {
    let dictionary_entries = [
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Utf8("red").encode().unwrap(),
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Utf8("blue").encode().unwrap(),
        },
    ];
    let dictionary = dictionary_fixture_index_and_payload(&dictionary_entries);
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 0,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "name".into(),
                logical: CoveLogicalType::Utf8,
                physical: CovePhysicalKind::FileCode,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    };
    semantic_profile_cove_file(
        PrimaryProfile::HarborExecution,
        FEATURE_TABLE_PROFILE | FEATURE_FILE_DICTIONARY | FEATURE_HARBOR_PROFILE,
        0,
        vec![
            SectionPayload {
                section_kind: SectionKind::FileDictionaryIndex as u16,
                profile: PrimaryProfile::Mixed as u8,
                flags: 0,
                item_count: dictionary_entries.len() as u64,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_FILE_DICTIONARY,
                optional_features: 0,
                data: dictionary.0,
            },
            SectionPayload {
                section_kind: SectionKind::FileDictionaryPayload as u16,
                profile: PrimaryProfile::Mixed as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_FILE_DICTIONARY,
                optional_features: 0,
                data: dictionary.1,
            },
            SectionPayload {
                section_kind: SectionKind::TableCatalog as u16,
                profile: PrimaryProfile::TableScan as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_TABLE_PROFILE,
                optional_features: 0,
                data: catalog.serialize().unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::HarborMountHints as u16,
                profile: PrimaryProfile::HarborExecution as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_HARBOR_PROFILE,
                optional_features: 0,
                data: valid_harbor_mount_hints().serialize().to_vec(),
            },
        ],
    )
}

pub(super) fn invalid_harbor_mount_hints_payload() -> Vec<u8> {
    let mut data = valid_harbor_mount_hints().serialize().to_vec();
    data[29] = 1;
    data
}

pub(super) fn valid_object_catalog() -> ObjectTypeCatalog {
    object_catalog_with_property(CoveLogicalType::Bool, CovePhysicalKind::Boolean)
}

pub(super) fn object_catalog_with_property(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> ObjectTypeCatalog {
    ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: cove_core::profile::cove_o::OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type,
                physical_kind,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    }
}
