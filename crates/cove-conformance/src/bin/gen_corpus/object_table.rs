use super::*;

pub(super) fn invalid_object_catalog() -> ObjectTypeCatalog {
    let mut catalog = valid_object_catalog();
    let property = catalog.types[0].properties[0].clone();
    catalog.types[0].properties.push(property);
    catalog
}

pub(super) fn old_layout_object_catalog_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&5u16.to_le_bytes());
    out.extend_from_slice(b"Thing");
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

pub(super) fn valid_temporal_segment_index() -> TemporalSegmentIndex {
    TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(5, &valid_temporal_rows())],
    }
}

pub(super) fn cove_o_object_catalog_section() -> SectionPayload {
    cove_o_object_catalog_section_for_property(CoveLogicalType::Bool, CovePhysicalKind::Boolean)
}

pub(super) fn cove_o_object_catalog_section_for_property(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> SectionPayload {
    let catalog = object_catalog_with_property(logical_type, physical_kind);
    cove_o_object_catalog_section_from_catalog(catalog)
}

pub(super) fn cove_o_object_catalog_section_from_catalog(
    catalog: ObjectTypeCatalog,
) -> SectionPayload {
    SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: catalog.types.len() as u64,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    }
}

pub(super) fn cove_o_temporal_segment_index_section(
    segments: &[(u32, &[TemporalRowEntryV1])],
) -> SectionPayload {
    cove_o_temporal_segment_index_section_for_object_type(1, segments)
}

pub(super) fn cove_o_temporal_segment_index_section_for_object_type(
    object_type_id: u32,
    segments: &[(u32, &[TemporalRowEntryV1])],
) -> SectionPayload {
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: segments
            .iter()
            .map(|(segment_id, rows)| {
                temporal_segment_entry_for_rows_object_type(*segment_id, object_type_id, rows)
            })
            .collect(),
    };
    SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: index.entries.len() as u64,
        row_count: segments.iter().map(|(_, rows)| rows.len() as u64).sum(),
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    }
}

pub(super) fn cove_o_temporal_segment_data_section(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
) -> SectionPayload {
    cove_o_temporal_segment_data_section_for_object_type(segment_id, 1, rows)
}

pub(super) fn cove_o_temporal_segment_data_section_for_object_type(
    segment_id: u32,
    object_type_id: u32,
    rows: &[TemporalRowEntryV1],
) -> SectionPayload {
    SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: temporal_segment_data_payload_for_object_type(segment_id, object_type_id, rows),
    }
}

pub(super) fn cove_o_property_stats_only_file(
    required_features: u64,
    page_flags: u32,
    non_null_count: u32,
    null_count: u32,
) -> Vec<u8> {
    cove_o_property_stats_only_file_with_property(
        required_features,
        page_flags,
        non_null_count,
        null_count,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        None,
    )
}

pub(super) fn cove_o_property_stats_only_file_with_property(
    required_features: u64,
    page_flags: u32,
    non_null_count: u32,
    null_count: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    zone_stats_payload: Option<Vec<u8>>,
) -> Vec<u8> {
    let rows = valid_temporal_rows();
    let segment_payload = temporal_segment_data_payload_with_property_stats_only(
        5,
        &rows,
        page_flags,
        non_null_count,
        null_count,
        logical_type,
        physical_kind,
    );
    let mut index_entry = temporal_segment_entry_for_rows(5, &rows);
    index_entry.length = segment_payload.len() as u64;
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![index_entry],
    };
    let mut sections = vec![
        cove_o_object_catalog_section_for_property(logical_type, physical_kind),
        SectionPayload {
            section_kind: SectionKind::TemporalSegmentIndex as u16,
            profile: PrimaryProfile::ObjectTemporal as u8,
            flags: 0,
            item_count: 1,
            row_count: rows.len() as u64,
            compression: 0,
            alignment_log2: 0,
            required_features: FEATURE_OBJECT_PROFILE,
            optional_features: 0,
            data: index.serialize().unwrap(),
        },
        SectionPayload {
            section_kind: SectionKind::TemporalSegmentData as u16,
            profile: PrimaryProfile::ObjectTemporal as u8,
            flags: 0,
            item_count: 1,
            row_count: rows.len() as u64,
            compression: 0,
            alignment_log2: 0,
            required_features: FEATURE_OBJECT_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
            optional_features: 0,
            data: segment_payload,
        },
    ];
    if let Some(data) = zone_stats_payload {
        sections.push(SectionPayload {
            section_kind: SectionKind::ZoneStats as u16,
            profile: PrimaryProfile::TableScan as u8,
            flags: 0,
            item_count: (data.len() / ZONE_STATS_ENTRY_LEN) as u64,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: 0,
            data,
        });
    }
    semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        required_features,
        0,
        sections,
    )
}

pub(super) fn cove_o_property_constant_stats() -> ZoneStatsEntry {
    let scalar = StatScalar {
        kind: StatKind::Int64,
        bytes: 42i64.to_le_bytes().to_vec(),
        truncated: false,
    };
    ZoneStatsEntry {
        table_id: 0,
        segment_id: 5,
        morsel_id: 0,
        column_id: 1,
        non_null_count: valid_temporal_rows().len() as u32,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: valid_temporal_rows().len() as u64,
            null_count: 0,
            min: Some(scalar.clone()),
            max: Some(scalar),
            flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
        },
        min_domain_rank: 0,
        max_domain_rank: 0,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

pub(super) fn cove_o_zone_stats_payload(entries: Vec<ZoneStatsEntry>) -> Vec<u8> {
    ZoneStatsSection { entries }.serialize().unwrap()
}

pub(super) fn cove_o_float64_nan_stats_payload() -> Vec<u8> {
    let scalar = stat_scalar_raw(StatKind::Float64Bits, &f64::NAN.to_bits().to_le_bytes());
    let mut out = [0u8; ZONE_STATS_ENTRY_LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..8].copy_from_slice(&5u32.to_le_bytes());
    out[8..12].copy_from_slice(&0u32.to_le_bytes());
    out[12..16].copy_from_slice(&1u32.to_le_bytes());
    out[16..20].copy_from_slice(&(valid_temporal_rows().len() as u32).to_le_bytes());
    out[20..24].copy_from_slice(&0u32.to_le_bytes());
    out[24..28].copy_from_slice(&(valid_temporal_rows().len() as u32).to_le_bytes());
    out[28..32].copy_from_slice(&1u32.to_le_bytes());
    out[32..36].copy_from_slice(&1u32.to_le_bytes());
    out[36..40].copy_from_slice(
        &(ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT | ZoneStatFlags::HAS_NAN)
            .bits()
            .to_le_bytes(),
    );
    out[40..60].copy_from_slice(&scalar);
    out[60..80].copy_from_slice(&scalar);
    out[88..92].copy_from_slice(&u32::MAX.to_le_bytes());
    out[92..96].copy_from_slice(&u32::MAX.to_le_bytes());
    out.to_vec()
}

pub(super) fn cove_o_property_filecode_stats_only_file() -> Vec<u8> {
    let rows = valid_temporal_rows();
    let segment_payload = temporal_segment_data_payload_with_property_stats_only(
        5,
        &rows,
        PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL,
        rows.len() as u32,
        0,
        CoveLogicalType::UInt64,
        CovePhysicalKind::FileCode,
    );
    let mut index_entry = temporal_segment_entry_for_rows(5, &rows);
    index_entry.length = segment_payload.len() as u64;
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![index_entry],
    };
    let dictionary_entries = [uint64_dictionary_entry()];
    let dictionary = dictionary_fixture_index_and_payload(&dictionary_entries);
    semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE
            | FEATURE_TABLE_PROFILE
            | FEATURE_FILE_DICTIONARY
            | FEATURE_PAGE_PAYLOAD_ELISION,
        0,
        vec![
            cove_o_object_catalog_section_for_property(
                CoveLogicalType::UInt64,
                CovePhysicalKind::FileCode,
            ),
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
                section_kind: SectionKind::TemporalSegmentIndex as u16,
                profile: PrimaryProfile::ObjectTemporal as u8,
                flags: 0,
                item_count: 1,
                row_count: rows.len() as u64,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_OBJECT_PROFILE,
                optional_features: 0,
                data: index.serialize().unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::TemporalSegmentData as u16,
                profile: PrimaryProfile::ObjectTemporal as u8,
                flags: 0,
                item_count: 1,
                row_count: rows.len() as u64,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_OBJECT_PROFILE | FEATURE_PAGE_PAYLOAD_ELISION,
                optional_features: 0,
                data: segment_payload,
            },
            SectionPayload {
                section_kind: SectionKind::ZoneStats as u16,
                profile: PrimaryProfile::TableScan as u8,
                flags: 0,
                item_count: 1,
                row_count: 0,
                compression: 0,
                alignment_log2: 0,
                required_features: 0,
                optional_features: 0,
                data: cove_o_zone_stats_payload(vec![cove_o_property_filecode_constant_stats()]),
            },
        ],
    )
}

