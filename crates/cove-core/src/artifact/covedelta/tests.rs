use super::*;
use crate::constants::{CoveLogicalType, CovePhysicalKind};
use crate::profile::cove_o::{
    ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1, TemporalSegmentHeaderV1,
    TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
};

fn minimal_delta() -> CoveDeltaFile {
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

fn object_delta() -> CoveDeltaFile {
    let mut delta = minimal_delta();
    delta.header.csn_min = 1;
    delta.header.csn_max = 1;
    delta.header.commit_time_range_start_us = 10;
    delta.header.commit_time_range_end_us = 10;
    delta.sections[0].payload = temporal_segment_payload();
    delta
}

fn object_delta_with_sparse_patch_section() -> CoveDeltaFile {
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

fn object_delta_with_dictionary_overlay_entries(
    entries: &[DeltaDictionaryEntryV1],
) -> CoveDeltaFile {
    object_delta_with_dictionary_overlay_entries_and_features(
        entries,
        dictionary_overlay_required_features(entries),
    )
}

fn object_delta_with_dictionary_overlay_entries_and_features(
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

fn push_string_table_section(delta: &mut CoveDeltaFile, value_refs: &[u32]) {
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

fn sample_inline_value(value_ref: u32) -> DeltaInlineValueV1 {
    DeltaInlineValueV1 {
        value_ref,
        value_tag: ValueTag::Utf8 as u16,
        flags: 0,
        value: vec![1, b'x'],
        checksum: 0,
    }
}

fn dictionary_overlay_required_features(entries: &[DeltaDictionaryEntryV1]) -> u64 {
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

fn object_delta_with_descriptor_table_sections() -> CoveDeltaFile {
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

fn object_delta_with_extra_section(
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

fn object_delta_with_catalog_patch_section(patch: ObjectTypeCatalog) -> CoveDeltaFile {
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

fn object_delta_with_projection_patch_section(payload: Vec<u8>) -> CoveDeltaFile {
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

fn delta_projection_patch_section(
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

fn projection_patch_payload() -> Vec<u8> {
    br#"{"schema_id":"org.coveformat.covemap.v2","section_id":68,"mapping_id":"crm-map","mapping_version":"v1","projections":[{"projection_id":"delta_projection","output_table":"delta_projection","row_grain":"one_row_per_object","anchor":{"object_type":"Company"},"temporal_mode":{"as_of":"latest_committed"},"multi_value_policy":"first","columns":[{"name":"company_name","value":"name","logical_type":"utf8"}],"output_modes":["json"]}]}"#.to_vec()
}

fn sample_sidecar_parent_ref(parent_ref: u32) -> DeltaParentRefV1 {
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

fn sample_sidecar_hint(parent_ref: u32, hint_kind: u16) -> DeltaSidecarHintV1 {
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

fn delta_sidecar_hint_section(
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

fn object_delta_with_sidecar_hint_section(
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

fn sample_catalog_patch() -> ObjectTypeCatalog {
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

fn object_delta_with_anchor_and_touched_sections() -> CoveDeltaFile {
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

fn object_delta_with_touched_property_bitmap_section(descriptor_kind: Option<u8>) -> CoveDeltaFile {
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

fn object_delta_with_branch_identity_section() -> CoveDeltaFile {
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

fn object_delta_with_tombstone_set_section() -> CoveDeltaFile {
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

fn temporal_segment_payload() -> Vec<u8> {
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

fn tombstone_temporal_segment_payload() -> Vec<u8> {
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

fn checkpoint_temporal_segment_payload() -> Vec<u8> {
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

fn temporal_segment_payload_for_row(row: TemporalRowEntryV1) -> Vec<u8> {
    temporal_segment_payload_for_rows(&[row])
}

fn temporal_segment_payload_for_rows(rows: &[TemporalRowEntryV1]) -> Vec<u8> {
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

fn sample_anchor() -> DeltaContinuationAnchorV1 {
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

fn sample_state_hash_descriptor() -> DeltaStateHashDescriptorV1 {
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

fn sample_state_hash_material() -> CoveObjectDeltaStateHashV1 {
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

fn sample_sparse_patch_record() -> DeltaSparsePatchRecordV1 {
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

fn sample_dictionary_entry() -> DeltaDictionaryEntryV1 {
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

fn sample_parent_alias_dictionary_entry() -> DeltaDictionaryEntryV1 {
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

fn sample_canonical_hash_hint_dictionary_entry() -> DeltaDictionaryEntryV1 {
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

fn sample_scope_descriptor() -> DeltaScopeDescriptorV1 {
    DeltaScopeDescriptorV1 {
        scope_ref: 0,
        scope_kind: 0,
        flags: 0,
        scope_id: [0; 16],
        checksum: 0,
    }
}

fn sample_summary_descriptor(summary_kind: u8) -> DeltaSummaryDescriptorV1 {
    DeltaSummaryDescriptorV1 {
        summary_ref: 0,
        summary_kind,
        flags: 0,
        payload_ref: 0,
        item_count: 1,
        checksum: 0,
    }
}

fn sample_branch_identity() -> DeltaBranchIdentityV1 {
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

fn sample_touched_range() -> DeltaTouchedObjectRangeV1 {
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

fn sample_tombstone_range() -> DeltaTouchedObjectRangeV1 {
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

fn sample_object_point(goid: [u8; 16]) -> DeltaObjectPointLookupV1 {
    DeltaObjectPointLookupV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        object_type_id: 1,
        branch_identity_ref: 0,
        goid,
    }
}

#[test]
fn covedelta_round_trips_and_discovers_tail() {
    let delta = minimal_delta();
    let bytes = delta.serialize().unwrap();
    assert!(bytes.ends_with(&MAGIC_COVEDELTA));
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert_eq!(parsed.header.delta_artifact_id, [1; 16]);
    assert_eq!(parsed.parent_refs.len(), 1);
    assert_eq!(parsed.sections.len(), 1);
    assert_eq!(parsed.sections[0].payload, b"delta-payload");
    assert_eq!(
        parsed.sections[0].entry.section_kind,
        CoveDeltaSectionKind::TemporalSegmentData as u16
    );
}

#[test]
fn covedelta_rejects_missing_lineage_parent_ref() {
    let mut delta = minimal_delta();
    delta.parent_refs.clear();
    let bytes = delta.serialize().unwrap();
    assert!(matches!(
        CoveDeltaFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("exactly one lineage parent")
    ));
}

#[test]
fn covedelta_rejects_duplicate_lineage_parent_refs() {
    let mut delta = minimal_delta();
    let mut duplicate_parent = delta.parent_refs[0].clone();
    duplicate_parent.parent_ref = 1;
    duplicate_parent.artifact_id = [10; 16];
    delta.parent_refs.push(duplicate_parent);
    let bytes = delta.serialize().unwrap();
    assert!(matches!(
        CoveDeltaFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("exactly one lineage parent")
    ));
}

#[test]
fn covedelta_rejects_corrupt_tail_and_section_payload() {
    let mut bytes = minimal_delta().serialize().unwrap();
    *bytes.last_mut().unwrap() = b'X';
    assert!(matches!(
        CoveDeltaFile::parse(&bytes),
        Err(CoveError::BadMagic)
    ));

    let mut bytes = minimal_delta().serialize().unwrap();
    let header = CoveDeltaHeaderV1::parse(&bytes[..COVEDELTA_HEADER_LEN as usize]).unwrap();
    bytes[header.parent_refs_offset as usize + COVEDELTA_PARENT_REF_LEN] ^= 0xff;
    assert!(matches!(
        CoveDeltaFile::parse(&bytes),
        Err(CoveError::ChecksumMismatch)
    ));
}

#[test]
fn covedelta_rejects_unflagged_source_publish_range() {
    let mut header = CoveDeltaHeaderV1::new([1; 16], [2; 16], [3; 16], [4; 16]);
    header.source_publish_range_start_us = 10;
    header.source_publish_range_end_us = 20;
    let bytes = header.serialize();
    assert!(CoveDeltaHeaderV1::parse(&bytes).is_err());
}

#[test]
fn covedelta_object_delta_validates_temporal_segments() {
    let bytes = object_delta().serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let segments = parsed.validate_object_delta_sections().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].header.segment_id, 1);
    assert_eq!(segments[0].rows.len(), 1);
}

#[test]
fn covedelta_parse_rejects_overlapping_section_payload_region() {
    let mut bytes = object_delta().serialize().unwrap();
    rewrite_first_section_payload_offset(&mut bytes, 0);

    assert!(matches!(
        CoveDeltaFile::parse(&bytes),
        Err(CoveError::BadSection(message))
            if message.contains("payload regions must be canonical")
    ));
}

fn rewrite_first_section_payload_offset(bytes: &mut [u8], offset: u64) {
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

#[test]
fn covedelta_object_delta_rejects_duplicate_record_id_for_object_branch() {
    let mut delta = object_delta();
    delta.header.csn_max = 2;
    delta.header.commit_time_range_end_us = 20;
    let duplicate_record_id = [0xAB; 16];
    delta.sections[0].payload = temporal_segment_payload_for_rows(&[
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: duplicate_record_id,
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [7; 16],
            record_id: duplicate_record_id,
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
    ]);

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("duplicate record_id")
    ));
}

#[test]
fn covedelta_object_delta_allows_same_record_id_for_different_object() {
    let mut delta = object_delta();
    delta.header.csn_max = 2;
    delta.header.commit_time_range_end_us = 20;
    let shared_record_id = [0xAB; 16];
    delta.sections[0].payload = temporal_segment_payload_for_rows(&[
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: shared_record_id,
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [8; 16],
            record_id: shared_record_id,
            record_kind: RecordKind::Delta,
            prev_ref: None,
        },
    ]);

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.temporal_segments[0].rows.len(), 2);
}

#[test]
fn covedelta_object_delta_effective_fingerprint_refs_inherit_and_override() {
    let mut delta = object_delta();
    delta.parent_refs[0].schema_fingerprint_ref = 11;
    delta.parent_refs[0].object_catalog_fingerprint_ref = 12;
    delta.parent_refs[0].semantic_map_fingerprint_ref = 13;
    delta.parent_refs[0].projection_fingerprint_ref = 14;
    delta.header.object_catalog_fingerprint_ref = 22;
    delta.header.projection_fingerprint_ref = 24;

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.effective_schema_fingerprint_ref, 11);
    assert_eq!(validation.effective_object_catalog_fingerprint_ref, 22);
    assert_eq!(validation.effective_semantic_map_fingerprint_ref, 13);
    assert_eq!(validation.effective_projection_fingerprint_ref, 24);
}

#[test]
fn covedelta_object_delta_rejects_parent_ref_without_digest_binding() {
    let mut delta = object_delta();
    delta.parent_refs[0].digest_algorithm = DigestAlgorithm::None as u16;

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("cryptographic digest_algorithm")
    ));
}

#[test]
fn covedelta_object_delta_accepts_checkpoint_baseline_feature() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
    delta.sections[0].entry.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
    delta.sections[0].payload = checkpoint_temporal_segment_payload();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.checkpoint_row_count, 1);
    assert_eq!(
        validation.temporal_segments[0].rows[0].record_kind,
        RecordKind::Snapshot
    );
}

#[test]
fn covedelta_object_delta_rejects_checkpoint_feature_without_checkpoint_rows() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
    delta.sections[0].entry.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("checkpoint-baseline feature")
    ));
}

#[test]
fn covedelta_object_delta_parses_sparse_patch_ops() {
    let bytes = object_delta_with_sparse_patch_section()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.temporal_segments.len(), 1);
    assert_eq!(validation.sparse_patch_records.len(), 1);
    assert_eq!(
        validation.sparse_patch_records[0].changed_properties.len(),
        5
    );
}

#[test]
fn covedelta_object_delta_accepts_inline_dictionary_overlay() {
    let entry = sample_dictionary_entry();
    let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();
    let bytes = object_delta_with_dictionary_overlay_entries(std::slice::from_ref(&entry))
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
}

#[test]
fn covedelta_object_delta_accepts_parent_alias_dictionary_overlay() {
    let entry = sample_parent_alias_dictionary_entry();
    let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();

    let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
}

#[test]
fn covedelta_object_delta_rejects_parent_alias_without_required_feature() {
    let entry = sample_parent_alias_dictionary_entry();

    let bytes = object_delta_with_dictionary_overlay_entries_and_features(
        &[entry],
        DELTA_FEATURE_INLINE_DICTIONARY,
    )
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("parent dictionary alias feature")
    ));
}

#[test]
fn covedelta_object_delta_rejects_parent_alias_unknown_parent_ref() {
    let mut entry = sample_parent_alias_dictionary_entry();
    entry.parent_ref = 9;

    let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("unknown parent_ref")
    ));
}

#[test]
fn covedelta_object_delta_accepts_canonical_hash_dictionary_hint() {
    let entry = sample_canonical_hash_hint_dictionary_entry();
    let expected = DeltaDictionaryEntryV1::parse(&entry.serialize().unwrap()).unwrap();

    let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.dictionary_overlay_entries, vec![expected]);
}

#[test]
fn covedelta_object_delta_rejects_zero_canonical_hash_dictionary_hint() {
    let mut entry = sample_canonical_hash_hint_dictionary_entry();
    entry.canonical_hash128 = [0; 16];

    let bytes = object_delta_with_dictionary_overlay_entries(&[entry])
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("non-zero canonical_hash128")
    ));
}

#[test]
fn covedelta_object_delta_rejects_required_canonical_hash_dictionary_hint() {
    let entry = sample_canonical_hash_hint_dictionary_entry();

    let bytes = object_delta_with_dictionary_overlay_entries_and_features(
        &[entry],
        DELTA_FEATURE_INLINE_DICTIONARY,
    )
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("must not be required")
    ));
}

#[test]
fn covedelta_object_delta_rejects_duplicate_dictionary_overlay_local_code() {
    let first = sample_dictionary_entry();
    let mut second = sample_dictionary_entry();
    second.inline_value_ref = 1;

    let bytes = object_delta_with_dictionary_overlay_entries(&[first, second])
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("duplicate local dictionary code")
    ));
}

#[test]
fn covedelta_object_delta_parses_descriptor_tables() {
    let bytes = object_delta_with_descriptor_table_sections()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.scope_descriptors.len(), 1);
    assert_eq!(validation.scope_descriptors[0].scope_ref, 0);
    assert_eq!(validation.temporal_role_summary_descriptors.len(), 1);
    assert_eq!(
        validation.temporal_role_summary_descriptors[0].summary_kind,
        DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE
    );
    assert_eq!(validation.touched_summary_descriptors.len(), 1);
    assert_eq!(validation.tombstone_summary_descriptors.len(), 1);
}

#[test]
fn covedelta_object_delta_rejects_sparse_scope_descriptor_refs() {
    let mut delta = object_delta_with_descriptor_table_sections();
    let mut descriptor = sample_scope_descriptor();
    descriptor.scope_ref = 1;
    delta.sections[1].payload = descriptor.serialize().unwrap().to_vec();

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("dense zero-based by scope_ref")
    ));
}

#[test]
fn covedelta_object_delta_rejects_sparse_summary_descriptor_refs() {
    let mut delta = object_delta_with_descriptor_table_sections();
    let mut descriptor = sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET);
    descriptor.summary_ref = 1;
    delta.sections[3].payload = descriptor.serialize().unwrap().to_vec();

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("dense zero-based by summary_ref")
    ));
}

#[test]
fn covedelta_object_delta_projection_property_skip_uses_sparse_ops() {
    let bytes = object_delta_with_sparse_patch_section()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    let point = sample_object_point([7; 16]);

    assert!(!validation.can_skip_delta_for_projection_properties(point, &[1]));
    assert!(!validation.can_skip_delta_for_projection_properties(point, &[2]));
    assert!(!validation.can_skip_delta_for_projection_properties(point, &[4]));
    assert!(!validation.can_skip_delta_for_projection_properties(point, &[5]));
    assert!(!validation.can_skip_delta_for_projection_properties(point, &[]));
    assert!(validation.can_skip_delta_for_projection_properties(point, &[99]));
    assert!(
        !validation.can_skip_delta_for_projection_properties(sample_object_point([8; 16]), &[99])
    );
}

#[test]
fn covedelta_object_delta_projection_property_skip_uses_exact_touched_absence() {
    let bytes = object_delta_with_anchor_and_touched_sections()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert!(validation.can_skip_delta_for_projection_properties(sample_object_point([8; 16]), &[1]));
}

#[test]
fn covedelta_object_delta_projection_property_skip_keeps_row_tombstones() {
    let mut delta = object_delta_with_sparse_patch_section();
    let mut record = sample_sparse_patch_record();
    record.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
        property_id: 1,
        property_op: DELTA_PROPERTY_OP_TOMBSTONE,
        tombstone_kind: DELTA_TOMBSTONE_KIND_OBJECT,
        value_ref: DELTA_REF_NONE,
        redaction_ref: DELTA_REF_NONE,
        flags: 0,
    }];
    delta.sections[1].payload = record.serialize().unwrap();

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert!(
        !validation.can_skip_delta_for_projection_properties(sample_object_point([7; 16]), &[99])
    );
}

