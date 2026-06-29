use super::*;
use crate::constants::{CoveLogicalType, CovePhysicalKind};
use crate::profile::cove_o::{
    ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1, TemporalSegmentHeaderV1,
    TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
};

pub(super) fn minimal_delta() -> CoveDeltaFile {
    CoveDeltaFile {
        header: CoveDeltaHeaderV1::new([1; 16], [2; 16], [3; 16], [4; 16]),
        parent_refs: vec![DeltaParentRefV1 {
            parent_ref: 0,
            parent_kind: 0,
            flags: DELTA_PARENT_REF_LINEAGE_PARENT,
            artifact_id: [9; 16],
            snapshot_id: [4; 16],
            file_len: 1024,
            footer_crc32c: 7,
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
            payload: b"delta-payload".to_vec(),
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
}

pub(super) fn object_delta() -> CoveDeltaFile {
    let mut delta = minimal_delta();
    delta.header.csn_min = 1;
    delta.header.csn_max = 1;
    delta.header.commit_time_range_start_us = 10;
    delta.header.commit_time_range_end_us = 10;
    delta.sections[0].payload = temporal_segment_payload();
    delta
}

pub(super) fn object_delta_with_sparse_patch_section() -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_SPARSE_PATCH_ROWS;
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 2,
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
        payload: sample_sparse_patch_record().serialize().unwrap(),
    });
    push_string_table_section(&mut delta, &[0]);
    delta
}

pub(super) fn object_delta_with_dictionary_overlay_entries(
    entries: &[DeltaDictionaryEntryV1],
) -> CoveDeltaFile {
    object_delta_with_dictionary_overlay_entries_and_features(
        entries,
        dictionary_overlay_required_features(entries),
    )
}

pub(super) fn object_delta_with_dictionary_overlay_entries_and_features(
    entries: &[DeltaDictionaryEntryV1],
    required_delta_features: u64,
) -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features = required_delta_features;
    let mut payload = Vec::new();
    for entry in entries {
        payload.extend_from_slice(&entry.serialize().unwrap());
    }
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 2,
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
            required_delta_features,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    });
    let inline_value_refs = entries
        .iter()
        .filter_map(|entry| {
            (entry.entry_kind == DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE)
                .then_some(entry.inline_value_ref)
        })
        .collect::<Vec<_>>();
    if !inline_value_refs.is_empty() {
        push_string_table_section(&mut delta, &inline_value_refs);
    }
    delta
}

pub(super) fn push_string_table_section(delta: &mut CoveDeltaFile, value_refs: &[u32]) {
    let max_ref = value_refs.iter().copied().max().unwrap_or(0);
    let values = (0..=max_ref).map(sample_inline_value).collect::<Vec<_>>();
    let payload = values
        .iter()
        .map(DeltaInlineValueV1::serialize)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: delta.sections.len() as u32 + 1,
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
    });
}

pub(super) fn sample_inline_value(value_ref: u32) -> DeltaInlineValueV1 {
    DeltaInlineValueV1 {
        value_ref,
        value_tag: ValueTag::Utf8 as u16,
        flags: 0,
        value: vec![1, b'x'],
        checksum: 0,
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

pub(super) fn object_delta_with_descriptor_table_sections() -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 2,
            section_kind: CoveDeltaSectionKind::ScopeTable as u16,
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
        payload: sample_scope_descriptor().serialize().unwrap().to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 3,
            section_kind: CoveDeltaSectionKind::TemporalRoleSummaryTable as u16,
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
        payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE)
            .serialize()
            .unwrap()
            .to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 4,
            section_kind: CoveDeltaSectionKind::TouchedSummaryTable as u16,
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
        payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET)
            .serialize()
            .unwrap()
            .to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 5,
            section_kind: CoveDeltaSectionKind::TombstoneSummaryTable as u16,
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
        payload: sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET)
            .serialize()
            .unwrap()
            .to_vec(),
    });
    delta
}

pub(super) fn object_delta_with_extra_section(
    section_kind: CoveDeltaSectionKind,
    required_delta_features: u64,
    payload: Vec<u8>,
) -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.sections.push(CoveDeltaSection {
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
        payload,
    });
    delta
}

pub(super) fn object_delta_with_catalog_patch_section(patch: ObjectTypeCatalog) -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 2,
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
    });
    delta
}

pub(super) fn object_delta_with_projection_patch_section(payload: Vec<u8>) -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_PROJECTION_PATCH;
    delta.header.projection_fingerprint_ref = 41;
    delta.sections.push(delta_projection_patch_section(
        2,
        payload,
        DELTA_FEATURE_PROJECTION_PATCH,
    ));
    delta
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

pub(super) fn projection_patch_payload() -> Vec<u8> {
    br#"{"schema_id":"org.coveformat.covemap.v2","section_id":68,"mapping_id":"crm-map","mapping_version":"v1","projections":[{"projection_id":"delta_projection","output_table":"delta_projection","row_grain":"one_row_per_object","anchor":{"object_type":"Company"},"temporal_mode":{"as_of":"latest_committed"},"multi_value_policy":"first","columns":[{"name":"company_name","value":"name","logical_type":"utf8"}],"output_modes":["json"]}]}"#.to_vec()
}

