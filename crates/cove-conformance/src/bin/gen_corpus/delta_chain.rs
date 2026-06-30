use super::*;

pub(super) fn valid_covx_file() -> Vec<u8> {
    CovxFile {
        header: CovxHeaderV1::new([0x11; 16], 0, 1_700_000_000_000_000),
        referenced_files: vec![CovxReferencedFileV1 {
            file_id: [0x22; 16],
            file_len: 244,
            footer_crc32c: checksum::crc32c(b"footer"),
            digest_algorithm: 1,
            digest: vec![0x33; 32],
        }],
        postscript: CovxPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn valid_covm_file() -> Vec<u8> {
    CovmFile {
        header: CovmHeaderV1::new([0x55; 16], 1, 0, 1_700_000_000_000_000),
        files: vec![CovmFileEntryV1 {
            file_id: [0x66; 16],
            uri: "file:///dataset/part-0.cove".into(),
            file_len: 244,
            footer_crc32c: checksum::crc32c(b"footer"),
            digest_algorithm: 1,
            digest: vec![0x77; 32],
            row_count: 10,
            segment_count: 1,
            file_stats_ref: 0,
            file_exact_set_ref: 0,
            flags: 0,
        }],
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn covm_delta_required_non_delta_reader_file() -> Vec<u8> {
    let mut file = CovmFile::parse_delta_aware(&valid_covm_file()).unwrap();
    file.postscript.flags = COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED;
    file.serialize().unwrap()
}

pub(super) fn valid_covedelta_file() -> Vec<u8> {
    covedelta_file_with_parent_refs(vec![valid_covedelta_lineage_parent_ref()])
}

pub(super) fn covedelta_missing_lineage_parent_file() -> Vec<u8> {
    covedelta_file_with_parent_refs(Vec::new())
}

pub(super) fn covedelta_duplicate_lineage_parent_file() -> Vec<u8> {
    let mut duplicate_parent = valid_covedelta_lineage_parent_ref();
    duplicate_parent.parent_ref = 1;
    duplicate_parent.artifact_id = [0x85; 16];
    covedelta_file_with_parent_refs(vec![valid_covedelta_lineage_parent_ref(), duplicate_parent])
}

pub(super) fn valid_covedelta_lineage_parent_ref() -> DeltaParentRefV1 {
    DeltaParentRefV1 {
        parent_ref: 0,
        parent_kind: 0,
        flags: DELTA_PARENT_REF_LINEAGE_PARENT,
        artifact_id: [0x84; 16],
        snapshot_id: [0x83; 16],
        file_len: 244,
        footer_crc32c: checksum::crc32c(b"footer"),
        digest_algorithm: 1,
        digest_len: 32,
        digest_ref: 0,
        uri_ref: 1,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    }
}

pub(super) fn covedelta_file_with_parent_refs(parent_refs: Vec<DeltaParentRefV1>) -> Vec<u8> {
    CoveDeltaFile {
        header: CoveDeltaHeaderV1::new([0x80; 16], [0x81; 16], [0x82; 16], [0x83; 16]),
        parent_refs,
        sections: vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 1,
                section_kind: CoveDeltaSectionKind::TemporalSegmentData as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: b"delta-temporal-segment-placeholder".to_vec(),
        }],
        footer: CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: COVEDELTA_HEADER_LEN as u64,
            section_directory_offset: 0,
            section_directory_length: 0,
            section_count: 0,
            parent_ref_count: 0,
            footer_crc32c: 0,
            checksum: 0,
        },
        postscript: CoveDeltaPostscriptV1 {
            required_delta_features: 0,
            optional_delta_features: 0,
            file_len: 0,
            footer_offset: 0,
            footer_length: COVEDELTA_FOOTER_LEN as u64,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn valid_covedelta_object_delta_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows(&valid_temporal_rows())
}

pub(super) fn covedelta_object_delta_file_with_rows(rows: &[TemporalRowEntryV1]) -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(rows, 0, Vec::new())
}

pub(super) fn covedelta_object_delta_checkpoint_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_CHECKPOINT_BASELINES,
        Vec::new(),
    )
}

pub(super) fn covedelta_object_delta_checkpoint_missing_rows_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &delta_only_temporal_rows(),
        DELTA_FEATURE_CHECKPOINT_BASELINES,
        Vec::new(),
    )
}

pub(super) fn covedelta_object_delta_with_anchor_touched_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_CONTINUATION_ANCHORS | DELTA_FEATURE_EXACT_TOUCHED_SET,
        vec![
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 2,
                    section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_CONTINUATION_ANCHORS,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: valid_delta_continuation_anchor().serialize().to_vec(),
            },
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 3,
                    section_kind: CoveDeltaSectionKind::TouchedObjectSet as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_EXACT_TOUCHED_SET,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: valid_delta_touched_object_range().serialize().to_vec(),
            },
            valid_delta_state_hash_section(4),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_touched_property_bitmap_file(
    descriptor_kind: Option<u8>,
) -> Vec<u8> {
    let mut touched = valid_delta_touched_object_range();
    touched.property_bitmap_ref = 0;
    let mut sections = vec![
        CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_CONTINUATION_ANCHORS,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: valid_delta_continuation_anchor().serialize().to_vec(),
        },
        CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 3,
                section_kind: CoveDeltaSectionKind::TouchedObjectSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOUCHED_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: touched.serialize().to_vec(),
        },
        valid_delta_state_hash_section(4),
    ];
    if let Some(summary_kind) = descriptor_kind {
        sections.push(delta_summary_descriptor_section(
            5,
            CoveDeltaSectionKind::TouchedSummaryTable,
            &[valid_delta_summary_descriptor(summary_kind)],
        ));
    }
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_CONTINUATION_ANCHORS | DELTA_FEATURE_EXACT_TOUCHED_SET,
        sections,
    )
}

pub(super) fn covedelta_object_delta_with_underinclusive_touched_file() -> Vec<u8> {
    let mut touched = valid_delta_touched_object_range();
    touched.min_goid = [0x01; 16];
    touched.max_goid = [0x01; 16];
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_EXACT_TOUCHED_SET,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::TouchedObjectSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOUCHED_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: touched.serialize().to_vec(),
        }],
    )
}

pub(super) fn covedelta_object_delta_with_underinclusive_anchor_file() -> Vec<u8> {
    let mut anchor = valid_delta_continuation_anchor();
    anchor.goid = [0x01; 16];
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_CONTINUATION_ANCHORS,
        vec![
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 2,
                    section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_CONTINUATION_ANCHORS,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: anchor.serialize().to_vec(),
            },
            valid_delta_state_hash_section(3),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_unresolved_anchor_state_hash_file() -> Vec<u8> {
    let mut anchor = valid_delta_continuation_anchor();
    anchor.predecessor_state_hash_ref = 1;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_CONTINUATION_ANCHORS,
        vec![
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 2,
                    section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_CONTINUATION_ANCHORS,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: anchor.serialize().to_vec(),
            },
            valid_delta_state_hash_section(3),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_single_scope_anchor_mismatch_file() -> Vec<u8> {
    let mut anchor = valid_delta_continuation_anchor();
    anchor.scope_id = [1; 16];
    anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_ONLY;
    let mut delta = CoveDeltaFile::parse(&covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::ContinuationAnchors as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: anchor.serialize().to_vec(),
        }],
    ))
    .unwrap();
    delta.header.flags |= DELTA_FLAG_SINGLE_SCOPE;
    delta.serialize().unwrap()
}

pub(super) fn covedelta_object_delta_with_sparse_patch_file() -> Vec<u8> {
    let mut record = valid_delta_sparse_patch_record();
    record.changed_properties[0].value_ref = 0;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_SPARSE_PATCH_ROWS,
        vec![
            delta_property_ops_section(2, record.serialize().unwrap()),
            delta_string_table_section(3, 1),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_dictionary_overlay_file() -> Vec<u8> {
    let mut entry = valid_delta_dictionary_entry();
    entry.inline_value_ref = 0;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_INLINE_DICTIONARY,
        vec![
            delta_dictionary_overlay_section(2, &[entry]),
            delta_string_table_section(3, 1),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_parent_alias_dictionary_overlay_file() -> Vec<u8> {
    let mut entry = valid_delta_dictionary_entry();
    entry.entry_kind = DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS;
    entry.inline_value_ref = DELTA_REF_NONE;
    entry.parent_ref = 0;
    entry.parent_dictionary_id = 1;
    entry.parent_code = 2;
    entry.parent_dictionary_digest_ref = 3;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_PARENT_DICTIONARY_ALIASES,
        vec![delta_dictionary_overlay_section(2, &[entry])],
    )
}

pub(super) fn covedelta_object_delta_with_parent_alias_dictionary_overlay_missing_feature_file(
) -> Vec<u8> {
    let mut entry = valid_delta_dictionary_entry();
    entry.entry_kind = DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS;
    entry.inline_value_ref = DELTA_REF_NONE;
    entry.parent_ref = 0;
    entry.parent_dictionary_id = 1;
    entry.parent_code = 2;
    entry.parent_dictionary_digest_ref = 3;
    let mut section = delta_dictionary_overlay_section(2, &[entry]);
    section.entry.required_delta_features = DELTA_FEATURE_INLINE_DICTIONARY;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_INLINE_DICTIONARY,
        vec![section],
    )
}

pub(super) fn covedelta_object_delta_with_parent_alias_dictionary_overlay_unknown_parent_ref_file(
) -> Vec<u8> {
    let mut entry = valid_delta_dictionary_entry();
    entry.entry_kind = DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS;
    entry.inline_value_ref = DELTA_REF_NONE;
    entry.parent_ref = 9;
    entry.parent_dictionary_id = 1;
    entry.parent_code = 2;
    entry.parent_dictionary_digest_ref = 3;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_PARENT_DICTIONARY_ALIASES,
        vec![delta_dictionary_overlay_section(2, &[entry])],
    )
}

pub(super) fn covedelta_object_delta_with_hash_hint_dictionary_overlay_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        vec![delta_dictionary_overlay_section(
            2,
            &[valid_delta_canonical_hash_hint_dictionary_entry()],
        )],
    )
}

pub(super) fn covedelta_object_delta_with_zero_hash_hint_dictionary_overlay_file() -> Vec<u8> {
    let mut entry = valid_delta_canonical_hash_hint_dictionary_entry();
    entry.canonical_hash128 = [0; 16];
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        vec![delta_dictionary_overlay_section(2, &[entry])],
    )
}

pub(super) fn covedelta_object_delta_with_required_hash_hint_dictionary_overlay_file() -> Vec<u8> {
    let entry = valid_delta_canonical_hash_hint_dictionary_entry();
    let mut section = delta_dictionary_overlay_section(2, &[entry]);
    section.entry.required_delta_features = DELTA_FEATURE_INLINE_DICTIONARY;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_INLINE_DICTIONARY,
        vec![section],
    )
}

pub(super) fn covedelta_object_delta_with_duplicate_dictionary_overlay_file() -> Vec<u8> {
    let first = valid_delta_dictionary_entry();
    let mut second = valid_delta_dictionary_entry();
    second.inline_value_ref = 1;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_INLINE_DICTIONARY,
        vec![
            delta_dictionary_overlay_section(2, &[first, second]),
            delta_string_table_section(3, 2),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_descriptor_tables_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        delta_descriptor_table_sections(
            valid_delta_scope_descriptor(),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET),
        ),
    )
}