#[test]
fn covedelta_object_delta_parses_catalog_patch() {
    let patch = sample_catalog_patch();
    let bytes = object_delta_with_catalog_patch_section(patch.clone())
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.catalog_patches, vec![patch]);
}

#[test]
fn covedelta_object_delta_rejects_catalog_patch_item_count_mismatch() {
    let mut delta = object_delta_with_catalog_patch_section(sample_catalog_patch());
    delta.sections[1].entry.item_count = 2;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("catalog patch item_count")
    ));
}

#[test]
fn covedelta_object_delta_accepts_projection_patch() {
    let bytes = object_delta_with_projection_patch_section(projection_patch_payload())
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.projection_patches.len(), 1);
    assert_eq!(
        validation.projection_patches[0].projections[0].projection_id,
        "delta_projection"
    );
}

#[test]
fn covedelta_object_delta_rejects_required_projection_patch_without_section() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_PROJECTION_PATCH;
    delta.header.projection_fingerprint_ref = 41;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("requires projection patch section")
    ));
}

#[test]
fn covedelta_object_delta_rejects_projection_patch_without_fingerprint() {
    let mut delta = object_delta_with_projection_patch_section(projection_patch_payload());
    delta.header.projection_fingerprint_ref = 0;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("projection fingerprint")
    ));
}

