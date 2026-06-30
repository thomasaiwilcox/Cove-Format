use super::*;

pub(super) fn codec_descriptor_payload() -> Vec<u8> {
    CodecExtensionDescriptorV2 {
        codec_id: 1,
        namespace: "org.cove".into(),
        name: "candidate-fsst-like".into(),
        version_major: 1,
        version_minor: 0,
        codec_family: 0,
        logical_type_mask: 1,
        physical_kind_mask: 1,
        requirement: CodecRequirementV2::OptionalWithFallback,
        fallback_policy: CodecFallbackPolicyV2::CoreEncodingPayloadPresent,
        parameter_schema_kind: 0,
        flags: 0,
        specification_status: CodecSpecificationStatusV2::Candidate,
        required_feature_bit: 0,
        optional_feature_bit: FEATURE_CODEC_EXTENSION_REGISTRY,
        spec_digest_algorithm: 1,
        spec_digest: vec![0x42; 32],
        conformance_vector_ref: u32::MAX,
        fallback_ref: 1,
        private_payload_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
}

pub(super) fn stable_fsst_descriptor_payload() -> Vec<u8> {
    CodecExtensionDescriptorV2 {
        codec_id: 1,
        namespace: "org.coveformat.codec".into(),
        name: "fsst-utf8".into(),
        version_major: 2,
        version_minor: 0,
        codec_family: 0,
        logical_type_mask: 1u64 << (CoveLogicalType::Utf8 as u32),
        physical_kind_mask: 1u64 << (CovePhysicalKind::VarBytes as u32),
        requirement: CodecRequirementV2::OptionalWithFallback,
        fallback_policy: CodecFallbackPolicyV2::CoreEncodingPayloadPresent,
        parameter_schema_kind: 0,
        flags: 0,
        specification_status: CodecSpecificationStatusV2::StableRegistered,
        required_feature_bit: 0,
        optional_feature_bit: FEATURE_REGISTERED_ENCODINGS,
        spec_digest_algorithm: 1,
        spec_digest: b"COVE-FSST-UTF8-V2-SPEC-DIGEST".to_vec(),
        conformance_vector_ref: u32::MAX,
        fallback_ref: 0,
        private_payload_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
}

pub(super) fn cfs2_payload(values: &[&[u8]]) -> Vec<u8> {
    let mut value_bytes = Vec::new();
    let mut offsets = Vec::with_capacity(values.len() + 1);
    offsets.push(0u32);
    for value in values {
        let next = offsets.last().copied().unwrap() + value.len() as u32;
        offsets.push(next);
        value_bytes.extend_from_slice(value);
    }
    let null_bitmap = vec![0u8; values.len().div_ceil(8)];
    let offsets_len = offsets.len() * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"CFS2");
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    out.extend_from_slice(&(null_bitmap.len() as u32).to_le_bytes());
    out.extend_from_slice(&(offsets_len as u32).to_le_bytes());
    out.extend_from_slice(&null_bitmap);
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&value_bytes);
    out
}

pub(super) fn corrupt_registered_envelope_preserving_buffer_crc(bytes: &mut [u8]) {
    let descriptor_offset = COLUMN_PAGE_PAYLOAD_HEADER_LEN + COVE_ENCODING_NODE_LEN;
    let other_offset = u64::from_le_bytes(
        bytes[descriptor_offset + 8..descriptor_offset + 16]
            .try_into()
            .unwrap(),
    ) as usize;
    let other_length = u64::from_le_bytes(
        bytes[descriptor_offset + 16..descriptor_offset + 24]
            .try_into()
            .unwrap(),
    ) as usize;
    bytes[other_offset] ^= 1;
    let checksum = checksum::crc32c(&bytes[other_offset..other_offset + other_length]);
    bytes[descriptor_offset + 24..descriptor_offset + 28].copy_from_slice(&checksum.to_le_bytes());
    debug_assert_eq!(descriptor_offset + PAGE_BUFFER_DESCRIPTOR_LEN, other_offset);
}

pub(super) fn registered_encoding_envelope() -> RegisteredEncodingEnvelopeV2 {
    RegisteredEncodingEnvelopeV2 {
        codec_id: 1,
        codec_version_major: 1,
        codec_version_minor: 0,
        logical_len: 4,
        non_null_count: 3,
        params_offset: 72,
        params_length: 8,
        encoded_payload_offset: 80,
        encoded_payload_length: 32,
        fallback_payload_offset: 112,
        fallback_payload_length: 16,
        decoded_uncompressed_length: 64,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn registered_encoding_envelope_payload() -> Vec<u8> {
    registered_encoding_envelope().serialize().unwrap().to_vec()
}

pub(super) fn registered_encoding_envelope_bad_fallback_payload() -> Vec<u8> {
    let mut bytes = registered_encoding_envelope_payload();
    bytes[40..48].copy_from_slice(&0u64.to_le_bytes());
    bytes[48..56].copy_from_slice(&16u64.to_le_bytes());
    rewrite_fixed_record_checksum(&mut bytes, 68);
    bytes
}

pub(super) fn layout_fixture_table() -> TableEntry {
    TableEntry {
        table_id: 1,
        namespace: "public".into(),
        name: "events".into(),
        row_count: 2048,
        primary_sort_key_count: 0,
        clustering_key_count: 0,
        flags: 0,
        columns: vec![ColumnEntry {
            column_id: 1,
            name: "id".into(),
            logical: CoveLogicalType::Int64,
            physical: CovePhysicalKind::NumCode,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }],
    }
}

pub(super) fn layout_fixture_segments() -> Vec<TableSegmentIndexEntryV1> {
    vec![
        TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id: 1,
            row_start: 0,
            row_count: 1024,
            morsel_count: 4,
            morsel_row_count: 256,
            column_count: 1,
            offset: 4096,
            length: 8192,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        },
        TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id: 2,
            row_start: 1024,
            row_count: 1024,
            morsel_count: 4,
            morsel_row_count: 256,
            column_count: 1,
            offset: 12288,
            length: 8192,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        },
    ]
}

pub(super) fn layout_plan_payload() -> Vec<u8> {
    let table = layout_fixture_table();
    let segments = layout_fixture_segments();
    let splits = build_default_scan_split_index(&table, &segments).unwrap();
    build_default_layout_plan(&table, &segments, Some(&splits))
        .unwrap()
        .serialize()
        .unwrap()
}

pub(super) fn fast_metadata_entry() -> FastMetadataIndexEntryV2 {
    FastMetadataIndexEntryV2 {
        target_kind: 1,
        flags: 0,
        table_id: 1,
        column_id: 1,
        segment_id: 1,
        morsel_id: 1,
        section_id: 1,
        local_id: 1,
        offset: 128,
        length: 64,
        checksum_or_crc32c: 0,
        reserved: 0,
    }
}

pub(super) fn fast_metadata_index_payload() -> Vec<u8> {
    FastMetadataIndexV2 {
        header: FastMetadataIndexHeaderV2 {
            entry_count: 1,
            entry_len: FastMetadataIndexEntryV2::LEN as u16,
            index_kind: 0,
            flags: 0,
            entries_offset: FastMetadataIndexHeaderV2::LEN as u64,
            entries_length: FastMetadataIndexEntryV2::LEN as u64,
            checksum: 0,
        },
        entries: vec![fast_metadata_entry()],
    }
    .serialize()
    .unwrap()
}

pub(super) fn fast_metadata_index_duplicate_payload() -> Vec<u8> {
    let header = FastMetadataIndexHeaderV2 {
        entry_count: 2,
        entry_len: FastMetadataIndexEntryV2::LEN as u16,
        index_kind: 0,
        flags: 0,
        entries_offset: FastMetadataIndexHeaderV2::LEN as u64,
        entries_length: (2 * FastMetadataIndexEntryV2::LEN) as u64,
        checksum: 0,
    };
    let entry = fast_metadata_entry().serialize().unwrap();
    let mut out = header.serialize().unwrap().to_vec();
    out.extend_from_slice(&entry);
    out.extend_from_slice(&entry);
    out
}

pub(super) fn page_cluster_entry(cluster_id: u32) -> PageClusterEntryV2 {
    PageClusterEntryV2 {
        cluster_id,
        section_id: 1,
        offset: 256 + (u64::from(cluster_id) * 128),
        length: 128,
        table_id: 1,
        segment_id: 1,
        first_morsel_id: cluster_id - 1,
        morsel_count: 1,
        first_page_ref: cluster_id - 1,
        page_count: 1,
        preferred_read_alignment: 4096,
        preferred_coalesce_distance: 65536,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn page_cluster_directory_payload() -> Vec<u8> {
    PageClusterDirectoryV2 {
        header: PageClusterDirectoryHeaderV2 {
            cluster_count: 1,
            flags: 0,
            checksum: 0,
        },
        entries: vec![page_cluster_entry(1)],
    }
    .serialize()
    .unwrap()
}

pub(super) fn page_cluster_directory_duplicate_payload() -> Vec<u8> {
    let header = PageClusterDirectoryHeaderV2 {
        cluster_count: 2,
        flags: 0,
        checksum: 0,
    };
    let entry = page_cluster_entry(1).serialize().unwrap();
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&entry);
    out.extend_from_slice(&entry);
    out
}

pub(super) fn layout_root_node() -> LayoutPlanNodeV2 {
    LayoutPlanNodeV2 {
        node_id: 1,
        parent_node_id: u32::MAX,
        node_kind: 0,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        segment_id: u32::MAX,
        first_morsel_id: 0,
        morsel_count: 0,
        row_start: 0,
        row_count: 0,
        section_id: 0,
        cluster_id: 0,
        first_child_index: 0,
        child_count: 0,
        stats_ref: u32::MAX,
        split_ref: u32::MAX,
        checksum: 0,
    }
}

pub(super) fn layout_plan_duplicate_node_payload() -> Vec<u8> {
    let header = LayoutPlanHeaderV2 {
        layout_id: 2,
        node_count: 2,
        root_node_id: 1,
        flags: 0,
        checksum: 0,
    };
    let node = layout_root_node();
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&node.serialize());
    out.extend_from_slice(&node.serialize());
    out
}

pub(super) fn layout_plan_bad_child_range_payload() -> Vec<u8> {
    let header = LayoutPlanHeaderV2 {
        layout_id: 3,
        node_count: 1,
        root_node_id: 1,
        flags: 0,
        checksum: 0,
    };
    let mut node = layout_root_node();
    node.first_child_index = 1;
    node.child_count = 1;
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&node.serialize());
    out
}

pub(super) fn scan_split_index_payload() -> Vec<u8> {
    build_default_scan_split_index(&layout_fixture_table(), &layout_fixture_segments())
        .unwrap()
        .serialize()
        .unwrap()
}