pub(super) fn cove_o_property_filecode_constant_stats() -> ZoneStatsEntry {
    let scalar = StatScalar {
        kind: StatKind::UInt64,
        bytes: 0u64.to_le_bytes().to_vec(),
        truncated: false,
    };
    ZoneStatsEntry {
        table_id: 0,
        segment_id: 5,
        morsel_id: 0,
        column_id: 1,
        non_null_count: valid_temporal_rows().len() as u32,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: valid_temporal_rows().len() as u64,
            null_count: 0,
            min: Some(scalar.clone()),
            max: Some(scalar),
            flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
        },
        min_domain_rank: 0,
        max_domain_rank: 0,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

pub(super) fn cove_o_trust_manifest_filecode_reassignment_file() -> Vec<u8> {
    let rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0xA1; 16],
        record_id: [0xA2; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let dictionary_entries = [
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Utf8("beta").encode().unwrap(),
        },
        DictionaryFixtureEntry {
            value_tag: ValueTag::Utf8,
            storage_class: StorageClass::Inline,
            canonical_bytes: CanonicalValue::Utf8("alpha").encode().unwrap(),
        },
    ];
    let (dictionary_index, dictionary_payload) =
        dictionary_fixture_index_and_payload(&dictionary_entries);
    let dictionary = FileDictionary::parse(&dictionary_index, &dictionary_payload).unwrap();
    let segment_payload = temporal_segment_data_payload_with_filecode_property(5, &rows, &[1]);
    let segment = TemporalSegmentData::parse(&segment_payload).unwrap();
    let mut index_entry = temporal_segment_entry_for_rows(5, &rows);
    index_entry.length = segment_payload.len() as u64;
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![index_entry],
    };

    semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE
            | FEATURE_TABLE_PROFILE
            | FEATURE_FILE_DICTIONARY
            | FEATURE_TRUST_CHAIN,
        0,
        vec![
            cove_o_object_catalog_section_for_property(
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
            ),
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
                data: dictionary_index,
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
                data: dictionary_payload,
            },
            SectionPayload {
                section_kind: SectionKind::TemporalSegmentIndex as u16,
                profile: PrimaryProfile::ObjectTemporal as u8,
                flags: 0,
                item_count: 1,
                row_count: rows.len() as u64,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_OBJECT_PROFILE,
                optional_features: 0,
                data: index.serialize().unwrap(),
            },
            SectionPayload {
                section_kind: SectionKind::TemporalSegmentData as u16,
                profile: PrimaryProfile::ObjectTemporal as u8,
                flags: 0,
                item_count: 1,
                row_count: rows.len() as u64,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_OBJECT_PROFILE,
                optional_features: 0,
                data: segment_payload,
            },
            SectionPayload {
                section_kind: SectionKind::TrustManifest as u16,
                profile: PrimaryProfile::ObjectTemporal as u8,
                flags: 0,
                item_count: rows.len() as u64,
                row_count: rows.len() as u64,
                compression: 0,
                alignment_log2: 0,
                required_features: FEATURE_TRUST_CHAIN,
                optional_features: 0,
                data: trust_manifest_payload_for_segment_with_dictionary(
                    &segment,
                    Some(&dictionary),
                ),
            },
        ],
    )
}

pub(super) fn stat_scalar_raw(kind: StatKind, value: &[u8]) -> [u8; STAT_SCALAR_ENCODED_LEN] {
    let mut out = [0u8; STAT_SCALAR_ENCODED_LEN];
    out[0] = kind as u8;
    out[2..4].copy_from_slice(&(value.len() as u16).to_le_bytes());
    out[4..4 + value.len()].copy_from_slice(value);
    out
}

pub(super) fn valid_temporal_rows() -> Vec<TemporalRowEntryV1> {
    vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [0; 16],
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [0; 16],
            record_id: [1; 16],
            record_kind: RecordKind::Snapshot,
            prev_ref: Some(CoveRecordRefV1 {
                segment_id: 5,
                row_index: 0,
                target_kind: 0,
            }),
        },
    ]
}

pub(super) fn delta_only_temporal_rows() -> Vec<TemporalRowEntryV1> {
    vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0; 16],
        record_id: [0; 16],
        record_kind: RecordKind::Delta,
        prev_ref: None,
    }]
}

pub(super) fn duplicate_record_id_temporal_rows() -> Vec<TemporalRowEntryV1> {
    vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [0xAA; 16],
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [0; 16],
            record_id: [0xAA; 16],
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
    ]
}

pub(super) fn valid_tombstone_temporal_rows() -> Vec<TemporalRowEntryV1> {
    vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0x09; 16],
        record_id: [0x0A; 16],
        record_kind: RecordKind::Tombstone,
        prev_ref: None,
    }]
}

pub(super) fn temporal_segment_data_payload(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
) -> Vec<u8> {
    temporal_segment_data_payload_for_object_type(segment_id, 1, rows)
}

pub(super) fn temporal_segment_data_payload_for_object_type(
    segment_id: u32,
    object_type_id: u32,
    rows: &[TemporalRowEntryV1],
) -> Vec<u8> {
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let payload = TemporalSegmentData {
        header: TemporalSegmentHeaderV1 {
            segment_id,
            object_type_id,
            time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
            time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
            csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
            csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
            row_count: rows.len() as u32,
            morsel_count: u32::from(!rows.is_empty()),
            morsel_row_count: if rows.is_empty() {
                0
            } else {
                rows.len() as u32
            },
            column_count: 0,
            row_directory_offset,
            column_directory_offset: row_end,
            page_index_offset: row_end,
            data_offset: row_end,
            flags: 0,
            checksum: 0,
        },
        rows: rows.to_vec(),
        property_columns: Vec::new(),
    };
    let mut out = payload.header.serialize().to_vec();
    for row in &payload.rows {
        out.extend_from_slice(&row.serialize());
    }
    out
}

pub(super) fn temporal_segment_data_payload_with_property_stats_only(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
    page_flags: u32,
    non_null_count: u32,
    null_count: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> Vec<u8> {
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 1,
        logical_type,
        physical_kind,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: 0,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count,
        null_count,
        encoding_root: u32::MAX,
        page_offset: 0,
        page_length: 0,
        uncompressed_length: 0,
        stats_ref: 0,
        flags: page_flags,
        checksum: checksum::crc32c(&[]),
    };
    let mut out = header.serialize().to_vec();
    for row in rows {
        out.extend_from_slice(&row.serialize());
    }
    out.extend_from_slice(&directory.serialize());
    out.extend_from_slice(&page.serialize());
    out
}