pub(super) fn covedelta_object_delta_with_sparse_scope_descriptor_file() -> Vec<u8> {
    let mut scope = valid_delta_scope_descriptor();
    scope.scope_ref = 1;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        delta_descriptor_table_sections(
            scope,
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET),
        ),
    )
}

pub(super) fn covedelta_object_delta_with_sparse_summary_descriptor_file() -> Vec<u8> {
    let mut touched = valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET);
    touched.summary_ref = 1;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        delta_descriptor_table_sections(
            valid_delta_scope_descriptor(),
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE),
            touched,
            valid_delta_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET),
        ),
    )
}

pub(super) fn covedelta_object_delta_with_catalog_patch_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_object_type_and_sections(
        &catalog_patch_temporal_rows(),
        2,
        0,
        vec![catalog_patch_section(
            2,
            catalog_patch_new_widget_object_type(),
        )],
    )
}

pub(super) fn covedelta_object_delta_with_projection_patch_file() -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_PROJECTION_PATCH,
        vec![delta_projection_patch_section(
            2,
            delta_projection_patch_payload(),
            DELTA_FEATURE_PROJECTION_PATCH,
        )],
    ))
    .unwrap();
    delta.header.projection_fingerprint_ref = 41;
    delta.serialize().unwrap()
}

pub(super) fn covedelta_object_delta_with_projection_patch_missing_section_file() -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_PROJECTION_PATCH,
        Vec::new(),
    ))
    .unwrap();
    delta.header.projection_fingerprint_ref = 41;
    delta.serialize().unwrap()
}

pub(super) fn covedelta_object_delta_with_projection_patch_missing_fingerprint_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_PROJECTION_PATCH,
        vec![delta_projection_patch_section(
            2,
            delta_projection_patch_payload(),
            DELTA_FEATURE_PROJECTION_PATCH,
        )],
    )
}

pub(super) fn covedelta_object_delta_with_index_hints_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::IndexHints,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        true,
    )
}

pub(super) fn covedelta_object_delta_with_index_hints_missing_section_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_INDEX_HINTS,
        Vec::new(),
    )
}

pub(super) fn covedelta_object_delta_with_index_hints_lineage_parent_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::IndexHints,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        false,
    )
}

pub(super) fn covedelta_object_delta_with_coverage_patch_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::CoveragePatch,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
        true,
    )
}

pub(super) fn covedelta_object_delta_with_coverage_patch_wrong_kind_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::CoveragePatch,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
        true,
    )
}

pub(super) fn covedelta_with_layout_hints_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::LayoutHints,
        0,
        0,
        DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS,
        true,
    )
}

pub(super) fn covedelta_with_layout_hints_wrong_kind_file() -> Vec<u8> {
    covedelta_object_delta_with_sidecar_hint_file(
        CoveDeltaSectionKind::LayoutHints,
        0,
        0,
        DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
        true,
    )
}

pub(super) fn covedelta_object_delta_with_sidecar_hint_file(
    section_kind: CoveDeltaSectionKind,
    header_required_delta_features: u64,
    section_required_delta_features: u64,
    hint_kind: u16,
    ancillary_parent: bool,
) -> Vec<u8> {
    let hint_parent_ref = if ancillary_parent { 1 } else { 0 };
    let mut delta = CoveDeltaFile::parse(&covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        header_required_delta_features,
        vec![delta_sidecar_hint_section(
            2,
            section_kind,
            section_required_delta_features,
            delta_sidecar_hint(hint_parent_ref, hint_kind),
        )],
    ))
    .unwrap();
    if ancillary_parent {
        delta.parent_refs.push(delta_sidecar_parent_ref(1));
    }
    delta.serialize().unwrap()
}

pub(super) fn covedelta_object_delta_with_corrupt_optional_index_file() -> Vec<u8> {
    covedelta_object_delta_with_corrupt_optional_section_file(
        CoveDeltaSectionKind::IndexHints,
        b"corrupt optional delta-local index",
    )
}

pub(super) fn covedelta_object_delta_with_optional_index_unknown_required_file() -> Vec<u8> {
    covedelta_object_delta_with_optional_section_file(
        CoveDeltaSectionKind::IndexHints,
        b"unsupported required optional delta-local index",
        1u64 << 63,
    )
}

pub(super) fn covedelta_object_delta_with_corrupt_optional_section_file(
    section_kind: CoveDeltaSectionKind,
    payload: &[u8],
) -> Vec<u8> {
    covedelta_object_delta_with_optional_section_file(section_kind, payload, 0)
}

pub(super) fn covedelta_object_delta_with_optional_section_file(
    section_kind: CoveDeltaSectionKind,
    payload: &[u8],
    required_delta_features: u64,
) -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: section_kind as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: payload.to_vec(),
        }],
    )
}

pub(super) fn covedelta_object_delta_with_mismatched_sparse_patch_file() -> Vec<u8> {
    let mut record = valid_delta_sparse_patch_record();
    record.goid = [0x01; 16];
    record.changed_properties[0].value_ref = 0;
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_SPARSE_PATCH_ROWS,
        vec![
            delta_property_ops_section(2, record.serialize().unwrap()),
            delta_string_table_section(3, 1),
        ],
    )
}

pub(super) fn covedelta_object_delta_with_evidence_patch_missing_map_fingerprint_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        DELTA_FEATURE_MAP_EVIDENCE_PATCH,
        vec![delta_map_evidence_patch_section(
            2,
            delta_map_evidence_patch_payload(),
        )],
    )
}

pub(super) fn delta_property_ops_section(section_id: u32, payload: Vec<u8>) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::PropertyOps as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: DELTA_FEATURE_SPARSE_PATCH_ROWS,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn delta_dictionary_overlay_section(
    section_id: u32,
    entries: &[DeltaDictionaryEntryV1],
) -> CoveDeltaSection {
    let mut payload = Vec::new();
    for entry in entries {
        payload.extend_from_slice(&entry.serialize().unwrap());
    }
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::DictionaryOverlay as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: entries.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: dictionary_overlay_required_features(entries),
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn delta_string_table_section(section_id: u32, value_count: u32) -> CoveDeltaSection {
    let values = (0..value_count)
        .map(valid_delta_inline_value)
        .collect::<Vec<_>>();
    let payload = values
        .iter()
        .map(DeltaInlineValueV1::serialize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::StringTable as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: values.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn dictionary_overlay_required_features(entries: &[DeltaDictionaryEntryV1]) -> u64 {
    let mut required = 0;
    for entry in entries {
        match entry.entry_kind {
            DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE => {
                required |= DELTA_FEATURE_INLINE_DICTIONARY;
            }
            DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS => {
                required |= DELTA_FEATURE_PARENT_DICTIONARY_ALIASES;
            }
            _ => {}
        }
    }
    required
}

pub(super) fn delta_descriptor_table_sections(
    scope: DeltaScopeDescriptorV1,
    temporal_role: DeltaSummaryDescriptorV1,
    touched: DeltaSummaryDescriptorV1,
    tombstone: DeltaSummaryDescriptorV1,
) -> Vec<CoveDeltaSection> {
    vec![
        delta_scope_descriptor_section(2, &[scope]),
        delta_summary_descriptor_section(
            3,
            CoveDeltaSectionKind::TemporalRoleSummaryTable,
            &[temporal_role],
        ),
        delta_summary_descriptor_section(4, CoveDeltaSectionKind::TouchedSummaryTable, &[touched]),
        delta_summary_descriptor_section(
            5,
            CoveDeltaSectionKind::TombstoneSummaryTable,
            &[tombstone],
        ),
    ]
}

pub(super) fn delta_scope_descriptor_section(
    section_id: u32,
    descriptors: &[DeltaScopeDescriptorV1],
) -> CoveDeltaSection {
    let mut payload = Vec::new();
    for descriptor in descriptors {
        payload.extend_from_slice(&descriptor.serialize().unwrap());
    }
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::ScopeTable as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: descriptors.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn delta_summary_descriptor_section(
    section_id: u32,
    section_kind: CoveDeltaSectionKind,
    descriptors: &[DeltaSummaryDescriptorV1],
) -> CoveDeltaSection {
    let mut payload = Vec::new();
    for descriptor in descriptors {
        payload.extend_from_slice(&descriptor.serialize().unwrap());
    }
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: section_kind as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: descriptors.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn catalog_patch_temporal_rows() -> Vec<TemporalRowEntryV1> {
    vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: [0x0D; 16],
        record_id: [0x0E; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }]
}

pub(super) fn catalog_patch_new_widget_object_type() -> ObjectTypeCatalog {
    ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 2,
            type_name: "Widget".into(),
            flags: cove_core::profile::cove_o::OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: Vec::new(),
        }],
    }
}

pub(super) fn catalog_patch_reinterprets_parent_property() -> ObjectTypeCatalog {
    ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: cove_core::profile::cove_o::OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    }
}

pub(super) fn catalog_patch_section(section_id: u32, patch: ObjectTypeCatalog) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::CatalogPatch as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: patch.types.len() as u64,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: patch.serialize().unwrap(),
    }
}

pub(super) fn covedelta_object_delta_with_tombstone_set_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_tombstone_temporal_rows(),
        DELTA_FEATURE_EXACT_TOMBSTONE_SET,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::TombstoneSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOMBSTONE_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: valid_delta_tombstone_object_range().serialize().to_vec(),
        }],
    )
}

pub(super) fn covedelta_object_membership_scope_case() -> Vec<u8> {
    let matching_scope_id = [0; 16];
    let colliding_scope_id = [1; 16];
    let tombstoned_goid = [0x09; 16];
    serde_json::to_vec_pretty(&json!({
        "delta": covedelta_object_delta_with_touched_tombstone_sets_file(),
        "checks": [
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": tombstoned_goid,
                "expect_touched": "present",
                "expect_can_skip": false,
                "expect_tombstone": "present",
                "expect_suppress_parent_latest_state": true,
                "requested_property_ids": [1],
                "expect_projection_property_skip": false
            },
            {
                "scope_kind": 0,
                "scope_id": colliding_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": tombstoned_goid,
                "expect_touched": "absent",
                "expect_can_skip": true,
                "expect_tombstone": "absent",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [1],
                "expect_projection_property_skip": true
            }
        ]
    }))
    .unwrap()
}

pub(super) fn covedelta_object_projection_property_skip_case() -> Vec<u8> {
    let matching_scope_id = [0; 16];
    let matching_goid = [0; 16];
    let missing_goid = [1; 16];
    serde_json::to_vec_pretty(&json!({
        "delta": covedelta_object_delta_with_sparse_patch_file(),
        "checks": [
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": matching_goid,
                "expect_touched": "unavailable",
                "expect_can_skip": false,
                "expect_tombstone": "unavailable",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [99],
                "expect_projection_property_skip": true
            },
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": matching_goid,
                "expect_touched": "unavailable",
                "expect_can_skip": false,
                "expect_tombstone": "unavailable",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [1],
                "expect_projection_property_skip": false
            },
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": matching_goid,
                "expect_touched": "unavailable",
                "expect_can_skip": false,
                "expect_tombstone": "unavailable",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [4],
                "expect_projection_property_skip": false
            },
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": matching_goid,
                "expect_touched": "unavailable",
                "expect_can_skip": false,
                "expect_tombstone": "unavailable",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [],
                "expect_projection_property_skip": false
            },
            {
                "scope_kind": 0,
                "scope_id": matching_scope_id,
                "object_type_id": 1,
                "branch_identity_ref": 0,
                "goid": missing_goid,
                "expect_touched": "unavailable",
                "expect_can_skip": false,
                "expect_tombstone": "unavailable",
                "expect_suppress_parent_latest_state": false,
                "requested_property_ids": [99],
                "expect_projection_property_skip": false
            }
        ]
    }))
    .unwrap()
}