#[test]
fn covedelta_object_delta_ignores_corrupt_optional_projection_patch() {
    let mut delta = object_delta();
    delta.sections.push(delta_projection_patch_section(
        2,
        b"corrupt projection patch".to_vec(),
        0,
    ));
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert!(validation.projection_patches.is_empty());
}

#[test]
fn covedelta_object_delta_accepts_required_index_hints() {
    let bytes = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::IndexHints,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
    )
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.index_hints.len(), 1);
    assert_eq!(validation.index_hints[0].parent_ref, 1);
}

#[test]
fn covedelta_object_delta_rejects_required_index_hints_without_section() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_INDEX_HINTS;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("requires index hint section")
    ));
}

#[test]
fn covedelta_object_delta_rejects_index_hint_lineage_parent_ref() {
    let mut delta = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::IndexHints,
        DELTA_FEATURE_INDEX_HINTS,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
    );
    delta.parent_refs.truncate(1);
    delta.sections[1].payload = sample_sidecar_hint(0, DELTA_SIDECAR_HINT_KIND_COVI_INDEX)
        .serialize()
        .to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("ancillary parent artifact")
    ));
}

#[test]
fn covedelta_object_delta_accepts_required_coverage_patch() {
    let bytes = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::CoveragePatch,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
    )
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.coverage_patches.len(), 1);
    assert_eq!(
        validation.coverage_patches[0].hint_kind,
        DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH
    );
}