pub(super) fn temporal_segment_data_payload_with_filecode_property(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
    codes: &[u32],
) -> Vec<u8> {
    assert_eq!(rows.len(), codes.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut value_bytes = Vec::with_capacity(codes.len() * 4);
    for code in codes {
        value_bytes.extend_from_slice(&code.to_le_bytes());
    }
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::FileCode,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 1,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directory = TableColumnDirectoryEntryV1 {
        column_id: 1,
        logical_type: CoveLogicalType::Utf8,
        physical_kind: CovePhysicalKind::FileCode,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let page = ColumnPageIndexEntryV1 {
        column_id: 1,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::FileCode as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };
    let mut out = header.serialize().to_vec();
    for row in rows {
        out.extend_from_slice(&row.serialize());
    }
    out.extend_from_slice(&directory.serialize());
    out.extend_from_slice(&page.serialize());
    out.extend_from_slice(&payload);
    out
}

pub(super) fn trust_manifest_payload(segment_id: u32, rows: &[TemporalRowEntryV1]) -> Vec<u8> {
    let mut out = (rows.len() as u32).to_le_bytes().to_vec();
    let mut prev = [0u8; 32];
    for (row_index, row) in rows.iter().enumerate() {
        out.extend_from_slice(&segment_id.to_le_bytes());
        out.extend_from_slice(&(row_index as u32).to_le_bytes());
        prev = cove_core::trust_chain::chain(&prev, &row.trust_payload()).unwrap();
        out.extend_from_slice(&prev);
    }
    out
}

pub(super) fn trust_manifest_payload_for_segment_with_dictionary(
    segment: &TemporalSegmentData,
    dictionary: Option<&FileDictionary>,
) -> Vec<u8> {
    let mut out = (segment.rows.len() as u32).to_le_bytes().to_vec();
    let mut prev = [0u8; 32];
    for row_index in 0..segment.rows.len() {
        out.extend_from_slice(&segment.header.segment_id.to_le_bytes());
        out.extend_from_slice(&(row_index as u32).to_le_bytes());
        let payload =
            temporal_row_trust_payload(segment, row_index as u32, dictionary, &[]).unwrap();
        prev = cove_core::trust_chain::chain(&prev, &payload).unwrap();
        out.extend_from_slice(&prev);
    }
    out
}

pub(super) fn invalid_temporal_segment_index() -> TemporalSegmentIndex {
    TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry(1, 2, 2, 0, 0, 1)],
    }
}

pub(super) fn temporal_segment_entry_for_rows(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
) -> TemporalSegmentIndexEntryV1 {
    temporal_segment_entry_for_rows_object_type(segment_id, 1, rows)
}

pub(super) fn temporal_segment_entry_for_rows_object_type(
    segment_id: u32,
    object_type_id: u32,
    rows: &[TemporalRowEntryV1],
) -> TemporalSegmentIndexEntryV1 {
    let (delta_count, snapshot_count, baseline_count, tombstone_count) =
        temporal_row_kind_counts(rows);
    TemporalSegmentIndexEntryV1 {
        segment_id,
        object_type_id,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        delta_count,
        snapshot_count,
        baseline_count,
        tombstone_count,
        min_goid: rows.iter().map(|row| row.goid).min().unwrap_or([0; 16]),
        max_goid: rows.iter().map(|row| row.goid).max().unwrap_or([0; 16]),
        offset: 0,
        length: temporal_segment_data_payload(segment_id, rows).len() as u64,
        checksum: 0,
    }
}

pub(super) fn temporal_row_kind_counts(rows: &[TemporalRowEntryV1]) -> (u32, u32, u32, u32) {
    let mut delta_count = 0;
    let mut snapshot_count = 0;
    let mut baseline_count = 0;
    let mut tombstone_count = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta_count += 1,
            RecordKind::Snapshot => snapshot_count += 1,
            RecordKind::Baseline => baseline_count += 1,
            RecordKind::Tombstone => tombstone_count += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta_count, snapshot_count, baseline_count, tombstone_count)
}

pub(super) fn temporal_segment_entry(
    segment_id: u32,
    row_count: u32,
    delta_count: u32,
    snapshot_count: u32,
    baseline_count: u32,
    tombstone_count: u32,
) -> TemporalSegmentIndexEntryV1 {
    TemporalSegmentIndexEntryV1 {
        segment_id,
        object_type_id: 1,
        time_range_start_us: 10,
        time_range_end_us: 20,
        csn_min: 1,
        csn_max: 2,
        row_count,
        delta_count,
        snapshot_count,
        baseline_count,
        tombstone_count,
        min_goid: [0; 16],
        max_goid: [1; 16],
        offset: 128,
        length: 4096,
        checksum: 0,
    }
}

pub(super) fn cove_t_scan_table_file() -> Vec<u8> {
    let mut writer = ScanProfileCoveWriter::new(valid_table_catalog());
    writer.push_segment(ScanSegment::new(1, 0, 0, 10, 1));
    writer.write().unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegisteredFixtureKind {
    SupportedStable,
    SupportedStableNoFallback,
    UnsupportedWithFallback,
    UnsupportedNoFallback,
    FallbackMismatch,
    MalformedEnvelope,
}

pub(super) fn cove_t_registered_codec_file(kind: RegisteredFixtureKind) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 9,
            namespace: "public".into(),
            name: "registered_names".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "name".into(),
                logical: CoveLogicalType::Utf8,
                physical: CovePhysicalKind::VarBytes,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    };
    let fallback_values = match kind {
        RegisteredFixtureKind::FallbackMismatch => [&b"alpha"[..], &b"wrong"[..], &b"gamma"[..]],
        _ => [&b"alpha"[..], &b"beta"[..], &b"gamma"[..]],
    };
    let fallback = (!matches!(
        kind,
        RegisteredFixtureKind::SupportedStableNoFallback
            | RegisteredFixtureKind::UnsupportedNoFallback
    ))
    .then(|| {
        ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::VarBytes,
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            None,
            varbytes_payload(&fallback_values),
        )
        .unwrap()
    });
    let codec_id = match kind {
        RegisteredFixtureKind::UnsupportedWithFallback
        | RegisteredFixtureKind::UnsupportedNoFallback
        | RegisteredFixtureKind::MalformedEnvelope => 9001,
        RegisteredFixtureKind::SupportedStable
        | RegisteredFixtureKind::SupportedStableNoFallback
        | RegisteredFixtureKind::FallbackMismatch => 1,
    };
    let mut registered_payload = ColumnPagePayloadV1::build_registered_single_node(
        3,
        3,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        codec_id,
        2,
        0,
        cfs2_payload(&[&b"alpha"[..], &b"beta"[..], &b"gamma"[..]]),
        fallback,
    )
    .unwrap();
    if kind == RegisteredFixtureKind::MalformedEnvelope {
        corrupt_registered_envelope_preserving_buffer_crc(&mut registered_payload);
    }
    let mut segment = ScanSegment::new(9, 0, 0, 3, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(3, registered_payload)
            .with_encoding_root(CoveEncodingKind::RegisteredEncoding as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    if matches!(
        kind,
        RegisteredFixtureKind::SupportedStable
            | RegisteredFixtureKind::SupportedStableNoFallback
            | RegisteredFixtureKind::FallbackMismatch
    ) {
        writer.push_extra_section(SectionPayload {
            section_kind: SectionKind::CodecExtensionRegistry as u16,
            profile: PrimaryProfile::CodecExtension as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: FEATURE_REGISTERED_ENCODINGS,
            data: stable_fsst_descriptor_payload(),
        });
    }
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn cove_t_bool_numcode_file(declared_numeric: bool) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "flags".into(),
            row_count: if declared_numeric { 3 } else { 0 },
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "active_code".into(),
                logical: CoveLogicalType::Bool,
                physical: CovePhysicalKind::NumCode,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: if declared_numeric {
                    COLUMN_FLAG_BOOL_DECLARED_NUMERIC
                } else {
                    0
                },
            }],
        }],
    };
    if declared_numeric {
        let mut writer = ScanProfileCoveWriter::new(catalog);
        writer.push_segment(ScanSegment::new(1, 0, 0, 3, 1));
        writer.write().unwrap()
    } else {
        let mut writer = MinimalCoveWriter::new();
        writer.primary_profile = PrimaryProfile::TableScan as u8;
        writer.required_features = FEATURE_TABLE_PROFILE;
        writer.sections.push(SectionPayload {
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
        });
        writer.write().unwrap()
    }
}