pub(super) fn covedelta_covi_base_tombstone_overlay_case() -> Vec<u8> {
    let tombstoned_goid = [0x09; 16];
    let base_record_id = [0x51; 16];
    let tombstone_record_id = [0x52; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: tombstoned_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let tombstone_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: tombstoned_goid,
        record_id: tombstone_record_id,
        record_kind: RecordKind::Tombstone,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows_and_sections(
        &tombstone_rows,
        DELTA_FEATURE_EXACT_TOMBSTONE_SET,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::TombstoneSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOMBSTONE_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: valid_delta_tombstone_object_range().serialize().to_vec(),
        }],
    );
    json_fixture_bytes(json!({
        "base": base,
        "delta": delta,
        "covi": covi_object_path_tombstone_overlay_artifact(),
        "context": {
            "file_id": covi_test_file_id(),
            "file_len": covi_test_file_len(),
            "footer_crc32c": covi_test_footer_crc32c(),
            "dataset_id": covi_test_file_id(),
            "allow_file_code_keys": true,
            "file_digest_algorithm": DigestAlgorithm::Sha256 as u16,
            "file_digest": covi_test_digest()
        },
        "operation": {
            "object_type_id": 1,
            "path_ref": 7,
            "key": object_path_key_bytes()
        },
        "expect_base_candidate_count": 1,
        "expect_corrected_candidate_count": 0
    }))
}

pub(super) fn covedelta_tombstone_reconstruction_case() -> Vec<u8> {
    let tombstoned_goid = [0x09; 16];
    let base_record_id = [0x08; 16];
    let tombstone_record_id = [0x0A; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: tombstoned_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let tombstone_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: tombstoned_goid,
        record_id: tombstone_record_id,
        record_kind: RecordKind::Tombstone,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows(&tombstone_rows);

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_states": [],
        "expect_states_with_tombstones": [{
            "object_type_id": 1,
            "branch_key": 0,
            "goid": tombstoned_goid,
            "latest_record_id": tombstone_record_id,
            "latest_timestamp_us": 20,
            "latest_csn": 2,
            "latest_record_kind": "Tombstone",
            "tombstone_status": "tombstoned",
            "property_count": 0
        }]
    }))
    .unwrap()
}

pub(super) fn covedelta_association_update_reconstruction_case() -> Vec<u8> {
    let association_object_type_id = 2;
    let association_goid = [0x31; 16];
    let base_record_id = [0x32; 16];
    let delta_record_id = [0x33; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: association_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: association_goid,
        record_id: delta_record_id,
        record_kind: RecordKind::Delta,
        prev_ref: None,
    }];
    let catalog = association_object_catalog(association_object_type_id);
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section_from_catalog(catalog),
            cove_o_temporal_segment_index_section_for_object_type(
                association_object_type_id,
                &[(5, &base_rows)],
            ),
            cove_o_temporal_segment_data_section_for_object_type(
                5,
                association_object_type_id,
                &base_rows,
            ),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows_object_type_and_sections(
        &delta_rows,
        association_object_type_id,
        0,
        Vec::new(),
    );

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_states": [{
            "object_type_id": association_object_type_id,
            "object_type_name": "Association:member_of",
            "association_type": "member_of",
            "branch_key": 0,
            "goid": association_goid,
            "latest_record_id": delta_record_id,
            "latest_timestamp_us": 20,
            "latest_csn": 2,
            "latest_record_kind": "Delta",
            "tombstone_status": "live",
            "property_count": 0
        }]
    }))
    .unwrap()
}

pub(super) fn covedelta_evidence_patch_inherited_map_fingerprint_case() -> Vec<u8> {
    let inherited_schema_fingerprint_ref = 40u32;
    let inherited_object_catalog_fingerprint_ref = 41u32;
    let inherited_semantic_map_fingerprint_ref = 42u32;
    let inherited_projection_fingerprint_ref = 43u32;
    let patched_projection_fingerprint_ref = 44u32;
    let base_goid = [0x41; 16];
    let base_record_id = [0x42; 16];
    let delta_goid = [0x43; 16];
    let delta_record_id = [0x44; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: base_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: delta_goid,
        record_id: delta_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows_and_sections(
        &delta_rows,
        DELTA_FEATURE_MAP_EVIDENCE_PATCH,
        vec![delta_map_evidence_patch_section(
            2,
            delta_map_evidence_patch_payload(),
        )],
    );
    let mut delta_file = CoveDeltaFile::parse(&delta).unwrap();
    delta_file.parent_refs[0].schema_fingerprint_ref = inherited_schema_fingerprint_ref;
    delta_file.parent_refs[0].object_catalog_fingerprint_ref =
        inherited_object_catalog_fingerprint_ref;
    delta_file.parent_refs[0].semantic_map_fingerprint_ref = inherited_semantic_map_fingerprint_ref;
    delta_file.parent_refs[0].projection_fingerprint_ref = inherited_projection_fingerprint_ref;
    delta_file.header.projection_fingerprint_ref = patched_projection_fingerprint_ref;
    let delta = delta_file.serialize().unwrap();

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_delta_effective_schema_fingerprint_refs": [
            inherited_schema_fingerprint_ref
        ],
        "expect_delta_effective_object_catalog_fingerprint_refs": [
            inherited_object_catalog_fingerprint_ref
        ],
        "expect_delta_effective_semantic_map_fingerprint_refs": [
            inherited_semantic_map_fingerprint_ref
        ],
        "expect_delta_effective_projection_fingerprint_refs": [
            patched_projection_fingerprint_ref
        ],
        "expect_delta_evidence_patch_counts": [1],
        "expect_states": [
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": base_goid,
                "latest_record_id": base_record_id,
                "latest_timestamp_us": 10,
                "latest_csn": 1,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            },
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": delta_goid,
                "latest_record_id": delta_record_id,
                "latest_timestamp_us": 20,
                "latest_csn": 2,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            }
        ],
        "expect_evidence_index": {
            "mapping_id": "crm-map",
            "mapping_version": "v1",
            "entry_count": 1,
            "embedded_evidence_section_count": 1,
            "contains": [{
                "source_id": "crm",
                "source_row_identity": "crm:42",
                "rule_id": "company_identity",
                "assertion_id": "delta-evidence:42",
                "output_object_id": "company:delta-42",
                "operation_metadata": {
                    "source_operation_kind": "insert",
                    "operation_effect": "evidence_addition",
                    "operation_target": "object"
                }
            }]
        }
    }))
    .unwrap()
}

pub(super) fn covedelta_projection_patch_inherited_projection_fingerprint_case() -> Vec<u8> {
    let inherited_projection_fingerprint_ref = 43u32;
    let patched_projection_fingerprint_ref = 44u32;
    let base_goid = [0x51; 16];
    let base_record_id = [0x52; 16];
    let delta_goid = [0x53; 16];
    let delta_record_id = [0x54; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: base_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: delta_goid,
        record_id: delta_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows_and_sections(
        &delta_rows,
        DELTA_FEATURE_PROJECTION_PATCH,
        vec![delta_projection_patch_section(
            2,
            delta_projection_patch_payload(),
            DELTA_FEATURE_PROJECTION_PATCH,
        )],
    );
    let mut delta_file = CoveDeltaFile::parse(&delta).unwrap();
    delta_file.parent_refs[0].projection_fingerprint_ref = inherited_projection_fingerprint_ref;
    delta_file.header.projection_fingerprint_ref = patched_projection_fingerprint_ref;
    let delta = delta_file.serialize().unwrap();

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_delta_effective_projection_fingerprint_refs": [
            patched_projection_fingerprint_ref
        ],
        "expect_delta_projection_patch_counts": [1],
        "expect_states": [
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": base_goid,
                "latest_record_id": base_record_id,
                "latest_timestamp_us": 10,
                "latest_csn": 1,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            },
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": delta_goid,
                "latest_record_id": delta_record_id,
                "latest_timestamp_us": 20,
                "latest_csn": 2,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            }
        ]
    }))
    .unwrap()
}

pub(super) fn delta_map_evidence_patch_payload() -> Vec<u8> {
    map_payload_bytes(covemap_payload_value(
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "crm-map",
            "mapping_version": "v1",
            "entries": [{
                "source_id": "crm",
                "source_row_identity": "crm:42",
                "rule_id": "company_identity",
                "assertion_id": "delta-evidence:42",
                "output_object_id": "company:delta-42",
                "observed_schema_fingerprint": "schema:v1",
                "observed_snapshot_digest": "digest:v1",
                "operation_metadata": {
                    "source_operation_kind": "insert",
                    "operation_effect": "evidence_addition",
                    "operation_target": "object"
                }
            }]
        }),
    ))
}

pub(super) fn delta_map_evidence_patch_section(
    section_id: u32,
    payload: Vec<u8>,
) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::EvidencePatch as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: DELTA_FEATURE_MAP_EVIDENCE_PATCH,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn delta_projection_patch_payload() -> Vec<u8> {
    map_payload_bytes(covemap_payload_value(
        SectionKind::MapProjectionCatalog,
        json!({
            "mapping_id": "crm-map",
            "mapping_version": "v1",
            "projections": [{
                "projection_id": "delta_projection",
                "output_table": "delta_projection",
                "row_grain": "one_row_per_object",
                "anchor": {"object_type": "Company"},
                "temporal_mode": {"as_of": "latest_committed"},
                "multi_value_policy": "first",
                "columns": [{
                    "name": "company_name",
                    "value": "name",
                    "logical_type": "utf8"
                }],
                "output_modes": ["json"]
            }]
        }),
    ))
}

pub(super) fn delta_projection_patch_section(
    section_id: u32,
    payload: Vec<u8>,
    required_delta_features: u64,
) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::ProjectionPatch as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

pub(super) fn delta_sidecar_parent_ref(parent_ref: u32) -> DeltaParentRefV1 {
    DeltaParentRefV1 {
        parent_ref,
        parent_kind: 0,
        flags: 0,
        artifact_id: [0xD0; 16],
        snapshot_id: [0xD1; 16],
        file_len: 4096,
        footer_crc32c: checksum::crc32c(b"delta-sidecar-footer"),
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest_ref: parent_ref + 2,
        uri_ref: parent_ref + 3,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    }
}

pub(super) fn delta_sidecar_hint(parent_ref: u32, hint_kind: u16) -> DeltaSidecarHintV1 {
    DeltaSidecarHintV1 {
        hint_ref: 0,
        hint_kind,
        flags: 0,
        parent_ref,
        target_section_id: 7,
        scope_ref: DELTA_REF_NONE,
        object_type_id: 1,
        chain_digest_ref: 5,
        checksum: 0,
    }
}

pub(super) fn delta_sidecar_hint_section(
    section_id: u32,
    section_kind: CoveDeltaSectionKind,
    required_delta_features: u64,
    hint: DeltaSidecarHintV1,
) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: section_kind as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: hint.serialize().to_vec(),
    }
}

pub(super) fn association_object_catalog(association_object_type_id: u32) -> ObjectTypeCatalog {
    ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: association_object_type_id,
            type_name: "Association:member_of".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT,
            properties: Vec::new(),
        }],
    }
}