pub(super) fn scan_split_index_duplicate_payload() -> Vec<u8> {
    let mut bytes = scan_split_index_payload();
    let second_entry = ScanSplitIndexHeaderV2::LEN + ScanSplitEntryV2::LEN;
    bytes[second_entry..second_entry + 4].copy_from_slice(&1u32.to_le_bytes());
    rewrite_fixed_record_checksum(
        &mut bytes[second_entry..second_entry + ScanSplitEntryV2::LEN],
        72,
    );
    bytes
}

pub(super) fn runtime_hints_payload() -> Vec<u8> {
    runtime_hints_payload_with_required(false)
}

pub(super) fn runtime_hints_payload_with_required(required: bool) -> Vec<u8> {
    RuntimeCompatibilityHintV2 {
        hint_id: 1,
        hint_kind: RuntimeHintKindV2::EngineAdapter,
        required,
        flags: 0,
        namespace: if required {
            "example.invalid".into()
        } else {
            "org.cove".into()
        },
        name: if required {
            "not-registered".into()
        } else {
            "datafusion".into()
        },
        version_major: 1,
        version_minor: 0,
        payload_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
}

pub(super) fn runtime_operation_case_payload() -> Vec<u8> {
    json_fixture_bytes(json!({
        "hints": [
            {
                "hint_id": 1,
                "kind": "engine_adapter",
                "required": true,
                "namespace": "org.cove",
                "name": "datafusion",
                "version_major": 1,
                "version_minor": 0
            },
            {
                "hint_id": 2,
                "kind": "predicate_kernel",
                "required": true,
                "namespace": "org.cove",
                "name": "missing-kernel",
                "version_major": 1,
                "version_minor": 0
            },
            {
                "hint_id": 3,
                "kind": "codec_registry",
                "required": false,
                "namespace": "org.cove",
                "name": "optional-codec",
                "version_major": 1,
                "version_minor": 0
            }
        ],
        "supported": [
            {
                "kind": "engine_adapter",
                "namespace": "org.cove",
                "name": "datafusion",
                "version_major": 1,
                "version_minor": 0
            }
        ],
        "expect_unsupported_required_hint_ids": [2]
    }))
}

pub(super) fn coverage_provider_descriptor(
    provider_id: u32,
    under_inclusive: bool,
) -> CoverageProviderDescriptorV2 {
    CoverageProviderDescriptorV2 {
        provider_id,
        provider_kind: 1,
        profile: PrimaryProfile::TableScan as u8,
        granularity: CoverageGranularityV2::Page,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: if under_inclusive {
            CoverageExactnessV2::ApproximateMayUnderInclude
        } else {
            CoverageExactnessV2::Exact
        },
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: 1,
        referenced_path_ref: u32::MAX,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref: 1,
        predicate_form_ref: u32::MAX,
        producer_ref: u32::MAX,
        checksum: 0,
    }
}

pub(super) fn coverage_provider_registry_payload(
    under_inclusive: bool,
    duplicate: bool,
) -> Vec<u8> {
    let first = coverage_provider_descriptor(1, under_inclusive).serialize();
    let second_id = if duplicate { 1 } else { 2 };
    let second = coverage_provider_descriptor(second_id, false).serialize();
    let mut out = first.to_vec();
    if duplicate {
        out.extend_from_slice(&second);
    }
    out
}

pub(super) fn coverage_set_payload(under_inclusive: bool) -> Vec<u8> {
    coverage_set_payload_with_predicate(under_inclusive, u32::MAX)
}

pub(super) fn coverage_set_payload_with_predicate(
    under_inclusive: bool,
    predicate_form_ref: u32,
) -> Vec<u8> {
    let exactness = if under_inclusive {
        CoverageExactnessV2::ApproximateMayUnderInclude
    } else {
        CoverageExactnessV2::Exact
    };
    let header = CoverageSetHeaderV2 {
        coverage_set_id: 1,
        provider_id: 1,
        granularity: CoverageGranularityV2::Dataset,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness,
        flags: 0,
        predicate_form_ref,
        snapshot_validity_ref: 1,
        total_fragment_count: 1,
        covered_fragment_count: 1,
        required_fragment_count_estimate: 1,
        coverage_degree_ppm: 1_000_000,
        tightness_degree_ppm: 1_000_000,
        entries_offset: CoverageSetHeaderV2::LEN as u64,
        entries_length: CoverageSetEntryV2::LEN as u64,
        checksum: 0,
    };
    let entry = CoverageSetEntryV2 {
        target_kind: CoverageGranularityV2::Dataset,
        flags: 0,
        file_ref: u32::MAX,
        table_id: u32::MAX,
        segment_id: u32::MAX,
        morsel_id: u32::MAX,
        page_ref: u32::MAX,
        object_type_id: u32::MAX,
        path_ref: u32::MAX,
        dimensional_bucket_ref: u32::MAX,
        row_start: 0,
        row_count: 0,
        row_ordinal_bitmap_ref: u32::MAX,
        byte_range_ref: u32::MAX,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&entry.serialize());
    out
}

pub(super) fn coverage_proof_record(
    proof_id: u32,
    coverage_set_bytes: &[u8],
) -> CoverageProofRecordV2 {
    CoverageProofRecordV2 {
        proof_id,
        provider_id: 1,
        coverage_set_id: 1,
        predicate_form_ref: 1,
        proof_kind: CoverageProofKindV2::ZoneMap,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::Dataset,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref: 1,
        coverage_set_checksum: coverage_set_payload_checksum(coverage_set_bytes),
        proof_payload_ref: u32::MAX,
        checksum: 0,
    }
}

pub(super) fn coverage_proof_record_payload() -> Vec<u8> {
    let coverage_set = coverage_set_payload_with_predicate(false, 1);
    coverage_proof_record(1, &coverage_set)
        .serialize()
        .unwrap()
        .to_vec()
}

pub(super) fn coverage_proof_record_duplicate_payload() -> Vec<u8> {
    let mut bytes = coverage_proof_record_payload();
    bytes.extend_from_slice(&coverage_proof_record_payload());
    bytes
}

pub(super) fn coverage_proof_record_underinclusive_payload() -> Vec<u8> {
    let mut bytes = coverage_proof_record_payload();
    bytes[19] = CoverageExactnessV2::ApproximateMayUnderInclude as u8;
    rewrite_fixed_record_checksum(&mut bytes, 36);
    bytes
}

pub(super) fn coverage_proof_record_bad_null_semantics_payload() -> Vec<u8> {
    let mut bytes = coverage_proof_record_payload();
    bytes[21] = 255;
    rewrite_fixed_record_checksum(&mut bytes, 36);
    bytes
}

pub(super) enum CoverageProofFixtureCase {
    Valid,
    BadCoverageSetChecksum,
    StaleSnapshot,
}

pub(super) fn coverage_proof_case_payload(case: CoverageProofFixtureCase) -> Vec<u8> {
    let coverage_set = coverage_set_payload_with_predicate(false, 1);
    let mut record = coverage_proof_record(1, &coverage_set);
    let selected_snapshot_validity_ref = match case {
        CoverageProofFixtureCase::Valid | CoverageProofFixtureCase::BadCoverageSetChecksum => 1,
        CoverageProofFixtureCase::StaleSnapshot => 2,
    };
    if matches!(case, CoverageProofFixtureCase::BadCoverageSetChecksum) {
        record.coverage_set_checksum ^= 1;
    }
    json_fixture_bytes(json!({
        "coverage_set": coverage_set,
        "proof_record": record.serialize().unwrap().to_vec(),
        "selected_snapshot_validity_ref": selected_snapshot_validity_ref,
        "expect_pruning_safe": true,
    }))
}

pub(super) fn predicate_normal_form() -> PredicateNormalFormV2 {
    PredicateNormalFormV2 {
        predicate_form_id: 1,
        form_kind: PredicateFormKindV2::IntervalPredicateForm,
        flags: 0,
        logical_context_ref: 1,
        payload_offset: 0,
        payload_length: 0,
        checksum: 0,
    }
}

pub(super) fn predicate_normal_form_payload() -> Vec<u8> {
    predicate_normal_form().serialize().unwrap().to_vec()
}

pub(super) fn predicate_normal_form_bad_operand_ref_payload() -> Vec<u8> {
    let mut bytes = predicate_normal_form_payload();
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    rewrite_fixed_record_checksum(&mut bytes, 28);
    bytes
}

pub(super) fn interval_predicate() -> IntervalPredicateV2 {
    IntervalPredicateV2 {
        column_or_path_ref: 1,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        null_policy: IntervalNullPolicyV2::SqlUnknown,
        bound_kind: IntervalBoundKindV2::LowerUpper,
        flags: 0,
        lower_inclusive: 1,
        upper_inclusive: 1,
        reserved: 0,
        lower_value_ref: 1,
        upper_value_ref: 2,
        checksum: 0,
    }
}

pub(super) fn interval_predicate_payload() -> Vec<u8> {
    interval_predicate().serialize().unwrap().to_vec()
}

pub(super) fn interval_predicate_bad_bounds_payload() -> Vec<u8> {
    let mut bytes = interval_predicate_payload();
    bytes[16..20].copy_from_slice(&9u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&2u32.to_le_bytes());
    rewrite_fixed_record_checksum(&mut bytes, 24);
    bytes
}

pub(super) fn coverage_plan_candidate(candidate_id: u32) -> CoveragePlanCandidateV2 {
    CoveragePlanCandidateV2 {
        candidate_id,
        predicate_fragment_ref: 1,
        provider_id: 1,
        provider_type: 1,
        flags: COVERAGE_PLAN_FLAG_PRUNING_CANDIDATE,
        estimated_lookup_cost_ns: 10,
        estimated_coverage_size_bytes: 1024,
        estimated_read_cost_ns: 20,
        estimated_decode_cost_ns: 30,
        estimated_materialisation_cost_ns: 40,
        baseline_scan_cost_estimate_ns: 1000,
        max_allowed_estimated_cost_ns: 200,
        confidence_ppm: 900_000,
        calibration_epoch: 1,
        observed_error_bounds_ref: u32::MAX,
        fallback_policy: CoverageFallbackPolicyV2::FullScanFallback,
        reserved: [0; 3],
        checksum: 0,
    }
}

pub(super) fn coverage_plan_candidate_payload() -> Vec<u8> {
    coverage_plan_candidate(1).serialize().unwrap().to_vec()
}

pub(super) fn coverage_plan_candidate_duplicate_payload() -> Vec<u8> {
    let mut out = coverage_plan_candidate_payload();
    out.extend_from_slice(&coverage_plan_candidate_payload());
    out
}

pub(super) fn coverage_plan_candidate_underinclusive_payload() -> Vec<u8> {
    let mut bytes = coverage_plan_candidate_payload();
    let flags = COVERAGE_PLAN_FLAG_PRUNING_CANDIDATE | COVERAGE_PLAN_FLAG_MAY_UNDER_INCLUDE;
    bytes[14..16].copy_from_slice(&flags.to_le_bytes());
    rewrite_fixed_record_checksum(&mut bytes, 92);
    bytes
}

pub(super) fn covi_empty_payload() -> Vec<u8> {
    CoviArtifactV2::new_empty([0x11; 16], [0x22; 16])
        .serialize_empty()
        .unwrap()
}

pub(super) fn covi_single_section_payload() -> Vec<u8> {
    CoviArtifactV2::serialize_single_section(
        [0x11; 16],
        [0x22; 16],
        CoviSectionKindV2::StringTable,
        b"org.cove\0",
    )
    .unwrap()
}

pub(super) fn index_capability() -> IndexCapabilityV2 {
    IndexCapabilityV2 {
        capability_id: 1,
        index_root_id: 1,
        flags: 0,
        supports_eq: 1,
        supports_range: 0,
        supports_membership: 1,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 1,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 1,
        supports_join_coverage: 0,
        supports_index_only: 1,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 1,
        coverage_provider_ref: 1,
        checksum: 0,
    }
}

pub(super) fn index_capability_payload() -> Vec<u8> {
    index_capability().serialize().unwrap().to_vec()
}

pub(super) fn index_capability_bad_boolean_payload() -> Vec<u8> {
    let mut bytes = index_capability_payload();
    bytes[12] = 2;
    rewrite_fixed_record_checksum(&mut bytes, 36);
    bytes
}

pub(super) fn index_only_capability() -> IndexOnlyCapabilityV2 {
    IndexOnlyCapabilityV2 {
        capability_id: 1,
        aggregate_kind: 0,
        predicate_supported: 1,
        exactness: IndexCapabilityExactnessV2::Exact,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref: 1,
        required_visibility_overlay_ref: u32::MAX,
        checksum: 0,
    }
}

pub(super) fn index_only_capability_payload() -> Vec<u8> {
    index_only_capability().serialize().unwrap().to_vec()
}

pub(super) fn index_only_capability_bad_aggregate_payload() -> Vec<u8> {
    let mut bytes = index_only_capability_payload();
    bytes[4..6].copy_from_slice(&99u16.to_le_bytes());
    rewrite_fixed_record_checksum(&mut bytes, 18);
    bytes
}

pub(super) struct CoviHardeningCase {
    pub(super) path: &'static str,
    pub(super) expect: &'static str,
    pub(super) error_code: Option<&'static str>,
    pub(super) sections: &'static [&'static str],
    pub(super) payload: Vec<u8>,
}