pub(super) fn bool_numcode_catalog(row_count: u64) -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "flags".into(),
            row_count,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "active_code".into(),
                logical: CoveLogicalType::Bool,
                physical: CovePhysicalKind::NumCode,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: COLUMN_FLAG_BOOL_DECLARED_NUMERIC,
            }],
        }],
    }
}

pub(super) fn cove_t_bool_numcode_invalid_value_file() -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, 1, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(1, 2u64.to_le_bytes().to_vec())
            .with_counts(1, 0)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(bool_numcode_catalog(1));
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn cove_t_constant_numcode_high_bits_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "codes".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "raw_u64".into(),
                logical: CoveLogicalType::UInt64,
                physical: CovePhysicalKind::NumCode,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    };
    let raw = i64::MAX as u64 + 1;
    let payload = ConstantPayload {
        value: i64::from_le_bytes(raw.to_le_bytes()),
        row_count: 2,
    }
    .encode()
    .to_vec();
    let mut segment = ScanSegment::new(1, 0, 0, 2, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(2, payload)
            .with_counts(2, 0)
            .with_encoding_root(CoveEncodingKind::Constant as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn filecode_typed_page_catalog(row_count: u64) -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "labels".into(),
            row_count,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "label".into(),
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
    }
}

pub(super) fn single_entry_file_dictionary() -> FileDictionary {
    let (index, payload) = dictionary_fixture_index_and_payload(&[utf8_dictionary_entry()]);
    FileDictionary::parse(&index, &payload).unwrap()
}

pub(super) fn single_entry_uint64_file_dictionary() -> FileDictionary {
    let (index, payload) = dictionary_fixture_index_and_payload(&[uint64_dictionary_entry()]);
    FileDictionary::parse(&index, &payload).unwrap()
}

pub(super) fn utf8_dictionary_entry() -> DictionaryFixtureEntry {
    DictionaryFixtureEntry {
        value_tag: ValueTag::Utf8,
        storage_class: StorageClass::Inline,
        canonical_bytes: CanonicalValue::Utf8("alpha").encode().unwrap(),
    }
}

pub(super) fn uint64_dictionary_entry() -> DictionaryFixtureEntry {
    DictionaryFixtureEntry {
        value_tag: ValueTag::UInt64,
        storage_class: StorageClass::Inline,
        canonical_bytes: CanonicalValue::Uint { width: 8, value: 0 }
            .encode()
            .unwrap(),
    }
}

pub(super) fn cove_t_filecode_typed_page_file(
    encoding: CoveEncodingKind,
    values: Vec<u8>,
    row_count: u64,
) -> Vec<u8> {
    let row_count_u32 = u32::try_from(row_count).unwrap();
    let mut segment = ScanSegment::new(1, 0, 0, row_count_u32, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(row_count_u32, values)
            .with_counts(row_count_u32, 0)
            .with_encoding_root(encoding as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(filecode_typed_page_catalog(row_count));
    let dictionary = single_entry_file_dictionary();
    writer.push_file_dictionary(&dictionary);
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn cove_t_filecode_missing_dictionary_file() -> Vec<u8> {
    let catalog = filecode_typed_page_catalog(1);
    let table_catalog_payload = catalog.serialize().unwrap();
    let valid =
        cove_t_filecode_typed_page_file(CoveEncodingKind::FileCode, 0u32.to_le_bytes().to_vec(), 1);
    let validated = reader::validate_bytes(&valid).unwrap();
    let segment_data_entry = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::TableSegmentData as u16)
        .unwrap();
    let segment_data = valid
        [segment_data_entry.offset as usize..segment_data_entry.end_offset().unwrap() as usize]
        .to_vec();

    let segment_index_len = TableSegmentIndex {
        flags: 0,
        entries: vec![TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id: 0,
            row_start: 0,
            row_count: 1,
            morsel_count: 1,
            morsel_row_count: 4096,
            column_count: 1,
            offset: 0,
            length: segment_data.len() as u64,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap()
    .len();
    let segment_data_offset = HEADER_SIZE + table_catalog_payload.len() + segment_index_len;
    let segment_index_payload = TableSegmentIndex {
        flags: 0,
        entries: vec![TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id: 0,
            row_start: 0,
            row_count: 1,
            morsel_count: 1,
            morsel_row_count: 4096,
            column_count: 1,
            offset: segment_data_offset as u64,
            length: segment_data.len() as u64,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap();

    let mut writer = MinimalCoveWriter::new();
    writer.required_features = FEATURE_TABLE_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TableCatalog as u16,
        profile: PrimaryProfile::TableScan as u8,
        flags: 0,
        item_count: 1,
        row_count: 1,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: table_catalog_payload,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TableSegmentIndex as u16,
        profile: PrimaryProfile::TableScan as u8,
        flags: 0,
        item_count: 1,
        row_count: 1,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: segment_index_payload,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TableSegmentData as u16,
        profile: PrimaryProfile::TableScan as u8,
        flags: 0,
        item_count: 1,
        row_count: 1,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: segment_data,
    });
    writer.write().unwrap()
}

pub(super) fn cove_t_plain_varint_filecode_file(code: u64) -> Vec<u8> {
    let bytes = cove_t_filecode_typed_page_file(
        CoveEncodingKind::PlainVarint,
        cove_core::wire::encode_u64_leb128(0),
        1,
    );
    if code == 0 {
        bytes
    } else {
        rewrite_first_segment_page_values(bytes, &cove_core::wire::encode_u64_leb128(code))
    }
}

pub(super) fn cove_t_plain_varint_bool_numcode_invalid_value_file() -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, 1, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(1, cove_core::wire::encode_u64_leb128(2))
            .with_counts(1, 0)
            .with_encoding_root(CoveEncodingKind::PlainVarint as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(bool_numcode_catalog(1));
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn cove_t_constant_filecode_file(code: i64) -> Vec<u8> {
    let payload = ConstantPayload {
        value: 0,
        row_count: 1,
    }
    .encode()
    .to_vec();
    let bytes = cove_t_filecode_typed_page_file(CoveEncodingKind::Constant, payload, 1);
    if code == 0 {
        bytes
    } else {
        let payload = ConstantPayload {
            value: code,
            row_count: 1,
        }
        .encode()
        .to_vec();
        rewrite_first_segment_page_values(bytes, &payload)
    }
}

pub(super) fn cove_t_payload_elision_stats_only_all_null_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 6,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "status_code".into(),
                logical: CoveLogicalType::UInt32,
                physical: CovePhysicalKind::NumCode,
                nullable: true,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    };
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, Vec::new())
            .with_counts(0, 6)
            .with_encoding_root(u32::MAX)
            .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn cove_t_all_non_null_flag_without_elision_feature_file() -> Vec<u8> {
    let values = vec![0u8; 6 * 8];
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, values)
            .with_counts(6, 0)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)
            .with_flags(PAGE_FLAG_ALL_NON_NULL)],
    );
    let mut writer = ScanProfileCoveWriter::new(stats_only_constant_catalog(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
    ));
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn stats_only_constant_catalog(
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
) -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 6,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "status_code".into(),
                logical,
                physical,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    }
}