pub(super) fn covedelta_base_plus_one_delta_reconstruction_case() -> Vec<u8> {
    let base_goid = [0x01; 16];
    let base_record_id = [0x02; 16];
    let delta_goid = [0x0D; 16];
    let delta_record_id = [0x0E; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: base_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: delta_goid,
        record_id: delta_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows(&delta_rows);

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_states": [
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": base_goid,
                "latest_record_id": base_record_id,
                "latest_timestamp_us": 10,
                "latest_csn": 1,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            },
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": delta_goid,
                "latest_record_id": delta_record_id,
                "latest_timestamp_us": 20,
                "latest_csn": 2,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            }
        ]
    }))
    .unwrap()
}

pub(super) fn covedelta_compaction_equivalence_case() -> Vec<u8> {
    let base_goid = [0x01; 16];
    let base_record_id = [0x02; 16];
    let delta_goid = [0x0D; 16];
    let delta_record_id = [0x0E; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: base_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: delta_goid,
        record_id: delta_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let mut compacted_rows = base_rows.clone();
    compacted_rows.extend(delta_rows.iter().cloned());
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows(&delta_rows);
    let compacted = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &compacted_rows)]),
            cove_o_temporal_segment_data_section(5, &compacted_rows),
        ],
    );

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "compacted": compacted,
        "expect_states": [
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": base_goid,
                "latest_record_id": base_record_id,
                "latest_timestamp_us": 10,
                "latest_csn": 1,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            },
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": delta_goid,
                "latest_record_id": delta_record_id,
                "latest_timestamp_us": 20,
                "latest_csn": 2,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            }
        ]
    }))
    .unwrap()
}

pub(super) fn covedelta_catalog_patch_reconstruction_case() -> Vec<u8> {
    let base_goid = [0x01; 16];
    let base_record_id = [0x02; 16];
    let delta_goid = [0x0D; 16];
    let delta_record_id = [0x0E; 16];
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: base_goid,
        record_id: base_record_id,
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_with_catalog_patch_file();

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_states": [
            {
                "object_type_id": 1,
                "branch_key": 0,
                "goid": base_goid,
                "latest_record_id": base_record_id,
                "latest_timestamp_us": 10,
                "latest_csn": 1,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            },
            {
                "object_type_id": 2,
                "branch_key": 0,
                "goid": delta_goid,
                "latest_record_id": delta_record_id,
                "latest_timestamp_us": 20,
                "latest_csn": 2,
                "latest_record_kind": "Baseline",
                "tombstone_status": "live",
                "property_count": 0
            }
        ]
    }))
    .unwrap()
}

pub(super) fn covedelta_catalog_patch_reinterprets_parent_case() -> Vec<u8> {
    let base_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0x01; 16],
        record_id: [0x02; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let delta_rows = catalog_patch_temporal_rows();
    let base = semantic_profile_cove_file(
        PrimaryProfile::ObjectTemporal,
        FEATURE_OBJECT_PROFILE,
        0,
        vec![
            cove_o_object_catalog_section(),
            cove_o_temporal_segment_index_section(&[(5, &base_rows)]),
            cove_o_temporal_segment_data_section(5, &base_rows),
        ],
    );
    let delta = covedelta_object_delta_file_with_rows_and_sections(
        &delta_rows,
        0,
        vec![catalog_patch_section(
            2,
            catalog_patch_reinterprets_parent_property(),
        )],
    );

    serde_json::to_vec_pretty(&json!({
        "base": base,
        "deltas": [delta],
        "expect_states": []
    }))
    .unwrap()
}

pub(super) fn covedelta_object_delta_with_touched_tombstone_sets_file() -> Vec<u8> {
    let touched = valid_delta_tombstone_object_range();
    let tombstone = valid_delta_tombstone_object_range();
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_tombstone_temporal_rows(),
        DELTA_FEATURE_EXACT_TOUCHED_SET | DELTA_FEATURE_EXACT_TOMBSTONE_SET,
        vec![
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 2,
                    section_kind: CoveDeltaSectionKind::TouchedObjectSet as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_EXACT_TOUCHED_SET,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: touched.serialize().to_vec(),
            },
            CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id: 3,
                    section_kind: CoveDeltaSectionKind::TombstoneSet as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_EXACT_TOMBSTONE_SET,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: tombstone.serialize().to_vec(),
            },
        ],
    )
}

pub(super) fn covedelta_object_delta_with_underinclusive_tombstone_file() -> Vec<u8> {
    let mut tombstone = valid_delta_tombstone_object_range();
    tombstone.min_goid = [0x0B; 16];
    tombstone.max_goid = [0x0B; 16];
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_tombstone_temporal_rows(),
        DELTA_FEATURE_EXACT_TOMBSTONE_SET,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::TombstoneSet as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: DELTA_FEATURE_EXACT_TOMBSTONE_SET,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: tombstone.serialize().to_vec(),
        }],
    )
}

pub(super) fn covedelta_object_delta_with_branch_identity_file() -> Vec<u8> {
    covedelta_object_delta_file_with_rows_and_sections(
        &valid_temporal_rows(),
        0,
        vec![CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 2,
                section_kind: CoveDeltaSectionKind::BranchIdentityTable as u16,
                flags: 0,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                item_count: 1,
                compression: 0,
                encryption: 0,
                alignment_log2: 0,
                reserved0: 0,
                required_delta_features: 0,
                optional_delta_features: 0,
                crc32c: 0,
                checksum: 0,
            },
            payload: valid_delta_branch_identity().serialize().to_vec(),
        }],
    )
}

pub(super) fn covedelta_object_delta_file_with_rows_and_sections(
    rows: &[TemporalRowEntryV1],
    required_delta_features: u64,
    extra_sections: Vec<CoveDeltaSection>,
) -> Vec<u8> {
    covedelta_object_delta_file_with_rows_object_type_and_sections(
        rows,
        1,
        required_delta_features,
        extra_sections,
    )
}

pub(super) fn covedelta_object_delta_file_with_rows_object_type_and_sections(
    rows: &[TemporalRowEntryV1],
    object_type_id: u32,
    required_delta_features: u64,
    mut extra_sections: Vec<CoveDeltaSection>,
) -> Vec<u8> {
    let mut header = CoveDeltaHeaderV1::new([0x80; 16], [0x81; 16], [0x82; 16], [0x83; 16]);
    header.chain_ordinal = 1;
    header.chain_depth = 1;
    header.required_delta_features = required_delta_features;
    header.csn_min = rows.first().map(|row| row.csn).unwrap_or(0);
    header.csn_max = rows.last().map(|row| row.csn).unwrap_or(0);
    header.commit_time_range_start_us = rows.first().map(|row| row.timestamp_us).unwrap_or(0);
    header.commit_time_range_end_us = rows.last().map(|row| row.timestamp_us).unwrap_or(0);
    let mut sections = vec![CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 1,
            section_kind: CoveDeltaSectionKind::TemporalSegmentData as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: temporal_segment_data_payload_for_object_type(5, object_type_id, rows),
    }];
    sections.append(&mut extra_sections);
    CoveDeltaFile {
        header,
        parent_refs: vec![DeltaParentRefV1 {
            parent_ref: 0,
            parent_kind: 0,
            flags: DELTA_PARENT_REF_LINEAGE_PARENT,
            artifact_id: [0x84; 16],
            snapshot_id: [0x83; 16],
            file_len: 244,
            footer_crc32c: checksum::crc32c(b"footer"),
            digest_algorithm: 1,
            digest_len: 32,
            digest_ref: 0,
            uri_ref: 1,
            schema_fingerprint_ref: 0,
            object_catalog_fingerprint_ref: 0,
            semantic_map_fingerprint_ref: 0,
            projection_fingerprint_ref: 0,
            checksum: 0,
        }],
        sections,
        footer: CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: COVEDELTA_HEADER_LEN as u64,
            section_directory_offset: 0,
            section_directory_length: 0,
            section_count: 0,
            parent_ref_count: 0,
            footer_crc32c: 0,
            checksum: 0,
        },
        postscript: CoveDeltaPostscriptV1 {
            required_delta_features,
            optional_delta_features: 0,
            file_len: 0,
            footer_offset: 0,
            footer_length: COVEDELTA_FOOTER_LEN as u64,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn rewrite_covedelta_required_features(bytes: &mut [u8], required_delta_features: u64) {
    bytes[14..22].copy_from_slice(&required_delta_features.to_le_bytes());
    bytes[COVEDELTA_HEADER_LEN as usize - 4..COVEDELTA_HEADER_LEN as usize].fill(0);
    let header_crc = checksum::crc32c(&bytes[..COVEDELTA_HEADER_LEN as usize]);
    bytes[COVEDELTA_HEADER_LEN as usize - 4..COVEDELTA_HEADER_LEN as usize]
        .copy_from_slice(&header_crc.to_le_bytes());

    let postscript_start =
        bytes.len() - COVEDELTA_POSTSCRIPT_TAIL_SIZE - COVEDELTA_POSTSCRIPT_LEN as usize;
    let postscript_end = postscript_start + COVEDELTA_POSTSCRIPT_LEN as usize;
    bytes[postscript_start..postscript_start + 8]
        .copy_from_slice(&required_delta_features.to_le_bytes());
    bytes[postscript_end - 4..postscript_end].fill(0);
    let postscript_crc = checksum::crc32c(&bytes[postscript_start..postscript_end]);
    bytes[postscript_end - 4..postscript_end].copy_from_slice(&postscript_crc.to_le_bytes());
}

pub(super) fn valid_delta_continuation_anchor() -> DeltaContinuationAnchorV1 {
    DeltaContinuationAnchorV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        goid: [0; 16],
        parent_ref: 0,
        predecessor_csn: 0,
        predecessor_timestamp_us: 0,
        predecessor_record_id: [0xA2; 16],
        predecessor_state_hash_ref: 0,
        predecessor_trust_hash_ref: DELTA_REF_NONE,
        anchor_strength: DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn valid_delta_state_hash_section(section_id: u32) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id,
            section_kind: CoveDeltaSectionKind::StateHashTable as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: valid_delta_state_hash_descriptor().serialize().to_vec(),
    }
}