pub(super) fn covi_hardening_cases() -> Vec<CoviHardeningCase> {
    let valid_lookup = covi_case_payload(
        covi_row_range_artifact(),
        "valid",
        json!({"kind": "lookup_eq", "table_id": 1, "column_id": 10, "key": key_bytes(1)}),
        json!({"expect_row_range_count": 1}),
    );
    let posting_reps = covi_case_payload(
        covi_all_posting_representations_artifact(),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
    );
    let row_ordinals = covi_case_payload(
        covi_row_ordinal_artifact(false),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
    );
    let index_only = covi_case_payload(
        covi_index_only_artifact(IndexCapabilityExactnessV2::Exact),
        "valid",
        json!({"kind": "index_only", "table_id": 1, "column_id": 10, "aggregate_kind": "count"}),
        json!({"expect_row_count": 5}),
    );
    let stale_digest = covi_case_payload_with_context(
        covi_row_range_artifact(),
        "stale_ignored",
        json!({"kind": "validate"}),
        json!({}),
        json!({"file_digest": vec![0xEEu8; 32]}),
    );
    let bad_digest = covi_case_payload_with_context(
        covi_row_range_artifact(),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
        json!({"file_digest": vec![0xEEu8; 32]}),
    );
    let bad_root_enum = covi_case_payload(
        covi_bad_root_logical_type_artifact(),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
    );
    let malformed_unsorted_key = covi_case_payload(
        covi_malformed_unsorted_key_artifact(),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
    );
    let schema_stale = covi_case_payload_with_context(
        covi_row_range_artifact_with_snapshot_refs(7, u32::MAX, u32::MAX),
        "stale_ignored",
        json!({"kind": "validate"}),
        json!({}),
        json!({"schema_fingerprint_ref": 8}),
    );
    let map_stale = covi_case_payload_with_context(
        covi_row_range_artifact_with_snapshot_refs(u32::MAX, 3, u32::MAX),
        "stale_ignored",
        json!({"kind": "validate"}),
        json!({}),
        json!({"semantic_map_fingerprint_ref": 4}),
    );
    let visibility_stale = covi_case_payload_with_context(
        covi_row_range_artifact_with_snapshot_refs(u32::MAX, u32::MAX, 5),
        "stale_ignored",
        json!({"kind": "validate"}),
        json!({}),
        json!({"external_visibility_ref": 6}),
    );
    let chain_digest = covi_test_chain_digest();
    let delta_chain_valid = covi_case_payload_with_context(
        covi_row_range_artifact_with_delta_chain_digest(chain_digest.clone()),
        "valid",
        json!({"kind": "validate"}),
        json!({}),
        json!({
            "delta_chain_digest_algorithm": DigestAlgorithm::Sha256 as u16,
            "delta_chain_digest": chain_digest.clone()
        }),
    );
    let delta_chain_stale = covi_case_payload_with_context(
        covi_row_range_artifact_with_delta_chain_digest(chain_digest),
        "stale_ignored",
        json!({"kind": "validate"}),
        json!({}),
        json!({
            "delta_chain_digest_algorithm": DigestAlgorithm::Sha256 as u16,
            "delta_chain_digest": vec![0xEEu8; 32]
        }),
    );
    vec![
        CoviHardeningCase {
            path: "covi/validation_row_range_lookup_valid.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1"],
            payload: valid_lookup,
        },
        CoviHardeningCase {
            path: "covi/posting_representations_valid.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1"],
            payload: posting_reps,
        },
        CoviHardeningCase {
            path: "covi/row_ordinal_sets_valid.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1"],
            payload: row_ordinals,
        },
        CoviHardeningCase {
            path: "covi/index_only_count_valid.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.2"],
            payload: index_only,
        },
        CoviHardeningCase {
            path: "covi/stale_digest_ignored.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: stale_digest,
        },
        CoviHardeningCase {
            path: "covi/stale_schema_ignored.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: schema_stale,
        },
        CoviHardeningCase {
            path: "covi/stale_semantic_map_ignored.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: map_stale,
        },
        CoviHardeningCase {
            path: "covi/stale_visibility_ignored.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: visibility_stale,
        },
        CoviHardeningCase {
            path: "covi/delta_chain_digest_valid.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: delta_chain_valid,
        },
        CoviHardeningCase {
            path: "covi/stale_delta_chain_digest_ignored.json",
            expect: "accept",
            error_code: None,
            sections: &["§33.1", "§74"],
            payload: delta_chain_stale,
        },
        CoviHardeningCase {
            path: "covi/bad_digest_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_DIGEST_MISMATCH"),
            sections: &["§33.1"],
            payload: bad_digest,
        },
        CoviHardeningCase {
            path: "covi/cik2_bad_key_data_checksum.json",
            expect: "reject",
            error_code: Some("COVE_E_CHECKSUM_MISMATCH"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_bad_cik_checksum_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/missing_block_ref_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_missing_block_ref_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/wrong_block_root_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_wrong_block_root_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/unsorted_entries_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_unsorted_entries_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/root_bad_logical_type_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: bad_root_enum,
        },
        CoviHardeningCase {
            path: "covi/unsorted_malformed_key_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: malformed_unsorted_key,
        },
        CoviHardeningCase {
            path: "covi/duplicate_key_without_chain_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_duplicate_key_without_chain_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/row_range_not_coalesced_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_bad_row_range_payload_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/row_ordinal_high_bits_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_row_ordinal_artifact(true),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/byte_range_out_of_bounds_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_BAD_COVI"),
            sections: &["§33.1"],
            payload: covi_case_payload(
                covi_byte_range_out_of_bounds_artifact(),
                "valid",
                json!({"kind": "validate"}),
                json!({}),
            ),
        },
        CoviHardeningCase {
            path: "covi/approximate_index_only_exact_reject.json",
            expect: "reject",
            error_code: Some("COVE_E_INDEX_ONLY_UNSAFE"),
            sections: &["§33.2"],
            payload: covi_case_payload(
                covi_index_only_artifact(IndexCapabilityExactnessV2::Approximate),
                "valid",
                json!({"kind": "index_only", "table_id": 1, "column_id": 10, "aggregate_kind": "count"}),
                json!({}),
            ),
        },
    ]
}

pub(super) fn covi_case_payload(
    covi: Vec<u8>,
    expected_result: &str,
    operation: Value,
    extra: Value,
) -> Vec<u8> {
    covi_case_payload_with_context(covi, expected_result, operation, extra, json!({}))
}

pub(super) fn covi_case_payload_with_context(
    covi: Vec<u8>,
    expected_result: &str,
    operation: Value,
    mut extra: Value,
    context_extra: Value,
) -> Vec<u8> {
    let mut context = json!({
        "file_id": covi_test_file_id(),
        "file_len": covi_test_file_len(),
        "footer_crc32c": covi_test_footer_crc32c(),
        "dataset_id": covi_test_file_id(),
        "allow_file_code_keys": true,
        "file_digest_algorithm": DigestAlgorithm::Sha256 as u16,
        "file_digest": covi_test_digest(),
    });
    merge_json_object(&mut context, context_extra);
    let mut value = json!({
        "covi": covi,
        "context": context,
        "expected_result": expected_result,
        "operation": operation,
    });
    merge_json_object(&mut value, extra.take());
    json_fixture_bytes(value)
}