#[test]
fn covedelta_object_delta_rejects_coverage_patch_wrong_hint_kind() {
    let bytes = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::CoveragePatch,
        DELTA_FEATURE_COVERAGE_PATCH,
        DELTA_SIDECAR_HINT_KIND_COVI_INDEX,
    )
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("incompatible hint_kind")
    ));
}

#[test]
fn covedelta_layout_hints_validate_on_request() {
    let mut delta = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::LayoutHints,
        0,
        DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS,
    );
    delta.header.required_delta_features = 0;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert_eq!(parsed.validate_layout_hints().unwrap().len(), 1);
}

#[test]
fn covedelta_layout_hints_reject_wrong_hint_kind_on_request() {
    let mut delta = object_delta_with_sidecar_hint_section(
        CoveDeltaSectionKind::LayoutHints,
        0,
        DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
    );
    delta.header.required_delta_features = 0;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_layout_hints(),
        Err(CoveError::BadSection(message))
            if message.contains("incompatible hint_kind")
    ));
}

#[test]
fn covedelta_object_delta_rejects_required_sparse_patch_without_property_ops() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_SPARSE_PATCH_ROWS;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("sparse patch feature")
    ));
}

#[test]
fn covedelta_object_delta_rejects_sparse_patch_without_temporal_row() {
    let mut delta = object_delta_with_sparse_patch_section();
    let mut record = sample_sparse_patch_record();
    record.goid = [8; 16];
    delta.sections[1].payload = record.serialize().unwrap();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("no matching temporal delta row")
    ));
}