pub(super) fn valid_delta_state_hash_descriptor() -> DeltaStateHashDescriptorV1 {
    DeltaStateHashDescriptorV1 {
        state_hash_ref: 0,
        state_hash_kind: DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1,
        hash_algorithm: DigestAlgorithm::Sha256 as u16,
        hash_len: 32,
        hash_payload_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn covedelta_state_hash_recompute_case() -> Vec<u8> {
    let state = valid_delta_state_hash_material();
    let descriptor = valid_delta_state_hash_descriptor();
    let stored_hash = state.compute_hash(DigestAlgorithm::Sha256).unwrap();
    serde_json::to_vec_pretty(&json!({
        "descriptor": descriptor.serialize(),
        "stored_hash": stored_hash,
        "state": {
            "scope_kind": state.scope_kind,
            "scope_id": state.scope_id,
            "canonical_branch_identity": state.canonical_branch_identity.clone(),
            "object_type_id": state.object_type_id,
            "goid": state.goid,
            "predecessor_record_id": state.predecessor_record_id,
            "predecessor_csn": state.predecessor_csn,
            "predecessor_timestamp_us": state.predecessor_timestamp_us,
            "record_kind": "Delta",
            "tombstone_state": state.tombstone_state,
            "properties": state.properties.iter().map(|property| {
                json!({
                    "property_id": property.property_id,
                    "logical_type": property.logical_type,
                    "collation_id": property.collation_id,
                    "value_state": property.value_state,
                    "canonical_value": property.canonical_value.clone(),
                    "redaction_commitment": property.redaction_commitment.clone(),
                    "hidden_value_commitment": property.hidden_value_commitment.clone()
                })
            }).collect::<Vec<_>>()
        }
    }))
    .unwrap()
}

pub(super) fn valid_delta_state_hash_material() -> CoveObjectDeltaStateHashV1 {
    CoveObjectDeltaStateHashV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        canonical_branch_identity: b"main".to_vec(),
        object_type_id: 1,
        goid: [7; 16],
        predecessor_record_id: [3; 16],
        predecessor_csn: 0,
        predecessor_timestamp_us: 0,
        record_kind: RecordKind::Delta,
        tombstone_state: DELTA_OBJECT_STATE_TOMBSTONE_LIVE,
        properties: vec![
            CoveObjectDeltaStateHashPropertyV1 {
                property_id: 1,
                logical_type: CoveLogicalType::Utf8 as u16,
                collation_id: 0,
                value_state: DELTA_OBJECT_STATE_VALUE_VISIBLE,
                canonical_value: b"alice".to_vec(),
                redaction_commitment: Vec::new(),
                hidden_value_commitment: None,
            },
            CoveObjectDeltaStateHashPropertyV1 {
                property_id: 2,
                logical_type: CoveLogicalType::Utf8 as u16,
                collation_id: 0,
                value_state: DELTA_OBJECT_STATE_VALUE_NULL,
                canonical_value: Vec::new(),
                redaction_commitment: Vec::new(),
                hidden_value_commitment: None,
            },
            CoveObjectDeltaStateHashPropertyV1 {
                property_id: 3,
                logical_type: CoveLogicalType::Utf8 as u16,
                collation_id: 0,
                value_state: DELTA_OBJECT_STATE_VALUE_REDACTED,
                canonical_value: Vec::new(),
                redaction_commitment: b"redaction-commitment".to_vec(),
                hidden_value_commitment: None,
            },
        ],
    }
}

pub(super) fn valid_delta_sparse_patch_record() -> DeltaSparsePatchRecordV1 {
    DeltaSparsePatchRecordV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        branch_identity_ref: 0,
        object_type_id: 1,
        goid: [0; 16],
        record_id: [0; 16],
        timestamp_us: 10,
        csn: 1,
        record_kind: RecordKind::Delta,
        flags: 0,
        changed_properties: vec![
            DeltaSparsePatchPropertyOpV1 {
                property_id: 1,
                property_op: DELTA_PROPERTY_OP_SET_VALUE,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: 11,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
            DeltaSparsePatchPropertyOpV1 {
                property_id: 2,
                property_op: DELTA_PROPERTY_OP_SET_NULL,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: DELTA_REF_NONE,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
            DeltaSparsePatchPropertyOpV1 {
                property_id: 3,
                property_op: DELTA_PROPERTY_OP_CLEAR,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: DELTA_REF_NONE,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
            DeltaSparsePatchPropertyOpV1 {
                property_id: 4,
                property_op: DELTA_PROPERTY_OP_TOMBSTONE,
                tombstone_kind: DELTA_TOMBSTONE_KIND_PROPERTY,
                value_ref: DELTA_REF_NONE,
                redaction_ref: DELTA_REF_NONE,
                flags: 0,
            },
            DeltaSparsePatchPropertyOpV1 {
                property_id: 5,
                property_op: DELTA_PROPERTY_OP_REDACT,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: DELTA_REF_NONE,
                redaction_ref: 12,
                flags: 0,
            },
        ],
        checksum: 0,
    }
}

pub(super) fn valid_delta_dictionary_entry() -> DeltaDictionaryEntryV1 {
    DeltaDictionaryEntryV1 {
        local_dictionary_id: 1,
        local_code: 7,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        entry_kind: DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE,
        flags: 0,
        inline_value_ref: 0,
        parent_ref: DELTA_REF_NONE,
        parent_dictionary_id: 0,
        parent_code: 0,
        parent_dictionary_digest_ref: DELTA_REF_NONE,
        canonical_hash128: [0; 16],
        checksum: 0,
    }
}

pub(super) fn valid_delta_inline_value(value_ref: u32) -> DeltaInlineValueV1 {
    DeltaInlineValueV1 {
        value_ref,
        value_tag: ValueTag::Utf8 as u16,
        flags: 0,
        value: CanonicalValue::Utf8("delta-value").encode().unwrap(),
        checksum: 0,
    }
}

pub(super) fn valid_delta_canonical_hash_hint_dictionary_entry() -> DeltaDictionaryEntryV1 {
    DeltaDictionaryEntryV1 {
        local_dictionary_id: 2,
        local_code: 9,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        entry_kind: DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT,
        flags: 0,
        inline_value_ref: DELTA_REF_NONE,
        parent_ref: DELTA_REF_NONE,
        parent_dictionary_id: 0,
        parent_code: 0,
        parent_dictionary_digest_ref: DELTA_REF_NONE,
        canonical_hash128: [0x5A; 16],
        checksum: 0,
    }
}

pub(super) fn valid_delta_scope_descriptor() -> DeltaScopeDescriptorV1 {
    DeltaScopeDescriptorV1 {
        scope_ref: 0,
        scope_kind: 0,
        flags: 0,
        scope_id: [0; 16],
        checksum: 0,
    }
}

pub(super) fn valid_delta_summary_descriptor(summary_kind: u8) -> DeltaSummaryDescriptorV1 {
    DeltaSummaryDescriptorV1 {
        summary_ref: 0,
        summary_kind,
        flags: 0,
        payload_ref: 0,
        item_count: 1,
        checksum: 0,
    }
}

pub(super) fn covedelta_sparse_patch_state_reconstruction_case() -> Vec<u8> {
    let mut first = valid_delta_sparse_patch_record();
    first.record_id = [1; 16];
    first.timestamp_us = 10;
    first.csn = 1;

    let mut second = valid_delta_sparse_patch_record();
    second.record_id = [2; 16];
    second.timestamp_us = 20;
    second.csn = 2;
    second.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
        property_id: 1,
        property_op: DELTA_PROPERTY_OP_SET_VALUE,
        tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
        value_ref: 22,
        redaction_ref: DELTA_REF_NONE,
        flags: 0,
    }];

    serde_json::to_vec_pretty(&json!({
        "records": [
            first.serialize().unwrap(),
            second.serialize().unwrap()
        ],
        "expect_state": {
            "scope_kind": first.scope_kind,
            "scope_id": first.scope_id,
            "branch_identity_ref": first.branch_identity_ref,
            "object_type_id": first.object_type_id,
            "goid": first.goid,
            "latest_record_id": second.record_id,
            "latest_timestamp_us": second.timestamp_us,
            "latest_csn": second.csn,
            "latest_record_kind": "Delta",
            "tombstone_status": "live",
            "properties": [
                {
                    "property_id": 1,
                    "state": "value_ref",
                    "value_ref": 22
                },
                {
                    "property_id": 2,
                    "state": "null"
                },
                {
                    "property_id": 3,
                    "state": "clear"
                },
                {
                    "property_id": 4,
                    "state": "tombstone",
                    "tombstone_kind": DELTA_TOMBSTONE_KIND_PROPERTY
                },
                {
                    "property_id": 5,
                    "state": "redacted",
                    "redaction_ref": 12
                }
            ]
        }
    }))
    .unwrap()
}

pub(super) fn sparse_patch_record_missing_value_ref_bytes() -> Vec<u8> {
    let mut bytes = valid_delta_sparse_patch_record().serialize().unwrap();
    let value_ref_offset = DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + 4 + 1 + 1 + 2;
    bytes[value_ref_offset..value_ref_offset + 4].copy_from_slice(&DELTA_REF_NONE.to_le_bytes());
    rewrite_sparse_patch_record_crc(&mut bytes);
    bytes
}

pub(super) fn sparse_patch_record_null_with_payload_bytes() -> Vec<u8> {
    let mut bytes = valid_delta_sparse_patch_record().serialize().unwrap();
    let property_offset = DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + DELTA_SPARSE_PATCH_PROPERTY_OP_LEN;
    let value_ref_offset = property_offset + 4 + 1 + 1 + 2;
    bytes[value_ref_offset..value_ref_offset + 4].copy_from_slice(&99u32.to_le_bytes());
    rewrite_sparse_patch_record_crc(&mut bytes);
    bytes
}

pub(super) fn sparse_patch_record_bad_tombstone_kind_bytes() -> Vec<u8> {
    let mut bytes = valid_delta_sparse_patch_record().serialize().unwrap();
    let property_offset =
        DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + (3 * DELTA_SPARSE_PATCH_PROPERTY_OP_LEN);
    let tombstone_kind_offset = property_offset + 4 + 1;
    bytes[tombstone_kind_offset] = DELTA_TOMBSTONE_KIND_NONE;
    rewrite_sparse_patch_record_crc(&mut bytes);
    bytes
}

pub(super) fn sparse_patch_record_missing_redaction_ref_bytes() -> Vec<u8> {
    let mut bytes = valid_delta_sparse_patch_record().serialize().unwrap();
    let property_offset =
        DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + (4 * DELTA_SPARSE_PATCH_PROPERTY_OP_LEN);
    let redaction_ref_offset = property_offset + 4 + 1 + 1 + 2 + 4;
    bytes[redaction_ref_offset..redaction_ref_offset + 4]
        .copy_from_slice(&DELTA_REF_NONE.to_le_bytes());
    rewrite_sparse_patch_record_crc(&mut bytes);
    bytes
}

pub(super) fn sparse_patch_record_unsorted_properties_bytes() -> Vec<u8> {
    let mut bytes = valid_delta_sparse_patch_record().serialize().unwrap();
    let first_property_offset = DELTA_SPARSE_PATCH_RECORD_HEADER_LEN;
    let second_property_offset =
        DELTA_SPARSE_PATCH_RECORD_HEADER_LEN + DELTA_SPARSE_PATCH_PROPERTY_OP_LEN;
    bytes[first_property_offset..first_property_offset + 4].copy_from_slice(&2u32.to_le_bytes());
    bytes[second_property_offset..second_property_offset + 4].copy_from_slice(&1u32.to_le_bytes());
    rewrite_sparse_patch_record_crc(&mut bytes);
    bytes
}

pub(super) fn rewrite_sparse_patch_record_crc(bytes: &mut [u8]) {
    let checksum_offset = bytes.len() - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(bytes);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn valid_delta_branch_identity() -> DeltaBranchIdentityV1 {
    DeltaBranchIdentityV1 {
        branch_identity_ref: 0,
        branch_identity_kind: DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF,
        flags: 0,
        branch_value_ref: 11,
        branch_hash128: [0xA5; 16],
        branch_catalog_fingerprint_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_delta_touched_object_range() -> DeltaTouchedObjectRangeV1 {
    DeltaTouchedObjectRangeV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        min_goid: [0; 16],
        max_goid: [0; 16],
        touched_count: 1,
        property_bitmap_ref: DELTA_REF_NONE,
        object_set_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_delta_tombstone_object_range() -> DeltaTouchedObjectRangeV1 {
    DeltaTouchedObjectRangeV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        min_goid: [0x09; 16],
        max_goid: [0x09; 16],
        touched_count: 1,
        property_bitmap_ref: DELTA_REF_NONE,
        object_set_ref: 0,
        checksum: 0,
    }
}

pub(super) fn valid_covm_delta_chain_extension() -> Vec<u8> {
    valid_covm_delta_chain_extension_model()
        .serialize()
        .unwrap()
}

pub(super) fn covm_delta_chain_extension_duplicate_artifact_id() -> Vec<u8> {
    let base_snapshot_id = [0x91; 16];
    let mid_snapshot_id = [0x92; 16];
    let result_snapshot_id = [0x95; 16];
    let extension = CovmDeltaChainExtensionV1::new(
        [0x90; 16],
        base_snapshot_id,
        result_snapshot_id,
        covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]),
        vec![
            covm_delta_artifact_ref(1, 0x94, mid_snapshot_id, base_snapshot_id),
            covm_delta_artifact_ref(2, 0x95, result_snapshot_id, mid_snapshot_id),
        ],
    );
    let mut bytes = extension.serialize().unwrap();
    rewrite_covm_delta_ref_artifact_id(&mut bytes, 1, [0x94; 16]);
    bytes
}

pub(super) fn valid_covm_delta_chain_extension_model() -> CovmDeltaChainExtensionV1 {
    let base_snapshot_id = [0x91; 16];
    let result_snapshot_id = [0x92; 16];
    let mut extension = CovmDeltaChainExtensionV1::new(
        [0x90; 16],
        base_snapshot_id,
        result_snapshot_id,
        covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]),
        vec![covm_delta_artifact_ref(
            1,
            0x94,
            result_snapshot_id,
            base_snapshot_id,
        )],
    );
    extension.csn_min = 7;
    extension.csn_max = 8;
    extension.created_at_us = 1_700_000_000_000_000;
    extension.effective_schema_fingerprint_ref = 1;
    extension.effective_object_catalog_fingerprint_ref = 2;
    extension.effective_projection_fingerprint_ref = 3;
    extension.effective_semantic_map_fingerprint_ref = 4;
    CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap()
}