pub(super) fn merge_json_object(target: &mut Value, source: Value) {
    let Some(source) = source.as_object() else {
        return;
    };
    let target = target.as_object_mut().expect("target is object");
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

pub(super) fn covi_test_file_id() -> [u8; 16] {
    [0x41; 16]
}

pub(super) fn covi_test_snapshot_id() -> [u8; 16] {
    [0x42; 16]
}

pub(super) fn covi_test_file_len() -> u64 {
    4096
}

pub(super) fn covi_test_footer_crc32c() -> u32 {
    0xA1B2_C3D4
}

pub(super) fn covi_test_digest() -> Vec<u8> {
    vec![0xA5; 32]
}

pub(super) fn covi_test_chain_digest() -> Vec<u8> {
    vec![0xCD; 32]
}

pub(super) fn key_bytes(value: u8) -> Vec<u8> {
    let mut out = cove_core::wire::encode_u64_leb128(ValueTag::Int64 as u64);
    out.extend_from_slice(&(value as i64).to_le_bytes());
    out
}

pub(super) fn covi_row_range_artifact() -> Vec<u8> {
    covi_row_range_artifact_with_snapshot_refs(u32::MAX, u32::MAX, u32::MAX)
}

pub(super) fn covi_row_range_artifact_with_snapshot_refs(
    schema_ref: u32,
    semantic_map_ref: u32,
    visibility_ref: u32,
) -> Vec<u8> {
    let postings = vec![posting_spec(
        CoviPostingRepresentationV2::RowRangeList,
        row_range_payload(&[CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 2,
            morsel_id: 3,
            row_start: 4,
            row_count: 2,
            flags: 0,
            checksum: 0,
        }]),
        1,
        u32::MAX,
    )];
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        postings,
        Vec::new(),
        None,
        RootOverrides {
            schema_ref,
            semantic_map_ref,
            visibility_ref,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_row_range_artifact_with_delta_chain_digest(
    delta_chain_digest: Vec<u8>,
) -> Vec<u8> {
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![posting_spec(
            CoviPostingRepresentationV2::RowRangeList,
            row_range_payload(&[CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 2,
                morsel_id: 3,
                row_start: 4,
                row_count: 2,
                flags: 0,
                checksum: 0,
            }]),
            1,
            u32::MAX,
        )],
        Vec::new(),
        None,
        RootOverrides {
            delta_chain_digest: Some(delta_chain_digest),
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_object_path_tombstone_overlay_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![object_path_key_bytes()],
        vec![posting_spec(
            CoviPostingRepresentationV2::ObjectPathRefs,
            CoviObjectPathPostingV2 {
                file_ref: 0,
                object_type_id: 1,
                path_ref: 7,
                segment_id: 5,
                row_start: 0,
                row_count: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .unwrap()
            .to_vec(),
            1,
            u32::MAX,
        )],
        Vec::new(),
        None,
        RootOverrides {
            indexed_target_kind: CoviIndexedTargetKindV2::ObjectPath,
            table_id: u32::MAX,
            column_id: u32::MAX,
            object_type_id: 1,
            path_ref: 7,
            logical_type: CoveLogicalType::Utf8 as u16,
            physical_kind: CovePhysicalKind::VarBytes,
            key_encoding_kind: CoviKeyEncodingKindV2::ObjectPathTuple,
            comparator_kind: CoviComparatorKindV2::ObjectPathLexicographic,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn object_path_key_bytes() -> Vec<u8> {
    b"/company/name".to_vec()
}

pub(super) fn covi_all_posting_representations_artifact() -> Vec<u8> {
    let postings = vec![
        posting_spec(
            CoviPostingRepresentationV2::SortedFileRefs,
            CoviFileRefPostingV2 {
                file_ref: 0,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::SortedSegmentRefs,
            CoviSegmentRefPostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::SortedPageRefs,
            CoviPageRefPostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                page_ref: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::SortedMorselRefs,
            CoviMorselRefPostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::RowRangeList,
            row_range_payload(&[CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                row_start: 10,
                row_count: 2,
                flags: 0,
                checksum: 0,
            }]),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::ByteRangeList,
            CoviByteRangePostingV2 {
                file_ref: 0,
                section_id: 1,
                offset: 16,
                length: 8,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .unwrap()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::ObjectPathRefs,
            CoviObjectPathPostingV2 {
                file_ref: 0,
                object_type_id: 1,
                path_ref: 1,
                segment_id: 1,
                row_start: 20,
                row_count: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .unwrap()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::DimensionalBucketRefs,
            CoviDimensionalBucketPostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                dimensional_bucket_ref: 1,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .to_vec(),
            1,
            u32::MAX,
        ),
        posting_spec(
            CoviPostingRepresentationV2::CoverageSetRef,
            Vec::new(),
            1,
            1,
        ),
    ];
    covi_artifact_for_postings(
        (1..=postings.len() as u8).map(key_bytes).collect(),
        postings,
        Vec::new(),
        None,
        RootOverrides {
            coverage_set_ref: 1,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_row_ordinal_artifact(invalid_high_bits: bool) -> Vec<u8> {
    let row_payload = vec![0b0000_0111];
    let row_set = CoviRowOrdinalSetHeaderV2 {
        row_ordinal_set_ref: 0,
        file_ref: 0,
        table_id: 1,
        segment_id: 1,
        morsel_id: 1,
        bitmap_kind: cove_index::CoviBitmapKindV2::DenseBitsetLsb0,
        flags: 0,
        reserved: 0,
        universe_row_count: 5,
        set_row_count: 3,
        payload_offset: 0,
        payload_length: row_payload.len() as u64,
        checksum: 0,
    };
    let mut payload = row_payload;
    let refs_offset = payload.len();
    payload.extend_from_slice(&0u32.to_le_bytes());
    let postings = vec![CoviPostingsHeaderV2 {
        postings_ref: 0,
        index_root_id: 0,
        representation: CoviPostingRepresentationV2::RowOrdinalBitmap,
        target_granularity: CoverageGranularityV2::RowOrdinalSet as u8,
        flags: 0,
        item_count: 1,
        payload_offset: refs_offset as u64,
        payload_length: 4,
        coverage_set_ref: u32::MAX,
        checksum: 0,
    }];
    let block = CoviPostingsBlockV2 {
        header: postings_header(0),
        postings,
        row_ordinal_sets: vec![row_set],
        payload,
    }
    .serialize()
    .unwrap();
    let keys = vec![key_bytes(1)];
    let (key_block, entries) = covi_key_and_entries(&keys, &[0], &[u32::MAX], &[u32::MAX], 0);
    let entry_block = entry_block_payload(0, entries, u32::MAX);
    let artifact = covi_artifact_from_blocks(
        keys.len() as u64,
        key_block,
        entry_block,
        block,
        None,
        RootOverrides::default(),
    );
    if invalid_high_bits {
        mutate_covi_section_payload(artifact, 4, |payload| {
            let bitset_offset = CoviPostingsBlockHeaderV2::LEN
                + CoviPostingsHeaderV2::LEN
                + CoviRowOrdinalSetHeaderV2::LEN;
            payload[bitset_offset] |= 0b1000_0000;
        })
    } else {
        artifact
    }
}

pub(super) fn covi_index_only_artifact(exactness: IndexCapabilityExactnessV2) -> Vec<u8> {
    let aggregate = CoviAggregateAnswerBlockV2 {
        header: CoviAggregateAnswerBlockHeaderV2 {
            magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
            aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
            aggregate_block_id: 0,
            index_root_id: 0,
            aggregate_answer_count: 1,
            aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
            aggregate_payload_offset: 0,
            aggregate_payload_length: 0,
            flags: 0,
            checksum: 0,
        },
        answers: vec![CoviAggregateAnswerV2 {
            aggregate_answer_ref: 0,
            index_root_id: 0,
            aggregate_kind: 0,
            exactness: exactness as u8,
            null_semantics: 0,
            flags: 0,
            row_count: 5,
            null_count: 0,
            non_null_count: 5,
            value_ref: u32::MAX,
            predicate_form_ref: u32::MAX,
            snapshot_validity_ref: 0,
            checksum: 0,
        }],
        payload: Vec::new(),
    }
    .serialize()
    .unwrap();
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![posting_spec(
            CoviPostingRepresentationV2::RowRangeList,
            row_range_payload(&[CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                row_start: 0,
                row_count: 5,
                flags: 0,
                checksum: 0,
            }]),
            1,
            u32::MAX,
        )],
        Vec::new(),
        Some(aggregate),
        RootOverrides {
            supports_index_only: true,
            capability_exactness: exactness,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_bad_cik_checksum_artifact() -> Vec<u8> {
    mutate_covi_section_payload(covi_row_range_artifact(), 2, |payload| {
        let last = payload.len() - 1;
        payload[last] ^= 1;
    })
}

pub(super) fn covi_missing_block_ref_artifact() -> Vec<u8> {
    let overrides = RootOverrides {
        key_section_id: 99,
        ..RootOverrides::default()
    };
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![posting_spec(
            CoviPostingRepresentationV2::RowRangeList,
            row_range_payload(&[CoviRowRangePostingV2 {
                file_ref: 0,
                table_id: 1,
                segment_id: 1,
                morsel_id: 1,
                row_start: 0,
                row_count: 1,
                flags: 0,
                checksum: 0,
            }]),
            1,
            u32::MAX,
        )],
        Vec::new(),
        None,
        overrides,
    )
}

pub(super) fn covi_wrong_block_root_artifact() -> Vec<u8> {
    let (key_block, entries) =
        covi_key_and_entries(&[key_bytes(1)], &[0], &[u32::MAX], &[u32::MAX], 7);
    let entry_block = entry_block_payload(0, entries, u32::MAX);
    let postings_block = postings_block_payload(vec![posting_spec(
        CoviPostingRepresentationV2::RowRangeList,
        row_range_payload(&[CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 1,
            row_start: 0,
            row_count: 1,
            flags: 0,
            checksum: 0,
        }]),
        1,
        u32::MAX,
    )]);
    covi_artifact_from_blocks(
        1,
        key_block,
        entry_block,
        postings_block,
        None,
        RootOverrides::default(),
    )
}

pub(super) fn covi_unsorted_entries_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![key_bytes(2), key_bytes(1)],
        vec![
            posting_spec(
                CoviPostingRepresentationV2::SortedFileRefs,
                CoviFileRefPostingV2 {
                    file_ref: 0,
                    flags: 0,
                    checksum: 0,
                }
                .serialize()
                .to_vec(),
                1,
                u32::MAX,
            ),
            posting_spec(
                CoviPostingRepresentationV2::SortedFileRefs,
                CoviFileRefPostingV2 {
                    file_ref: 0,
                    flags: 0,
                    checksum: 0,
                }
                .serialize()
                .to_vec(),
                1,
                u32::MAX,
            ),
        ],
        Vec::new(),
        None,
        RootOverrides::default(),
    )
}

pub(super) fn covi_bad_root_logical_type_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![row_range_posting_spec()],
        Vec::new(),
        None,
        RootOverrides {
            logical_type: 254,
            unchecked_root: true,
            unchecked_artifact: true,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_malformed_unsorted_key_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![vec![0xfe]],
        vec![row_range_posting_spec()],
        Vec::new(),
        None,
        RootOverrides {
            index_kind: CoviIndexKindV2::Hash,
            unchecked_artifact: true,
            ..RootOverrides::default()
        },
    )
}

pub(super) fn covi_duplicate_key_without_chain_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![key_bytes(1), key_bytes(1)],
        vec![
            posting_spec(
                CoviPostingRepresentationV2::SortedSegmentRefs,
                CoviSegmentRefPostingV2 {
                    file_ref: 0,
                    table_id: 1,
                    segment_id: 1,
                    flags: 0,
                    checksum: 0,
                }
                .serialize()
                .to_vec(),
                1,
                u32::MAX,
            ),
            posting_spec(
                CoviPostingRepresentationV2::SortedSegmentRefs,
                CoviSegmentRefPostingV2 {
                    file_ref: 0,
                    table_id: 1,
                    segment_id: 2,
                    flags: 0,
                    checksum: 0,
                }
                .serialize()
                .to_vec(),
                1,
                u32::MAX,
            ),
        ],
        Vec::new(),
        None,
        RootOverrides::default(),
    )
}

pub(super) fn covi_bad_row_range_payload_artifact() -> Vec<u8> {
    let rows = row_range_payload(&[
        CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 1,
            row_start: 0,
            row_count: 2,
            flags: 0,
            checksum: 0,
        },
        CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 1,
            row_start: 4,
            row_count: 2,
            flags: 0,
            checksum: 0,
        },
    ]);
    let artifact = covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![posting_spec(
            CoviPostingRepresentationV2::RowRangeList,
            rows,
            2,
            u32::MAX,
        )],
        Vec::new(),
        None,
        RootOverrides::default(),
    );
    mutate_covi_section_payload(artifact, 4, |payload| {
        let second_row =
            CoviPostingsBlockHeaderV2::LEN + CoviPostingsHeaderV2::LEN + CoviRowRangePostingV2::LEN;
        payload[second_row + 16..second_row + 24].copy_from_slice(&2u64.to_le_bytes());
        payload[second_row + 36..second_row + 40].fill(0);
        let crc = checksum::crc32c(&payload[second_row..second_row + CoviRowRangePostingV2::LEN]);
        payload[second_row + 36..second_row + 40].copy_from_slice(&crc.to_le_bytes());
    })
}

pub(super) fn covi_byte_range_out_of_bounds_artifact() -> Vec<u8> {
    covi_artifact_for_postings(
        vec![key_bytes(1)],
        vec![posting_spec(
            CoviPostingRepresentationV2::ByteRangeList,
            CoviByteRangePostingV2 {
                file_ref: 0,
                section_id: 1,
                offset: covi_test_file_len() + 1,
                length: 8,
                flags: 0,
                checksum: 0,
            }
            .serialize()
            .unwrap()
            .to_vec(),
            1,
            u32::MAX,
        )],
        Vec::new(),
        None,
        RootOverrides::default(),
    )
}

#[derive(Clone)]
pub(super) struct PostingSpec {
    representation: CoviPostingRepresentationV2,
    payload: Vec<u8>,
    item_count: u64,
    coverage_set_ref: u32,
}

pub(super) fn posting_spec(
    representation: CoviPostingRepresentationV2,
    payload: Vec<u8>,
    item_count: u64,
    coverage_set_ref: u32,
) -> PostingSpec {
    PostingSpec {
        representation,
        payload,
        item_count,
        coverage_set_ref,
    }
}

pub(super) fn row_range_posting_spec() -> PostingSpec {
    posting_spec(
        CoviPostingRepresentationV2::RowRangeList,
        row_range_payload(&[CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 1,
            row_start: 0,
            row_count: 1,
            flags: 0,
            checksum: 0,
        }]),
        1,
        u32::MAX,
    )
}