#[test]
fn covedelta_object_delta_parses_anchor_and_touched_sections() {
    let bytes = object_delta_with_anchor_and_touched_sections()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.temporal_segments.len(), 1);
    assert_eq!(validation.continuation_anchors.len(), 1);
    assert_eq!(validation.state_hash_descriptors.len(), 1);
    assert!(validation.has_touched_object_set_section);
    assert_eq!(validation.touched_object_ranges.len(), 1);
}

#[test]
fn covedelta_object_delta_accepts_touched_property_bitmap_ref() {
    let bytes = object_delta_with_touched_property_bitmap_section(Some(
        DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP,
    ))
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.touched_object_ranges[0].property_bitmap_ref, 0);
    assert_eq!(
        validation.touched_summary_descriptors[0].summary_kind,
        DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP
    );
}

#[test]
fn covedelta_object_delta_rejects_touched_property_bitmap_missing_descriptor() {
    let bytes = object_delta_with_touched_property_bitmap_section(None)
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("property_bitmap_ref does not resolve")
    ));
}

#[test]
fn covedelta_object_delta_rejects_touched_property_bitmap_wrong_descriptor_kind() {
    let bytes = object_delta_with_touched_property_bitmap_section(Some(
        DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET,
    ))
    .serialize()
    .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("property bitmap descriptor")
    ));
}

#[test]
fn covedelta_object_delta_skips_exact_untouched_point_lookup() {
    let bytes = object_delta_with_anchor_and_touched_sections()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert_eq!(
        validation.exact_touched_membership(sample_object_point([7; 16])),
        DeltaExactObjectSetMembershipV1::Present
    );
    assert!(!validation.can_skip_delta_for_point_lookup(sample_object_point([7; 16])));
    assert_eq!(
        validation.exact_touched_membership(sample_object_point([8; 16])),
        DeltaExactObjectSetMembershipV1::Absent
    );
    assert!(validation.can_skip_delta_for_point_lookup(sample_object_point([8; 16])));
}

#[test]
fn covedelta_object_delta_does_not_skip_without_exact_touched_set() {
    let bytes = object_delta().serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert_eq!(
        validation.exact_touched_membership(sample_object_point([7; 16])),
        DeltaExactObjectSetMembershipV1::Unavailable
    );
    assert!(!validation.can_skip_delta_for_point_lookup(sample_object_point([7; 16])));
}

#[test]
fn covedelta_object_delta_rejects_underinclusive_touched_set() {
    let mut delta = object_delta_with_anchor_and_touched_sections();
    let mut touched = sample_touched_range();
    touched.min_goid = [8; 16];
    touched.max_goid = [8; 16];
    delta.sections[2].payload = touched.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("touched set under-includes")
    ));
}

#[test]
fn covedelta_object_delta_rejects_underinclusive_continuation_anchor() {
    let mut delta = object_delta_with_anchor_and_touched_sections();
    let mut anchor = sample_anchor();
    anchor.goid = [8; 16];
    delta.sections[1].payload = anchor.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("continuation anchors under-include")
    ));
}

#[test]
fn covedelta_object_delta_rejects_stale_continuation_anchor() {
    let mut delta = object_delta_with_anchor_and_touched_sections();
    let mut anchor = sample_anchor();
    anchor.predecessor_csn = 1;
    delta.sections[1].payload = anchor.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("continuation anchors under-include")
    ));
}

#[test]
fn covedelta_object_delta_rejects_unresolved_anchor_state_hash_ref() {
    let mut delta = object_delta_with_anchor_and_touched_sections();
    let mut anchor = sample_anchor();
    anchor.predecessor_state_hash_ref = 1;
    delta.sections[1].payload = anchor.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("predecessor_state_hash_ref does not resolve")
    ));
}

#[test]
fn covedelta_object_delta_parses_tombstone_set_section() {
    let bytes = object_delta_with_tombstone_set_section()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.temporal_segments.len(), 1);
    assert!(validation.has_tombstone_object_set_section);
    assert_eq!(validation.tombstone_object_ranges.len(), 1);
}

#[test]
fn covedelta_object_delta_tombstone_summary_checks_parent_latest_state() {
    let bytes = object_delta_with_tombstone_set_section()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert_eq!(
        validation.exact_tombstone_membership(sample_object_point([9; 16])),
        DeltaExactObjectSetMembershipV1::Present
    );
    assert!(
        validation.should_suppress_parent_latest_state_for_tombstone(sample_object_point([9; 16]))
    );
    assert_eq!(
        validation.exact_tombstone_membership(sample_object_point([8; 16])),
        DeltaExactObjectSetMembershipV1::Absent
    );
    assert!(
        !validation.should_suppress_parent_latest_state_for_tombstone(sample_object_point([8; 16]))
    );
}