pub(super) fn valid_covm_delta_chain_summary() -> Vec<u8> {
    let extension = valid_covm_delta_chain_extension_model();
    CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![covm_delta_summary_entry(
            extension.ordered_delta_artifact_refs[0].clone(),
        )],
    )
    .serialize()
    .unwrap()
}

pub(super) fn covm_delta_chain_selection_case(corrupt_delta_digest: bool) -> Vec<u8> {
    covm_delta_chain_selection_case_with_summary_mutation(corrupt_delta_digest, |_| {})
}

pub(super) fn covm_delta_chain_selection_delta_count_case(delta_count: usize) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(&covm_delta_chain_selection_case(false)).unwrap();
    let delta = value
        .get("deltas")
        .and_then(serde_json::Value::as_array)
        .and_then(|deltas| deltas.first())
        .cloned()
        .unwrap();
    let deltas = (0..delta_count).map(|_| delta.clone()).collect::<Vec<_>>();
    value
        .as_object_mut()
        .unwrap()
        .insert("deltas".into(), json!(deltas));
    json_fixture_bytes(value)
}

pub(super) fn covm_delta_chain_selection_summary_underinclude_csn_case() -> Vec<u8> {
    covm_delta_chain_selection_case_with_summary_mutation(false, |entry| {
        entry.csn_min = entry.csn_min.saturating_add(1);
    })
}

pub(super) fn covm_delta_chain_selection_summary_underinclude_commit_case() -> Vec<u8> {
    covm_delta_chain_selection_case_with_summary_mutation(false, |entry| {
        entry.commit_time_end_us -= 1;
    })
}

pub(super) fn covm_delta_chain_selection_wrong_parent_snapshot_case() -> Vec<u8> {
    let delta_bytes = valid_covedelta_object_delta_file();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, &delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);

    let declared_base_snapshot_id = [0xEE; 16];
    let result_snapshot_id = delta.header.snapshot_id;
    let dataset_id = delta.header.dataset_id;
    let delta_ref = CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: declared_base_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    };
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        declared_base_snapshot_id,
        result_snapshot_id,
        covm_delta_artifact_ref(0, 0x93, declared_base_snapshot_id, [0; 16]),
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
    let mut summary_entry =
        covm_delta_summary_entry(extension.ordered_delta_artifact_refs[0].clone());
    summary_entry.required_delta_features = delta.header.required_delta_features;
    summary_entry.optional_delta_features = delta.header.optional_delta_features;
    summary_entry.csn_min = delta.header.csn_min;
    summary_entry.csn_max = delta.header.csn_max;
    summary_entry.commit_time_start_us = delta.header.commit_time_range_start_us;
    summary_entry.commit_time_end_us = delta.header.commit_time_range_end_us;
    let summary = CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![summary_entry],
    );
    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "summary": summary.serialize().unwrap(),
        "deltas": [delta_bytes]
    }))
}

pub(super) fn covm_delta_chain_selection_with_base_case(stale_lineage_parent: bool) -> Vec<u8> {
    let dataset_id = [0x81; 16];
    let base_snapshot_id = [0x82; 16];
    let result_snapshot_id = [0x83; 16];
    let base_bytes = b"base snapshot artifact bytes".to_vec();
    let base_ref = covm_base_artifact_ref_from_bytes(&base_bytes, [0x93; 16], base_snapshot_id);

    let delta_bytes = selected_chain_covedelta_file(
        [0x94; 16],
        dataset_id,
        result_snapshot_id,
        base_snapshot_id,
        1,
    );
    let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    align_covedelta_lineage_parent_with_covm_ref(&mut delta, &base_ref);
    if stale_lineage_parent {
        delta.parent_refs[0].artifact_id = [0xEF; 16];
    }
    let delta_bytes = delta.serialize().unwrap();
    let delta_ref = covm_delta_artifact_ref_from_delta_bytes(&delta_bytes);
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_ref,
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let mut summary_entry =
        covm_delta_summary_entry(extension.ordered_delta_artifact_refs[0].clone());
    summary_entry.required_delta_features = delta.header.required_delta_features;
    summary_entry.optional_delta_features = delta.header.optional_delta_features;
    summary_entry.csn_min = delta.header.csn_min;
    summary_entry.csn_max = delta.header.csn_max;
    summary_entry.commit_time_start_us = delta.header.commit_time_range_start_us;
    summary_entry.commit_time_end_us = delta.header.commit_time_range_end_us;
    let summary = CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![summary_entry],
    );

    json_fixture_bytes(json!({
        "base": base_bytes,
        "extension": extension.serialize().unwrap(),
        "summary": summary.serialize().unwrap(),
        "deltas": [delta_bytes]
    }))
}

pub(super) fn covm_delta_chain_selection_reordered_deltas_case() -> Vec<u8> {
    let dataset_id = [0x81; 16];
    let base_snapshot_id = [0x82; 16];
    let mid_snapshot_id = [0x83; 16];
    let result_snapshot_id = [0x84; 16];
    let first_delta_bytes =
        selected_chain_covedelta_file([0x90; 16], dataset_id, mid_snapshot_id, base_snapshot_id, 1);
    let second_delta_bytes = selected_chain_covedelta_file(
        [0x91; 16],
        dataset_id,
        result_snapshot_id,
        mid_snapshot_id,
        2,
    );
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]),
        vec![
            covm_delta_artifact_ref_from_delta_bytes(&first_delta_bytes),
            covm_delta_artifact_ref_from_delta_bytes(&second_delta_bytes),
        ],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "deltas": [second_delta_bytes, first_delta_bytes]
    }))
}

pub(super) fn covm_delta_chain_selection_required_summary_case(
    corrupt_summary: bool,
    include_summary: bool,
) -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&valid_covedelta_object_delta_file()).unwrap();
    let base_snapshot_id = delta.header.parent_snapshot_id;
    let result_snapshot_id = delta.header.snapshot_id;
    let dataset_id = delta.header.dataset_id;
    let base_ref = covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]);
    align_covedelta_lineage_parent_with_covm_ref(&mut delta, &base_ref);
    let delta_bytes = delta.serialize().unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, &delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);

    let delta_ref = CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: delta.header.parent_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    };
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_ref,
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
    let mut summary_entry =
        covm_delta_summary_entry(extension.ordered_delta_artifact_refs[0].clone());
    summary_entry.required_delta_features = delta.header.required_delta_features;
    summary_entry.optional_delta_features = delta.header.optional_delta_features;
    summary_entry.csn_min = delta.header.csn_min;
    summary_entry.csn_max = delta.header.csn_max;
    summary_entry.commit_time_start_us = delta.header.commit_time_range_start_us;
    summary_entry.commit_time_end_us = delta.header.commit_time_range_end_us;
    let summary = CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![summary_entry],
    );
    let mut summary_bytes = summary.serialize().unwrap();

    let mut extension = extension;
    extension.chain_summary_kind = COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1;
    extension.chain_summary_ref = 1;
    extension.chain_summary_offset = 0;
    extension.chain_summary_length = summary_bytes.len() as u64;
    extension.chain_summary_crc32c = checksum::crc32c(&summary_bytes);
    extension.chain_summary_digest_algorithm = DigestAlgorithm::Sha256 as u16;
    extension.chain_summary_digest =
        compute_digest(DigestAlgorithm::Sha256, &summary_bytes).unwrap();
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

    if corrupt_summary {
        let last = summary_bytes.len() - 1;
        summary_bytes[last] ^= 0xFF;
    }

    let mut fixture = json!({
        "extension": extension.serialize().unwrap(),
        "deltas": [delta_bytes]
    });
    if include_summary {
        fixture
            .as_object_mut()
            .unwrap()
            .insert("summary".into(), json!(summary_bytes));
    }
    json_fixture_bytes(fixture)
}

pub(super) fn covm_delta_chain_selection_summary_wrong_chain_digest_case() -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&valid_covedelta_object_delta_file()).unwrap();
    let base_snapshot_id = delta.header.parent_snapshot_id;
    let result_snapshot_id = delta.header.snapshot_id;
    let dataset_id = delta.header.dataset_id;
    let base_ref = covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]);
    align_covedelta_lineage_parent_with_covm_ref(&mut delta, &base_ref);
    let delta_bytes = delta.serialize().unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, &delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);

    let delta_ref = CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: delta.header.parent_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    };
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_ref,
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
    let mut summary_entry =
        covm_delta_summary_entry(extension.ordered_delta_artifact_refs[0].clone());
    summary_entry.required_delta_features = delta.header.required_delta_features;
    summary_entry.optional_delta_features = delta.header.optional_delta_features;
    summary_entry.csn_min = delta.header.csn_min;
    summary_entry.csn_max = delta.header.csn_max;
    summary_entry.commit_time_start_us = delta.header.commit_time_range_start_us;
    summary_entry.commit_time_end_us = delta.header.commit_time_range_end_us;
    let mut summary = CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![summary_entry],
    );
    summary.chain_digest[0] ^= 0xFF;

    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "summary": summary.serialize().unwrap(),
        "deltas": [delta_bytes]
    }))
}

pub(super) fn covm_delta_chain_selection_required_summary_digest_mismatch_case() -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(
        &covm_delta_chain_selection_required_summary_case(false, true),
    )
    .unwrap();
    let extension_bytes = value
        .get("extension")
        .and_then(serde_json::Value::as_array)
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap() as u8)
        .collect::<Vec<_>>();
    let mut extension = CovmDeltaChainExtensionV1::parse(&extension_bytes).unwrap();
    extension.chain_summary_digest[0] ^= 0xFF;

    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "summary": value.get("summary").unwrap(),
        "deltas": value.get("deltas").unwrap()
    }))
}