#[derive(Clone)]
pub(super) struct RootOverrides {
    key_section_id: u32,
    coverage_set_ref: u32,
    schema_ref: u32,
    semantic_map_ref: u32,
    visibility_ref: u32,
    delta_chain_digest: Option<Vec<u8>>,
    indexed_target_kind: CoviIndexedTargetKindV2,
    table_id: u32,
    column_id: u32,
    object_type_id: u32,
    property_id: u32,
    path_ref: u32,
    index_kind: CoviIndexKindV2,
    logical_type: u16,
    physical_kind: CovePhysicalKind,
    key_encoding_kind: CoviKeyEncodingKindV2,
    comparator_kind: CoviComparatorKindV2,
    unchecked_root: bool,
    unchecked_artifact: bool,
    supports_index_only: bool,
    capability_exactness: IndexCapabilityExactnessV2,
}

impl Default for RootOverrides {
    fn default() -> Self {
        Self {
            key_section_id: 2,
            coverage_set_ref: u32::MAX,
            schema_ref: u32::MAX,
            semantic_map_ref: u32::MAX,
            visibility_ref: u32::MAX,
            delta_chain_digest: None,
            indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
            table_id: 1,
            column_id: 10,
            object_type_id: u32::MAX,
            property_id: u32::MAX,
            path_ref: u32::MAX,
            index_kind: CoviIndexKindV2::Sorted,
            logical_type: CoveLogicalType::Int64 as u16,
            physical_kind: CovePhysicalKind::NumCode,
            key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            unchecked_root: false,
            unchecked_artifact: false,
            supports_index_only: false,
            capability_exactness: IndexCapabilityExactnessV2::Exact,
        }
    }
}

pub(super) fn covi_artifact_for_postings(
    keys: Vec<Vec<u8>>,
    postings: Vec<PostingSpec>,
    _row_sets: Vec<CoviRowOrdinalSetHeaderV2>,
    aggregate_block: Option<Vec<u8>>,
    overrides: RootOverrides,
) -> Vec<u8> {
    let posting_refs = (0..postings.len() as u32).collect::<Vec<_>>();
    let aggregate_refs = vec![aggregate_block.as_ref().map(|_| 0).unwrap_or(u32::MAX); keys.len()];
    let duplicate_refs = vec![u32::MAX; keys.len()];
    let (key_block, entries) = covi_key_and_entries_with_kinds(
        &keys,
        &posting_refs,
        &aggregate_refs,
        &duplicate_refs,
        0,
        overrides.key_encoding_kind,
        overrides.comparator_kind,
    );
    let entry_block = entry_block_payload(
        0,
        entries,
        aggregate_block.as_ref().map(|_| 0).unwrap_or(u32::MAX),
    );
    let postings_block = postings_block_payload(postings);
    covi_artifact_from_blocks(
        keys.len() as u64,
        key_block,
        entry_block,
        postings_block,
        aggregate_block,
        overrides,
    )
}

pub(super) fn covi_artifact_from_blocks(
    distinct_count: u64,
    key_block: Vec<u8>,
    entry_block: Vec<u8>,
    postings_block: Vec<u8>,
    aggregate_block: Option<Vec<u8>>,
    overrides: RootOverrides,
) -> Vec<u8> {
    let aggregate_section_id = aggregate_block.as_ref().map(|_| 5).unwrap_or(u32::MAX);
    let mut string_table = covi_test_digest();
    let (delta_chain_digest_algorithm, delta_chain_digest_len, delta_chain_digest_offset) =
        if let Some(delta_chain_digest) = &overrides.delta_chain_digest {
            let offset = string_table.len() as u64;
            string_table.extend_from_slice(delta_chain_digest);
            (
                DigestAlgorithm::Sha256 as u16,
                delta_chain_digest.len() as u16,
                offset,
            )
        } else {
            (DigestAlgorithm::None as u16, 0, 0)
        };
    let referenced_file = CoviReferencedFileV2 {
        file_ref: 0,
        flags: 0,
        file_id: covi_test_file_id(),
        file_len: covi_test_file_len(),
        footer_crc32c: covi_test_footer_crc32c(),
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest_offset: 0,
        uri_ref: u32::MAX,
        schema_fingerprint_ref: overrides.schema_ref,
        checksum: 0,
    };
    let snapshot = CoviSnapshotValidityV2 {
        snapshot_validity_ref: 0,
        dataset_id: covi_test_file_id(),
        snapshot_id: covi_test_snapshot_id(),
        schema_fingerprint_ref: overrides.schema_ref,
        semantic_map_fingerprint_ref: overrides.semantic_map_ref,
        external_visibility_ref: overrides.visibility_ref,
        data_checksum_root_ref: u32::MAX,
        delta_chain_digest_algorithm,
        delta_chain_digest_len,
        delta_chain_digest_offset,
        valid_from_us: 0,
        valid_until_us: i64::MAX,
        flags: 0,
        checksum: 0,
    };
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: overrides.indexed_target_kind,
        index_kind: overrides.index_kind,
        coverage_granularity: CoverageGranularityV2::Morsel as u8,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: CoverageExactnessV2::Exact as u8,
        flags: 0,
        table_id: overrides.table_id,
        column_id: overrides.column_id,
        object_type_id: overrides.object_type_id,
        property_id: overrides.property_id,
        path_ref: overrides.path_ref,
        semantic_dimension_ref: u32::MAX,
        logical_type: overrides.logical_type,
        physical_kind: overrides.physical_kind as u8,
        key_encoding_kind: overrides.key_encoding_kind as u8,
        comparator_kind: overrides.comparator_kind as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: distinct_count.max(5),
        distinct_count,
        null_count: 0,
        min_key_ref: if distinct_count == 0 { u32::MAX } else { 0 },
        max_key_ref: if distinct_count == 0 {
            u32::MAX
        } else {
            distinct_count as u32 - 1
        },
        key_block_section_id: overrides.key_section_id,
        entry_block_section_id: 3,
        postings_block_section_id: 4,
        aggregate_block_section_id: aggregate_section_id,
        coverage_set_ref: overrides.coverage_set_ref,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 1,
        supports_range: 1,
        supports_membership: 0,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: u8::from(overrides.supports_index_only),
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: u8::from(overrides.supports_index_only),
        exactness: overrides.capability_exactness,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let mut sections = vec![
        CoviSectionPayloadV2 {
            section_id: 1,
            section_kind: CoviSectionKindV2::StringTable,
            item_count: u64::from(overrides.delta_chain_digest.is_some()) + 1,
            payload: string_table,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: 2,
            section_kind: CoviSectionKindV2::KeyBlock,
            payload: key_block,
            item_count: distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: 3,
            section_kind: CoviSectionKindV2::EntryBlock,
            payload: entry_block,
            item_count: distinct_count,
            required_features: 0,
            optional_features: 0,
        },
        CoviSectionPayloadV2 {
            section_id: 4,
            section_kind: CoviSectionKindV2::PostingsBlock,
            payload: postings_block,
            item_count: distinct_count,
            required_features: 0,
            optional_features: 0,
        },
    ];
    if let Some(aggregate_block) = aggregate_block {
        sections.push(CoviSectionPayloadV2 {
            section_id: 5,
            section_kind: CoviSectionKindV2::AggregateAnswerBlock,
            payload: aggregate_block,
            item_count: 1,
            required_features: 0,
            optional_features: 0,
        });
    }
    if overrides.supports_index_only {
        sections.push(CoviSectionPayloadV2 {
            section_id: 6,
            section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
            payload: covi_count_index_only_capability_payload(0, 0),
            item_count: 1,
            required_features: 0,
            optional_features: FEATURE_INDEX_ONLY_CAPABILITY,
        });
    }
    if overrides.unchecked_artifact {
        return serialize_covi_artifact_unchecked(
            covi_test_file_id(),
            covi_test_snapshot_id(),
            UncheckedCoviArtifactRefs {
                referenced_files: &[referenced_file],
                snapshot_validity: &[snapshot],
                index_roots: &[root],
                capabilities: &[capability],
                section_payloads: &sections,
                unchecked_roots: overrides.unchecked_root,
            },
        );
    }
    CoviArtifactV2::serialize_with_sections(
        covi_test_file_id(),
        covi_test_snapshot_id(),
        &[referenced_file],
        &[snapshot],
        &[root],
        &[capability],
        &sections,
    )
    .unwrap()
}