pub(super) fn sample_sidecar_parent_ref(parent_ref: u32) -> DeltaParentRefV1 {
    DeltaParentRefV1 {
        parent_ref,
        parent_kind: 0,
        flags: 0,
        artifact_id: [0xA0; 16],
        snapshot_id: [0xA1; 16],
        file_len: 2048,
        footer_crc32c: 8,
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

pub(super) fn sample_sidecar_hint(parent_ref: u32, hint_kind: u16) -> DeltaSidecarHintV1 {
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

pub(super) fn object_delta_with_sidecar_hint_section(
    section_kind: CoveDeltaSectionKind,
    required_delta_features: u64,
    hint_kind: u16,
) -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features = required_delta_features;
    delta.parent_refs.push(sample_sidecar_parent_ref(1));
    delta.sections.push(delta_sidecar_hint_section(
        2,
        section_kind,
        required_delta_features,
        sample_sidecar_hint(1, hint_kind),
    ));
    delta
}

pub(super) fn sample_catalog_patch() -> ObjectTypeCatalog {
    ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 2,
            type_name: "Widget".into(),
            flags: 0,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    }
}

pub(super) fn object_delta_with_anchor_and_touched_sections() -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features =
        DELTA_FEATURE_CONTINUATION_ANCHORS | DELTA_FEATURE_EXACT_TOUCHED_SET;
    delta.sections.push(CoveDeltaSection {
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
        payload: sample_anchor().serialize().to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
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
        payload: sample_touched_range().serialize().to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 4,
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
        payload: sample_state_hash_descriptor().serialize().to_vec(),
    });
    delta
}

pub(super) fn object_delta_with_touched_property_bitmap_section(
    descriptor_kind: Option<u8>,
) -> CoveDeltaFile {
    let mut delta = object_delta_with_anchor_and_touched_sections();
    let mut touched = sample_touched_range();
    touched.property_bitmap_ref = 0;
    delta.sections[2].payload = touched.serialize().to_vec();
    if let Some(summary_kind) = descriptor_kind {
        delta.sections.push(CoveDeltaSection {
            entry: CoveDeltaSectionDirectoryEntryV1 {
                section_id: 5,
                section_kind: CoveDeltaSectionKind::TouchedSummaryTable as u16,
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
            payload: sample_summary_descriptor(summary_kind)
                .serialize()
                .unwrap()
                .to_vec(),
        });
    }
    delta
}

pub(super) fn object_delta_with_branch_identity_section() -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.sections.push(CoveDeltaSection {
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
        payload: sample_branch_identity().serialize().to_vec(),
    });
    delta
}

pub(super) fn object_delta_with_tombstone_set_section() -> CoveDeltaFile {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_EXACT_TOMBSTONE_SET;
    delta.sections[0].payload = tombstone_temporal_segment_payload();
    delta.sections.push(CoveDeltaSection {
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
        payload: sample_tombstone_range().serialize().to_vec(),
    });
    delta
}

pub(super) fn temporal_segment_payload() -> Vec<u8> {
    temporal_segment_payload_for_row(TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [7; 16],
        record_id: [8; 16],
        record_kind: RecordKind::Delta,
        prev_ref: None,
    })
}

pub(super) fn tombstone_temporal_segment_payload() -> Vec<u8> {
    temporal_segment_payload_for_row(TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [9; 16],
        record_id: [10; 16],
        record_kind: RecordKind::Tombstone,
        prev_ref: None,
    })
}

pub(super) fn checkpoint_temporal_segment_payload() -> Vec<u8> {
    temporal_segment_payload_for_row(TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [7; 16],
        record_id: [11; 16],
        record_kind: RecordKind::Snapshot,
        prev_ref: None,
    })
}

pub(super) fn temporal_segment_payload_for_row(row: TemporalRowEntryV1) -> Vec<u8> {
    temporal_segment_payload_for_rows(&[row])
}

pub(super) fn temporal_segment_payload_for_rows(rows: &[TemporalRowEntryV1]) -> Vec<u8> {
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_end =
        row_directory_offset + (rows.len() as u64).saturating_mul(TEMPORAL_ROW_ENTRY_LEN as u64);
    let header = TemporalSegmentHeaderV1 {
        segment_id: 1,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: 1,
        morsel_row_count: rows.len() as u32,
        column_count: 0,
        row_directory_offset,
        column_directory_offset: row_end,
        page_index_offset: row_end,
        data_offset: row_end,
        flags: 0,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    for row in rows {
        out.extend_from_slice(&row.serialize());
    }
    out
}

pub(super) fn sample_anchor() -> DeltaContinuationAnchorV1 {
    DeltaContinuationAnchorV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        goid: [7; 16],
        parent_ref: 0,
        predecessor_csn: 0,
        predecessor_timestamp_us: 0,
        predecessor_record_id: [3; 16],
        predecessor_state_hash_ref: 0,
        predecessor_trust_hash_ref: DELTA_REF_NONE,
        anchor_strength: DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
        flags: 0,
        checksum: 0,
    }
}