#[test]
fn covedelta_object_delta_tombstone_membership_ignores_range_false_positives() {
    let mut delta = object_delta_with_tombstone_set_section();
    let mut tombstone = sample_tombstone_range();
    tombstone.min_goid = [8; 16];
    tombstone.max_goid = [10; 16];
    tombstone.touched_count = 1;
    delta.sections[1].payload = tombstone.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();

    assert_eq!(
        validation.exact_tombstone_membership(sample_object_point([9; 16])),
        DeltaExactObjectSetMembershipV1::Present
    );
    assert!(
        validation.should_suppress_parent_latest_state_for_tombstone(sample_object_point([9; 16]))
    );
    assert_eq!(
        validation.exact_tombstone_membership(sample_object_point([8; 16])),
        DeltaExactObjectSetMembershipV1::Absent
    );
    assert!(
        !validation.should_suppress_parent_latest_state_for_tombstone(sample_object_point([8; 16]))
    );
}

#[test]
fn covedelta_object_delta_rejects_underinclusive_tombstone_set() {
    let mut delta = object_delta_with_tombstone_set_section();
    let mut tombstone = sample_tombstone_range();
    tombstone.min_goid = [10; 16];
    tombstone.max_goid = [10; 16];
    delta.sections[1].payload = tombstone.serialize().to_vec();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("tombstone set under-includes")
    ));
}

#[test]
fn covedelta_object_delta_rejects_required_tombstone_set_without_section() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_EXACT_TOMBSTONE_SET;
    delta.sections[0].payload = tombstone_temporal_segment_payload();
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("tombstone-set feature")
    ));
}

#[test]
fn covedelta_object_delta_parses_branch_identity_section() {
    let bytes = object_delta_with_branch_identity_section()
        .serialize()
        .unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.branch_identities.len(), 1);
    assert_eq!(validation.branch_identities[0].branch_identity_ref, 0);
}

#[test]
fn covedelta_object_delta_rejects_required_anchor_without_anchor_section() {
    let mut delta = object_delta();
    delta.header.required_delta_features = DELTA_FEATURE_CONTINUATION_ANCHORS;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("continuation-anchor feature")
    ));
}

#[test]
fn covedelta_object_delta_rejects_single_scope_anchor_mismatch() {
    let mut delta = object_delta();
    delta.header.flags |= DELTA_FLAG_SINGLE_SCOPE;
    let mut anchor = sample_anchor();
    anchor.scope_id = [1; 16];
    anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_ONLY;
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
            required_delta_features: 0,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: anchor.serialize().to_vec(),
    });

    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert!(matches!(
        parsed.validate_object_delta(),
        Err(CoveError::BadSection(message))
            if message.contains("single-scope continuation anchor scope")
    ));
}

#[test]
fn covedelta_object_delta_rejects_placeholder_temporal_payload() {
    let bytes = minimal_delta().serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed.validate_object_delta_sections(),
        Err(CoveError::BufferTooShort)
    );
}

#[test]
fn covedelta_object_delta_rejects_unknown_required_feature() {
    let mut delta = object_delta();
    delta.header.required_delta_features = 1u64 << 63;
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed.validate_object_delta_sections(),
        Err(CoveError::UnknownRequiredFeature(1u64 << 63))
    );
}

#[test]
fn covedelta_object_delta_falls_back_for_optional_section_required_features() {
    let mut delta = object_delta_with_extra_section(
        CoveDeltaSectionKind::IndexHints,
        1u64 << 63,
        b"unsupported optional index hints".to_vec(),
    );
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 3,
            section_kind: CoveDeltaSectionKind::CoveragePatch as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 1u64 << 63,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: b"unsupported optional coverage patch".to_vec(),
    });
    delta.sections.push(CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 4,
            section_kind: CoveDeltaSectionKind::LayoutHints as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count: 1,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: 1u64 << 63,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload: b"unsupported optional layout hints".to_vec(),
    });
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    let validation = parsed.validate_object_delta().unwrap();
    assert_eq!(validation.temporal_segments.len(), 1);
}

#[test]
fn covedelta_object_delta_rejects_unknown_section_feature_on_required_section() {
    let delta = object_delta_with_extra_section(
        CoveDeltaSectionKind::TouchedObjectSet,
        1u64 << 63,
        Vec::new(),
    );
    let bytes = delta.serialize().unwrap();
    let parsed = CoveDeltaFile::parse(&bytes).unwrap();
    assert_eq!(
        parsed.validate_object_delta(),
        Err(CoveError::UnknownRequiredFeature(1u64 << 63))
    );
}

#[test]
fn continuation_anchor_roundtrip_and_existing_patch_strength() {
    let anchor = sample_anchor();
    let bytes = anchor.serialize();
    let parsed = DeltaContinuationAnchorV1::parse(&bytes).unwrap();
    assert_eq!(parsed.object_type_id, 1);
    parsed.validate_for_existing_object_patch().unwrap();
}

#[test]
fn continuation_anchor_rejects_weak_existing_patch_anchor() {
    let mut anchor = sample_anchor();
    anchor.anchor_strength = DELTA_ANCHOR_STRENGTH_KEY_AND_RECORD_ID;
    let parsed = DeltaContinuationAnchorV1::parse(&anchor.serialize()).unwrap();
    assert!(matches!(
        parsed.validate_for_existing_object_patch(),
        Err(CoveError::BadSection(message))
            if message.contains("KeyRecordAndStateHash")
    ));
}

#[test]
fn continuation_anchor_rejects_bad_checksum() {
    let mut bytes = sample_anchor().serialize();
    bytes[3] ^= 0xFF;
    assert_eq!(
        DeltaContinuationAnchorV1::parse(&bytes),
        Err(CoveError::ChecksumMismatch)
    );
}