pub(super) fn covi_count_index_only_capability_payload(
    capability_id: u32,
    snapshot_validity_ref: u32,
) -> Vec<u8> {
    IndexOnlyCapabilityV2 {
        capability_id,
        aggregate_kind: 0,
        predicate_supported: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref,
        required_visibility_overlay_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
    .to_vec()
}

pub(super) struct UncheckedCoviArtifactRefs<'a> {
    referenced_files: &'a [CoviReferencedFileV2],
    snapshot_validity: &'a [CoviSnapshotValidityV2],
    index_roots: &'a [CoviIndexRootV2],
    capabilities: &'a [IndexCapabilityV2],
    section_payloads: &'a [CoviSectionPayloadV2],
    unchecked_roots: bool,
}

pub(super) fn serialize_covi_artifact_unchecked(
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    refs: UncheckedCoviArtifactRefs<'_>,
) -> Vec<u8> {
    let UncheckedCoviArtifactRefs {
        referenced_files,
        snapshot_validity,
        index_roots,
        capabilities,
        section_payloads,
        unchecked_roots,
    } = refs;
    let section_directory_length = section_payloads.len() * COVI_SECTION_ENTRY_LEN;
    let mut cursor = (COVI_HEADER_LEN as usize) + section_directory_length;
    let mut regions = Vec::new();
    let referenced_files_offset =
        append_unchecked_region(&mut regions, &mut cursor, referenced_files, |item| {
            item.serialize().unwrap().to_vec()
        });
    let snapshot_validity_offset =
        append_unchecked_region(&mut regions, &mut cursor, snapshot_validity, |item| {
            item.serialize().to_vec()
        });
    let index_roots_offset =
        append_unchecked_region(&mut regions, &mut cursor, index_roots, |item| {
            if unchecked_roots {
                serialize_covi_index_root_unchecked(item).to_vec()
            } else {
                item.serialize().unwrap().to_vec()
            }
        });
    let capabilities_offset =
        append_unchecked_region(&mut regions, &mut cursor, capabilities, |item| {
            item.serialize().unwrap().to_vec()
        });
    let mut sections = Vec::with_capacity(section_payloads.len());
    let mut section_bytes = Vec::new();
    for payload in section_payloads {
        let len = payload.payload.len();
        sections.push(CoviSectionEntryV2 {
            section_id: payload.section_id,
            section_kind: payload.section_kind,
            flags: 0,
            offset: cursor as u64,
            length: len as u64,
            uncompressed_length: len as u64,
            item_count: payload.item_count,
            compression: CompressionCodec::None as u8,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_features: payload.required_features,
            optional_features: payload.optional_features,
            crc32c: checksum::crc32c(&payload.payload),
            checksum: 0,
        });
        cursor += len;
        section_bytes.extend_from_slice(&payload.payload);
    }
    let file_len = cursor + COVI_TAIL_LEN;
    let string_table_section_ref = section_payloads
        .iter()
        .find(|section| section.section_kind == CoviSectionKindV2::StringTable)
        .map(|section| section.section_id)
        .unwrap_or(u32::MAX);
    let header = CoviHeaderV2 {
        magic: MAGIC_COVI,
        header_len: COVI_HEADER_LEN,
        version_major: VERSION_MAJOR_V1,
        version_minor: 0,
        flags: 0,
        index_artifact_id: [0u8; 16],
        dataset_id,
        snapshot_id,
        section_count: section_payloads.len() as u32,
        referenced_file_count: referenced_files.len() as u32,
        snapshot_validity_count: snapshot_validity.len() as u32,
        index_root_count: index_roots.len() as u32,
        capability_count: capabilities.len() as u32,
        section_directory_offset: COVI_HEADER_LEN as u64,
        section_directory_length: section_directory_length as u64,
        referenced_files_offset,
        snapshot_validity_offset,
        index_roots_offset,
        capabilities_offset,
        string_table_section_ref,
        created_at_us: 0,
        reserved: [0u8; 24],
        checksum: 0,
    };
    let postscript = CoviPostscriptV2 {
        required_features: FEATURE_SECONDARY_INDEX_ARTIFACT,
        optional_features: 0,
        file_len: file_len as u64,
        header_offset: 0,
        header_length: COVI_HEADER_LEN as u64,
        checksum: 0,
    };
    let mut out = Vec::with_capacity(file_len);
    out.extend_from_slice(&header.serialize());
    for section in &sections {
        out.extend_from_slice(&section.serialize().unwrap());
    }
    out.extend_from_slice(&regions);
    out.extend_from_slice(&section_bytes);
    out.extend_from_slice(&postscript.serialize());
    out.extend_from_slice(&POSTSCRIPT_VERSION_V1.to_le_bytes());
    out.extend_from_slice(&(COVI_POSTSCRIPT_LEN as u16).to_le_bytes());
    out.extend_from_slice(&MAGIC_COVI);
    out
}

pub(super) fn append_unchecked_region<T>(
    out: &mut Vec<u8>,
    cursor: &mut usize,
    items: &[T],
    serialize: impl Fn(&T) -> Vec<u8>,
) -> u64 {
    let offset = *cursor as u64;
    for item in items {
        let bytes = serialize(item);
        *cursor += bytes.len();
        out.extend_from_slice(&bytes);
    }
    offset
}

pub(super) fn serialize_covi_index_root_unchecked(
    root: &CoviIndexRootV2,
) -> [u8; CoviIndexRootV2::LEN] {
    let mut out = [0u8; CoviIndexRootV2::LEN];
    out[0..4].copy_from_slice(&root.index_root_id.to_le_bytes());
    out[4..6].copy_from_slice(&(root.indexed_target_kind as u16).to_le_bytes());
    out[6..8].copy_from_slice(&(root.index_kind as u16).to_le_bytes());
    out[8] = root.coverage_granularity;
    out[9] = root.proof_strength;
    out[10] = root.exactness;
    out[11] = root.flags;
    out[12..16].copy_from_slice(&root.table_id.to_le_bytes());
    out[16..20].copy_from_slice(&root.column_id.to_le_bytes());
    out[20..24].copy_from_slice(&root.object_type_id.to_le_bytes());
    out[24..28].copy_from_slice(&root.property_id.to_le_bytes());
    out[28..32].copy_from_slice(&root.path_ref.to_le_bytes());
    out[32..36].copy_from_slice(&root.semantic_dimension_ref.to_le_bytes());
    out[36..38].copy_from_slice(&root.logical_type.to_le_bytes());
    out[38] = root.physical_kind;
    out[39] = root.key_encoding_kind;
    out[40..42].copy_from_slice(&root.comparator_kind.to_le_bytes());
    out[42..44].copy_from_slice(&root.collation_id.to_le_bytes());
    out[44] = root.null_semantics;
    out[45] = root.sort_order;
    out[46..54].copy_from_slice(&root.value_count.to_le_bytes());
    out[54..62].copy_from_slice(&root.distinct_count.to_le_bytes());
    out[62..70].copy_from_slice(&root.null_count.to_le_bytes());
    out[70..74].copy_from_slice(&root.min_key_ref.to_le_bytes());
    out[74..78].copy_from_slice(&root.max_key_ref.to_le_bytes());
    out[78..82].copy_from_slice(&root.key_block_section_id.to_le_bytes());
    out[82..86].copy_from_slice(&root.entry_block_section_id.to_le_bytes());
    out[86..90].copy_from_slice(&root.postings_block_section_id.to_le_bytes());
    out[90..94].copy_from_slice(&root.aggregate_block_section_id.to_le_bytes());
    out[94..98].copy_from_slice(&root.coverage_set_ref.to_le_bytes());
    out[98..102].copy_from_slice(&root.capability_ref.to_le_bytes());
    out[102..106].copy_from_slice(&root.snapshot_validity_ref.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[106..110].copy_from_slice(&crc.to_le_bytes());
    out
}

pub(super) fn covi_key_and_entries(
    keys: &[Vec<u8>],
    posting_refs: &[u32],
    aggregate_refs: &[u32],
    duplicate_refs: &[u32],
    key_root_id: u32,
) -> (Vec<u8>, Vec<CoviIndexEntryV2>) {
    covi_key_and_entries_with_kinds(
        keys,
        posting_refs,
        aggregate_refs,
        duplicate_refs,
        key_root_id,
        CoviKeyEncodingKindV2::CanonicalValueBytes,
        CoviComparatorKindV2::CanonicalOrdering,
    )
}

pub(super) fn covi_key_and_entries_with_kinds(
    keys: &[Vec<u8>],
    posting_refs: &[u32],
    aggregate_refs: &[u32],
    duplicate_refs: &[u32],
    key_root_id: u32,
    encoding_kind: CoviKeyEncodingKindV2,
    comparator_kind: CoviComparatorKindV2,
) -> (Vec<u8>, Vec<CoviIndexEntryV2>) {
    let (key_block, key_refs) = covi_key_block(key_root_id, keys, encoding_kind, comparator_kind);
    let entries = key_refs
        .iter()
        .enumerate()
        .map(|(index, (offset, len))| {
            let entry_ref = index as u32;
            CoviIndexEntryV2 {
                entry_ref,
                index_root_id: 0,
                entry_id: u64::from(entry_ref),
                key_kind: encoding_kind,
                comparator_kind,
                flags: 0,
                key_offset: *offset,
                key_length: *len,
                key_hash64: u64::from(entry_ref),
                postings_ref: posting_refs.get(index).copied().unwrap_or(u32::MAX),
                coverage_set_ref: u32::MAX,
                aggregate_answer_ref: aggregate_refs.get(index).copied().unwrap_or(u32::MAX),
                next_duplicate_ref: duplicate_refs.get(index).copied().unwrap_or(u32::MAX),
                checksum: 0,
            }
        })
        .collect();
    (key_block, entries)
}

pub(super) fn covi_key_block(
    root_id: u32,
    keys: &[Vec<u8>],
    encoding_kind: CoviKeyEncodingKindV2,
    comparator_kind: CoviComparatorKindV2,
) -> (Vec<u8>, Vec<(u64, u32)>) {
    let mut key_data = Vec::new();
    let mut refs = Vec::new();
    for key in keys {
        refs.push((key_data.len() as u64, key.len() as u32));
        key_data.extend_from_slice(key);
    }
    let block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: root_id,
            index_root_id: root_id,
            key_count: keys.len() as u64,
            encoding_kind,
            comparator_kind,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: key_data.len() as u64,
            checksum: 0,
        },
        key_data,
    }
    .serialize()
    .unwrap();
    (block, refs)
}