pub(super) fn cove_t_payload_elision_stats_only_all_non_null_file(
    stats: Option<ZoneStatsEntry>,
) -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, Vec::new())
            .with_counts(0, 6)
            .with_encoding_root(u32::MAX)
            .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL)],
    );
    let mut writer = ScanProfileCoveWriter::new(value_stream_elision_catalog());
    writer.push_segment(segment);
    if let Some(stats) = stats {
        writer
            .push_zone_stats(&ZoneStatsSection {
                entries: vec![stats],
            })
            .unwrap();
    }
    rewrite_first_segment_page(writer.write().unwrap(), |page| {
        page.non_null_count = 6;
        page.null_count = 0;
        page.flags = (page.flags & !PAGE_FLAG_ALL_NULL) | PAGE_FLAG_ALL_NON_NULL;
    })
}

pub(super) fn cove_t_payload_elision_stats_only_all_non_null_float32_file() -> Vec<u8> {
    let scalar = StatScalar {
        kind: StatKind::Float64Bits,
        bytes: 1.0f64.to_bits().to_le_bytes().to_vec(),
        truncated: false,
    };
    let mut stats = valid_constant_page_stats();
    stats.stats.min = Some(scalar.clone());
    stats.stats.max = Some(scalar);
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, Vec::new())
            .with_counts(0, 6)
            .with_encoding_root(u32::MAX)
            .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL)],
    );
    let mut writer = ScanProfileCoveWriter::new(stats_only_constant_catalog(
        CoveLogicalType::Float32,
        CovePhysicalKind::NumCode,
    ));
    writer.push_segment(segment);
    writer
        .push_zone_stats(&ZoneStatsSection {
            entries: vec![stats],
        })
        .unwrap();
    rewrite_first_segment_page(writer.write().unwrap(), |page| {
        page.non_null_count = 6;
        page.null_count = 0;
        page.flags = (page.flags & !PAGE_FLAG_ALL_NULL) | PAGE_FLAG_ALL_NON_NULL;
    })
}

pub(super) fn cove_t_payload_elision_stats_only_all_non_null_filecode_file() -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, Vec::new())
            .with_counts(6, 0)
            .with_encoding_root(u32::MAX)
            .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL)],
    );
    let mut writer = ScanProfileCoveWriter::new(stats_only_constant_catalog(
        CoveLogicalType::UInt64,
        CovePhysicalKind::FileCode,
    ));
    writer.push_file_dictionary(&single_entry_uint64_file_dictionary());
    writer.push_segment(segment);
    writer
        .push_zone_stats(&ZoneStatsSection {
            entries: vec![filecode_constant_page_stats()],
        })
        .unwrap();
    writer.write().unwrap()
}

pub(super) fn cove_t_payload_elision_value_stream_mixed_constant_file() -> Vec<u8> {
    value_stream_elided_file(CoveEncodingKind::Constant)
}

pub(super) fn cove_t_payload_elision_value_stream_wrong_root_file() -> Vec<u8> {
    value_stream_elided_file(CoveEncodingKind::NumCode)
}

pub(super) fn cove_t_payload_elision_value_stream_missing_bitmap_file() -> Vec<u8> {
    let bytes = value_stream_elided_file_without_nulls();
    rewrite_first_segment_page(bytes, |page| {
        page.non_null_count = 4;
        page.null_count = 2;
        page.flags |= PAGE_FLAG_VALUE_STREAM_ELIDED;
    })
}

pub(super) fn cove_t_payload_elision_value_stream_missing_feature_file() -> Vec<u8> {
    clear_required_feature(
        cove_t_payload_elision_value_stream_mixed_constant_file(),
        FEATURE_PAGE_PAYLOAD_ELISION,
    )
}

pub(super) fn value_stream_elided_file(encoding: CoveEncodingKind) -> Vec<u8> {
    let mut payload = vec![0b0010_0100];
    payload.extend_from_slice(
        &ConstantPayload {
            value: 42,
            row_count: 6,
        }
        .encode(),
    );
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, payload)
            .with_counts(4, 2)
            .with_encoding_root(encoding as u32)
            .with_flags(PAGE_FLAG_VALUE_STREAM_ELIDED)],
    );
    let mut writer = ScanProfileCoveWriter::new(value_stream_elision_catalog());
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn value_stream_elided_file_without_nulls() -> Vec<u8> {
    let payload = ConstantPayload {
        value: 42,
        row_count: 6,
    }
    .encode()
    .to_vec();
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, payload)
            .with_counts(6, 0)
            .with_encoding_root(CoveEncodingKind::Constant as u32)
            .with_flags(PAGE_FLAG_VALUE_STREAM_ELIDED)],
    );
    let mut writer = ScanProfileCoveWriter::new(stats_only_constant_catalog(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
    ));
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn value_stream_elision_catalog() -> TableCatalog {
    let mut catalog =
        stats_only_constant_catalog(CoveLogicalType::Int64, CovePhysicalKind::NumCode);
    catalog.tables[0].columns[0].nullable = true;
    catalog
}

pub(super) fn cove_t_numcode_page_short_values_file() -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, 7u64.to_le_bytes().to_vec())
            .with_counts(6, 0)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(stats_only_constant_catalog(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
    ));
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn valid_constant_page_stats() -> ZoneStatsEntry {
    constant_page_stats_with_flags(ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT)
}

