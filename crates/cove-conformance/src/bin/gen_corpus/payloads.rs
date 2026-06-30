use super::*;

pub(super) fn arrow_bitmap_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn encoding_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn error_surface_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn suite_contract_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn nested_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn pruning_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn page_codec_fixture_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn map_payload_bytes(value: Value) -> Vec<u8> {
    json_fixture_bytes(value)
}

pub(super) fn covemap_payload_value(section_kind: SectionKind, mut value: Value) -> Value {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".to_string(),
            Value::String("org.coveformat.covemap.v2".to_string()),
        );
        object.insert(
            "section_id".to_string(),
            Value::Number((section_kind as u16).into()),
        );
    }
    value
}

pub(super) fn extension_registry_entry(
    extension_kind: ExtensionKind,
    required_feature_bit: u64,
    optional_feature_bit: u64,
    payload_ref: u32,
) -> ExtensionRegistryEntry {
    ExtensionRegistryEntry {
        extension_id: 7,
        namespace: b"org.example".to_vec(),
        name: b"patient-id".to_vec(),
        version_major: 1,
        version_minor: 0,
        extension_kind,
        required_feature_bit,
        optional_feature_bit,
        fallback_kind: 0,
        fallback_ref: 0,
        payload_ref,
        checksum: 0,
    }
}

pub(super) fn extension_registry_valid_payload() -> Vec<u8> {
    ExtensionRegistry {
        flags: 0,
        entries: vec![extension_registry_entry(
            ExtensionKind::VendorMetadata,
            0,
            1 << 20,
            0,
        )],
    }
    .serialize()
    .unwrap()
}

pub(super) fn extension_registry_bad_crc_payload() -> Vec<u8> {
    let mut bytes = extension_registry_valid_payload();
    *bytes.last_mut().unwrap() ^= 0xFF;
    bytes
}

pub(super) fn extension_registry_reserved_payload() -> Vec<u8> {
    let mut bytes = ExtensionRegistry {
        flags: 0,
        entries: Vec::new(),
    }
    .serialize()
    .unwrap();
    bytes[4] = 1;
    bytes
}

pub(super) fn extension_registry_trailing_payload() -> Vec<u8> {
    let mut bytes = ExtensionRegistry {
        flags: 0,
        entries: Vec::new(),
    }
    .serialize()
    .unwrap();
    bytes.push(0);
    bytes
}

pub(super) fn extension_registry_required_unknown_payload() -> Vec<u8> {
    ExtensionRegistry {
        flags: 0,
        entries: vec![extension_registry_entry(
            ExtensionKind::VendorMetadata,
            1 << 20,
            0,
            0,
        )],
    }
    .serialize()
    .unwrap()
}

pub(super) fn extension_registry_optional_no_fallback_payload(kind: ExtensionKind) -> Vec<u8> {
    ExtensionRegistry {
        flags: 0,
        entries: vec![extension_registry_entry(kind, 0, 1 << 20, 0)],
    }
    .serialize()
    .unwrap()
}

pub(super) fn extension_logical_type_payload(collation_id: u16) -> Vec<u8> {
    ExtensionLogicalTypeV1 {
        extension_id: 7,
        base_logical_type: CoveLogicalType::Utf8,
        canonical_value_tag: ValueTag::Utf8,
        collation_id,
        flags: 0,
        arrow_extension_name: "org.example.patient-id".into(),
        metadata_payload_ref: 0,
    }
    .serialize()
    .unwrap()
}

pub(super) fn extension_index_descriptor_payload(
    proof_capability: ExtensionProofCapability,
    false_negative_policy: ExtensionFalseNegativePolicy,
) -> Vec<u8> {
    ExtensionIndexDescriptorV1 {
        extension_id: 7,
        index_kind: 100,
        key_column_count: 1,
        proof_capability,
        false_negative_policy,
        flags: 0,
        payload_ref: 0,
    }
    .serialize()
    .to_vec()
}

pub(super) fn temporal_bloom_payload() -> Vec<u8> {
    TemporalBloomIndex {
        flags: 0,
        entries: vec![TemporalBloomEntryV1 {
            segment_id: 5,
            time_bucket_start_us: 1_700_000_000_000_000,
            time_bucket_end_us: 1_700_000_060_000_000,
            filter_offset: 0,
            filter_length: 0,
            checksum: 0,
        }],
    }
    .serialize(&[vec![0xA5, 0x5A, 0xC3, 0x3C]])
    .unwrap()
}