pub(super) fn entry_block_payload(
    root_id: u32,
    entries: Vec<CoviIndexEntryV2>,
    aggregate_block_id: u32,
) -> Vec<u8> {
    CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: root_id,
            index_root_id: 0,
            entry_count: entries.len() as u32,
            key_block_id: root_id,
            postings_block_id: root_id,
            aggregate_block_id,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: (entries.len() * CoviIndexEntryV2::LEN) as u64,
            flags: 0,
            checksum: 0,
        },
        entries,
    }
    .serialize()
    .unwrap()
}

pub(super) fn postings_header(root_id: u32) -> CoviPostingsBlockHeaderV2 {
    CoviPostingsBlockHeaderV2 {
        magic: CoviPostingsBlockHeaderV2::MAGIC,
        version_major: 2,
        version_minor: 0,
        header_len: CoviPostingsBlockHeaderV2::LEN as u16,
        postings_header_len: CoviPostingsHeaderV2::LEN as u16,
        postings_block_id: root_id,
        index_root_id: 0,
        postings_count: 0,
        row_ordinal_set_count: 0,
        postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
        row_ordinal_headers_offset: 0,
        postings_payload_offset: 0,
        postings_payload_length: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn postings_block_payload(posting_specs: Vec<PostingSpec>) -> Vec<u8> {
    let mut payload = Vec::new();
    let mut postings = Vec::new();
    for (index, spec) in posting_specs.into_iter().enumerate() {
        let offset = payload.len() as u64;
        let len = spec.payload.len() as u64;
        payload.extend_from_slice(&spec.payload);
        postings.push(CoviPostingsHeaderV2 {
            postings_ref: index as u32,
            index_root_id: 0,
            representation: spec.representation,
            target_granularity: CoverageGranularityV2::Morsel as u8,
            flags: 0,
            item_count: spec.item_count,
            payload_offset: offset,
            payload_length: len,
            coverage_set_ref: spec.coverage_set_ref,
            checksum: 0,
        });
    }
    CoviPostingsBlockV2 {
        header: postings_header(0),
        postings,
        row_ordinal_sets: Vec::new(),
        payload,
    }
    .serialize()
    .unwrap()
}

pub(super) fn row_range_payload(rows: &[CoviRowRangePostingV2]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        out.extend_from_slice(&row.serialize().unwrap());
    }
    out
}

pub(super) fn mutate_covi_section_payload(
    mut bytes: Vec<u8>,
    section_id: u32,
    mutate: impl FnOnce(&mut [u8]),
) -> Vec<u8> {
    let artifact = CoviArtifactV2::parse(&bytes).unwrap();
    let (section_index, section) = artifact
        .sections
        .iter()
        .enumerate()
        .find(|(_, section)| section.section_id == section_id)
        .unwrap();
    let start = section.offset as usize;
    let end = section.end_offset().unwrap() as usize;
    mutate(&mut bytes[start..end]);
    let crc = checksum::crc32c(&bytes[start..end]);
    let entry_start =
        artifact.header.section_directory_offset as usize + section_index * COVI_SECTION_ENTRY_LEN;
    bytes[entry_start + 60..entry_start + 64].copy_from_slice(&crc.to_le_bytes());
    bytes[entry_start + 64..entry_start + 68].fill(0);
    let entry_crc = checksum::crc32c(&bytes[entry_start..entry_start + COVI_SECTION_ENTRY_LEN]);
    bytes[entry_start + 64..entry_start + 68].copy_from_slice(&entry_crc.to_le_bytes());
    bytes
}

pub(super) fn cache_payload(under_inclusive: bool) -> Vec<u8> {
    let dataset_id = [0x11; 16];
    let snapshot_id = [0x22; 16];
    let exactness = if under_inclusive {
        CoverageExactnessV2::ApproximateMayUnderInclude
    } else {
        CoverageExactnessV2::Exact
    };
    CoverageCacheV2 {
        header: CoveCoverageCacheHeaderV2 {
            cache_format_namespace_ref: 1,
            cache_format_version_major: 2,
            cache_format_version_minor: 0,
            flags: 0,
            cache_id: [0x33; 16],
            dataset_id,
            snapshot_id,
            entry_count: 1,
            created_at_us: 0,
            producer_engine_ref: u32::MAX,
            reserved: [0u8; 32],
            checksum: 0,
        },
        entries: vec![coverage_cache_entry(1, dataset_id, snapshot_id, exactness)],
    }
    .serialize()
    .unwrap()
}

pub(super) fn coverage_cache_entry(
    entry_id: u64,
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    exactness: CoverageExactnessV2,
) -> CoverageCacheEntryV2 {
    CoverageCacheEntryV2 {
        entry_id,
        dataset_id,
        snapshot_id,
        predicate_normal_form_ref: 1,
        interval_normal_form_ref: u32::MAX,
        coverage_set_ref: 1,
        coverage_granularity: CoverageGranularityV2::Page,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness,
        flags: 0,
        actual_coverage_size_bytes: 128,
        actual_read_cost_ns: 100,
        created_at_us: 0,
        valid_until_snapshot_ref: u32::MAX,
        producer_engine_ref: u32::MAX,
        checksum: 0,
    }
}

pub(super) fn cache_stale_snapshot_payload() -> Vec<u8> {
    let dataset_id = [0x11; 16];
    let snapshot_id = [0x22; 16];
    let header = CoveCoverageCacheHeaderV2 {
        cache_format_namespace_ref: 1,
        cache_format_version_major: 2,
        cache_format_version_minor: 0,
        flags: 0,
        cache_id: [0x33; 16],
        dataset_id,
        snapshot_id,
        entry_count: 1,
        created_at_us: 0,
        producer_engine_ref: u32::MAX,
        reserved: [0u8; 32],
        checksum: 0,
    };
    let stale = coverage_cache_entry(1, dataset_id, [0x99; 16], CoverageExactnessV2::Exact);
    let mut out = header.serialize().to_vec();
    out.extend_from_slice(&stale.serialize());
    out
}

pub(super) fn cache_duplicate_entry_payload() -> Vec<u8> {
    let dataset_id = [0x11; 16];
    let snapshot_id = [0x22; 16];
    CoverageCacheV2 {
        header: CoveCoverageCacheHeaderV2 {
            cache_format_namespace_ref: 1,
            cache_format_version_major: 2,
            cache_format_version_minor: 0,
            flags: 0,
            cache_id: [0x33; 16],
            dataset_id,
            snapshot_id,
            entry_count: 2,
            created_at_us: 0,
            producer_engine_ref: u32::MAX,
            reserved: [0u8; 32],
            checksum: 0,
        },
        entries: vec![
            coverage_cache_entry(1, dataset_id, snapshot_id, CoverageExactnessV2::Exact),
            coverage_cache_entry(1, dataset_id, snapshot_id, CoverageExactnessV2::Exact),
        ],
    }
    .serialize()
    .unwrap()
}

pub(super) fn feature_binding_section() -> SectionFeatureBindingSectionV2 {
    SectionFeatureBindingSectionV2 {
        header: SectionFeatureBindingSectionHeaderV2 {
            magic: *b"SFB2",
            version_major: 2,
            version_minor: 0,
            header_len: SectionFeatureBindingSectionHeaderV2::LEN as u16,
            entry_len: SectionFeatureBindingV2::LEN as u16,
            binding_count: 1,
            payload_ref_count: 1,
            feature_word_count: 1,
            bindings_offset: 0,
            payload_refs_offset: 0,
            feature_words_offset: 0,
            payload_data_offset: 0,
            payload_data_length: 0,
            flags: 0,
            checksum: 0,
        },
        bindings: vec![SectionFeatureBindingV2 {
            binding_id: 0,
            section_id: 0,
            scope: FeatureScopeV2::OperationRequired,
            profile: PrimaryProfile::TableScan as u8,
            operation_kind: OperationKindV2::CoveragePlanning,
            required_word_count: 1,
            optional_word_count: 0,
            required_feature_word_index: 0,
            optional_feature_word_index: u32::MAX,
            required_first_feature_word_number: 1,
            optional_first_feature_word_number: u32::MAX,
            binding_payload_ref: 1,
            target_local_ref: u64::MAX,
            flags: 0,
            checksum: 0,
        }],
        payload_refs: vec![SectionFeatureBindingPayloadRefV2 {
            binding_payload_ref: 1,
            payload_kind: SectionFeatureBindingPayloadKindV2::OperationRequirement,
            operation_kind: OperationKindV2::CoveragePlanning,
            profile: PrimaryProfile::TableScan as u8,
            flags: 0,
            reserved: 0,
            payload_offset: 0,
            payload_length: 4,
            checksum: 0,
        }],
        feature_words: vec![1],
        payload_data: b"SFB!".to_vec(),
    }
}

pub(super) fn feature_binding_section_payload() -> Vec<u8> {
    feature_binding_section().serialize().unwrap()
}

pub(super) fn feature_binding_bad_payload_ref_payload() -> Vec<u8> {
    let mut bytes = feature_binding_section_payload();
    let binding_start = SectionFeatureBindingSectionHeaderV2::LEN;
    bytes[binding_start + 36..binding_start + 40].copy_from_slice(&2u32.to_le_bytes());
    rewrite_fixed_record_checksum(
        &mut bytes[binding_start..binding_start + SectionFeatureBindingV2::LEN],
        52,
    );
    bytes
}

pub(super) fn feature_binding_empty_payload_contract_payload() -> Vec<u8> {
    let mut section = feature_binding_section();
    section.payload_refs[0].payload_length = 0;
    serialize_invalid_feature_binding_section(&section)
}

pub(super) fn feature_binding_bad_scope_payload() -> Vec<u8> {
    let mut bytes = feature_binding_section_payload();
    let binding_start = SectionFeatureBindingSectionHeaderV2::LEN;
    bytes[binding_start + 8] = 99;
    bytes
}

pub(super) fn serialize_invalid_feature_binding_section(
    section: &SectionFeatureBindingSectionV2,
) -> Vec<u8> {
    let invalid_payload_length = section.payload_refs[0].payload_length;
    let mut valid = section.clone();
    valid.payload_refs[0].payload_length = 4;
    let mut bytes = valid.serialize().unwrap();
    let payload_ref_start =
        SectionFeatureBindingSectionHeaderV2::LEN + SectionFeatureBindingV2::LEN;
    bytes[payload_ref_start + 20..payload_ref_start + 28]
        .copy_from_slice(&invalid_payload_length.to_le_bytes());
    rewrite_fixed_record_checksum(
        &mut bytes[payload_ref_start..payload_ref_start + SectionFeatureBindingPayloadRefV2::LEN],
        28,
    );
    bytes
}

pub(super) fn feature_binding_header_narrowing_payload() -> Vec<u8> {
    let mut bytes = feature_binding_section_payload();
    let binding_start = SectionFeatureBindingSectionHeaderV2::LEN;
    bytes[binding_start + 28..binding_start + 32].copy_from_slice(&0u32.to_le_bytes());
    rewrite_fixed_record_checksum(
        &mut bytes[binding_start..binding_start + SectionFeatureBindingV2::LEN],
        52,
    );
    bytes
}

pub(super) fn zero_copy_entry() -> ZeroCopyBufferMapEntryV2 {
    ZeroCopyBufferMapEntryV2 {
        target_id: 1,
        table_id: 1,
        column_id: 1,
        segment_id: 1,
        morsel_id: 0,
        page_ref: 1,
        buffer_id: 0,
        buffer_kind: 0,
        logical_type: CoveLogicalType::Utf8 as u16,
        physical_kind: CovePhysicalKind::VarBytes as u8,
        source_endianness: 0,
        required_alignment_log2: 3,
        null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2::OneMeansNull,
        source_offset_width_bits: 0,
        target_offset_width_bits: 0,
        dictionary_key_width_bits: 0,
        dictionary_semantics: ZeroCopyDictionarySemanticsV2::NoDictionary,
        lifetime_scope: ZeroCopyLifetimeScopeV2::ReaderSession,
        nested_layout_kind: ZeroCopyNestedLayoutKindV2::NotNested,
        compression_required_none: 1,
        target_buffer_role: ZeroCopyTargetBufferRoleV2::Values,
        source_buffer_role: ZeroCopySourceBufferRoleV2::CoveValues,
        target_type_ref: u32::MAX,
        dictionary_values_ref: u32::MAX,
        child_layout_ref: u32::MAX,
        owner_lifetime_ref: u32::MAX,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn zero_copy_map_payload(entry: ZeroCopyBufferMapEntryV2) -> Vec<u8> {
    ZeroCopyBufferMapV2 {
        header: ZeroCopyBufferMapHeaderV2 {
            map_count: 1,
            target_count: 1,
            flags: 0,
            checksum: 0,
        },
        targets: vec![ZeroCopyTargetV2 {
            target_id: 1,
            namespace: "org.apache.arrow".into(),
            target_name: "arrow".into(),
            version_major: 1,
            version_minor: 0,
            flags: 0,
        }],
        entries: vec![entry],
    }
    .serialize()
    .unwrap()
}

pub(super) fn zero_copy_map_bad_target_ref_payload() -> Vec<u8> {
    let mut bytes = zero_copy_map_payload(zero_copy_entry());
    let target_len = ZeroCopyTargetV2 {
        target_id: 1,
        namespace: "org.apache.arrow".into(),
        target_name: "arrow".into(),
        version_major: 1,
        version_minor: 0,
        flags: 0,
    }
    .serialize()
    .unwrap()
    .len();
    let entry_start = ZeroCopyBufferMapHeaderV2::LEN + target_len;
    bytes[entry_start..entry_start + 4].copy_from_slice(&2u32.to_le_bytes());
    rewrite_fixed_record_checksum(
        &mut bytes[entry_start..entry_start + ZeroCopyBufferMapEntryV2::LEN],
        68,
    );
    bytes
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ZeroCopyFixtureCase {
    Compatible,
    Compressed,
    NullPolarity,
    Dictionary,
    Nested,
    Lifetime,
    Overlay,
    UnknownRole,
}

pub(super) fn zero_copy_compat_case_payload(case: ZeroCopyFixtureCase) -> Vec<u8> {
    let mut entry = zero_copy_entry();
    let mut active_visibility_overlay = false;
    let mut accepts_cove_null_bitmap_polarity = true;
    let mut require_reader_session_lifetime = false;
    let expect = match case {
        ZeroCopyFixtureCase::Compatible => "Compatible",
        ZeroCopyFixtureCase::Compressed => {
            entry.compression_required_none = 0;
            "CompressedBuffer"
        }
        ZeroCopyFixtureCase::NullPolarity => {
            entry.null_bitmap_polarity = ZeroCopyNullBitmapPolarityV2::OneMeansValid;
            accepts_cove_null_bitmap_polarity = false;
            "NullPolarityMismatch"
        }
        ZeroCopyFixtureCase::Dictionary => {
            entry.dictionary_semantics = ZeroCopyDictionarySemanticsV2::RequiresRemap;
            "DictionaryMismatch"
        }
        ZeroCopyFixtureCase::Nested => {
            entry.nested_layout_kind = ZeroCopyNestedLayoutKindV2::CoveNativeNested;
            "NestedLayoutMismatch"
        }
        ZeroCopyFixtureCase::Lifetime => {
            entry.lifetime_scope = ZeroCopyLifetimeScopeV2::Page;
            require_reader_session_lifetime = true;
            "InsufficientLifetime"
        }
        ZeroCopyFixtureCase::Overlay => {
            active_visibility_overlay = true;
            "ActiveVisibilityOverlay"
        }
        ZeroCopyFixtureCase::UnknownRole => {
            entry.target_buffer_role = ZeroCopyTargetBufferRoleV2::Extension;
            "UnknownRole"
        }
    };
    json_fixture_bytes(json!({
        "section": zero_copy_map_payload(entry),
        "active_visibility_overlay": active_visibility_overlay,
        "accepts_cove_null_bitmap_polarity": accepts_cove_null_bitmap_polarity,
        "require_reader_session_lifetime": require_reader_session_lifetime,
        "expect": expect
    }))
}

pub(super) fn arrow_view_materialization_case_payload() -> Vec<u8> {
    let selected_a = b"selected-visible-payload-alpha".as_ref();
    let hidden = b"hidden-unselected-payload-beta".as_ref();
    let selected_b = b"selected-visible-payload-gamma".as_ref();
    encoding_fixture_bytes(json!({
        "logical": "Utf8",
        "physical": "VarBytes",
        "encoding": "VarBytes",
        "row_count": 3,
        "payload": varbytes_payload(&[selected_a, hidden, selected_b]),
        "selection": [0, 2],
        "expect_type": "Utf8View",
        "expect": ["selected-visible-payload-alpha", "selected-visible-payload-gamma"],
        "expect_absent_in_buffers": ["hidden-unselected-payload-beta"]
    }))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SidecarValidityCase {
    Digest,
    Schema,
    SemanticMap,
}

pub(super) fn sidecar_validity_payload(case: SidecarValidityCase) -> Vec<u8> {
    json_fixture_bytes(json!({
        "covi": covi_empty_payload(),
        "digest_matches": !matches!(case, SidecarValidityCase::Digest),
        "schema_matches": !matches!(case, SidecarValidityCase::Schema),
        "semantic_map_matches": !matches!(case, SidecarValidityCase::SemanticMap),
        "expect": "StaleIgnored"
    }))
}

pub(super) fn visibility_safety_payload(overlay_aware: bool) -> Vec<u8> {
    json_fixture_bytes(json!({
        "active_visibility_overlay": true,
        "overlay_aware": overlay_aware,
        "zero_copy_section": zero_copy_map_payload(zero_copy_entry()),
        "expect_index_only_allowed": overlay_aware,
        "expect_metadata_only_allowed": overlay_aware,
        "expect_zero_copy": if overlay_aware { "Compatible" } else { "ActiveVisibilityOverlay" }
    }))
}

pub(super) fn rewrite_fixed_record_checksum(bytes: &mut [u8], checksum_offset: usize) {
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(bytes);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn cove_with_optional_layout_section() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.required_features = FEATURE_TABLE_PROFILE;
    writer.optional_features = FEATURE_LAYOUT_PLAN;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::LayoutPlan as u16,
        profile: PrimaryProfile::LayoutPlanning as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_LAYOUT_PLAN,
        data: layout_plan_payload(),
    });
    writer.write().unwrap()
}

pub(super) fn cove_with_required_bad_layout_section() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.required_features = FEATURE_TABLE_PROFILE | FEATURE_LAYOUT_PLAN;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::LayoutPlan as u16,
        profile: PrimaryProfile::LayoutPlanning as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_LAYOUT_PLAN,
        optional_features: 0,
        data: layout_plan_bad_child_range_payload(),
    });
    writer.write().unwrap()
}