#[test]
fn state_hash_descriptor_roundtrip_and_rejects_bad_payload_ref() {
    let descriptor = sample_state_hash_descriptor();
    let bytes = descriptor.serialize();
    let parsed = DeltaStateHashDescriptorV1::parse(&bytes).unwrap();
    assert_eq!(
        parsed.state_hash_kind,
        DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1
    );

    let mut missing_payload = sample_state_hash_descriptor();
    missing_payload.hash_payload_ref = DELTA_REF_NONE;
    assert!(matches!(
        DeltaStateHashDescriptorV1::parse(&missing_payload.serialize()),
        Err(CoveError::BadSection(message))
            if message.contains("hash_payload_ref")
    ));

    let mut reserved = sample_state_hash_descriptor();
    reserved.flags = 1;
    assert_eq!(
        DeltaStateHashDescriptorV1::parse(&reserved.serialize()),
        Err(CoveError::ReservedNotZero)
    );
}

#[test]
fn state_hash_descriptor_rejects_algorithm_len_mismatch() {
    let mut descriptor = sample_state_hash_descriptor();
    descriptor.hash_len = 31;
    assert!(matches!(
        DeltaStateHashDescriptorV1::parse(&descriptor.serialize()),
        Err(CoveError::BadSection(message))
            if message.contains("hash_len")
    ));
}

#[test]
fn dictionary_overlay_entry_roundtrip_and_rejects_bad_checksum() {
    let entry = sample_dictionary_entry();
    let bytes = entry.serialize().unwrap();
    let parsed = DeltaDictionaryEntryV1::parse(&bytes).unwrap();
    assert_eq!(parsed.local_dictionary_id, 1);
    assert_eq!(parsed.local_code, 7);
    assert_eq!(parsed.entry_kind, DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE);

    let mut corrupt = bytes;
    corrupt[4] ^= 0xFF;
    assert_eq!(
        DeltaDictionaryEntryV1::parse(&corrupt),
        Err(CoveError::ChecksumMismatch)
    );
}

#[test]
fn scope_descriptor_roundtrip_and_rejects_reserved_flags() {
    let descriptor = sample_scope_descriptor();
    let bytes = descriptor.serialize().unwrap();
    let parsed = DeltaScopeDescriptorV1::parse(&bytes).unwrap();
    assert_eq!(parsed.scope_ref, 0);
    assert_eq!(parsed.scope_id, [0; 16]);

    let mut reserved = descriptor;
    reserved.flags = 1;
    assert_eq!(reserved.serialize(), Err(CoveError::ReservedNotZero));
}

#[test]
fn summary_descriptor_roundtrip_and_rejects_unknown_kind() {
    let descriptor = sample_summary_descriptor(DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET);
    let bytes = descriptor.serialize().unwrap();
    let parsed = DeltaSummaryDescriptorV1::parse(&bytes).unwrap();
    assert_eq!(parsed.summary_ref, 0);
    assert_eq!(
        parsed.summary_kind,
        DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET
    );

    let mut unknown = descriptor;
    unknown.summary_kind = 200;
    assert!(matches!(
        unknown.serialize(),
        Err(CoveError::BadSection(message))
            if message.contains("unknown summary_kind")
    ));
}

#[test]
fn object_delta_state_hash_material_is_stable_and_canonical() {
    let state = sample_state_hash_material();
    let hash = state.compute_hash(DigestAlgorithm::Sha256).unwrap();
    assert_eq!(hash.len(), 32);

    let same_state = sample_state_hash_material();
    assert_eq!(
        hash,
        same_state.compute_hash(DigestAlgorithm::Sha256).unwrap()
    );

    let mut changed_state = sample_state_hash_material();
    changed_state.properties[0].canonical_value = b"bob".to_vec();
    assert_ne!(
        hash,
        changed_state.compute_hash(DigestAlgorithm::Sha256).unwrap()
    );

    let mut unsorted = sample_state_hash_material();
    unsorted.properties.swap(0, 1);
    assert!(matches!(
        unsorted.canonical_material(),
        Err(CoveError::BadSection(message))
            if message.contains("sorted by unique property_id")
    ));
}

#[test]
fn sparse_patch_record_roundtrip_and_rejects_bad_checksum() {
    let record = sample_sparse_patch_record();
    let bytes = record.serialize().unwrap();
    let parsed = DeltaSparsePatchRecordV1::parse(&bytes).unwrap();
    assert_eq!(parsed.changed_properties.len(), 5);

    let mut corrupt = bytes;
    corrupt[4] ^= 0xFF;
    assert_eq!(
        DeltaSparsePatchRecordV1::parse(&corrupt),
        Err(CoveError::ChecksumMismatch)
    );
}

#[test]
fn sparse_patch_record_rejects_unsorted_properties() {
    let mut record = sample_sparse_patch_record();
    record.changed_properties.swap(0, 1);
    assert!(matches!(
        record.serialize(),
        Err(CoveError::BadSection(message))
            if message.contains("sorted by unique property_id")
    ));
}

#[test]
fn sparse_patch_record_rejects_invalid_operation_payload_refs() {
    let mut missing_value = sample_sparse_patch_record();
    missing_value.changed_properties[0].value_ref = DELTA_REF_NONE;
    assert!(matches!(
        missing_value.serialize(),
        Err(CoveError::BadSection(message))
            if message.contains("requires value_ref")
    ));

    let mut missing_redaction = sample_sparse_patch_record();
    missing_redaction.changed_properties[4].redaction_ref = DELTA_REF_NONE;
    assert!(matches!(
        missing_redaction.serialize(),
        Err(CoveError::BadSection(message))
            if message.contains("requires only redaction_ref")
    ));
}