pub(super) fn covm_delta_chain_selection_unadvertised_required_delta_feature_case() -> Vec<u8> {
    let mut delta_bytes = valid_covedelta_object_delta_file();
    rewrite_covedelta_required_features(&mut delta_bytes, DELTA_FEATURE_EXACT_TOUCHED_SET);
    let mut delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let base_snapshot_id = delta.header.parent_snapshot_id;
    let result_snapshot_id = delta.header.snapshot_id;
    let dataset_id = delta.header.dataset_id;
    let base_ref = covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]);
    align_covedelta_lineage_parent_with_covm_ref(&mut delta, &base_ref);
    let delta_bytes = delta.serialize().unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, &delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);

    let delta_ref = CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: delta.header.parent_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    };
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_ref,
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();

    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "deltas": [delta_bytes]
    }))
}

pub(super) fn covm_delta_chain_selection_case_with_summary_mutation(
    corrupt_delta_digest: bool,
    mutate_summary_entry: impl FnOnce(&mut DeltaChainSummaryEntryV1),
) -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&valid_covedelta_object_delta_file()).unwrap();
    let base_snapshot_id = delta.header.parent_snapshot_id;
    let result_snapshot_id = delta.header.snapshot_id;
    let dataset_id = delta.header.dataset_id;
    let base_ref = covm_delta_artifact_ref(0, 0x93, base_snapshot_id, [0; 16]);
    align_covedelta_lineage_parent_with_covm_ref(&mut delta, &base_ref);
    let delta_bytes = delta.serialize().unwrap();
    let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, &delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    if corrupt_delta_digest {
        digest_array[0] ^= 0xFF;
    }

    let delta_ref = CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: delta.header.parent_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    };
    let extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_ref,
        vec![delta_ref],
    );
    let extension = CovmDeltaChainExtensionV1::parse(&extension.serialize().unwrap()).unwrap();
    let mut summary_entry =
        covm_delta_summary_entry(extension.ordered_delta_artifact_refs[0].clone());
    summary_entry.required_delta_features = delta.header.required_delta_features;
    summary_entry.optional_delta_features = delta.header.optional_delta_features;
    summary_entry.csn_min = delta.header.csn_min;
    summary_entry.csn_max = delta.header.csn_max;
    summary_entry.commit_time_start_us = delta.header.commit_time_range_start_us;
    summary_entry.commit_time_end_us = delta.header.commit_time_range_end_us;
    mutate_summary_entry(&mut summary_entry);
    let summary = CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        vec![summary_entry],
    );
    json_fixture_bytes(json!({
        "extension": extension.serialize().unwrap(),
        "summary": summary.serialize().unwrap(),
        "deltas": [delta_bytes]
    }))
}

pub(super) fn selected_chain_covedelta_file(
    artifact_id: [u8; 16],
    dataset_id: [u8; 16],
    snapshot_id: [u8; 16],
    parent_snapshot_id: [u8; 16],
    chain_ordinal: u32,
) -> Vec<u8> {
    let mut file = CoveDeltaFile::parse(&valid_covedelta_object_delta_file()).unwrap();
    file.header.delta_artifact_id = artifact_id;
    file.header.dataset_id = dataset_id;
    file.header.snapshot_id = snapshot_id;
    file.header.parent_snapshot_id = parent_snapshot_id;
    file.header.chain_ordinal = chain_ordinal;
    file.header.chain_depth = chain_ordinal;
    if let Some(parent_ref) = file.parent_refs.first_mut() {
        parent_ref.snapshot_id = parent_snapshot_id;
    }
    file.serialize().unwrap()
}

pub(super) fn align_covedelta_lineage_parent_with_covm_ref(
    delta: &mut CoveDeltaFile,
    parent_ref: &CovmDeltaArtifactRefV1,
) {
    let lineage_parent = delta
        .parent_refs
        .iter_mut()
        .find(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0)
        .unwrap();
    lineage_parent.artifact_id = parent_ref.artifact_id;
    lineage_parent.snapshot_id = parent_ref.snapshot_id;
    lineage_parent.file_len = parent_ref.file_len;
    lineage_parent.footer_crc32c = parent_ref.footer_crc32c;
    lineage_parent.digest_algorithm = parent_ref.digest_algorithm;
    lineage_parent.digest_len = parent_ref.digest_len;
    lineage_parent.uri_ref = parent_ref.uri_ref;
}

pub(super) fn covm_base_artifact_ref_from_bytes(
    base_bytes: &[u8],
    artifact_id: [u8; 16],
    snapshot_id: [u8; 16],
) -> CovmDeltaArtifactRefV1 {
    let digest = compute_digest(DigestAlgorithm::Sha256, base_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    CovmDeltaArtifactRefV1 {
        chain_ordinal: 0,
        flags: 0,
        artifact_id,
        snapshot_id,
        parent_snapshot_id: [0; 16],
        file_len: base_bytes.len() as u64,
        footer_crc32c: 0,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 1,
        checksum: 0,
    }
}

pub(super) fn covm_delta_artifact_ref_from_delta_bytes(
    delta_bytes: &[u8],
) -> CovmDeltaArtifactRefV1 {
    let delta = CoveDeltaFile::parse(delta_bytes).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, delta_bytes).unwrap();
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    CovmDeltaArtifactRefV1 {
        chain_ordinal: delta.header.chain_ordinal,
        flags: 0,
        artifact_id: delta.header.delta_artifact_id,
        snapshot_id: delta.header.snapshot_id,
        parent_snapshot_id: delta.header.parent_snapshot_id,
        file_len: delta_bytes.len() as u64,
        footer_crc32c: delta.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: delta.header.chain_ordinal,
        checksum: 0,
    }
}

pub(super) fn covm_delta_pruning_case(
    as_of_csn: Option<u64>,
    as_of_commit_timestamp_us: Option<i64>,
    expect_selected: &[u32],
    expect_skipped: &[(u32, &str)],
    missing_commit_fields: bool,
) -> Vec<u8> {
    covm_delta_pruning_case_with_source_range(
        as_of_csn,
        as_of_commit_timestamp_us,
        None,
        expect_selected,
        expect_skipped,
        missing_commit_fields,
    )
}

pub(super) fn covm_delta_pruning_case_with_source_range(
    as_of_csn: Option<u64>,
    as_of_commit_timestamp_us: Option<i64>,
    source_publish_range_us: Option<(i64, i64)>,
    expect_selected: &[u32],
    expect_skipped: &[(u32, &str)],
    missing_commit_fields: bool,
) -> Vec<u8> {
    let summary = valid_covm_delta_pruning_summary(missing_commit_fields);
    let skipped = expect_skipped
        .iter()
        .map(|(chain_ordinal, reason)| {
            json!({
                "chain_ordinal": chain_ordinal,
                "reason": reason
            })
        })
        .collect::<Vec<_>>();
    let mut fixture = json!({
        "summary": summary,
        "as_of_csn": as_of_csn,
        "as_of_commit_timestamp_us": as_of_commit_timestamp_us,
        "expect_selected": expect_selected,
        "expect_skipped": skipped
    });
    if let Some((start_us, end_us)) = source_publish_range_us {
        let fixture_object = fixture.as_object_mut().unwrap();
        fixture_object.insert("source_publish_range_start_us".into(), json!(start_us));
        fixture_object.insert("source_publish_range_end_us".into(), json!(end_us));
    }
    if !missing_commit_fields {
        let as_of_csn_prunes = expect_skipped
            .iter()
            .filter(|(_, reason)| *reason == "as_of_csn_before_delta")
            .count();
        let commit_time_range_prunes = expect_skipped
            .iter()
            .filter(|(_, reason)| *reason == "as_of_commit_before_delta")
            .count();
        let source_publish_range_prunes = expect_skipped
            .iter()
            .filter(|(_, reason)| *reason == "source_publish_range_outside_delta")
            .count();
        fixture.as_object_mut().unwrap().insert(
            "expect_metrics".into(),
            json!({
                "delta_chain_depth": expect_selected.len() + expect_skipped.len(),
                "selected_delta_count": expect_selected.len(),
                "skipped_delta_count": expect_skipped.len(),
                "delta_artifacts_planned_to_open": expect_selected.len(),
                "delta_artifacts_skipped_before_open": expect_skipped.len(),
                "as_of_csn_prunes": as_of_csn_prunes,
                "commit_time_range_prunes": commit_time_range_prunes,
                "source_publish_range_prunes": source_publish_range_prunes
            }),
        );
    }
    json_fixture_bytes(fixture)
}

pub(super) fn covm_delta_pruning_valid_time_no_summary_case() -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(&covm_delta_pruning_case(None, None, &[1, 2], &[], false)).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("as_of_valid_time_us".into(), json!(5));
    json_fixture_bytes(value)
}

pub(super) fn valid_covm_delta_pruning_summary(missing_commit_fields: bool) -> Vec<u8> {
    let mut entries = vec![
        covm_delta_pruning_summary_entry(
            covm_delta_artifact_ref(1, 0xA1, [0xB1; 16], [0xB0; 16]),
            10,
            20,
            1_000,
            1_100,
        ),
        covm_delta_pruning_summary_entry(
            covm_delta_artifact_ref(2, 0xA2, [0xB2; 16], [0xB1; 16]),
            30,
            40,
            2_000,
            2_100,
        ),
    ];
    if missing_commit_fields {
        entries[0].time_field_presence_flags &= !DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT;
        entries[0].commit_time_start_us = 0;
        entries[0].commit_time_end_us = 0;
    }

    let summary = CovmDeltaChainSummaryV1::new(
        [0xA0; 16],
        [0xB2; 16],
        DigestAlgorithm::Sha256 as u16,
        vec![0xCC; 32],
        entries,
    );
    summary.serialize().unwrap()
}

pub(super) fn covm_delta_pruning_summary_entry(
    reference: CovmDeltaArtifactRefV1,
    csn_min: u64,
    csn_max: u64,
    commit_time_start_us: i64,
    commit_time_end_us: i64,
) -> DeltaChainSummaryEntryV1 {
    let mut entry = covm_delta_summary_entry(reference);
    entry.csn_min = csn_min;
    entry.csn_max = csn_max;
    entry.commit_time_start_us = commit_time_start_us;
    entry.commit_time_end_us = commit_time_end_us;
    entry.source_publish_range_start_us = commit_time_start_us;
    entry.source_publish_range_end_us = commit_time_end_us;
    entry
}

pub(super) fn covm_delta_summary_entry(
    reference: CovmDeltaArtifactRefV1,
) -> DeltaChainSummaryEntryV1 {
    DeltaChainSummaryEntryV1 {
        chain_ordinal: reference.chain_ordinal,
        delta_artifact_id: reference.artifact_id,
        delta_artifact_ref: reference,
        required_delta_features: 0,
        optional_delta_features: 0,
        csn_min: 7,
        csn_max: 8,
        commit_time_start_us: 1_700_000_000_000_000,
        commit_time_end_us: 1_700_000_000_000_010,
        artifact_created_at_us: 1_700_000_000_000_020,
        first_published_at_us: 1_700_000_000_000_030,
        selected_snapshot_published_at_us: 1_700_000_000_000_040,
        time_field_presence_flags: DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT
            | DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        time_summary_exactness_flags: DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
        source_publish_range_start_us: 1_700_000_000_000_001,
        source_publish_range_end_us: 1_700_000_000_000_002,
        scope_summary_ref: 0,
        branch_summary_ref: 0,
        object_type_summary_ref: 0,
        goid_range_summary_ref: 0,
        touched_summary_ref: 0,
        tombstone_summary_ref: 0,
        property_summary_ref: 0,
        temporal_role_summary_ref: 0,
        delta_header_range_offset: 0,
        delta_header_range_length: 64,
        hot_summary_range_offset: 64,
        hot_summary_range_length: 32,
        checksum: 0,
    }
}