pub(super) fn temporal_bloom_bad_crc_payload() -> Vec<u8> {
    let mut bytes = temporal_bloom_payload();
    bytes[8 + TEMPORAL_BLOOM_ENTRY_LEN - 1] ^= 0xFF;
    bytes
}

pub(super) fn temporal_bloom_filter_oob_payload() -> Vec<u8> {
    let mut bytes = temporal_bloom_payload();
    let pos = 8usize;
    let bad_offset = (bytes.len() as u64) + 8;
    bytes[pos + 20..pos + 28].copy_from_slice(&bad_offset.to_le_bytes());
    rewrite_temporal_bloom_entry_crc(&mut bytes);
    bytes
}

pub(super) fn temporal_bloom_inverted_bucket_payload() -> Vec<u8> {
    let mut bytes = temporal_bloom_payload();
    let pos = 8usize;
    bytes[pos + 4..pos + 12].copy_from_slice(&20i64.to_le_bytes());
    bytes[pos + 12..pos + 20].copy_from_slice(&10i64.to_le_bytes());
    rewrite_temporal_bloom_entry_crc(&mut bytes);
    bytes
}

pub(super) fn rewrite_temporal_bloom_entry_crc(bytes: &mut [u8]) {
    let pos = 8usize;
    let mut entry = [0u8; TEMPORAL_BLOOM_ENTRY_LEN];
    entry.copy_from_slice(&bytes[pos..pos + TEMPORAL_BLOOM_ENTRY_LEN]);
    entry[36..40].fill(0);
    let crc = checksum::crc32c(&entry);
    bytes[pos + 36..pos + 40].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn parquet_primitives_valid_file() -> Vec<u8> {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "active",
            Arc::new(BooleanArray::from(vec![true, false, true])) as ArrayRef,
        ),
        (
            "id",
            Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
        ),
        (
            "score",
            Arc::new(Float64Array::from(vec![1.5, 2.0, 3.25])) as ArrayRef,
        ),
        (
            "city",
            Arc::new(StringArray::from(vec!["sea", "lon", "par"])) as ArrayRef,
        ),
        (
            "blob",
            Arc::new(BinaryArray::from(vec![
                b"aa".as_ref(),
                b"bb".as_ref(),
                b"cc".as_ref(),
            ])) as ArrayRef,
        ),
        (
            "event_date",
            Arc::new(Date32Array::from(vec![19000, 19001, 19002])) as ArrayRef,
        ),
        (
            "ts_us",
            Arc::new(TimestampMicrosecondArray::from(vec![1000, 2000, 3000])) as ArrayRef,
        ),
    ])
    .unwrap();
    parquet_file_bytes(&batch)
}

pub(super) fn parquet_nullable_valid_file() -> Vec<u8> {
    let batch = RecordBatch::try_from_iter(vec![(
        "id",
        Arc::new(Int64Array::from(vec![Some(1), None, Some(3)])) as ArrayRef,
    )])
    .unwrap();
    parquet_file_bytes(&batch)
}

pub(super) fn parquet_nested_unsupported_file() -> Vec<u8> {
    let mut builder = ListBuilder::new(Time32MillisecondBuilder::new());
    builder.values().append_value(1_000);
    builder.values().append_value(2_000);
    builder.append(true);
    builder.append(false);
    let batch = RecordBatch::try_from_iter(vec![("times", Arc::new(builder.finish()) as ArrayRef)])
        .unwrap();
    parquet_file_bytes(&batch)
}

pub(super) fn parquet_file_bytes(batch: &RecordBatch) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = ArrowWriter::try_new(&mut cursor, batch.schema(), None).unwrap();
        writer.write(batch).unwrap();
        writer.close().unwrap();
    }
    cursor.into_inner()
}

pub(super) fn run_end_payload_bytes(values: &[i64], run_ends: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for run_end in run_ends {
        out.extend_from_slice(&run_end.to_le_bytes());
    }
    out
}