#[test]
fn sparse_patch_record_applies_operations_and_preserves_omitted_properties() {
    let record = sample_sparse_patch_record();
    let mut state = BTreeMap::from([
        (3, DeltaSparsePatchPropertyStateV1::ValueRef(99)),
        (99, DeltaSparsePatchPropertyStateV1::ValueRef(100)),
    ]);
    record.apply_to_property_state(&mut state).unwrap();

    assert_eq!(
        state.get(&1),
        Some(&DeltaSparsePatchPropertyStateV1::ValueRef(0))
    );
    assert_eq!(state.get(&2), Some(&DeltaSparsePatchPropertyStateV1::Null));
    assert_eq!(state.get(&3), Some(&DeltaSparsePatchPropertyStateV1::Clear));
    assert_eq!(
        state.get(&4),
        Some(&DeltaSparsePatchPropertyStateV1::Tombstone(
            DELTA_TOMBSTONE_KIND_PROPERTY
        ))
    );
    assert_eq!(
        state.get(&5),
        Some(&DeltaSparsePatchPropertyStateV1::Redacted { redaction_ref: 12 })
    );
    assert_eq!(
        state.get(&99),
        Some(&DeltaSparsePatchPropertyStateV1::ValueRef(100))
    );
}

#[test]
fn sparse_patch_state_table_applies_records_in_csn_order() {
    let mut first = sample_sparse_patch_record();
    first.record_id = [1; 16];
    first.timestamp_us = 10;
    first.csn = 1;
    first.changed_properties = vec![
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
    ];
    let mut second = sample_sparse_patch_record();
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

    let table = reconstruct_sparse_patch_state_table(&[second.clone(), first.clone()]).unwrap();
    let state = table.get(&first.object_key()).unwrap();
    assert_eq!(state.latest_record_id, [2; 16]);
    assert_eq!(state.latest_timestamp_us, 20);
    assert_eq!(state.latest_csn, 2);
    assert_eq!(
        state.tombstone_status,
        DeltaSparseObjectTombstoneStatusV1::Live
    );
    assert_eq!(
        state.properties.get(&1),
        Some(&DeltaSparsePatchPropertyStateV1::ValueRef(22))
    );
    assert_eq!(
        state.properties.get(&2),
        Some(&DeltaSparsePatchPropertyStateV1::Null)
    );
}

#[test]
fn sparse_patch_state_table_rejects_duplicate_record_id_for_object() {
    let first = sample_sparse_patch_record();
    let mut second = sample_sparse_patch_record();
    second.csn = 2;
    second.timestamp_us = 20;

    assert!(matches!(
        reconstruct_sparse_patch_state_table(&[first, second]),
        Err(CoveError::BadSection(message))
            if message.contains("duplicate record_id")
    ));
}

#[test]
fn sparse_patch_state_table_tracks_object_tombstone() {
    let mut record = sample_sparse_patch_record();
    record.changed_properties = vec![DeltaSparsePatchPropertyOpV1 {
        property_id: 1,
        property_op: DELTA_PROPERTY_OP_TOMBSTONE,
        tombstone_kind: DELTA_TOMBSTONE_KIND_OBJECT,
        value_ref: DELTA_REF_NONE,
        redaction_ref: DELTA_REF_NONE,
        flags: 0,
    }];

    let table = reconstruct_sparse_patch_state_table(&[record.clone()]).unwrap();
    let state = table.get(&record.object_key()).unwrap();
    assert_eq!(
        state.tombstone_status,
        DeltaSparseObjectTombstoneStatusV1::Tombstoned
    );
    assert_eq!(
        state.properties.get(&1),
        Some(&DeltaSparsePatchPropertyStateV1::Tombstone(
            DELTA_TOMBSTONE_KIND_OBJECT
        ))
    );
}

#[test]
fn touched_object_range_roundtrip_and_rejects_inverted_range() {
    let range = sample_touched_range();
    let bytes = range.serialize();
    let parsed = DeltaTouchedObjectRangeV1::parse(&bytes).unwrap();
    assert_eq!(parsed.touched_count, 1);

    let mut inverted = sample_touched_range();
    inverted.min_goid = [8; 16];
    assert!(matches!(
        DeltaTouchedObjectRangeV1::parse(&inverted.serialize()),
        Err(CoveError::BadSection(message))
            if message.contains("min_goid")
    ));
}

#[test]
fn branch_identity_roundtrip_and_rejects_missing_value_ref() {
    let identity = sample_branch_identity();
    let bytes = identity.serialize();
    let parsed = DeltaBranchIdentityV1::parse(&bytes).unwrap();
    assert_eq!(
        parsed.branch_identity_kind,
        DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF
    );

    let mut missing_ref = sample_branch_identity();
    missing_ref.branch_value_ref = DELTA_REF_NONE;
    assert!(matches!(
        DeltaBranchIdentityV1::parse(&missing_ref.serialize()),
        Err(CoveError::BadSection(message))
            if message.contains("branch_value_ref")
    ));

    let mut reserved = sample_branch_identity();
    reserved.flags = 1;
    assert_eq!(
        DeltaBranchIdentityV1::parse(&reserved.serialize()),
        Err(CoveError::ReservedNotZero)
    );
}