pub(super) fn covm_delta_artifact_ref(
    chain_ordinal: u32,
    artifact_byte: u8,
    snapshot_id: [u8; 16],
    parent_snapshot_id: [u8; 16],
) -> CovmDeltaArtifactRefV1 {
    CovmDeltaArtifactRefV1 {
        chain_ordinal,
        flags: 0,
        artifact_id: [artifact_byte; 16],
        snapshot_id,
        parent_snapshot_id,
        file_len: 4096 + u64::from(chain_ordinal),
        footer_crc32c: checksum::crc32c(&[artifact_byte; 8]),
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: [artifact_byte ^ 0x5A; 32],
        uri_ref: chain_ordinal + 1,
        checksum: 0,
    }
}

pub(super) fn rewrite_covm_delta_chain_required_features(
    bytes: &mut [u8],
    required_delta_features: u64,
) {
    bytes[8..16].copy_from_slice(&required_delta_features.to_le_bytes());
    rewrite_covm_delta_chain_header_crc(bytes);
}

pub(super) fn rewrite_covm_delta_ref_ordinal(
    bytes: &mut [u8],
    ref_index: usize,
    chain_ordinal: u32,
) {
    let offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN + ref_index * COVM_DELTA_ARTIFACT_REF_LEN;
    bytes[offset..offset + 4].copy_from_slice(&chain_ordinal.to_le_bytes());
    let checksum_offset = offset + COVM_DELTA_ARTIFACT_REF_LEN - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(&bytes[offset..offset + COVM_DELTA_ARTIFACT_REF_LEN]);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn rewrite_covm_delta_ref_artifact_id(
    bytes: &mut [u8],
    ref_index: usize,
    artifact_id: [u8; 16],
) {
    let offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN + ref_index * COVM_DELTA_ARTIFACT_REF_LEN;
    let artifact_id_offset = offset + 8;
    bytes[artifact_id_offset..artifact_id_offset + 16].copy_from_slice(&artifact_id);
    let checksum_offset = offset + COVM_DELTA_ARTIFACT_REF_LEN - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(&bytes[offset..offset + COVM_DELTA_ARTIFACT_REF_LEN]);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn rewrite_covm_delta_chain_header_crc(bytes: &mut [u8]) {
    let checksum_offset = COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(&bytes[..COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN]);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn rewrite_covm_delta_summary_entry_ordinal(
    bytes: &mut [u8],
    entry_index: usize,
    chain_ordinal: u32,
) {
    let offset = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize
        + 32
        + entry_index * COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN;
    bytes[offset..offset + 4].copy_from_slice(&chain_ordinal.to_le_bytes());
    rewrite_covm_delta_summary_entry_crc(bytes, offset);
}

pub(super) fn rewrite_covm_delta_summary_entry_required_features(
    bytes: &mut [u8],
    entry_index: usize,
    required_delta_features: u64,
) {
    let offset = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize
        + 32
        + entry_index * COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN;
    let required_offset = offset + 4 + COVM_DELTA_ARTIFACT_REF_LEN + 16;
    bytes[required_offset..required_offset + 8]
        .copy_from_slice(&required_delta_features.to_le_bytes());
    rewrite_covm_delta_summary_entry_crc(bytes, offset);
}

pub(super) fn rewrite_covm_delta_summary_entry_csn_min(
    bytes: &mut [u8],
    entry_index: usize,
    csn_min: u64,
) {
    let offset = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize
        + 32
        + entry_index * COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN;
    let csn_min_offset = offset + 4 + COVM_DELTA_ARTIFACT_REF_LEN + 16 + 8 + 8;
    bytes[csn_min_offset..csn_min_offset + 8].copy_from_slice(&csn_min.to_le_bytes());
    rewrite_covm_delta_summary_entry_crc(bytes, offset);
}

pub(super) fn rewrite_covm_delta_summary_entry_commit_start(
    bytes: &mut [u8],
    entry_index: usize,
    commit_time_start_us: i64,
) {
    let offset = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize
        + 32
        + entry_index * COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN;
    let commit_start_offset = offset + 4 + COVM_DELTA_ARTIFACT_REF_LEN + 16 + 8 + 8 + 8 + 8;
    bytes[commit_start_offset..commit_start_offset + 8]
        .copy_from_slice(&commit_time_start_us.to_le_bytes());
    rewrite_covm_delta_summary_entry_crc(bytes, offset);
}

pub(super) fn rewrite_covm_delta_summary_entry_crc(bytes: &mut [u8], offset: usize) {
    let checksum_offset = offset + COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(&bytes[offset..offset + COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN]);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn rewrite_covm_delta_chain_summary_header_crc(bytes: &mut [u8]) {
    let checksum_offset = COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize - 4;
    bytes[checksum_offset..checksum_offset + 4].fill(0);
    let crc = checksum::crc32c(&bytes[..COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN as usize]);
    bytes[checksum_offset..checksum_offset + 4].copy_from_slice(&crc.to_le_bytes());
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SidecarFreshnessCase {
    Valid,
    FileId,
    FileLen,
    FooterCrc,
    Digest,
    Corrupt,
}

pub(super) fn sidecar_freshness_payload(case: SidecarFreshnessCase) -> Vec<u8> {
    let cove = MinimalCoveWriter::write_empty_file().unwrap();
    let (mut file_id, mut file_len, mut footer_crc32c, mut digest) = cove_identity(&cove);
    if matches!(case, SidecarFreshnessCase::FileId) {
        file_id[0] ^= 0xFF;
    }
    if matches!(case, SidecarFreshnessCase::FileLen) {
        file_len += 1;
    }
    if matches!(case, SidecarFreshnessCase::FooterCrc) {
        footer_crc32c ^= 0xFFFF;
    }
    if matches!(case, SidecarFreshnessCase::Digest) {
        digest[0] ^= 0xFF;
    }

    let (covx, covm, expect) = if matches!(case, SidecarFreshnessCase::Corrupt) {
        (
            b"not a covx".to_vec(),
            b"not a covm".to_vec(),
            "StaleIgnored",
        )
    } else {
        (
            covx_for_reference(file_id, file_len, footer_crc32c, &digest),
            covm_for_reference(file_id, file_len, footer_crc32c, &digest),
            if matches!(case, SidecarFreshnessCase::Valid) {
                "Valid"
            } else {
                "StaleIgnored"
            },
        )
    };

    serde_json::to_vec_pretty(&json!({
        "cove": cove,
        "covx": covx,
        "covm": covm,
        "expect_covx": expect,
        "expect_covm": expect
    }))
    .unwrap()
}

pub(super) fn cove_identity(cove: &[u8]) -> ([u8; 16], u64, u32, Vec<u8>) {
    let validated = reader::validate_bytes(cove).unwrap();
    let digest = compute_digest(DigestAlgorithm::Sha256, cove).unwrap();
    (
        validated.header.file_id,
        validated.postscript.file_len,
        validated.postscript.footer.crc32c,
        digest,
    )
}

pub(super) fn covx_for_reference(
    file_id: [u8; 16],
    file_len: u64,
    footer_crc32c: u32,
    digest: &[u8],
) -> Vec<u8> {
    CovxFile {
        header: CovxHeaderV1::new([0x91; 16], 1, 1_700_000_000_000_000),
        referenced_files: vec![CovxReferencedFileV1 {
            file_id,
            file_len,
            footer_crc32c,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest: digest.to_vec(),
        }],
        postscript: CovxPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn covm_for_reference(
    file_id: [u8; 16],
    file_len: u64,
    footer_crc32c: u32,
    digest: &[u8],
) -> Vec<u8> {
    CovmFile {
        header: CovmHeaderV1::new([0x92; 16], 1, 1, 1_700_000_000_000_000),
        files: vec![CovmFileEntryV1 {
            file_id,
            uri: "file:///dataset/part-0.cove".into(),
            file_len,
            footer_crc32c,
            digest_algorithm: DigestAlgorithm::Sha256 as u16,
            digest: digest.to_vec(),
            row_count: 0,
            segment_count: 0,
            file_stats_ref: 0,
            file_exact_set_ref: 0,
            flags: 0,
        }],
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn valid_covemap_file() -> Vec<u8> {
    CovemapFile {
        header: CovemapHeaderV1::new([0x33; 16], 1_700_000_000_000_000),
        mapping_version: "example/v1".into(),
        sections: vec![
            CovemapSection {
                entry: CovemapSectionEntryV1 {
                    section_id: SectionKind::MapSourceCatalog as u32,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    compression: CompressionCodec::None as u8,
                    payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
                    required: true,
                    reserved: 0,
                    checksum: 0,
                },
                payload: map_payload_bytes(covemap_payload_value(
                    SectionKind::MapSourceCatalog,
                    json!({
                        "mapping_id": "m1",
                        "mapping_version": "example/v1",
                        "sources": [{
                            "source_id": "crm",
                            "schema_fingerprint": "schema:v1",
                            "snapshot_digest": "digest:v1",
                            "row_identity_rules": ["crm.pk"],
                            "replay_claimed": true
                        }]
                    }),
                )),
            },
            CovemapSection {
                entry: CovemapSectionEntryV1 {
                    section_id: SectionKind::MapFunctionRegistry as u32,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    compression: CompressionCodec::None as u8,
                    payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
                    required: false,
                    reserved: 0,
                    checksum: 0,
                },
                payload: map_payload_bytes(covemap_payload_value(
                    SectionKind::MapFunctionRegistry,
                    json!({
                        "mapping_id": "m1",
                        "mapping_version": "example/v1",
                        "functions": [{
                            "function_id": "trim_lower",
                            "version": "v1",
                            "deterministic": true,
                            "dependency": "pure"
                        }]
                    }),
                )),
            },
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

pub(super) fn covemap_lz4_missing_feature_file() -> Vec<u8> {
    let mut file = CovemapFile::parse(&valid_covemap_file()).unwrap();
    file.sections[0].entry.compression = CompressionCodec::Lz4 as u8;
    let mut bytes = file.serialize().unwrap();
    rewrite_covemap_feature_bits(&mut bytes, FEATURE_SEMANTIC_MAP, 0);
    bytes
}

pub(super) fn rewrite_covemap_feature_bits(
    bytes: &mut [u8],
    required_features: u64,
    optional_features: u64,
) {
    let mut header = CovemapHeaderV1::parse(bytes).unwrap();
    header.required_features = required_features;
    header.optional_features = optional_features;
    bytes[..COVEMAP_HEADER_LEN as usize].copy_from_slice(&header.serialize());

    let mut postscript = CovemapPostscriptV1::parse_from_tail(bytes).unwrap();
    postscript.required_features = required_features;
    postscript.optional_features = optional_features;
    let tail_start = bytes.len() - (COVEMAP_POSTSCRIPT_LEN as usize + COVEMAP_POSTSCRIPT_TAIL_SIZE);
    bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
}