pub(super) fn sample_state_hash_descriptor() -> DeltaStateHashDescriptorV1 {
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

pub(super) fn sample_state_hash_material() -> CoveObjectDeltaStateHashV1 {
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
                logical_type: 1,
                collation_id: 0,
                value_state: DELTA_OBJECT_STATE_VALUE_VISIBLE,
                canonical_value: b"alice".to_vec(),
                redaction_commitment: Vec::new(),
                hidden_value_commitment: None,
            },
            CoveObjectDeltaStateHashPropertyV1 {
                property_id: 2,
                logical_type: 1,
                collation_id: 0,
                value_state: DELTA_OBJECT_STATE_VALUE_NULL,
                canonical_value: Vec::new(),
                redaction_commitment: Vec::new(),
                hidden_value_commitment: None,
            },
        ],
    }
}

pub(super) fn sample_sparse_patch_record() -> DeltaSparsePatchRecordV1 {
    DeltaSparsePatchRecordV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        branch_identity_ref: 0,
        object_type_id: 1,
        goid: [7; 16],
        record_id: [8; 16],
        timestamp_us: 10,
        csn: 1,
        record_kind: RecordKind::Delta,
        flags: 0,
        changed_properties: vec![
            DeltaSparsePatchPropertyOpV1 {
                property_id: 1,
                property_op: DELTA_PROPERTY_OP_SET_VALUE,
                tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                value_ref: 0,
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

pub(super) fn sample_dictionary_entry() -> DeltaDictionaryEntryV1 {
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

pub(super) fn sample_parent_alias_dictionary_entry() -> DeltaDictionaryEntryV1 {
    DeltaDictionaryEntryV1 {
        local_dictionary_id: 1,
        local_code: 8,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        entry_kind: DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS,
        flags: 0,
        inline_value_ref: DELTA_REF_NONE,
        parent_ref: 0,
        parent_dictionary_id: 1,
        parent_code: 2,
        parent_dictionary_digest_ref: 3,
        canonical_hash128: [9; 16],
        checksum: 0,
    }
}

pub(super) fn sample_canonical_hash_hint_dictionary_entry() -> DeltaDictionaryEntryV1 {
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

pub(super) fn sample_scope_descriptor() -> DeltaScopeDescriptorV1 {
    DeltaScopeDescriptorV1 {
        scope_ref: 0,
        scope_kind: 0,
        flags: 0,
        scope_id: [0; 16],
        checksum: 0,
    }
}

pub(super) fn sample_summary_descriptor(summary_kind: u8) -> DeltaSummaryDescriptorV1 {
    DeltaSummaryDescriptorV1 {
        summary_ref: 0,
        summary_kind,
        flags: 0,
        payload_ref: 0,
        item_count: 1,
        checksum: 0,
    }
}

pub(super) fn sample_branch_identity() -> DeltaBranchIdentityV1 {
    DeltaBranchIdentityV1 {
        branch_identity_ref: 0,
        branch_identity_kind: DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF,
        flags: 0,
        branch_value_ref: 11,
        branch_hash128: [5; 16],
        branch_catalog_fingerprint_ref: 0,
        checksum: 0,
    }
}

pub(super) fn sample_touched_range() -> DeltaTouchedObjectRangeV1 {
    DeltaTouchedObjectRangeV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        min_goid: [7; 16],
        max_goid: [7; 16],
        touched_count: 1,
        property_bitmap_ref: DELTA_REF_NONE,
        object_set_ref: 0,
        checksum: 0,
    }
}

pub(super) fn sample_tombstone_range() -> DeltaTouchedObjectRangeV1 {
    DeltaTouchedObjectRangeV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        min_goid: [9; 16],
        max_goid: [9; 16],
        touched_count: 1,
        property_bitmap_ref: DELTA_REF_NONE,
        object_set_ref: 0,
        checksum: 0,
    }
}

pub(super) fn sample_object_point(goid: [u8; 16]) -> DeltaObjectPointLookupV1 {
    DeltaObjectPointLookupV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        goid,
    }
}

pub(super) fn rewrite_first_section_payload_offset(bytes: &mut [u8], offset: u64) {
    let parsed = CoveDeltaFile::parse(bytes).unwrap();
    let directory_offset = parsed.header.section_directory_offset as usize;
    let entry_range = directory_offset..directory_offset + COVEDELTA_SECTION_DIRECTORY_ENTRY_LEN;
    let mut entry = CoveDeltaSectionDirectoryEntryV1::parse(&bytes[entry_range.clone()]).unwrap();
    let start = offset as usize;
    let end = start + entry.length as usize;
    entry.offset = offset;
    entry.crc32c = checksum::crc32c(&bytes[start..end]);
    bytes[entry_range].copy_from_slice(&entry.serialize());
}