pub(super) fn constant_page_stats_with_flags(flags: ZoneStatFlags) -> ZoneStatsEntry {
    let scalar = StatScalar {
        kind: StatKind::Int64,
        bytes: 42i64.to_le_bytes().to_vec(),
        truncated: false,
    };
    ZoneStatsEntry {
        table_id: 1,
        segment_id: 0,
        morsel_id: 0,
        column_id: 1,
        non_null_count: 6,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: 6,
            null_count: 0,
            min: Some(scalar.clone()),
            max: Some(scalar),
            flags,
        },
        min_domain_rank: 0,
        max_domain_rank: 0,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

pub(super) fn filecode_constant_page_stats() -> ZoneStatsEntry {
    let scalar = StatScalar {
        kind: StatKind::UInt64,
        bytes: 0u64.to_le_bytes().to_vec(),
        truncated: false,
    };
    ZoneStatsEntry {
        table_id: 1,
        segment_id: 0,
        morsel_id: 0,
        column_id: 1,
        non_null_count: 6,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: 6,
            null_count: 0,
            min: Some(scalar.clone()),
            max: Some(scalar),
            flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
        },
        min_domain_rank: 0,
        max_domain_rank: 0,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

pub(super) fn wrong_scope_constant_page_stats() -> ZoneStatsEntry {
    let mut stats = valid_constant_page_stats();
    stats.morsel_id = 1;
    stats
}

pub(super) fn cove_t_payload_elision_missing_feature_file() -> Vec<u8> {
    clear_required_feature(
        cove_t_payload_elision_stats_only_all_null_file(),
        FEATURE_PAGE_PAYLOAD_ELISION,
    )
}

pub(super) fn clear_required_feature(mut bytes: Vec<u8>, feature: u64) -> Vec<u8> {
    let mut header = CoveHeaderV1::parse(&bytes).unwrap();
    header.required_features &= !feature;
    bytes[..HEADER_SIZE].copy_from_slice(&header.serialize());

    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    postscript.required_features &= !feature;
    let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
    bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
    bytes
}

pub(super) fn clear_optional_feature(mut bytes: Vec<u8>, feature: u64) -> Vec<u8> {
    let mut header = CoveHeaderV1::parse(&bytes).unwrap();
    header.optional_features &= !feature;
    bytes[..HEADER_SIZE].copy_from_slice(&header.serialize());

    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    postscript.optional_features &= !feature;
    let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
    bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
    bytes
}

pub(super) fn rewrite_first_segment_page(
    mut bytes: Vec<u8>,
    mutate: impl FnOnce(&mut ColumnPageIndexEntryV1),
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != SectionKind::TableSegmentData as u16 {
            continue;
        }
        let segment_start = section_entry.offset as usize;
        let segment_end = segment_start + section_entry.length as usize;
        let segment = TableSegmentPayloadV1::parse(&bytes[segment_start..segment_end]).unwrap();
        let column = segment.columns.first().unwrap();
        let page_start = segment_start + column.page_index_offset as usize;
        let mut page = ColumnPageIndexEntryV1::parse(&bytes[page_start..page_start + 60]).unwrap();
        mutate(&mut page);
        bytes[page_start..page_start + 60].copy_from_slice(&page.serialize());
        section_entry.crc32c = checksum::crc32c(&bytes[segment_start..segment_end]);
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE-T file did not contain TABLE_SEGMENT_DATA");
}

pub(super) fn rewrite_first_segment_page_values(mut bytes: Vec<u8>, replacement: &[u8]) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != SectionKind::TableSegmentData as u16 {
            continue;
        }
        let segment_start = section_entry.offset as usize;
        let segment_end = segment_start + section_entry.length as usize;
        let segment = TableSegmentPayloadV1::parse(&bytes[segment_start..segment_end]).unwrap();
        let column = segment.columns.first().unwrap();
        let page_index_start = segment_start + column.page_index_offset as usize;
        let mut page =
            ColumnPageIndexEntryV1::parse(&bytes[page_index_start..page_index_start + 60]).unwrap();
        let page_start = segment_start + page.page_offset as usize;
        let page_end = page_start + page.page_length as usize;
        let payload = ColumnPagePayloadV1::parse(&bytes[page_start..page_end]).unwrap();
        let value_buffer_index = payload
            .buffers
            .iter()
            .position(|buffer| buffer.kind == PageBufferKind::Values)
            .unwrap();
        let value_buffer = &payload.buffers[value_buffer_index];
        assert_eq!(value_buffer.length as usize, replacement.len());
        let value_start = page_start + value_buffer.offset as usize;
        let value_end = value_start + replacement.len();
        bytes[value_start..value_end].copy_from_slice(replacement);

        let mut updated_value_buffer = value_buffer.clone();
        updated_value_buffer.checksum = checksum::crc32c(replacement);
        let descriptor_start = page_start
            + payload.header.buffer_directory_offset as usize
            + value_buffer_index * PAGE_BUFFER_DESCRIPTOR_LEN;
        bytes[descriptor_start..descriptor_start + PAGE_BUFFER_DESCRIPTOR_LEN]
            .copy_from_slice(&updated_value_buffer.serialize());

        page.checksum = checksum::crc32c(&bytes[page_start..page_end]);
        bytes[page_index_start..page_index_start + 60].copy_from_slice(&page.serialize());
        section_entry.crc32c = checksum::crc32c(&bytes[segment_start..segment_end]);
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE-T file did not contain TABLE_SEGMENT_DATA");
}

pub(super) fn rewrite_first_section_payload(
    mut bytes: Vec<u8>,
    section_kind: SectionKind,
    mutate: impl FnOnce(&mut [u8]),
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != section_kind as u16 {
            continue;
        }
        let start = section_entry.offset as usize;
        let end = start + section_entry.length as usize;
        mutate(&mut bytes[start..end]);
        section_entry.crc32c = checksum::crc32c(&bytes[start..end]);
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE file did not contain requested section");
}

pub(super) fn corrupt_first_section_payload_crc(
    mut bytes: Vec<u8>,
    section_kind: SectionKind,
) -> Vec<u8> {
    let postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != section_kind as u16 {
            continue;
        }
        let start = section_entry.offset as usize;
        bytes[start] ^= 0x01;
        return bytes;
    }
    panic!("generated COVE file did not contain requested section");
}

pub(super) fn rewrite_first_section_kind(
    mut bytes: Vec<u8>,
    from: SectionKind,
    to: SectionKind,
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != from as u16 {
            continue;
        }
        section_entry.section_kind = to as u16;
        if to == SectionKind::VendorExtension {
            section_entry.profile = PrimaryProfile::Mixed as u8;
            section_entry.required_features = 0;
            section_entry.optional_features = 0;
        }
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE file did not contain requested section kind");
}

pub(super) fn rewrite_first_section_required_features(
    mut bytes: Vec<u8>,
    section_kind: SectionKind,
    required_features: u64,
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != section_kind as u16 {
            continue;
        }
        section_entry.required_features = required_features;
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE file did not contain requested section kind");
}

pub(super) fn rewrite_first_section_optional_features(
    mut bytes: Vec<u8>,
    section_kind: SectionKind,
    optional_features: u64,
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != section_kind as u16 {
            continue;
        }
        section_entry.optional_features = optional_features;
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE file did not contain requested section kind");
}

pub(super) fn cove_t_local_codebook_lz4_file() -> Vec<u8> {
    cove_t_local_codebook_page_codec_file(CompressionCodec::Lz4)
}

pub(super) fn cove_t_page_codec_missing_file_feature_file(codec: CompressionCodec) -> Vec<u8> {
    clear_optional_feature(
        cove_t_local_codebook_page_codec_file(codec),
        page_codec_feature_bit(codec),
    )
}

pub(super) fn cove_t_page_codec_missing_section_feature_file(codec: CompressionCodec) -> Vec<u8> {
    rewrite_first_section_optional_features(
        cove_t_local_codebook_page_codec_file(codec),
        SectionKind::TableSegmentData,
        0,
    )
}

pub(super) fn page_codec_feature_bit(codec: CompressionCodec) -> u64 {
    match codec {
        CompressionCodec::None => 0,
        CompressionCodec::Lz4 => FEATURE_CODEC_LZ4,
        CompressionCodec::Zstd => FEATURE_CODEC_ZSTD,
        _ => 0,
    }
}

pub(super) fn cove_t_local_codebook_page_codec_file(codec: CompressionCodec) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 6,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "status_code".into(),
                logical: CoveLogicalType::UInt32,
                physical: CovePhysicalKind::NumCode,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    };
    let payload = LocalCodebookPayload {
        values: LocalCodebookValues::NumCode(vec![100, 200, 300]),
        indexes: LocalIndexPayload::BitPacked(
            BitPackedPayload::pack(&[0, 1, 2, 1, 0, 2], 2).unwrap(),
        ),
    };
    let mut segment = ScanSegment::new(1, 0, 0, 6, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(6, payload.encode())
            .with_compression(codec)
            .with_encoding_root(CoveEncodingKind::LocalCodebook as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn nested_column_catalog(
    column_name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    row_count: u32,
) -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: row_count as u64,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: column_name.into(),
                logical,
                physical,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    }
}