pub(super) fn cove_with_runtime_hints_section(required_hint: bool) -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.required_features = FEATURE_TABLE_PROFILE;
    writer.optional_features = FEATURE_RUNTIME_COMPATIBILITY_HINTS;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::RuntimeCompatibilityHints as u16,
        profile: PrimaryProfile::RuntimeCompatibility as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        data: runtime_hints_payload_with_required(required_hint),
    });
    writer.write().unwrap()
}

pub(super) fn cove_with_optional_zero_copy_section() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.required_features = FEATURE_TABLE_PROFILE;
    writer.optional_features = FEATURE_ZERO_COPY_BUFFER_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ZeroCopyBufferMap as u16,
        profile: 0,
        flags: 0,
        item_count: 0,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_ZERO_COPY_BUFFER_MAP,
        data: Vec::new(),
    });
    writer.write().unwrap()
}

pub(super) fn rewrite_cove_feature_bits(
    bytes: &mut [u8],
    required_features: u64,
    optional_features: u64,
) {
    bytes[16..24].copy_from_slice(&required_features.to_le_bytes());
    bytes[24..32].copy_from_slice(&optional_features.to_le_bytes());
    bytes[156..160].fill(0);
    let header_crc = checksum::crc32c(&bytes[..HEADER_SIZE]);
    bytes[156..160].copy_from_slice(&header_crc.to_le_bytes());

    let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
    bytes[tail_start..tail_start + 8].copy_from_slice(&required_features.to_le_bytes());
    bytes[tail_start + 8..tail_start + 16].copy_from_slice(&optional_features.to_le_bytes());
    bytes[tail_start + 60..tail_start + 64].fill(0);
    let postscript_crc = checksum::crc32c(&bytes[tail_start..tail_start + POSTSCRIPT_SIZE]);
    bytes[tail_start + 60..tail_start + 64].copy_from_slice(&postscript_crc.to_le_bytes());
}

pub(super) fn set_cove_scoped_feature_section_ids(
    bytes: &mut [u8],
    feature_set_section_id: u32,
    profile_capability_section_id: u32,
) {
    bytes[76..80].copy_from_slice(&feature_set_section_id.to_le_bytes());
    bytes[80..84].copy_from_slice(&profile_capability_section_id.to_le_bytes());
    bytes[156..160].fill(0);
    let header_crc = checksum::crc32c(&bytes[..HEADER_SIZE]);
    bytes[156..160].copy_from_slice(&header_crc.to_le_bytes());
}