pub(super) fn sparse_payload_bytes(row_count: u32, fill: i64, overrides: &[(u32, i64)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&fill.to_le_bytes());
    out.extend_from_slice(&(overrides.len() as u32).to_le_bytes());
    for (position, value) in overrides {
        out.extend_from_slice(&position.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(super) fn patched_base_payload_bytes(base: &[i64], patches: &[(u32, i64)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(base.len() as u32).to_le_bytes());
    for value in base {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&(patches.len() as u32).to_le_bytes());
    for (position, value) in patches {
        out.extend_from_slice(&position.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub(super) fn varbytes_payload(values: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    out
}

pub(super) fn assert_error_code_coverage(entries: &[Value]) {
    let covered = entries
        .iter()
        .filter_map(|entry| entry.get("error_code").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let missing = CoveError::ALL_SPEC_CODES
        .iter()
        .copied()
        .filter(|code| !covered.contains(code))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "manifest is missing Spec §76 error_code coverage for: {}",
        missing.join(", ")
    );
}

pub(super) fn cove_file_with_section(
    required_features: u64,
    section_kind: SectionKind,
    profile: PrimaryProfile,
    section_required_features: u64,
    data: Vec<u8>,
) -> Vec<u8> {
    profile_cove_file(
        required_features,
        0,
        section_kind,
        profile,
        section_required_features,
        0,
        data,
    )
}

pub(super) fn semantic_profile_cove_file(
    primary_profile: PrimaryProfile,
    required_features: u64,
    optional_features: u64,
    sections: Vec<SectionPayload>,
) -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = primary_profile as u8;
    writer.required_features = required_features;
    writer.optional_features = optional_features;
    writer.sections = sections;
    writer.write().unwrap()
}

pub(super) fn profile_cove_file(
    required_features: u64,
    optional_features: u64,
    section_kind: SectionKind,
    profile: PrimaryProfile,
    section_required_features: u64,
    section_optional_features: u64,
    data: Vec<u8>,
) -> Vec<u8> {
    semantic_profile_cove_file(
        PrimaryProfile::Mixed,
        required_features,
        optional_features,
        vec![SectionPayload {
            section_kind: section_kind as u16,
            profile: profile as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: section_required_features,
            optional_features: section_optional_features,
            data,
        }],
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compressed_profile_cove_file(
    required_features: u64,
    optional_features: u64,
    section_kind: SectionKind,
    profile: PrimaryProfile,
    section_required_features: u64,
    section_optional_features: u64,
    compression: CompressionCodec,
    data: Vec<u8>,
) -> Vec<u8> {
    semantic_profile_cove_file(
        PrimaryProfile::Mixed,
        required_features,
        optional_features,
        vec![SectionPayload {
            section_kind: section_kind as u16,
            profile: profile as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: compression as u8,
            alignment_log2: 0,
            required_features: section_required_features,
            optional_features: section_optional_features,
            data,
        }],
    )
}

pub(super) fn cove_with_unknown_optional_feature() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.optional_features = 1u64 << 63;
    writer.write().unwrap()
}

pub(super) fn cove_with_unknown_required_feature() -> Vec<u8> {
    let writer = MinimalCoveWriter::new();
    let mut bytes = writer.write().unwrap();
    rewrite_cove_feature_bits(&mut bytes, FEATURE_TABLE_PROFILE | (1u64 << 63), 0);
    bytes
}

pub(super) fn cove_with_unknown_section_required_feature() -> Vec<u8> {
    let bytes = cove_file_with_section(
        0,
        SectionKind::VendorExtension,
        PrimaryProfile::Mixed,
        0,
        Vec::new(),
    );
    rewrite_first_section_required_features(bytes, SectionKind::VendorExtension, 1u64 << 63)
}

pub(super) fn feature_scope_use_fixture(
    path: &str,
    expect: &str,
    error_code: Option<&str>,
    requested_profile: Option<u8>,
    requested_operation: Option<OperationKindV2>,
    needed_sections: &[u32],
    needed_pages: &[(u32, u64)],
) -> Value {
    let mut entry = fixture(
        path,
        "feature_scope_use_case",
        expect,
        error_code,
        &["§11.1", "§11.2", "§11.3", "§73"],
    );
    if let Some(profile) = requested_profile {
        entry["requested_profile"] = json!(profile);
    }
    if let Some(operation) = requested_operation {
        entry["requested_operation"] = json!(operation as u16);
    }
    if !needed_sections.is_empty() {
        entry["needed_sections"] = json!(needed_sections);
    }
    if !needed_pages.is_empty() {
        entry["needed_pages"] = json!(needed_pages
            .iter()
            .map(|(section_id, target)| json!([section_id, target]))
            .collect::<Vec<_>>());
    }
    entry
}

pub(super) const UNKNOWN_EXTENDED_FEATURE: u64 = 1;

pub(super) fn cove_with_operation_scoped_unknown_feature() -> Vec<u8> {
    cove_with_scoped_unknown_feature(scoped_feature_entry(
        FeatureScopeV2::OperationRequired,
        PrimaryProfile::TableScan as u8,
        OperationKindV2::CoveragePlanning,
        0,
        u64::MAX,
    ))
}

pub(super) fn cove_with_section_scoped_unknown_feature() -> Vec<u8> {
    cove_with_scoped_unknown_feature(scoped_feature_entry(
        FeatureScopeV2::SectionRequired,
        PrimaryProfile::TableScan as u8,
        OperationKindV2::None,
        3,
        u64::MAX,
    ))
}

pub(super) fn cove_with_page_scoped_unknown_feature() -> Vec<u8> {
    cove_with_scoped_unknown_feature(scoped_feature_entry(
        FeatureScopeV2::PageRequired,
        PrimaryProfile::TableScan as u8,
        OperationKindV2::None,
        3,
        cove_column_page_target_ref(11, 12),
    ))
}

pub(super) fn scoped_feature_entry(
    scope: FeatureScopeV2,
    profile: u8,
    operation_kind: OperationKindV2,
    section_id: u32,
    target_local_ref: u64,
) -> ProfileCapabilityEntryV2 {
    ProfileCapabilityEntryV2 {
        profile,
        scope,
        operation_kind,
        global_feature_word_index: 1,
        required_mask: UNKNOWN_EXTENDED_FEATURE,
        optional_mask: 0,
        section_id,
        target_local_ref,
        flags: 0,
        reserved: 0,
        checksum: 0,
    }
}

pub(super) fn cove_with_scoped_unknown_feature(entry: ProfileCapabilityEntryV2) -> Vec<u8> {
    let required_features = FEATURE_TABLE_PROFILE | FEATURE_EXTENDED_FEATURE_SET;
    let extended = ExtendedFeatureSetV2 {
        header: ExtendedFeatureSetHeaderV2 {
            word_count: 2,
            required_word_count: 2,
            optional_word_count: 1,
            flags: 0,
            checksum: 0,
        },
        required_feature_words: vec![required_features, UNKNOWN_EXTENDED_FEATURE],
        optional_feature_words: vec![0],
    }
    .serialize()
    .unwrap();
    let matrix = ProfileCapabilityMatrixV2 {
        header: ProfileCapabilityMatrixHeaderV2 {
            magic: *b"PCM2",
            version_major: 2,
            header_len: ProfileCapabilityMatrixHeaderV2::LEN as u16,
            entry_len: ProfileCapabilityEntryV2::LEN as u16,
            reserved: 0,
            entry_count: 1,
            flags: 0,
            entries_offset: ProfileCapabilityMatrixHeaderV2::LEN as u64,
            entries_length: ProfileCapabilityEntryV2::LEN as u64,
            checksum: 0,
        },
        entries: vec![entry],
    }
    .serialize()
    .unwrap();
    let mut writer = MinimalCoveWriter::new();
    writer.required_features = required_features;
    writer.sections.extend([
        SectionPayload {
            section_kind: SectionKind::ExtendedFeatureSet as u16,
            profile: 0,
            flags: 0,
            item_count: 0,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: 0,
            data: extended,
        },
        SectionPayload {
            section_kind: SectionKind::ProfileCapabilityMatrix as u16,
            profile: 0,
            flags: 0,
            item_count: 0,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: 0,
            data: matrix,
        },
        SectionPayload {
            section_kind: SectionKind::VendorExtension as u16,
            profile: 0,
            flags: 0,
            item_count: 0,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: 0,
            data: Vec::new(),
        },
    ]);
    let mut bytes = writer.write().unwrap();
    set_cove_scoped_feature_section_ids(&mut bytes, 1, 2);
    bytes
}