pub(super) fn nested_column_cove_file(
    column_name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    row_count: u32,
    nested_schema: NestedSchemaSectionV1,
    payload: Vec<u8>,
) -> Vec<u8> {
    let mut segment = ScanSegment::new(1, 0, 0, row_count, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(row_count, payload)
            .with_encoding_root(CoveEncodingKind::Canonical as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(nested_column_catalog(
        column_name,
        logical,
        physical,
        row_count,
    ));
    writer.push_nested_schema(&nested_schema).unwrap();
    writer.push_segment(segment);
    writer.write().unwrap()
}

pub(super) fn nested_encoding_node(
    node_id: u16,
    encoding_kind: CoveEncodingKind,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    logical_len: u32,
    child_count: u16,
    buffer_count: u16,
) -> CoveEncodingNodeV1 {
    CoveEncodingNodeV1 {
        node_id,
        encoding_kind,
        logical_type,
        physical_kind,
        flags: 0,
        logical_len,
        child_count,
        buffer_count,
        params_offset: 0,
        params_length: 0,
        stats_id: 0,
        reserved: 0,
    }
}

pub(super) fn nested_numcode_values(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(super) fn nested_varbytes_values(values: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

pub(super) fn list_nested_schema() -> NestedSchemaSectionV1 {
    NestedSchemaSectionV1::new(vec![NestedSchemaEntryV1 {
        table_id: 1,
        column_id: 1,
        root: NestedSchemaNodeV1 {
            name: "tags".into(),
            logical: CoveLogicalType::List,
            physical: CovePhysicalKind::List,
            nullable: false,
            precision: 0,
            scale: 0,
            collation_id: 0,
            flags: 0,
            fixed_size_list_len: 0,
            children: vec![NestedSchemaNodeV1::scalar(
                "item",
                CoveLogicalType::Int32,
                CovePhysicalKind::NumCode,
                false,
            )],
        },
    }])
}

pub(super) fn struct_nested_schema() -> NestedSchemaSectionV1 {
    NestedSchemaSectionV1::new(vec![NestedSchemaEntryV1 {
        table_id: 1,
        column_id: 1,
        root: NestedSchemaNodeV1 {
            name: "address".into(),
            logical: CoveLogicalType::Struct,
            physical: CovePhysicalKind::Struct,
            nullable: false,
            precision: 0,
            scale: 0,
            collation_id: 0,
            flags: 0,
            fixed_size_list_len: 0,
            children: vec![
                NestedSchemaNodeV1::scalar(
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
                NestedSchemaNodeV1::scalar(
                    "score",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        },
    }])
}

pub(super) fn map_nested_schema() -> NestedSchemaSectionV1 {
    NestedSchemaSectionV1::new(vec![NestedSchemaEntryV1 {
        table_id: 1,
        column_id: 1,
        root: NestedSchemaNodeV1 {
            name: "labels".into(),
            logical: CoveLogicalType::Map,
            physical: CovePhysicalKind::Map,
            nullable: false,
            precision: 0,
            scale: 0,
            collation_id: 0,
            flags: 0,
            fixed_size_list_len: 0,
            children: vec![
                NestedSchemaNodeV1::scalar(
                    "key",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
                NestedSchemaNodeV1::scalar(
                    "value",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        },
    }])
}

pub(super) fn nested_list_payload(child_row_count: u32) -> Vec<u8> {
    ColumnPagePayloadV1::build_tree(
        3,
        0,
        vec![
            nested_encoding_node(
                0,
                CoveEncodingKind::Canonical,
                CoveLogicalType::List,
                CovePhysicalKind::List,
                3,
                1,
                1,
            ),
            nested_encoding_node(
                1,
                CoveEncodingKind::NumCode,
                CoveLogicalType::Int32,
                CovePhysicalKind::NumCode,
                child_row_count,
                0,
                1,
            ),
        ],
        vec![
            (
                PageBufferKind::ChildLayout,
                ListLayoutPayload {
                    layout: ListLayout {
                        offsets: vec![0, 2, 2, 5],
                    },
                    child_row_count,
                }
                .encode(),
            ),
            (
                PageBufferKind::Values,
                nested_numcode_values(&[100, 101, 102, 103, 104][..child_row_count as usize]),
            ),
        ],
    )
    .unwrap()
}

pub(super) fn nested_list_payload_with_root_null_bitmap(
    null_bitmap: Vec<u8>,
    child_row_count: u32,
) -> Vec<u8> {
    ColumnPagePayloadV1::build_tree(
        3,
        0,
        vec![
            nested_encoding_node(
                0,
                CoveEncodingKind::Canonical,
                CoveLogicalType::List,
                CovePhysicalKind::List,
                3,
                1,
                2,
            ),
            nested_encoding_node(
                1,
                CoveEncodingKind::NumCode,
                CoveLogicalType::Int32,
                CovePhysicalKind::NumCode,
                child_row_count,
                0,
                1,
            ),
        ],
        vec![
            (PageBufferKind::NullBitmap, null_bitmap),
            (
                PageBufferKind::ChildLayout,
                ListLayoutPayload {
                    layout: ListLayout {
                        offsets: vec![0, 2, 2, 5],
                    },
                    child_row_count,
                }
                .encode(),
            ),
            (
                PageBufferKind::Values,
                nested_numcode_values(&[100, 101, 102, 103, 104][..child_row_count as usize]),
            ),
        ],
    )
    .unwrap()
}

pub(super) fn nested_struct_payload(parent_null_handling_declared: bool) -> Vec<u8> {
    ColumnPagePayloadV1::build_tree(
        3,
        0,
        vec![
            nested_encoding_node(
                0,
                CoveEncodingKind::Canonical,
                CoveLogicalType::Struct,
                CovePhysicalKind::Struct,
                3,
                2,
                1,
            ),
            nested_encoding_node(
                1,
                CoveEncodingKind::VarBytes,
                CoveLogicalType::Utf8,
                CovePhysicalKind::VarBytes,
                3,
                0,
                1,
            ),
            nested_encoding_node(
                2,
                CoveEncodingKind::NumCode,
                CoveLogicalType::Int64,
                CovePhysicalKind::NumCode,
                3,
                0,
                1,
            ),
        ],
        vec![
            (
                PageBufferKind::ChildLayout,
                StructLayoutPayload {
                    layout: StructLayout {
                        field_row_counts: vec![3, 3],
                    },
                    parent_null_handling_declared,
                }
                .encode(),
            ),
            (
                PageBufferKind::Values,
                nested_varbytes_values(&["a", "b", "c"]),
            ),
            (PageBufferKind::Values, nested_numcode_values(&[1, 2, 3])),
        ],
    )
    .unwrap()
}

pub(super) fn nested_map_payload(canonical_keys: Vec<Vec<u8>>) -> Vec<u8> {
    ColumnPagePayloadV1::build_tree(
        2,
        0,
        vec![
            nested_encoding_node(
                0,
                CoveEncodingKind::Canonical,
                CoveLogicalType::Map,
                CovePhysicalKind::Map,
                2,
                2,
                1,
            ),
            nested_encoding_node(
                1,
                CoveEncodingKind::VarBytes,
                CoveLogicalType::Utf8,
                CovePhysicalKind::VarBytes,
                3,
                0,
                1,
            ),
            nested_encoding_node(
                2,
                CoveEncodingKind::NumCode,
                CoveLogicalType::Int64,
                CovePhysicalKind::NumCode,
                3,
                0,
                1,
            ),
        ],
        vec![
            (
                PageBufferKind::ChildLayout,
                MapLayoutPayload {
                    layout: MapLayout {
                        offsets: vec![0, 2, 3],
                        key_row_count: 3,
                        value_row_count: 3,
                        keys_are_scalar: true,
                        allow_duplicate_keys: false,
                        canonical_keys,
                    },
                }
                .encode(),
            ),
            (
                PageBufferKind::Values,
                nested_varbytes_values(&["env", "tier", "env"]),
            ),
            (PageBufferKind::Values, nested_numcode_values(&[10, 20, 30])),
        ],
    )
    .unwrap()
}

pub(super) fn cove_t_nested_list_valid_file() -> Vec<u8> {
    nested_column_cove_file(
        "tags",
        CoveLogicalType::List,
        CovePhysicalKind::List,
        3,
        list_nested_schema(),
        nested_list_payload(5),
    )
}

pub(super) fn cove_t_explicit_zero_null_bitmap_file() -> Vec<u8> {
    nested_column_cove_file(
        "tags",
        CoveLogicalType::List,
        CovePhysicalKind::List,
        3,
        list_nested_schema(),
        nested_list_payload_with_root_null_bitmap(vec![0], 5),
    )
}

pub(super) fn cove_t_nested_struct_valid_file() -> Vec<u8> {
    nested_column_cove_file(
        "address",
        CoveLogicalType::Struct,
        CovePhysicalKind::Struct,
        3,
        struct_nested_schema(),
        nested_struct_payload(true),
    )
}

pub(super) fn cove_t_nested_map_valid_file() -> Vec<u8> {
    nested_column_cove_file(
        "labels",
        CoveLogicalType::Map,
        CovePhysicalKind::Map,
        2,
        map_nested_schema(),
        nested_map_payload(vec![b"env".to_vec(), b"tier".to_vec(), b"env".to_vec()]),
    )
}

pub(super) fn cove_t_nested_missing_schema_file() -> Vec<u8> {
    rewrite_first_section_kind(
        cove_t_nested_list_valid_file(),
        SectionKind::NestedSchema,
        SectionKind::VendorExtension,
    )
}

pub(super) fn cove_t_nested_mismatched_schema_file() -> Vec<u8> {
    rewrite_first_section_payload(
        cove_t_nested_list_valid_file(),
        SectionKind::NestedSchema,
        |payload| {
            let name = b"tags";
            let logical_offset = 16 + 8 + 2 + name.len();
            payload[logical_offset..logical_offset + 2]
                .copy_from_slice(&(CoveLogicalType::Struct as u16).to_le_bytes());
            payload[logical_offset + 2] = CovePhysicalKind::Struct as u8;
        },
    )
}

pub(super) fn cove_t_nested_list_bad_child_count_file() -> Vec<u8> {
    nested_column_cove_file(
        "tags",
        CoveLogicalType::List,
        CovePhysicalKind::List,
        3,
        list_nested_schema(),
        nested_list_payload(4),
    )
}

pub(super) fn cove_t_nested_struct_missing_null_handling_file() -> Vec<u8> {
    nested_column_cove_file(
        "address",
        CoveLogicalType::Struct,
        CovePhysicalKind::Struct,
        3,
        struct_nested_schema(),
        nested_struct_payload(false),
    )
}

pub(super) fn cove_t_nested_map_duplicate_keys_file() -> Vec<u8> {
    nested_column_cove_file(
        "labels",
        CoveLogicalType::Map,
        CovePhysicalKind::Map,
        2,
        map_nested_schema(),
        nested_map_payload(vec![b"env".to_vec(), b"env".to_vec(), b"tier".to_vec()]),
    )
}

pub(super) fn valid_table_catalog() -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 10,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![ColumnEntry {
                column_id: 1,
                name: "active".into(),
                logical: CoveLogicalType::Bool,
                physical: CovePhysicalKind::Boolean,
                nullable: false,
                sort_order: 0,
                collation_id: 0,
                precision: 0,
                scale: 0,
                flags: 0,
            }],
        }],
    }
}

pub(super) fn duplicate_table_catalog() -> TableCatalog {
    let mut catalog = valid_table_catalog();
    let table = catalog.tables.remove(0);
    TableCatalog {
        flags: 0,
        tables: vec![table.clone(), table],
    }
}

pub(super) fn bad_pair_table_catalog() -> TableCatalog {
    let mut catalog = valid_table_catalog();
    catalog.tables[0].columns[0].physical = CovePhysicalKind::VarBytes;
    catalog
}

pub(super) fn valid_column_domain_payload() -> Vec<u8> {
    ColumnDomain::from_sorted_present_codes(&[1, 3], 4, 1, 1, CoveLogicalType::Utf8 as u16, 0, 0)
        .unwrap()
        .serialize()
        .unwrap()
}

pub(super) fn invalid_column_domain_payload() -> Vec<u8> {
    ColumnDomain {
        header: ColumnDomainHeaderV1 {
            table_or_object_id: 1,
            column_or_property_id: 1,
            logical_type: CoveLogicalType::Utf8 as u16,
            collation_id: 0,
            domain_count: 2,
            sorted_file_codes_offset: COLUMN_DOMAIN_HEADER_LEN as u64,
            file_code_to_rank_offset: (COLUMN_DOMAIN_HEADER_LEN + 8) as u64,
            flags: 0,
            checksum: 0,
        },
        sorted_file_codes: vec![5, 5],
        file_code_to_rank: Vec::new(),
    }
    .serialize()
    .unwrap()
}

pub(super) fn valid_table_segment_index() -> TableSegmentIndex {
    TableSegmentIndex {
        flags: 0,
        entries: vec![TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id: 0,
            row_start: 0,
            row_count: 10,
            morsel_count: 1,
            morsel_row_count: 4096,
            column_count: 1,
            offset: 512,
            length: 128,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }],
    }
}

pub(super) fn gap_table_segment_index() -> TableSegmentIndex {
    let mut index = valid_table_segment_index();
    index.entries[0].row_start = 5;
    index
}

pub(super) fn valid_table_segment_header() -> TableSegmentHeaderV1 {
    TableSegmentHeaderV1 {
        table_id: 1,
        segment_id: 0,
        row_start: 0,
        row_count: 10,
        morsel_count: 1,
        morsel_row_count: 4096,
        column_count: 1,
        morsel_directory_offset: TABLE_SEGMENT_HEADER_LEN as u64,
        column_directory_offset: (TABLE_SEGMENT_HEADER_LEN + 24) as u64,
        page_index_offset: (TABLE_SEGMENT_HEADER_LEN + 24) as u64,
        data_offset: (TABLE_SEGMENT_HEADER_LEN + 24) as u64,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn valid_row_morsel_directory() -> RowMorselDirectory {
    RowMorselDirectory {
        entries: vec![row_morsel(0, 0, 4096), row_morsel(1, 4096, 4)],
    }
}

pub(super) fn gap_row_morsel_directory() -> RowMorselDirectory {
    RowMorselDirectory {
        entries: vec![row_morsel(0, 0, 10), row_morsel(1, 20, 5)],
    }
}

pub(super) fn nonzero_first_row_morsel_directory() -> RowMorselDirectory {
    RowMorselDirectory {
        entries: vec![row_morsel(0, 5, 10)],
    }
}

pub(super) fn row_morsel(
    morsel_id: u32,
    first_row_in_segment: u32,
    row_count: u32,
) -> RowMorselEntryV1 {
    RowMorselEntryV1 {
        morsel_id,
        first_row_in_segment,
        row_count,
        flags: 0,
        stats_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_sort_key() -> SortKeyEntryV1 {
    SortKeyEntryV1 {
        column_id: 1,
        direction: SortDirection::Ascending,
        null_order: NullOrder::NullsLast,
        collation_id: 0,
    }
}

pub(super) fn valid_clustering_key() -> ClusteringKeyEntryV1 {
    ClusteringKeyEntryV1 {
        column_id: 1,
        clustering_strength: ClusteringStrength::PERFECT,
        reserved: [0; 3],
    }
}
