use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use cove_cache::CoveCoverageCacheHeaderV2;
use cove_core::{
    artifact::{
        coveai::{
            write_coveai_artifact, write_coveai_descriptor_bundle, write_covev_filecode_vectors,
            AiAssetRefV1, AiDescriptorTablesV1, AiDigestEntryV1, AiPayloadEncodingV1,
            AiPayloadRefEntryV1, AiPrivacySummaryEntryV1, AiRequirednessScopeV1, AiStorageKindV1,
            ChunkProfileV1, CoveAiArtifactKind, CoveAiDescriptorBundleBuild, CoveAiWritableSection,
            CoveVecFileCodeVectorBuild, DatasetSplitV1, DedupGroupV1, DeviceTransferHintV1,
            GenerationDecodingProfileV1, GeneratorProvenanceV1, HumanReviewEntryV1,
            ModelActorDescriptorV1, MultimodalSequenceElementV1, MultimodalSequencePackV1,
            PreferencePairEntryV1, TensorLayoutDescriptorV1, TextChunkEntryV1, TokenBlockHeaderV1,
            TokenSequencePackV1, TokenizedSpanV1, TokenizerProfileV1, TrainingEpochPlanV1,
            TrainingLabelEntryV1, TrainingProfileV1, TrainingSampleEntryV1,
        },
        covedelta::{
            CoveDeltaFile, CoveDeltaFooterV1, CoveDeltaHeaderV1, CoveDeltaPostscriptV1,
            DeltaParentRefV1, DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT,
            DELTA_PARENT_REF_LINEAGE_PARENT,
        },
        covemap::{
            CovemapFile, CovemapHeaderV1, CovemapPostscriptV1, CovemapSection,
            CovemapSectionEntryV1,
        },
        covm::{
            CovmAiSidecarExtensionV1, CovmAiSidecarRefV1, CovmFile, CovmFileEntryV1, CovmHeaderV1,
            CovmPostscriptV1,
        },
        covx::CovxFile,
    },
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        PrimaryProfile, SectionKind, FEATURE_SEMANTIC_MAP,
    },
    digest::compute_digest,
    profile::cove_o::{
        CoveObjectPropertyValue, CoveObjectState, CoveObjectTombstoneStatus, ObjectTypeEntryV1,
        PropertyEntryV1, RecordKind, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
    },
    reader::validate_bytes,
    table::{ColumnEntry, TableCatalog, TableEntry},
    utility::hex_decode_exact,
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use cove_coverage::{
    CoverageExactnessV2, CoverageGranularityV2, CoverageProofStrengthV2,
    CoverageProviderDescriptorV2,
};
use cove_layout::{LayoutPlanHeaderV2, LayoutPlanNodeV2, LayoutPlanV2};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeHintKindV2};

fn temp_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cove-cli-smoke-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn cove_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cove"))
}

fn sample_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/coveql")
        .join(name)
}

fn showcase_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_cove(args: &[&str]) -> Output {
    Command::new(cove_bin()).args(args).output().unwrap()
}

fn run_cove_with_stdin(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(cove_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn cove_t_events_bytes() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                ColumnEntry {
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
                },
                ColumnEntry {
                    column_id: 2,
                    name: "score".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let mut ids = Vec::new();
    let mut scores = Vec::new();
    for (id, score) in [(1u64, 10u64), (2, 20), (3, 30)] {
        ids.extend_from_slice(&id.to_le_bytes());
        scores.extend_from_slice(&score.to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 3, 2);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(3, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(3, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn cove_o_thing_bytes() -> Vec<u8> {
    let object_type = ObjectTypeEntryV1 {
        object_type_id: 1,
        type_name: "Thing".into(),
        flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        properties: vec![PropertyEntryV1 {
            property_id: 1,
            property_name: "active".into(),
            logical_type: CoveLogicalType::Bool,
            physical_kind: CovePhysicalKind::Boolean,
            nullable: false,
            collation_id: 0,
            flags: 0,
        }],
    };
    let state = CoveObjectState {
        object_type_id: 1,
        object_type_name: "Thing".into(),
        object_type_flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        branch_key: 0,
        goid: [0x51; 16],
        latest_record_id: [0x52; 16],
        latest_segment_id: 0,
        latest_row_index: 0,
        timestamp_us: 1_700_000_000_000_010,
        csn: 1,
        record_kind: RecordKind::Snapshot,
        tombstone_status: CoveObjectTombstoneStatus::Live,
        properties: vec![CoveObjectPropertyValue {
            property_id: 1,
            property_name: "active".into(),
            logical_type: CoveLogicalType::Bool,
            physical_kind: CovePhysicalKind::Boolean,
            flags: 0,
            value: serde_json::json!(true),
            redacted: false,
        }],
        association: None,
    };
    cove_map::compact_cove_o_from_object_states(vec![object_type], &[state]).unwrap()
}

fn cove_t_labels_bytes() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "labels".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                ColumnEntry {
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
                },
                ColumnEntry {
                    column_id: 2,
                    name: "label".into(),
                    logical: CoveLogicalType::Utf8,
                    physical: CovePhysicalKind::VarBytes,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let mut ids = Vec::new();
    for id in [1u64, 2] {
        ids.extend_from_slice(&id.to_le_bytes());
    }
    let mut labels = Vec::new();
    for label in ["supercalifragilistic", "short"] {
        labels.extend_from_slice(&(label.len() as u32).to_le_bytes());
        labels.extend_from_slice(label.as_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 2, 2);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(2, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(2, labels).with_encoding_root(CoveEncodingKind::VarBytes as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn cove_t_relational_bytes() -> Vec<u8> {
    fn column(column_id: u32, name: &str) -> ColumnEntry {
        ColumnEntry {
            column_id,
            name: name.into(),
            logical: CoveLogicalType::Int64,
            physical: CovePhysicalKind::NumCode,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    let catalog = TableCatalog {
        flags: 0,
        tables: vec![
            TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "people".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "score")],
            },
            TableEntry {
                table_id: 2,
                namespace: "public".into(),
                name: "bonus".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "bonus")],
            },
            TableEntry {
                table_id: 3,
                namespace: "public".into(),
                name: "people_more".into(),
                row_count: 2,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(1, "id"), column(2, "score")],
            },
        ],
    };

    fn numcode_page(values: &[u64]) -> ScanPageSpec {
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        ScanPageSpec::new(values.len() as u32, bytes)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)
    }

    fn segment(table_id: u32, rows: &[(u64, u64)]) -> ScanSegment {
        let mut segment = ScanSegment::new(table_id, 0, 0, rows.len() as u32, 2);
        let left = rows.iter().map(|(left, _)| *left).collect::<Vec<_>>();
        let right = rows.iter().map(|(_, right)| *right).collect::<Vec<_>>();
        segment.set_column_pages(1, vec![numcode_page(&left)]);
        segment.set_column_pages(2, vec![numcode_page(&right)]);
        segment
    }

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment(1, &[(1, 10), (2, 20)]));
    writer.push_segment(segment(2, &[(2, 5), (3, 7)]));
    writer.push_segment(segment(3, &[(2, 20), (3, 30)]));
    writer.write().unwrap()
}

fn cove_t_medium_bytes() -> Vec<u8> {
    fn column(column_id: u32, name: &str) -> ColumnEntry {
        ColumnEntry {
            column_id,
            name: name.into(),
            logical: CoveLogicalType::Int64,
            physical: CovePhysicalKind::NumCode,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 25,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(1, "id"), column(2, "bucket"), column(3, "score")],
        }],
    };
    let mut ids = Vec::new();
    let mut buckets = Vec::new();
    let mut scores = Vec::new();
    for id in 1u64..=25 {
        ids.extend_from_slice(&id.to_le_bytes());
        buckets.extend_from_slice(&(id % 5).to_le_bytes());
        scores.extend_from_slice(&(id * 3).to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, 25, 3);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(25, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(25, buckets).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        3,
        vec![ScanPageSpec::new(25, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn covemap_bytes() -> Vec<u8> {
    CovemapFile {
        header: CovemapHeaderV1::new([0x11; 16], 0),
        mapping_version: "test/v1".into(),
        sections: Vec::new(),
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

fn covemap_ai_policy_bytes() -> Vec<u8> {
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapAiProfileCatalog as u16,
        "mapping_id": "customer-map",
        "mapping_version": "2026.05",
        "profiles": [{
            "profile_id": "customer_ai_v1",
            "active": true,
            "slot_policy_ids": ["private_note_forbidden"]
        }],
        "slot_policies": [{
            "slot_policy_id": "private_note_forbidden",
            "path": "Customer.private_note",
            "role": "PolicyProtected",
            "decision": "Forbidden",
            "granularity": "SlotValue",
            "sensitivity": "Forbidden"
        }]
    }))
    .unwrap();
    CovemapFile {
        header: CovemapHeaderV1::new([0x33; 16], 0),
        mapping_version: "2026.05".into(),
        sections: vec![CovemapSection {
            entry: CovemapSectionEntryV1 {
                section_id: SectionKind::MapAiProfileCatalog as u32,
                offset: 0,
                length: 0,
                uncompressed_length: 0,
                compression: 0,
                payload_encoding: 0,
                required: false,
                reserved: 0,
                checksum: 0,
            },
            payload,
        }],
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

fn coverage_provider_bytes() -> Vec<u8> {
    CoverageProviderDescriptorV2 {
        provider_id: 1,
        provider_kind: 0,
        profile: PrimaryProfile::TableScan as u8,
        granularity: CoverageGranularityV2::File,
        proof_strength: CoverageProofStrengthV2::ExactTight,
        exactness: CoverageExactnessV2::Exact,
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: u32::MAX,
        referenced_path_ref: u32::MAX,
        logical_type: 0,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref: u32::MAX,
        predicate_form_ref: u32::MAX,
        producer_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .to_vec()
}

fn layout_plan_bytes() -> Vec<u8> {
    LayoutPlanV2 {
        header: LayoutPlanHeaderV2 {
            layout_id: 1,
            node_count: 1,
            root_node_id: 1,
            flags: 0,
            checksum: 0,
        },
        nodes: vec![LayoutPlanNodeV2 {
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
            section_id: u32::MAX,
            cluster_id: u32::MAX,
            first_child_index: 0,
            child_count: 0,
            stats_ref: u32::MAX,
            split_ref: u32::MAX,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap()
}

fn cache_bytes() -> Vec<u8> {
    CoveCoverageCacheHeaderV2 {
        cache_format_namespace_ref: 0,
        cache_format_version_major: 1,
        cache_format_version_minor: 0,
        flags: 0,
        cache_id: [1; 16],
        dataset_id: [2; 16],
        snapshot_id: [3; 16],
        entry_count: 0,
        created_at_us: 0,
        producer_engine_ref: u32::MAX,
        reserved: [0; 32],
        checksum: 0,
    }
    .serialize()
    .to_vec()
}

fn runtime_hint_bytes() -> Vec<u8> {
    RuntimeCompatibilityHintV2 {
        hint_id: 1,
        hint_kind: RuntimeHintKindV2::EngineAdapter,
        required: true,
        flags: 0,
        namespace: "org.cove".into(),
        name: "adapter".into(),
        version_major: 1,
        version_minor: 0,
        payload_ref: u32::MAX,
        checksum: 0,
    }
    .serialize()
    .unwrap()
}

fn simple_delta_bytes_for_base(base: &[u8]) -> Vec<u8> {
    let validated = validate_bytes(base).unwrap();
    simple_delta_bytes_for_base_with_snapshot(base, validated.header.file_id)
}

fn simple_delta_bytes_for_base_with_source_publish(
    base: &[u8],
    start_us: i64,
    end_us: i64,
) -> Vec<u8> {
    let mut delta = CoveDeltaFile::parse(&simple_delta_bytes_for_base(base)).unwrap();
    delta.header.flags |= DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT;
    delta.header.source_publish_range_start_us = start_us;
    delta.header.source_publish_range_end_us = end_us;
    delta.serialize().unwrap()
}

fn simple_delta_bytes_for_base_with_snapshot(base: &[u8], base_snapshot_id: [u8; 16]) -> Vec<u8> {
    let validated = validate_bytes(base).unwrap();
    let base_id = validated.header.file_id;
    let mut header = CoveDeltaHeaderV1::new([0xD1; 16], [0xD0; 16], [0xD2; 16], base_snapshot_id);
    header.chain_ordinal = 1;
    header.chain_depth = 1;
    header.csn_min = 1;
    header.csn_max = 1;
    header.commit_time_range_start_us = 1_700_000_000_000_001;
    header.commit_time_range_end_us = 1_700_000_000_000_001;
    header.created_at_us = 1_700_000_000_000_001;
    let parent = DeltaParentRefV1 {
        parent_ref: 0,
        parent_kind: 0,
        flags: DELTA_PARENT_REF_LINEAGE_PARENT,
        artifact_id: base_id,
        snapshot_id: base_snapshot_id,
        file_len: base.len() as u64,
        footer_crc32c: validated.postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest_ref: 0,
        uri_ref: 0,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    };
    CoveDeltaFile {
        header,
        parent_refs: vec![parent],
        sections: Vec::new(),
        footer: CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: 0,
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
            footer_length: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

fn simple_delta_bytes_for_delta_parent(parent: &[u8]) -> Vec<u8> {
    let parsed_parent = CoveDeltaFile::parse(parent).unwrap();
    let mut header = CoveDeltaHeaderV1::new(
        [0xD3; 16],
        parsed_parent.header.dataset_id,
        [0xD4; 16],
        parsed_parent.header.snapshot_id,
    );
    header.chain_ordinal = parsed_parent.header.chain_ordinal + 1;
    header.chain_depth = parsed_parent.header.chain_depth + 1;
    header.csn_min = parsed_parent.header.csn_max + 1;
    header.csn_max = parsed_parent.header.csn_max + 1;
    header.commit_time_range_start_us = parsed_parent.header.commit_time_range_end_us + 1;
    header.commit_time_range_end_us = parsed_parent.header.commit_time_range_end_us + 1;
    header.created_at_us = parsed_parent.header.created_at_us + 1;
    let parent_ref = DeltaParentRefV1 {
        parent_ref: 0,
        parent_kind: 0,
        flags: DELTA_PARENT_REF_LINEAGE_PARENT,
        artifact_id: parsed_parent.header.delta_artifact_id,
        snapshot_id: parsed_parent.header.snapshot_id,
        file_len: parent.len() as u64,
        footer_crc32c: parsed_parent.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest_ref: 0,
        uri_ref: parsed_parent.header.chain_ordinal,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    };
    CoveDeltaFile {
        header,
        parent_refs: vec![parent_ref],
        sections: Vec::new(),
        footer: CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: 0,
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
            footer_length: 0,
            checksum: 0,
        },
    }
    .serialize()
    .unwrap()
}

fn covm_manifest_for_members(members: &[(&str, &[u8])]) -> Vec<u8> {
    let files = members
        .iter()
        .map(|(uri, bytes)| {
            let validated = validate_bytes(bytes).unwrap();
            CovmFileEntryV1 {
                file_id: validated.header.file_id,
                uri: (*uri).into(),
                file_len: validated.postscript.file_len,
                footer_crc32c: validated.postscript.footer.crc32c,
                digest_algorithm: DigestAlgorithm::Sha256 as u16,
                digest: compute_digest(DigestAlgorithm::Sha256, bytes).unwrap(),
                row_count: 0,
                segment_count: 0,
                file_stats_ref: 0,
                file_exact_set_ref: 0,
                flags: 0,
            }
        })
        .collect::<Vec<_>>();
    CovmFile {
        header: CovmHeaderV1::new([0xC1; 16], 1, files.len() as u32, 1_700_000_000_000_000),
        files,
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

#[test]
fn inspect_and_query_checked_in_cove_o_sample() {
    let people = sample_path("people.cove");
    let people = people.to_str().unwrap();

    let inspect = run_cove(&["inspect", people, "--queries"]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_stdout.contains("COVE-O"), "stdout={inspect_stdout}");
    assert!(
        inspect_stdout.contains("object(Person)"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains("projection(`people_primitives.v1`)"),
        "stdout={inspect_stdout}"
    );

    let projection = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "projection(`people_primitives.v1`).where(score >= 20).select(score, status, nickname).take(2)",
    ]);
    assert!(
        projection.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&projection.stderr)
    );
    let projection_stdout = String::from_utf8_lossy(&projection.stdout);
    assert!(projection_stdout.contains(r#""score":30"#));
    assert!(projection_stdout.contains(r#""status":"closed""#));

    let evidence = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "evidence(Person, grain: object).select(source_id, source_row_identity).take(1)",
    ]);
    assert!(
        evidence.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&evidence.stderr)
    );
    let evidence_stdout = String::from_utf8_lossy(&evidence.stdout);
    assert!(evidence_stdout.contains(r#""source_id":"people""#));
    assert!(evidence_stdout.contains(r#""source_row_identity":"people:"#));
}

#[test]
fn cove_o_mapped_tables_expose_sql_like_query_suite() {
    let people = sample_path("people.cove");
    let people = people.to_str().unwrap();

    let inspect = run_cove(&["inspect", people, "--queries"]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect_stdout.contains("Tables:"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains("table(people)"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains(
            "table(people).select(rows: count(*), total: sum(score), average: avg(score))"
        ),
        "stdout={inspect_stdout}"
    );

    let aggregate = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).select(rows: count(*), total: sum(score), average: avg(score))",
    ]);
    assert!(
        aggregate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&aggregate.stderr)
    );
    let aggregate_stdout = String::from_utf8_lossy(&aggregate.stdout);
    assert!(aggregate_stdout.contains(r#""rows":3"#));
    assert!(aggregate_stdout.contains(r#""total":60"#));

    let ordered = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).orderBy(score, desc).select(score, status).take(2)",
    ]);
    assert!(
        ordered.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&ordered.stderr)
    );
    let ordered_lines = String::from_utf8_lossy(&ordered.stdout)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(ordered_lines[0].contains(r#""score":30"#));
    assert!(ordered_lines[1].contains(r#""score":20"#));

    let window = run_cove(&[
        "query",
        people,
        "--format",
        "jsonl",
        "table(people).window(orderBy: score).select(score, rn: row_number()).take(2)",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
}

#[test]
fn cove_t_tables_expose_relational_query_suite() {
    let file = temp_file("relational.cove");
    fs::write(&file, cove_t_relational_bytes()).unwrap();

    let join = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people) as p.join(table(bonus) as b, on: p.id == b.id, kind: inner).select(id: p.id, score: p.score, bonus: b.bonus)",
    ]);
    assert!(
        join.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&join.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&join.stdout).trim(),
        r#"{"id":2,"score":20,"bonus":5}"#
    );

    let union = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).union(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        union.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&union.stderr)
    );
    let union_stdout = String::from_utf8_lossy(&union.stdout);
    assert_eq!(
        union_stdout.lines().collect::<Vec<_>>(),
        vec![r#"{"id":1}"#, r#"{"id":2}"#, r#"{"id":3}"#]
    );

    let window = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).window(orderBy: score).select(id, rn: row_number())",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
}

#[test]
fn cove_t_tables_expose_full_relational_methods() {
    let file = temp_file("relational-full.cove");
    fs::write(&file, cove_t_relational_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let lookup = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.lookup(table(bonus) as b, on: p.id == b.id, cardinality: many).select(id: p.id, bonus: b.bonus).orderBy(id)",
    ]);
    assert!(
        lookup.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&lookup.stderr)
    );
    let lookup_stdout = String::from_utf8_lossy(&lookup.stdout);
    assert!(lookup_stdout.contains(r#""id":1,"bonus":null"#));
    assert!(lookup_stdout.contains(r#""id":2,"bonus":5"#));

    let semi = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.semiJoin(table(bonus) as b, on: p.id == b.id).select(id: p.id)",
    ]);
    assert!(
        semi.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&semi.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&semi.stdout).trim(), r#"{"id":2}"#);

    let anti = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people) as p.antiJoin(table(bonus) as b, on: p.id == b.id).select(id: p.id)",
    ]);
    assert!(
        anti.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&anti.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&anti.stdout).trim(), r#"{"id":1}"#);

    let intersect = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).intersect(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        intersect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&intersect.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&intersect.stdout).trim(),
        r#"{"id":2}"#
    );

    let except = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).except(table(people_more), all: false).select(id).orderBy(id)",
    ]);
    assert!(
        except.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&except.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&except.stdout).trim(),
        r#"{"id":1}"#
    );

    let recursive = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(people).withRecursive(name: reach, seed: table(people), step: table(people_more), key: id, maxIterations: 4).join(table(reach) as r, on: people.id == r.id, kind: right, cardinality: many).select(id: r.id).orderBy(id)",
    ]);
    assert!(
        recursive.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&recursive.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&recursive.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![r#"{"id":1}"#, r#"{"id":2}"#, r#"{"id":3}"#]
    );
}

#[test]
fn query_supports_physical_compare_and_graph_algorithm_modes() {
    let file = temp_file("physical-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let physical = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--batch-size",
        "2",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        physical.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&physical.stderr)
    );
    assert!(String::from_utf8_lossy(&physical.stdout).contains(r#""score":20"#));

    let people = sample_path("people.cove");
    let compare = run_cove(&[
        "query",
        "--compare",
        people.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(people).where(score >= 20).select(score).take(2)",
    ]);
    assert!(
        compare.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&compare.stderr)
    );
    assert!(String::from_utf8_lossy(&compare.stdout).contains(r#""score":30"#));

    let graph = run_cove(&[
        "query",
        people.to_str().unwrap(),
        "--format",
        "jsonl",
        "node(Person) as p.connectedComponents().degree(kind: total).select(id: p.goid, component_id, degree).take(2)",
    ]);
    assert!(
        graph.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&graph.stderr)
    );
    let graph_stdout = String::from_utf8_lossy(&graph.stdout);
    assert!(graph_stdout.contains(r#""component_id":0"#));
    assert!(graph_stdout.contains(r#""degree":0"#));
}

#[test]
fn query_covm_manifest_registers_cove_t_member_tables() {
    let member = cove_t_events_bytes();
    let manifest = covm_manifest_for_members(&[("events.cove", &member)]);
    let manifest_path = temp_file("dataset.covm");
    let member_path = temp_file("events-member.cove");
    fs::write(&manifest_path, manifest).unwrap();
    fs::write(&member_path, member).unwrap();

    let output = run_cove(&[
        "query",
        manifest_path.to_str().unwrap(),
        "--member",
        &format!("events.cove={}", member_path.display()),
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(r#""id":2,"score":20"#));
    assert!(stdout.contains(r#""id":3,"score":30"#));
}

#[test]
fn inspect_query_discovery_describes_covm_dataset_binding() {
    let member = cove_t_events_bytes();
    let manifest = covm_manifest_for_members(&[("events.cove", &member)]);
    let manifest_path = temp_file("query-discovery-dataset.covm");
    fs::write(&manifest_path, manifest).unwrap();

    let output = run_cove(&[
        "inspect",
        "--query-discovery",
        "--json",
        manifest_path.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["source_binding"]["source_kind"],
        serde_json::json!("covm_dataset_snapshot")
    );
    assert_eq!(
        value["source_binding"]["members"][0]["member_id"],
        serde_json::json!("events.cove")
    );
    assert!(value["source_binding"]["members"][0]["file_digest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
}

#[test]
fn query_covm_dataset_rejects_manifest_member_escape() {
    let member = cove_t_events_bytes();
    let manifest = covm_manifest_for_members(&[("../events.cove", &member)]);
    let manifest_path = temp_file("dataset-escape.covm");
    fs::write(&manifest_path, manifest).unwrap();

    let output = run_cove(&[
        "query",
        manifest_path.to_str().unwrap(),
        "--dataset",
        manifest_path.parent().unwrap().to_str().unwrap(),
        "--format",
        "jsonl",
        "table(events).take(1)",
    ]);

    assert!(
        !output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parent-directory"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn query_covm_manifest_auto_discovers_digest_bound_ai_sidecar_ref() {
    let member = cove_t_events_bytes();
    let validated = validate_bytes(&member).unwrap();
    let member_entry = CovmFileEntryV1 {
        file_id: validated.header.file_id,
        uri: "events.cove".into(),
        file_len: validated.postscript.file_len,
        footer_crc32c: validated.postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest: compute_digest(DigestAlgorithm::Sha256, &member).unwrap(),
        row_count: 0,
        segment_count: 0,
        file_stats_ref: 0,
        file_exact_set_ref: 0,
        flags: 0,
    };
    let manifest = CovmFile {
        header: CovmHeaderV1::new([0xCA; 16], 1, 1, 1_700_000_000_000_100),
        files: vec![member_entry.clone()],
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    };
    let mut vector_payload = Vec::new();
    for value in [1.0f32, 0.0, 0.0] {
        vector_payload.extend_from_slice(&value.to_le_bytes());
    }
    let sidecar = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id: [0xCB; 16],
        created_at_us: 1_700_000_000_000_101,
        dimension_count: 3,
        file_codes: vec![1],
        vector_payload,
    })
    .unwrap();
    let extension = CovmAiSidecarExtensionV1 {
        flags: 0,
        refs: vec![CovmAiSidecarRefV1::new(
            member_entry.file_id,
            CoveAiArtifactKind::CoveVec,
            "dataset.covev".into(),
            &sidecar,
        )
        .unwrap()],
    };
    let manifest_bytes = manifest
        .serialize_with_extension_region(&extension.serialize().unwrap())
        .unwrap();

    let manifest_path = temp_file("dataset-ai.covm");
    let member_path = temp_file("events.cove");
    let sidecar_path = temp_file("dataset.covev");
    fs::write(&manifest_path, manifest_bytes).unwrap();
    fs::write(&member_path, member).unwrap();
    fs::write(&sidecar_path, sidecar).unwrap();

    let output = run_cove(&[
        "query",
        manifest_path.to_str().unwrap(),
        "--dataset",
        manifest_path.parent().unwrap().to_str().unwrap(),
        "--format",
        "json",
        "# profiles: table, ai\ntable(events).embedding(fileCode: 1)",
    ]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["file_code"], serde_json::json!(1));
    assert_eq!(
        value[0]["result_authority"],
        serde_json::json!("PersistedPayloadDigest")
    );

    let mut stale_sidecar = fs::read(&sidecar_path).unwrap();
    stale_sidecar[0] ^= 0xFF;
    fs::write(&sidecar_path, stale_sidecar).unwrap();
    let stale = run_cove(&[
        "query",
        manifest_path.to_str().unwrap(),
        "--dataset",
        manifest_path.parent().unwrap().to_str().unwrap(),
        "--format",
        "json",
        "# profiles: table, ai\ntable(events).embedding(fileCode: 1)",
    ]);
    assert!(
        !stale.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&stale.stdout),
        String::from_utf8_lossy(&stale.stderr)
    );
    assert!(
        String::from_utf8_lossy(&stale.stderr).contains("no digest-valid COVM AI sidecar"),
        "stderr={}",
        String::from_utf8_lossy(&stale.stderr)
    );
}

#[test]
fn query_external_file_backed_tables_without_cove_artifact() {
    let csv = temp_file("external-people.csv");
    fs::write(&csv, "id,score,active\n1,10,true\n2,20,false\n3,30,true\n").unwrap();
    let external_arg = format!("people={}", csv.display());
    let output = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(people).where(score >= 20).select(id, score).orderBy(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![r#"{"id":2,"score":20}"#, r#"{"id":3,"score":30}"#]
    );

    let jsonl = temp_file("external-people.jsonl");
    fs::write(&jsonl, "{\"id\":1,\"score\":10}\n{\"id\":2,\"score\":20}\n").unwrap();
    let external_arg = format!("people={}", jsonl.display());
    let aggregate = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(people).select(rows: count(*), total: sum(score))",
    ]);

    assert!(
        aggregate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&aggregate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&aggregate.stdout).trim(),
        r#"{"rows":2,"total":30}"#
    );
}

#[test]
fn query_external_tables_join_with_cove_t_tables() {
    let file = temp_file("events-with-external.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let weights = temp_file("weights.json");
    fs::write(&weights, r#"[{"id":2,"weight":5},{"id":3,"weight":7}]"#).unwrap();
    let external_arg = format!("weights={}", weights.display());

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--external-table",
        &external_arg,
        "--format",
        "jsonl",
        "table(events) as e.join(table(weights) as w, on: e.id == w.id, kind: inner).select(id: e.id, score: e.score, weight: w.weight).orderBy(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .collect::<Vec<_>>(),
        vec![
            r#"{"id":2,"score":20,"weight":5}"#,
            r#"{"id":3,"score":30,"weight":7}"#
        ]
    );
}

#[test]
fn query_file_and_stdin_queries_execute() {
    let file = temp_file("events-query-file.cove");
    let query_file = temp_file("events-query.coveql");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    fs::write(&query_file, "table(events).where(score >= 20).select(id)").unwrap();

    let from_file = run_cove(&[
        "query",
        "--query-file",
        query_file.to_str().unwrap(),
        "--format",
        "jsonl",
        file.to_str().unwrap(),
    ]);
    assert!(
        from_file.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_file.stderr)
    );
    assert!(String::from_utf8_lossy(&from_file.stdout).contains(r#""id":2"#));

    let from_stdin = run_cove_with_stdin(
        &[
            "query",
            "--query-file",
            "-",
            "--format",
            "jsonl",
            file.to_str().unwrap(),
        ],
        "table(events).where(score >= 30).select(score)",
    );
    assert!(
        from_stdin.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_stdin.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&from_stdin.stdout).trim(),
        r#"{"score":30}"#
    );
}

#[test]
fn query_from_query_discovery_template_executes() {
    let file = temp_file("events-query-template.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        "--from-template",
        "table_filter_select_take",
        "--param",
        "score=20",
        "--param",
        "columns=id,score",
        "--format",
        "jsonl",
        file.to_str().unwrap(),
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"{"id":2,"score":20}"#
    );
}

#[test]
fn table_output_respects_max_cell_width() {
    let file = temp_file("labels.cove");
    fs::write(&file, cove_t_labels_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        "--max-cell-width",
        "12",
        file.to_str().unwrap(),
        "table(labels).select(label)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("supercali..."), "stdout={stdout}");
    assert!(stdout.contains("long cells truncated"), "stdout={stdout}");
}

#[test]
fn inspect_cove_t_prints_query_suggestions() {
    let file = temp_file("events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&["inspect", file.to_str().unwrap(), "--queries"]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("COVE-T"), "stdout={stdout}");
    assert!(stdout.contains("table(events)"));
    assert!(stdout.contains("table(events).select(id, score).take(10)"));
}

#[test]
fn query_discovery_cli_emits_manifest_and_validation_envelope() {
    let file = sample_path("events.cove");
    let path = file.to_str().unwrap();

    let inspect = run_cove(&[
        "inspect",
        "--query-discovery",
        "--json",
        "--policy",
        "public",
        "--audience",
        "smoke-agent",
        "--principal-class",
        "public_agent",
        "--policy-fingerprint",
        "sha256:smoke",
        path,
    ]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(
        manifest["schema"],
        serde_json::json!("cove.query_discovery.v1")
    );
    assert_eq!(
        manifest["authority"],
        serde_json::json!("advisory_discovery_not_archive_truth")
    );
    assert_eq!(manifest["coveql"]["profiles"], serde_json::json!(["table"]));
    assert_eq!(
        manifest["policy"]["audience"],
        serde_json::json!("smoke-agent")
    );
    assert_eq!(
        manifest["policy"]["principal_class"],
        serde_json::json!("public_agent")
    );
    assert_eq!(
        manifest["policy"]["policy_fingerprint"],
        serde_json::json!("sha256:smoke")
    );
    assert!(manifest.get("validation_status").is_none());
    assert!(manifest["surfaces"]["tables"][0]["root"]
        .as_str()
        .unwrap()
        .starts_with("table("));
    assert_eq!(
        manifest["examples"][0]["query_validation"],
        serde_json::json!("not_validated")
    );
    assert!(manifest["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "QD_EXAMPLE_VALIDATION_FAILED"));

    let alias = run_cove(&["inspect", "--agent", path]);
    assert!(
        alias.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&alias.stderr)
    );
    let alias_manifest: serde_json::Value = serde_json::from_slice(&alias.stdout).unwrap();
    assert_eq!(alias_manifest["schema"], manifest["schema"]);

    let doctor = run_cove(&["doctor", "--query-discovery", "--json", path]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(envelope["manifest"]["schema"], manifest["schema"]);
    assert_eq!(
        envelope["validation"]["validation_status"],
        serde_json::json!("valid")
    );
}

#[test]
fn inspect_ai_reports_coveai_sidecar() {
    let path = temp_file("empty.coveai");
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [9u8; 16], 99, &[]).unwrap();
    fs::write(&path, bytes).unwrap();

    let output = run_cove(&["inspect", "--ai", path.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("AI Inspect:"), "stdout={stdout}");
    assert!(stdout.contains(".coveai (CVA2)"), "stdout={stdout}");
    assert!(stdout.contains("Sections: 0"), "stdout={stdout}");
}

#[test]
fn train_export_reports_validated_training_metadata() {
    let path = temp_file("empty-training.coveai");
    let bytes =
        write_coveai_artifact(CoveAiArtifactKind::CoveAiBundle, [8u8; 16], 100, &[]).unwrap();
    fs::write(&path, bytes).unwrap();

    let output = run_cove(&["train", "export", path.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["artifact"], serde_json::json!("coveai"));
    assert_eq!(value["counts"]["samples_total"], serde_json::json!(0));
    assert_eq!(value["counts"]["samples_exported"], serde_json::json!(0));
    assert_eq!(
        value["payload_access"],
        serde_json::json!("StructurallyAllowed")
    );

    let jsonl = run_cove(&[
        "train",
        "export",
        path.to_str().unwrap(),
        "--format",
        "jsonl",
    ]);
    assert!(
        jsonl.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&jsonl.stderr)
    );
    assert!(jsonl.stdout.is_empty());
}

fn coveql_ai_descriptor_sidecar_bytes() -> Vec<u8> {
    let mut tables = AiDescriptorTablesV1::default();
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 1,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.digests.push(AiDigestEntryV1 {
        digest_ref: 2,
        digest_algorithm: 1,
        digest_len: 4,
        digest_payload_ref: 2,
        domain_hint: 1,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 1,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 8,
        decoded_length: 8,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.payload_refs.push(AiPayloadRefEntryV1 {
        payload_ref: 2,
        storage_kind: AiStorageKindV1::SectionDecodedRelative as u8,
        media_type_ref: 0,
        section_id: 10,
        uri_ref: 0,
        payload_offset: 0,
        section_payload_offset: 0,
        payload_length: 4,
        decoded_length: 4,
        integrity_ref: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.privacy_summaries.push(AiPrivacySummaryEntryV1 {
        privacy_summary_ref: 1,
        source_binding_ref: 0,
        sensitivity_mask: 0,
        sensitivity_bits_ref: 0,
        policy_ref: 0,
        visibility_scope_ref: 0,
        redaction_scope_ref: 0,
        retention_state: 0,
        disclosure_state: 0,
        flags: 0,
        crc32c: 0,
    });
    tables.chunk_profiles.push(ChunkProfileV1 {
        chunk_profile_id: 1,
        profile_name_ref: 0,
        chunker_namespace_ref: 0,
        chunker_name_ref: 0,
        chunker_version_major: 1,
        chunker_version_minor: 0,
        tokenizer_profile_ref: 1,
        boundary_kind: 1,
        overlap_policy: 0,
        parent_policy: 0,
        normalization_policy: 0,
        target_tokens: 128,
        min_tokens: 1,
        max_tokens: 256,
        overlap_tokens: 0,
        max_bytes: 4096,
        locale_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.text_chunks.push(TextChunkEntryV1 {
        chunk_id: 1,
        source_ref: 0,
        table_id: 1,
        column_id: 2,
        object_type_id: 3,
        property_id: 4,
        association_type_id: 0,
        path_ref: 0,
        source_row_ref: 7,
        source_object_ref: 8,
        source_value_hash_ref: 1,
        byte_start: 4,
        byte_length: 12,
        unicode_scalar_start: 4,
        unicode_scalar_length: 12,
        token_start: 0,
        token_count: 4,
        parent_chunk_id: 0,
        first_child_ref: 0,
        child_count: 0,
        previous_chunk_id: 0,
        next_chunk_id: 0,
        chunk_text_hash_ref: 2,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tokenizer_profiles.push(TokenizerProfileV1 {
        tokenizer_profile_id: 1,
        tokenizer_namespace_ref: 0,
        tokenizer_name_ref: 0,
        tokenizer_version_major: 1,
        tokenizer_version_minor: 0,
        vocab_digest_ref: 0,
        merges_digest_ref: 0,
        pre_tokenizer_digest_ref: 0,
        normalizer_digest_ref: 0,
        byte_encoder_digest_ref: 0,
        special_tokens_digest_ref: 0,
        added_tokens_digest_ref: 0,
        chat_template_ref: 0,
        unicode_version_ref: 0,
        truncation_policy_ref: 0,
        padding_policy_ref: 0,
        model_max_sequence_length: 4096,
        token_id_width: 2,
        byte_alignment_available: 0,
        reversible: 1,
        deterministic: 1,
        bos_token_id: 0,
        eos_token_id: 0,
        pad_token_id: 0,
        unk_token_id: 0,
        flags: 0,
        checksum: 0,
    });
    tables.token_blocks.push(TokenBlockHeaderV1 {
        token_block_id: 1,
        tokenizer_profile_id: 1,
        token_count: 4,
        token_id_width: 2,
        compression_codec: CompressionCodec::None as u8,
        layout_kind: 0,
        payload_ref: 1,
        payload_offset: 0,
        payload_length: 0,
        integrity_ref: 0,
        checksum: 0,
    });
    tables.tokenized_spans.push(TokenizedSpanV1 {
        tokenized_span_id: 1,
        chunk_id: 1,
        tokenizer_profile_id: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        byte_alignment_ref: 0,
        source_value_hash_ref: 1,
        flags: 0,
        checksum: 0,
    });
    tables.token_sequence_packs.push(TokenSequencePackV1 {
        sequence_pack_id: 1,
        tokenizer_profile_id: 1,
        training_profile_ref: 1,
        token_block_ref: 1,
        token_offset: 0,
        token_count: 4,
        source_span_count: 0,
        first_source_span_ref: 0,
        loss_mask_ref: 0,
        attention_mask_ref: 0,
        position_ids_ref: 0,
        labels_ref: 0,
        split_ref: 1,
        sample_weight_ppm: 1_000_000,
        flags: 0,
        checksum: 0,
    });
    tables.training_profiles.push(TrainingProfileV1 {
        training_profile_id: 1,
        profile_name_ref: 0,
        task_family: 1,
        modality_mask: 3,
        source_snapshot_ref: 0,
        map_profile_ref: 0,
        chunk_profile_ref: 1,
        tokenizer_profile_ref: 1,
        vector_space_ref: 0,
        multimodal_sequence_profile_ref: 0,
        split_policy_ref: 0,
        sampling_policy_ref: 0,
        dedup_policy_ref: 0,
        quality_policy_ref: 0,
        license_policy_ref: 0,
        redaction_policy_ref: 0,
        default_generator_provenance_ref: 0,
        reproducibility_class: 1,
        flags: 0,
        checksum: 0,
    });
    tables.dataset_splits.push(DatasetSplitV1 {
        split_id: 1,
        split_name_ref: 0,
        split_method: 1,
        source_snapshot_ref: 0,
        filter_policy_ref: 0,
        seed: 42,
        hash_function_ref: 0,
        stratification_path_ref: 0,
        grouping_ref: 0,
        ordering_policy_ref: 0,
        dedup_policy_ref: 0,
        sample_count: 1,
        first_sample_ref: 1,
        flags: 0,
        checksum: 0,
    });
    tables.dedup_groups.push(DedupGroupV1 {
        dedup_group_id: 1,
        dedup_policy_ref: 0,
        canonical_member_sample_id: 1,
        similarity_kind: 0,
        dedup_authority: 0,
        confidence_ppm: 1_000_000,
        first_member_ref: 1,
        member_count: 1,
        flags: 0,
        checksum: 0,
    });
    tables.training_epoch_plans.push(TrainingEpochPlanV1 {
        epoch_plan_id: 1,
        training_profile_id: 1,
        split_ref: 1,
        seed: 42,
        permutation_kind: 0,
        rng_algorithm_ref: 0,
        permutation_function_ref: 0,
        shard_count: 0,
        first_shard_ref: 0,
        shard_ref_count: 0,
        flags: 0,
        checksum: 0,
    });
    tables.training_samples.push(TrainingSampleEntryV1 {
        sample_id: 1,
        training_profile_id: 1,
        example_kind: 1,
        split_ref: 1,
        source_ref: 0,
        evidence_ref: 0,
        input_ref: 0,
        target_ref: 0,
        label_ref: 0,
        metadata_ref: 0,
        token_sequence_pack_ref: 1,
        multimodal_sequence_pack_ref: 1,
        vector_ref: 0,
        quality_score_ppm: 900_000,
        sample_weight_ppm: 1_000_000,
        dedup_group_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        teacher_model_ref: 0,
        generator_provenance_ref: 0,
        judge_generator_provenance_ref: 0,
        label_generator_provenance_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.model_actors.push(ModelActorDescriptorV1 {
        model_actor_id: 1,
        model_namespace_ref: 0,
        model_name_ref: 0,
        model_version_ref: 0,
        model_checkpoint_digest_ref: 0,
        provider_ref: 0,
        endpoint_ref: 0,
        endpoint_version_ref: 0,
        model_family_ref: 0,
        modality_mask: 3,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .generation_decoding_profiles
        .push(GenerationDecodingProfileV1 {
            decoding_profile_id: 1,
            temperature_micros: 0,
            top_p_micros: 1_000_000,
            top_k: 0,
            seed: 42,
            max_output_tokens: 128,
            stop_sequence_ref: 0,
            safety_policy_ref: 0,
            deterministic_claim: 0,
            flags: 0,
            checksum: 0,
        });
    tables.human_reviews.push(HumanReviewEntryV1 {
        human_review_id: 1,
        review_kind: 1,
        reviewer_role_ref: 0,
        review_time_us: 1_234,
        rating_ppm: 900_000,
        notes_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.generator_provenance.push(GeneratorProvenanceV1 {
        generator_provenance_id: 1,
        generator_kind: 1,
        model_actor_ref: 1,
        prompt_template_ref: 0,
        decoding_profile_ref: 1,
        toolchain_ref: 0,
        source_input_ref: 0,
        source_context_ref: 0,
        source_sample_ref: 1,
        parent_generator_provenance_ref: 0,
        generation_time_us: 1_235,
        confidence_ppm: 800_000,
        human_review_ref: 1,
        policy_ref: 0,
        reproducibility_class: 2,
        flags: 0,
        checksum: 0,
    });
    tables.training_labels.push(TrainingLabelEntryV1 {
        label_id: 1,
        label_kind: 1,
        label_authority: 3,
        label_payload_ref: 0,
        generator_provenance_ref: 1,
        human_review_ref: 1,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.preference_pairs.push(PreferencePairEntryV1 {
        preference_pair_id: 1,
        prompt_ref: 0,
        chosen_ref: 0,
        rejected_ref: 0,
        judge_generator_provenance_ref: 1,
        human_review_ref: 1,
        preference_strength_ppm: 750_000,
        confidence_ppm: 850_000,
        evidence_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.tensor_layouts.push(TensorLayoutDescriptorV1 {
        tensor_layout_id: 1,
        layout_name_ref: 0,
        rank: 2,
        dtype: 0,
        byte_order: 1,
        shape_ref: 0,
        stride_ref: 0,
        storage_offset_elements: 0,
        layout_kind: 0,
        memory_alignment_bytes: 64,
        preferred_page_alignment_bytes: 4096,
        tile_shape_ref: 0,
        block_shape_ref: 0,
        quantization_profile_ref: 0,
        sparsity_profile_ref: 0,
        framework_compatibility_ref: 0,
        device_affinity_hint: 0,
        flags: 0,
        checksum: 0,
    });
    tables.device_transfer_hints.push(DeviceTransferHintV1 {
        transfer_hint_id: 1,
        target_kind: 0,
        preferred_alignment_bytes: 64,
        preferred_chunk_bytes: 4096,
        pinned_memory_required: 0,
        contiguous_required: 1,
        zero_copy_possible: 1,
        runtime_registry_binding_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables.assets.push(AiAssetRefV1 {
        asset_ref_id: 1,
        parent_asset_ref: 0,
        asset_kind: 2,
        uri_ref: 0,
        embedded_section_ref: 0,
        media_type_ref: 0,
        byte_length: 0,
        digest_ref: 0,
        width: 64,
        height: 64,
        duration_us: 0,
        sample_rate_hz: 0,
        channel_count: 0,
        decode_profile_ref: 0,
        preprocessing_profile_ref: 0,
        transform_profile_ref: 0,
        transform_digest_ref: 0,
        tensor_layout_ref: 1,
        license_ref: 0,
        policy_ref: 0,
        flags: 0,
        checksum: 0,
    });
    tables
        .multimodal_sequence_packs
        .push(MultimodalSequencePackV1 {
            sequence_pack_id: 1,
            training_profile_id: 1,
            tokenizer_profile_id: 1,
            sequence_profile_ref: 0,
            element_count: 1,
            first_element_ref: 1,
            split_ref: 1,
            sample_weight_ppm: 1_000_000,
            loss_mask_ref: 0,
            attention_mask_ref: 0,
            position_map_ref: 0,
            label_ref: 0,
            source_snapshot_ref: 0,
            evidence_ref: 0,
            generator_provenance_ref: 0,
            flags: 0,
            checksum: 0,
        });
    tables
        .multimodal_sequence_elements
        .push(MultimodalSequenceElementV1 {
            element_id: 1,
            sequence_pack_id: 1,
            ordinal: 0,
            element_kind: 2,
            modality: 2,
            role: 1,
            tokenized_span_ref: 1,
            token_sequence_pack_ref: 1,
            asset_ref: 1,
            tensor_ref: 1,
            vector_ref: 0,
            byte_start: 0,
            byte_length: 0,
            time_start_us: 0,
            time_duration_us: 0,
            position_stream_ref: 0,
            evidence_ref: 0,
            policy_ref: 0,
            flags: 0,
            checksum: 0,
        });

    let payload_sections = vec![CoveAiWritableSection {
        section_id: 10,
        section_kind: SectionKind::AiPayloadBytes as u32,
        profile_kind: PrimaryProfile::CoveTok as u8,
        payload_encoding: AiPayloadEncodingV1::OpaqueBytes,
        requiredness_scope: AiRequirednessScopeV1::AdvisoryOnly,
        source_binding_ref: 0,
        required_ai_features: 0,
        optional_ai_features: 0,
        feature_binding_ref: 0,
        payload: vec![1, 0, 2, 0, 3, 0, 4, 0],
    }];
    write_coveai_descriptor_bundle(&CoveAiDescriptorBundleBuild {
        artifact_id: [44u8; 16],
        created_at_us: 1_002,
        payload_sections,
        descriptor_tables: tables,
    })
    .unwrap()
}

#[test]
fn vec_build_creates_valid_covev_sidecar() {
    let path = temp_file("vectors.covev");
    let cove_path = temp_file("events-ai-query.cove");
    fs::write(&cove_path, cove_t_events_bytes()).unwrap();

    let build = run_cove(&[
        "vec",
        "build",
        "--out",
        path.to_str().unwrap(),
        "--dimension",
        "3",
        "--file-code",
        "1",
        "--file-code",
        "2",
        "--deterministic",
        "--index",
        "hnsw",
        "--metric",
        "dot",
        "--created-at-us",
        "123",
    ]);
    let build_stdout = String::from_utf8_lossy(&build.stdout);
    assert!(
        build.status.success(),
        "stdout={build_stdout}\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        build_stdout.contains("2 FileCode vectors, dimension 3"),
        "stdout={build_stdout}"
    );

    let validate = run_cove(&["validate", path.to_str().unwrap()]);
    assert!(
        validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate.stderr)
    );

    let inspect = run_cove(&["inspect", "--ai", path.to_str().unwrap()]);
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect.status.success(),
        "stdout={inspect_stdout}\nstderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(
        inspect_stdout.contains(".covev (CVV2)"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout
            .contains("vector_spaces=1 vector_blocks=1 vector_entries=2 filecode_bindings=2"),
        "stdout={inspect_stdout}"
    );
    assert!(
        inspect_stdout.contains("Runtime: payload_exposure_eligible=true"),
        "stdout={inspect_stdout}"
    );

    let quantized_path = temp_file("vectors-int8.covev");
    let integrity_report_path = temp_file("vectors-int8.integrity.json");
    let quantized_build = run_cove(&[
        "vec",
        "build",
        "--out",
        quantized_path.to_str().unwrap(),
        "--dimension",
        "3",
        "--file-code",
        "1",
        "--file-code",
        "2",
        "--deterministic",
        "--index",
        "ivf-flat",
        "--metric",
        "l2",
        "--quantization",
        "int8",
        "--seed",
        "99",
        "--ef",
        "32",
        "--shard-count",
        "2",
        "--integrity-report",
        integrity_report_path.to_str().unwrap(),
    ]);
    let quantized_stdout = String::from_utf8_lossy(&quantized_build.stdout);
    assert!(
        quantized_build.status.success(),
        "stdout={quantized_stdout}\nstderr={}",
        String::from_utf8_lossy(&quantized_build.stderr)
    );
    assert!(
        quantized_stdout.contains("index=ivf-flat, metric=l2, quantization=int8"),
        "stdout={quantized_stdout}"
    );
    let quantized_validate = run_cove(&["validate", quantized_path.to_str().unwrap()]);
    assert!(
        quantized_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&quantized_validate.stderr)
    );
    let integrity_report: serde_json::Value =
        serde_json::from_slice(&fs::read(&integrity_report_path).unwrap()).unwrap();
    assert_eq!(integrity_report["build"]["metric"], serde_json::json!("l2"));
    assert_eq!(
        integrity_report["build"]["quantization"],
        serde_json::json!("int8")
    );
    assert_eq!(
        integrity_report["vector_spaces"][0]["element_type"],
        serde_json::json!(3)
    );
    assert!(
        integrity_report["payload_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0),
        "{integrity_report}"
    );

    let embedding = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--format",
        "json",
        "--cove-ai",
        path.to_str().unwrap(),
        cove_path.to_str().unwrap(),
        "# profiles: table, ai\ntable(events).embedding(fileCode: 1)",
    ]);
    let embedding_stdout = String::from_utf8_lossy(&embedding.stdout);
    assert!(
        embedding.status.success(),
        "stdout={embedding_stdout}\nstderr={}",
        String::from_utf8_lossy(&embedding.stderr)
    );
    let embedding_json: serde_json::Value = serde_json::from_str(&embedding_stdout).unwrap();
    assert_eq!(embedding_json[0]["file_code"], serde_json::json!(1));
    assert!(embedding_json[0]["embedding"].is_array());

    let embedding_by_ref = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--format",
        "json",
        "--cove-ai",
        path.to_str().unwrap(),
        cove_path.to_str().unwrap(),
        "# profiles: table, ai\ntable(events).embedding(vectorRef: 1)",
    ]);
    let embedding_by_ref_stdout = String::from_utf8_lossy(&embedding_by_ref.stdout);
    assert!(
        embedding_by_ref.status.success(),
        "stdout={embedding_by_ref_stdout}\nstderr={}",
        String::from_utf8_lossy(&embedding_by_ref.stderr)
    );
    let embedding_by_ref_json: serde_json::Value =
        serde_json::from_str(&embedding_by_ref_stdout).unwrap();
    assert_eq!(
        embedding_by_ref_json[0]["target_kind"],
        serde_json::json!("vector_ref")
    );

    let auto_sidecar_path = cove_path.with_extension("covev");
    fs::copy(&path, &auto_sidecar_path).unwrap();
    let auto_embedding = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--format",
        "json",
        cove_path.to_str().unwrap(),
        "# profiles: table, ai\ntable(events).embedding(fileCode: 1)",
    ]);
    let auto_embedding_stdout = String::from_utf8_lossy(&auto_embedding.stdout);
    assert!(
        auto_embedding.status.success(),
        "stdout={auto_embedding_stdout}\nstderr={}",
        String::from_utf8_lossy(&auto_embedding.stderr)
    );
    let auto_embedding_json: serde_json::Value =
        serde_json::from_str(&auto_embedding_stdout).unwrap();
    assert_eq!(auto_embedding_json[0]["file_code"], serde_json::json!(1));

    let similar = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--format",
        "json",
        "--cove-ai",
        path.to_str().unwrap(),
        cove_path.to_str().unwrap(),
        "# profiles: table, ai\ntable(events).similar(fileCode: 1, k: 2)",
    ]);
    let similar_stdout = String::from_utf8_lossy(&similar.stdout);
    assert!(
        similar.status.success(),
        "stdout={similar_stdout}\nstderr={}",
        String::from_utf8_lossy(&similar.stderr)
    );
    let similar_json: serde_json::Value = serde_json::from_str(&similar_stdout).unwrap();
    assert_eq!(similar_json[0]["query_file_code"], serde_json::json!(1));
    assert_eq!(similar_json[0]["rank"], serde_json::json!(1));

    let similar_hnsw = run_cove(&[
        "query",
        "--engine",
        "physical",
        "--format",
        "json",
        "--cove-ai",
        path.to_str().unwrap(),
        cove_path.to_str().unwrap(),
        "# profiles: table, ai\ntable(events).similar(fileCode: 1, k: 2, target: \"file_code\", index: \"hnsw\")",
    ]);
    let similar_hnsw_stdout = String::from_utf8_lossy(&similar_hnsw.stdout);
    assert!(
        similar_hnsw.status.success(),
        "stdout={similar_hnsw_stdout}\nstderr={}",
        String::from_utf8_lossy(&similar_hnsw.stderr)
    );
    let similar_hnsw_json: serde_json::Value = serde_json::from_str(&similar_hnsw_stdout).unwrap();
    assert_eq!(
        similar_hnsw_json[0]["requested_index"],
        serde_json::json!("hnsw")
    );
    assert_eq!(
        similar_hnsw_json[0]["fallback_used"],
        serde_json::json!(false)
    );
    assert_eq!(similar_hnsw_json[0]["exact"], serde_json::json!(false));
    assert_eq!(
        similar_hnsw_json[0]["result_authority"],
        serde_json::json!("ApproximateInternalAnn")
    );

    for method in ["hybrid", "rerank"] {
        let output = run_cove(&[
            "query",
            "--engine",
            "physical",
            "--format",
            "json",
            "--cove-ai",
            path.to_str().unwrap(),
            cove_path.to_str().unwrap(),
            &format!("# profiles: table, ai\ntable(events).{method}(fileCode: 1, k: 2)"),
        ]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "method={method} stdout={stdout}\nstderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(value[0]["method"], serde_json::json!(method));
        assert_eq!(
            value[0]["result_authority"],
            serde_json::json!("RuntimeAdvisory")
        );
    }

    let export = run_cove(&[
        "ai",
        "export",
        "vectors",
        path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let export_stdout = String::from_utf8_lossy(&export.stdout);
    assert!(
        export.status.success(),
        "stdout={export_stdout}\nstderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    let export_json: serde_json::Value = serde_json::from_str(&export_stdout).unwrap();
    assert_eq!(export_json["kind"], serde_json::json!("vectors"));
    assert!(export_json["records"].as_array().unwrap().len() >= 2);

    let arrow_path = temp_file("vectors-ai-export.arrow");
    let export_arrow = run_cove(&[
        "ai",
        "export",
        "vectors",
        path.to_str().unwrap(),
        "--format",
        "arrow",
        "--out",
        arrow_path.to_str().unwrap(),
    ]);
    assert!(
        export_arrow.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&export_arrow.stdout),
        String::from_utf8_lossy(&export_arrow.stderr)
    );
    assert!(
        fs::read(&arrow_path).unwrap().starts_with(b"ARROW1"),
        "AI Arrow export did not write an Arrow IPC file"
    );

    let parquet_path = temp_file("vectors-ai-export.parquet");
    let export_parquet = run_cove(&[
        "ai",
        "export",
        "vectors",
        path.to_str().unwrap(),
        "--format",
        "parquet",
        "--out",
        parquet_path.to_str().unwrap(),
    ]);
    assert!(
        export_parquet.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&export_parquet.stdout),
        String::from_utf8_lossy(&export_parquet.stderr)
    );
    assert!(
        fs::read(&parquet_path).unwrap().starts_with(b"PAR1"),
        "AI Parquet export did not write a Parquet file"
    );

    let webdataset_path = temp_file("vectors-ai-export.tar");
    let export_webdataset = run_cove(&[
        "ai",
        "export",
        "vectors",
        path.to_str().unwrap(),
        "--format",
        "webdataset",
        "--out",
        webdataset_path.to_str().unwrap(),
    ]);
    assert!(
        export_webdataset.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&export_webdataset.stdout),
        String::from_utf8_lossy(&export_webdataset.stderr)
    );
    let webdataset_bytes = fs::read(&webdataset_path).unwrap();
    assert_eq!(
        &webdataset_bytes[257..263],
        b"ustar\0",
        "AI WebDataset export did not write a ustar shard"
    );
}

#[test]
fn query_coveql_ai_descriptor_methods_smoke() {
    let sidecar_path = temp_file("descriptor-methods.coveai");
    let cove_path = temp_file("events-ai-descriptor-query.cove");
    fs::write(&sidecar_path, coveql_ai_descriptor_sidecar_bytes()).unwrap();
    fs::write(&cove_path, cove_t_events_bytes()).unwrap();

    for (query, expected_kind) in [
        (
            "# profiles: table, ai\ntable(events).chunks()",
            "text_chunk",
        ),
        (
            "# profiles: table, ai\ntable(events).tokens()",
            "token_block",
        ),
        (
            "# profiles: table, ai\ntable(events).context()",
            "rag_context_chunk",
        ),
        (
            "# profiles: table, ai\ntable(events).asPromptContext()",
            "rag_context_chunk",
        ),
        (
            "# profiles: table, ai\ntable(events).trainingSamples()",
            "training_sample",
        ),
        (
            "# profiles: table, ai\ntable(events).trainingSamples().split()",
            "dataset_split",
        ),
        (
            "# profiles: table, ai\ntable(events).trainingSamples().split().pack()",
            "token_sequence_pack",
        ),
        (
            "# profiles: table, ai\ntable(events).multimodal()",
            "multimodal_sequence_pack",
        ),
        (
            "# profiles: table, ai\ntable(events).generatorAudit()",
            "model_actor",
        ),
    ] {
        let output = run_cove(&[
            "query",
            "--engine",
            "physical",
            "--format",
            "json",
            "--cove-ai",
            sidecar_path.to_str().unwrap(),
            cove_path.to_str().unwrap(),
            query,
        ]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "query={query} stdout={stdout}\nstderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        let rows = value.as_array().expect("CoveQL-AI query returns JSON rows");
        assert!(
            rows.iter()
                .any(|row| row["record_kind"] == serde_json::json!(expected_kind)),
            "query={query} expected_kind={expected_kind} stdout={stdout}"
        );
    }

    let train_export = run_cove(&[
        "train",
        "export",
        sidecar_path.to_str().unwrap(),
        "--include-payloads",
        "--format",
        "jsonl",
    ]);
    let train_export_stdout = String::from_utf8_lossy(&train_export.stdout);
    assert!(
        train_export.status.success(),
        "stdout={train_export_stdout}\nstderr={}",
        String::from_utf8_lossy(&train_export.stderr)
    );
    assert!(
        train_export_stdout.contains("\"sample_id\":1"),
        "stdout={train_export_stdout}"
    );

    let train_arrow_path = temp_file("training-export.arrow");
    let train_export_arrow = run_cove(&[
        "train",
        "export",
        sidecar_path.to_str().unwrap(),
        "--format",
        "arrow",
        "--out",
        train_arrow_path.to_str().unwrap(),
    ]);
    assert!(
        train_export_arrow.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&train_export_arrow.stdout),
        String::from_utf8_lossy(&train_export_arrow.stderr)
    );
    assert!(
        fs::read(&train_arrow_path).unwrap().starts_with(b"ARROW1"),
        "COVE-TRAIN Arrow export did not write an Arrow IPC file"
    );

    let train_parquet_path = temp_file("training-export.parquet");
    let train_export_parquet = run_cove(&[
        "train",
        "export",
        sidecar_path.to_str().unwrap(),
        "--format",
        "parquet",
        "--out",
        train_parquet_path.to_str().unwrap(),
    ]);
    assert!(
        train_export_parquet.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&train_export_parquet.stdout),
        String::from_utf8_lossy(&train_export_parquet.stderr)
    );
    assert!(
        fs::read(&train_parquet_path).unwrap().starts_with(b"PAR1"),
        "COVE-TRAIN Parquet export did not write a Parquet file"
    );

    let train_webdataset_path = temp_file("training-export.tar");
    let train_export_webdataset = run_cove(&[
        "train",
        "export",
        sidecar_path.to_str().unwrap(),
        "--format",
        "webdataset",
        "--out",
        train_webdataset_path.to_str().unwrap(),
    ]);
    assert!(
        train_export_webdataset.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&train_export_webdataset.stdout),
        String::from_utf8_lossy(&train_export_webdataset.stderr)
    );
    let train_webdataset_bytes = fs::read(&train_webdataset_path).unwrap();
    assert_eq!(
        &train_webdataset_bytes[257..263],
        b"ustar\0",
        "COVE-TRAIN WebDataset export did not write a ustar shard"
    );

    let ai_export = run_cove(&[
        "ai",
        "export",
        "tokens",
        sidecar_path.to_str().unwrap(),
        "--include-payloads",
        "--format",
        "jsonl",
    ]);
    let ai_export_stdout = String::from_utf8_lossy(&ai_export.stdout);
    assert!(
        ai_export.status.success(),
        "stdout={ai_export_stdout}\nstderr={}",
        String::from_utf8_lossy(&ai_export.stderr)
    );
    assert!(
        ai_export_stdout.contains("\"record_kind\":\"token_block\""),
        "stdout={ai_export_stdout}"
    );
}

#[test]
fn ai_training_archive_adoption_workflows_smoke() {
    let source = temp_file("training-adoption-source.jsonl");
    let archive = temp_file("training-adoption.coveai");
    let mapped_source = temp_file("training-adoption-mapped-source.jsonl");
    let mapped_archive = temp_file("training-adoption-mapped.coveai");
    let mapping_file = temp_file("training-adoption-mapping.json");
    let stream_out = temp_file("training-adoption-stream.jsonl");
    let diff_report = temp_file("training-adoption-diff.json");
    let showcase_dir = temp_file("ai-training-showcase");
    fs::write(
        &source,
        serde_json::json!({
            "sample_id": "adopt-1",
            "instruction": "Explain COVE-AI training archives.",
            "input": "policy aware import",
            "output": "COVE-AI is the archive of record.",
            "split": "train"
        })
        .to_string()
            + "\n",
    )
    .unwrap();

    let dry_run = run_cove(&[
        "ai",
        "import",
        "jsonl",
        source.to_str().unwrap(),
        "--schema",
        "instruction",
        "--split-column",
        "split",
        "--dry-run",
    ]);
    let dry_run_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_run.status.success(),
        "stdout={dry_run_stdout}\nstderr={}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_run_json: serde_json::Value = serde_json::from_str(&dry_run_stdout).unwrap();
    assert_eq!(dry_run_json["sample_count"], serde_json::json!(1));

    let import = run_cove(&[
        "ai",
        "import",
        "jsonl",
        source.to_str().unwrap(),
        "--out",
        archive.to_str().unwrap(),
        "--schema",
        "instruction",
        "--split-column",
        "split",
        "--publish-covm",
    ]);
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(
        import.status.success(),
        "stdout={import_stdout}\nstderr={}",
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(archive.exists());
    assert!(archive.with_extension("covm").exists());

    let verify = run_cove(&["ai", "verify", archive.to_str().unwrap(), "--json"]);
    let verify_stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(
        verify.status.success(),
        "stdout={verify_stdout}\nstderr={}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let verify_json: serde_json::Value = serde_json::from_str(&verify_stdout).unwrap();
    assert_eq!(verify_json["training_sample_count"], serde_json::json!(1));

    fs::write(
        &mapped_source,
        [
            serde_json::json!({
                "sample_id": "mapped-1",
                "instruction": "Explain mapped COVE-AI training archives.",
                "output": "Mapped imports preserve richer training metadata.",
                "split": "train",
                "dedup": "mapped-source-1",
                "labels": [{"label": "accepted", "confidence": 0.9}],
                "generator": {"provider": "local", "model": "fixture-model", "version": "1"},
                "review": {"role": "reviewer", "rating": 1.0}
            })
            .to_string(),
            serde_json::json!({
                "sample_id": "mapped-2",
                "instruction": "Explain strict COVE-AI verification.",
                "output": "Strict verification requires replay metadata.",
                "split": "validation",
                "dedup": "mapped-source-2",
                "generator": {"provider": "local", "model": "fixture-model", "version": "1"}
            })
            .to_string(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    fs::write(
        &mapping_file,
        serde_json::json!({
            "split_field": "split",
            "dedup_key_field": "dedup",
            "labels_field": "labels",
            "generator_field": "generator",
            "human_review_field": "review",
            "epoch_plan": {"enabled": true, "seed": 42}
        })
        .to_string(),
    )
    .unwrap();
    let mapped_import = run_cove(&[
        "ai",
        "import",
        "jsonl",
        mapped_source.to_str().unwrap(),
        "--out",
        mapped_archive.to_str().unwrap(),
        "--schema",
        "instruction",
        "--mapping",
        mapping_file.to_str().unwrap(),
    ]);
    assert!(
        mapped_import.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&mapped_import.stdout),
        String::from_utf8_lossy(&mapped_import.stderr)
    );
    let mapped_verify = run_cove(&[
        "ai",
        "verify",
        mapped_archive.to_str().unwrap(),
        "--json",
        "--policy-report",
        "--strict-training",
    ]);
    let mapped_verify_stdout = String::from_utf8_lossy(&mapped_verify.stdout);
    assert!(
        mapped_verify.status.success(),
        "stdout={mapped_verify_stdout}\nstderr={}",
        String::from_utf8_lossy(&mapped_verify.stderr)
    );
    let mapped_verify_json: serde_json::Value =
        serde_json::from_str(&mapped_verify_stdout).unwrap();
    assert_eq!(
        mapped_verify_json["replayability"],
        serde_json::json!("replayable")
    );
    assert_eq!(
        mapped_verify_json["training_label_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        mapped_verify_json["generator_provenance_count"],
        serde_json::json!(2)
    );
    let mapped_train_export = run_cove(&[
        "train",
        "export",
        mapped_archive.to_str().unwrap(),
        "--strict-training",
        "--format",
        "json",
    ]);
    assert!(
        mapped_train_export.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&mapped_train_export.stdout),
        String::from_utf8_lossy(&mapped_train_export.stderr)
    );

    let covm_archive = archive.with_extension("covm");
    let verify_covm = run_cove(&[
        "ai",
        "verify",
        covm_archive.to_str().unwrap(),
        "--dataset",
        archive.parent().unwrap().to_str().unwrap(),
        "--json",
    ]);
    let verify_covm_stdout = String::from_utf8_lossy(&verify_covm.stdout);
    assert!(
        verify_covm.status.success(),
        "stdout={verify_covm_stdout}\nstderr={}",
        String::from_utf8_lossy(&verify_covm.stderr)
    );
    let verify_covm_json: serde_json::Value = serde_json::from_str(&verify_covm_stdout).unwrap();
    assert_eq!(
        verify_covm_json["training_sample_count"],
        serde_json::json!(1)
    );
    fs::write(
        &source,
        fs::read_to_string(&source).unwrap()
            + &serde_json::json!({
                "sample_id": "adopt-2",
                "instruction": "Mutate source",
                "output": "stale",
                "split": "train"
            })
            .to_string()
            + "\n",
    )
    .unwrap();
    let stale_covm = run_cove(&[
        "ai",
        "verify",
        covm_archive.to_str().unwrap(),
        "--dataset",
        archive.parent().unwrap().to_str().unwrap(),
        "--json",
    ]);
    assert!(
        !stale_covm.status.success(),
        "stale COVM AI source binding should fail verification"
    );

    let stream = run_cove(&[
        "ai",
        "stream",
        archive.to_str().unwrap(),
        "--format",
        "hf-jsonl",
        "--split",
        "train",
        "--include-payloads",
        "--out",
        stream_out.to_str().unwrap(),
    ]);
    assert!(
        stream.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&stream.stdout),
        String::from_utf8_lossy(&stream.stderr)
    );
    assert!(
        fs::read_to_string(&stream_out)
            .unwrap()
            .contains("\"payload_access\":\"allowed\""),
        "streamed archive should include policy-gated payloads"
    );

    let diff = run_cove(&[
        "ai",
        "diff",
        archive.to_str().unwrap(),
        archive.to_str().unwrap(),
        "--keys",
        "sample_id",
        "--report",
        diff_report.to_str().unwrap(),
    ]);
    assert!(
        diff.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&diff.stdout),
        String::from_utf8_lossy(&diff.stderr)
    );
    let diff_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&diff_report).unwrap()).unwrap();
    assert!(diff_json["added"].as_array().unwrap().is_empty());
    assert!(diff_json["removed"].as_array().unwrap().is_empty());
    assert!(diff_json["changed"].as_array().unwrap().is_empty());

    let showcase = run_cove(&[
        "showcase",
        "ai-training",
        "--profile",
        "quick",
        "--out",
        showcase_dir.to_str().unwrap(),
        "--force",
        "--json",
    ]);
    let showcase_stdout = String::from_utf8_lossy(&showcase.stdout);
    assert!(
        showcase.status.success(),
        "stdout={showcase_stdout}\nstderr={}",
        String::from_utf8_lossy(&showcase.stderr)
    );
    assert!(showcase_dir.join("training.coveai").exists());
    assert!(showcase_dir.join("training.hf.jsonl").exists());
    assert!(showcase_dir.join("training.parquet").exists());
    assert!(showcase_dir.join("training.tar").exists());
}

#[test]
fn inspect_ai_reports_covemap_ai_policy() {
    let path = temp_file("policy.covemap");
    fs::write(&path, covemap_ai_policy_bytes()).unwrap();

    let output = run_cove(&["inspect", "--ai", path.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout={stdout}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("COVE-MAP-AI:"), "stdout={stdout}");
    assert!(stdout.contains("active_profiles: 1"), "stdout={stdout}");
    assert!(stdout.contains("customer_ai_v1"), "stdout={stdout}");
    assert!(stdout.contains("forbidden_slots: 1"), "stdout={stdout}");
    assert!(stdout.contains("Customer.private_note"), "stdout={stdout}");
}

#[test]
fn query_cove_t_outputs_jsonl_and_honors_take() {
    let file = temp_file("events-query.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "jsonl",
        "--take",
        "2",
        "table(events).select(id, score)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(r#""id":1"#));
    assert!(lines[1].contains(r#""score":20"#));
}

#[test]
fn query_cove_t_outputs_table_json_and_csv() {
    let file = temp_file("events-formats.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let table = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        table.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&table.stderr)
    );
    let table_stdout = String::from_utf8_lossy(&table.stdout);
    assert!(table_stdout.contains("| id | score |"));
    assert!(table_stdout.contains("2 rows"));

    let json = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "json",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json_value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(json_value[0]["score"], serde_json::json!(20));

    let csv = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--format",
        "csv",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        csv.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&csv.stderr)
    );
    let csv_stdout = String::from_utf8_lossy(&csv.stdout);
    assert!(csv_stdout.starts_with("id,score\n"));
    assert!(csv_stdout.contains("2,20\n"));
}

#[test]
fn query_cove_t_explain_prints_coveql_explain() {
    let file = temp_file("events-explain.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        file.to_str().unwrap(),
        "--explain",
        "coded",
        "table(events).select(id)",
    ]);

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("schema_version: 0.1"), "stdout={stdout}");
    assert!(stdout.contains("mode: coded"), "stdout={stdout}");
    assert!(
        stdout.contains("operation: explain_table"),
        "stdout={stdout}"
    );
}

#[test]
fn optimize_generates_acceleration_manifest_and_auto_query_uses_it() {
    let file = temp_file("events-optimized.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let optimize = run_cove(&["optimize", file.to_str().unwrap()]);
    assert!(
        optimize.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&optimize.stderr)
    );
    let stdout = String::from_utf8_lossy(&optimize.stdout);
    assert!(stdout.contains("Generated sidecars"), "stdout={stdout}");

    let base = file.with_file_name("events-optimized");
    assert!(base.with_extension("covperf.json").exists());
    assert!(base.with_extension("covi").exists());
    assert!(base.with_extension("covx").exists());
    assert!(file.with_file_name("events-optimized.splits.bin").exists());
    assert!(file.with_file_name("events-optimized.layout.bin").exists());

    let inspect = run_cove(&["inspect", "--performance", file.to_str().unwrap()]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        inspect_stdout.contains("Performance:"),
        "stdout={inspect_stdout}"
    );
    assert!(inspect_stdout.contains("COVE-I"), "stdout={inspect_stdout}");

    let query = run_cove(&[
        "query",
        "--perf-report",
        "--strict-performance",
        "--format",
        "jsonl",
        file.to_str().unwrap(),
        "table(events).where(score >= 20).select(id, score).orderBy(id)",
    ]);
    assert!(
        query.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query_stdout = String::from_utf8_lossy(&query.stdout);
    assert!(query_stdout.contains("\"id\":2"), "stdout={query_stdout}");
    let query_stderr = String::from_utf8_lossy(&query.stderr);
    assert!(
        query_stderr.contains("Performance report"),
        "stderr={query_stderr}"
    );
    assert!(
        query_stderr.contains("usable sidecars"),
        "stderr={query_stderr}"
    );
}

#[test]
fn strict_performance_rejects_missing_acceleration_sidecars() {
    let file = temp_file("events-strict-missing.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let output = run_cove(&[
        "query",
        "--strict-performance",
        file.to_str().unwrap(),
        "table(events).select(id).take(1)",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("strict performance requested"),
        "stderr={stderr}"
    );
}

#[test]
fn query_covemap_sidecar_reports_guidance() {
    let file = temp_file("mapping.covemap");
    fs::write(&file, covemap_bytes()).unwrap();

    let output = run_cove(&["query", file.to_str().unwrap(), "table(events).take(1)"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("COVE-MAP mapping artifact"));
    assert!(stderr.contains("Use it with"));
}

#[test]
fn map_build_creates_adoption_bundle() {
    let out_dir = temp_file("map-build-bundle");
    let mapping = sample_path("people.covemap");
    let source = sample_path("people.jsonl");
    let output = run_cove(&[
        "map",
        "build",
        "--out-dir",
        out_dir.to_str().unwrap(),
        "--verify",
        "--force",
        "--json",
        mapping.to_str().unwrap(),
        source.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        manifest["format"],
        serde_json::json!("cove-map-build-manifest-v1")
    );
    let object = manifest["artifacts"]["object"]["path"].as_str().unwrap();
    assert!(out_dir.join(object).exists());
    let index = manifest["artifacts"]["indexes"][0]["path"]
        .as_str()
        .unwrap();
    assert!(out_dir.join(index).exists());
    assert!(out_dir.join("map-build-report.json").exists());
    assert!(out_dir.join("map-build-manifest.json").exists());
    assert!(out_dir.join("README.md").exists());

    let inspect_index = run_cove(&[
        "sidecar",
        "inspect",
        "index",
        out_dir.join(index).to_str().unwrap(),
    ]);
    assert!(
        inspect_index.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect_index.stderr)
    );
    assert!(String::from_utf8_lossy(&inspect_index.stdout).contains("COVE-I"));

    let doctor = run_cove(&[
        "map",
        "doctor",
        "--bundle-dir",
        out_dir.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(
        doctor_report["format"],
        serde_json::json!("cove-map-doctor-report-v1")
    );
    assert_eq!(doctor_report["status"], serde_json::json!("ok"));

    let suggestions = run_cove(&["map", "suggest", "--json", source.to_str().unwrap()]);
    assert!(
        suggestions.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&suggestions.stderr)
    );
    let suggestions: serde_json::Value = serde_json::from_slice(&suggestions.stdout).unwrap();
    assert_eq!(
        suggestions["format"],
        serde_json::json!("cove-map-suggestions-v1")
    );
    assert_eq!(suggestions["non_authoritative"], serde_json::json!(true));
}

#[test]
fn examples_and_doctor_commands_are_beginner_friendly() {
    let examples = run_cove(&["examples"]);
    assert!(
        examples.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&examples.stderr)
    );
    let examples_stdout = String::from_utf8_lossy(&examples.stdout);
    assert!(examples_stdout.contains("CoveQL examples"));
    assert!(examples_stdout.contains("cove inspect --queries --performance"));
    assert!(examples_stdout.contains("cove query examples/coveql/events.cove"));

    let examples_json = run_cove(&["examples", "--json"]);
    assert!(
        examples_json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&examples_json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&examples_json.stdout).unwrap();
    assert_eq!(value["sample_dir"], serde_json::json!("examples/coveql"));
    assert!(value["examples"].as_array().unwrap().len() >= 4);

    let file = temp_file("doctor events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let doctor = run_cove(&["doctor", file.to_str().unwrap()]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(doctor_stdout.contains("Doctor:"));
    assert!(doctor_stdout.contains("Queryable: yes"));
    assert!(doctor_stdout.contains("cove optimize"));
    assert!(doctor_stdout.contains("table(events).select(id, score).take(10)"));

    let doctor_json = run_cove(&["doctor", "--json", file.to_str().unwrap()]);
    assert!(
        doctor_json.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor_json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&doctor_json.stdout).unwrap();
    assert_eq!(value["queryable"], serde_json::json!(true));
    assert!(value["findings"].as_array().unwrap().iter().any(|finding| {
        finding
            .as_str()
            .unwrap_or_default()
            .contains("artifact exposes queryable rows")
    }));
}

#[test]
fn cli_outputs_have_stable_golden_shapes() {
    let file = temp_file("golden-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let table = run_cove(&[
        "query",
        file,
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        table.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&table.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&table.stdout),
        "+----+-------+\n| id | score |\n+----+-------+\n| 2  | 20    |\n| 3  | 30    |\n+----+-------+\n2 rows\n"
    );

    let csv = run_cove(&[
        "query",
        file,
        "--format",
        "csv",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        csv.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&csv.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&csv.stdout),
        "id,score\n2,20\n3,30\n"
    );

    let jsonl = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(events).where(score >= 20).select(id, score)",
    ]);
    assert!(
        jsonl.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&jsonl.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&jsonl.stdout),
        "{\"id\":2,\"score\":20}\n{\"id\":3,\"score\":30}\n"
    );
}

#[test]
fn cli_negative_paths_report_actionable_errors() {
    let missing = run_cove(&["inspect", "/tmp/cove-cli-definitely-missing-file.cove"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));

    let mixed_inspect = run_cove(&["inspect", "--queries", "--sections", "stats", "file.cove"]);
    assert!(!mixed_inspect.status.success());
    let stderr = String::from_utf8_lossy(&mixed_inspect.stderr);
    assert!(
        stderr.contains("cannot be combined") && stderr.contains("--sections"),
        "stderr={stderr}"
    );

    let bad_flag = run_cove(&["query", "--wat", "table(events).take(1)"]);
    assert!(!bad_flag.status.success());
    assert!(String::from_utf8_lossy(&bad_flag.stderr).contains("unknown query option"));

    let bad_format = run_cove(&[
        "query",
        "--external-table",
        "people=/tmp/nope.jsonl",
        "--format",
        "xml",
        "table(people).take(1)",
    ]);
    assert!(!bad_format.status.success());
    assert!(String::from_utf8_lossy(&bad_format.stderr).contains("unsupported --format"));

    let file = temp_file("negative-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let malformed = run_cove(&["query", file.to_str().unwrap(), "table(events).where("]);
    assert!(!malformed.status.success());
    let stderr = String::from_utf8_lossy(&malformed.stderr);
    assert!(stderr.contains("E_PARSE"), "stderr={stderr}");
    assert!(
        stderr.contains("Check the CoveQL syntax"),
        "stderr={stderr}"
    );

    let people = sample_path("people.cove");
    let unknown_table = run_cove(&[
        "query",
        people.to_str().unwrap(),
        "table(missing).select(id).take(1)",
    ]);
    assert!(!unknown_table.status.success());
    let stderr = String::from_utf8_lossy(&unknown_table.stderr);
    assert!(
        stderr.contains("E_UNKNOWN_TABLE_SURFACE"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("cove inspect --queries"), "stderr={stderr}");

    let bad_jsonl = temp_file("bad-external.jsonl");
    fs::write(&bad_jsonl, "{\"id\":1}\nnot-json\n").unwrap();
    let external_arg = format!("bad={}", bad_jsonl.display());
    let bad_external = run_cove(&[
        "query",
        "--external-table",
        &external_arg,
        "table(bad).select(id)",
    ]);
    assert!(!bad_external.status.success());
    assert!(String::from_utf8_lossy(&bad_external.stderr).contains("cannot parse JSONL"));
}

#[test]
fn parent_command_help_exits_successfully() {
    for args in [
        &["convert", "--help"][..],
        &["convert"][..],
        &["export", "--help"][..],
        &["export"][..],
        &["perf", "--help"][..],
        &["perf"][..],
        &["digest", "--help"][..],
        &["digest"][..],
    ] {
        let output = run_cove(args);
        assert!(
            output.status.success(),
            "args={args:?} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"), "args={args:?} stdout={stdout}");
    }
}

#[test]
fn command_specific_help_surfaces_reference_workflows() {
    let cases = [
        (
            &["query", "--help"][..],
            "Authority model:",
            "sidecar-backed execution",
        ),
        (
            &["inspect", "--help"][..],
            "Beginner inspect",
            "--performance",
        ),
        (
            &["optimize", "--help"][..],
            "Writes a sibling",
            "materialized readback",
        ),
        (
            &["sidecar", "--help"][..],
            "cove sidecar build covi",
            "--object-properties",
        ),
        (
            &["convert", "--help"][..],
            "cove convert parquet",
            "cove-to-source",
        ),
        (&["map", "--help"][..], "cove map build", "COVE-I"),
    ];
    for (args, expected_a, expected_b) in cases {
        let output = run_cove(args);
        assert!(
            output.status.success(),
            "args={args:?} stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage:"), "args={args:?} stdout={stdout}");
        assert!(
            stdout.contains(expected_a) && stdout.contains(expected_b),
            "args={args:?} stdout={stdout}"
        );
    }
}

#[test]
fn release_gate_help_does_not_run_gate() {
    let output = Command::new("sh")
        .arg("scripts/release-gates.sh")
        .arg("--help")
        .current_dir(workspace_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--quick"));
    assert!(!stdout.contains("cargo test"));
}

#[test]
fn paths_with_spaces_and_query_files_work() {
    let file = temp_file("events final (v1).cove");
    let query_file = temp_file("query with spaces.coveql");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    fs::write(
        &query_file,
        "table(events).where(score >= 30).select(id, score)",
    )
    .unwrap();

    let from_query_file = run_cove(&[
        "query",
        "--query-file",
        query_file.to_str().unwrap(),
        "--format",
        "jsonl",
        file.to_str().unwrap(),
    ]);
    assert!(
        from_query_file.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&from_query_file.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&from_query_file.stdout).trim(),
        r#"{"id":3,"score":30}"#
    );

    let inspect = run_cove(&["inspect", "--queries", file.to_str().unwrap()]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(stdout.contains("Try next:"));
    assert!(stdout.contains("events final (v1).cove"));
}

#[test]
fn medium_cove_t_fixture_covers_group_sort_and_windows() {
    let file = temp_file("metrics-medium.cove");
    fs::write(&file, cove_t_medium_bytes()).unwrap();
    let file = file.to_str().unwrap();

    let grouped = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(metrics).groupBy(bucket).select(bucket, rows: count(*), total: sum(score)).orderBy(bucket)",
    ]);
    assert!(
        grouped.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&grouped.stderr)
    );
    let grouped_stdout = String::from_utf8_lossy(&grouped.stdout);
    assert_eq!(grouped_stdout.lines().count(), 5);
    assert!(grouped_stdout.contains(r#""bucket":0,"rows":5"#));
    assert!(grouped_stdout.contains(r#""bucket":4,"rows":5"#));

    let window = run_cove(&[
        "query",
        file,
        "--format",
        "jsonl",
        "table(metrics).where(bucket == 2).window(partitionBy: bucket, orderBy: score).select(bucket, score, rn: row_number()).take(3)",
    ]);
    assert!(
        window.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&window.stderr)
    );
    let window_stdout = String::from_utf8_lossy(&window.stdout);
    assert!(window_stdout.contains(r#""rn":1"#));
    assert!(window_stdout.contains(r#""rn":2"#));
    assert!(window_stdout.contains(r#""rn":3"#));
}

#[test]
fn doctor_reports_sidecar_guidance_for_nonqueryable_artifacts() {
    let file = temp_file("mapping-doctor.covemap");
    fs::write(&file, covemap_bytes()).unwrap();

    let output = run_cove(&["doctor", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Queryable: no"));
    assert!(stdout.contains("COVE-MAP mapping artifact"));

    let output = run_cove(&["doctor", "--json", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["queryable"], serde_json::json!(false));
}

#[test]
fn unified_file_utility_commands_cover_validate_inspect_dump_export_and_perf() {
    let file = temp_file("unified-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();
    let path = file.to_str().unwrap();

    let validate = run_cove(&["validate", "--json", path]);
    assert!(
        validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(String::from_utf8_lossy(&validate.stdout).contains("\"ok\":true"));

    let inspect = run_cove(&["inspect", "--json", "--sections", "stats", path]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_json: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect_json["artifact"], serde_json::json!("COVE"));

    let dump = run_cove(&["dump", path, "--metadata"]);
    assert!(
        dump.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&dump.stderr)
    );
    assert!(String::from_utf8_lossy(&dump.stdout).contains("metadata"));

    let exported = temp_file("events-export.json");
    let export = run_cove(&[
        "export",
        "arrow",
        "--format",
        "json",
        path,
        exported.to_str().unwrap(),
        "--report",
        "-",
    ]);
    assert!(
        export.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(exported.metadata().unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&export.stdout).contains("\"rows\""));

    let pruning = run_cove(&["perf", "explain-pruning", path]);
    assert!(
        pruning.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&pruning.stderr)
    );
    assert!(String::from_utf8_lossy(&pruning.stdout).contains("\"version\""));

    let cost = run_cove(&["perf", "plan-cost", "--execute", path]);
    assert!(
        cost.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&cost.stderr)
    );
    assert!(String::from_utf8_lossy(&cost.stdout).contains("\"version\""));
}

#[test]
fn unified_convert_and_map_commands_delegate_existing_tools() {
    let csv = temp_file("unified-source.csv");
    let cove = temp_file("unified-source.cove");
    fs::write(&csv, "id,name\n1,Ada\n2,Linus\n").unwrap();

    let convert = run_cove(&[
        "convert",
        "csv",
        csv.to_str().unwrap(),
        cove.to_str().unwrap(),
        "--report",
        "-",
    ]);
    assert!(
        convert.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&convert.stderr)
    );
    assert!(cove.metadata().unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&convert.stdout).contains("\"source_identifier\""));

    let report = run_cove(&[
        "convert",
        "report",
        "--source-format",
        "csv",
        csv.to_str().unwrap(),
    ]);
    assert!(
        report.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&report.stderr)
    );
    assert!(String::from_utf8_lossy(&report.stdout).contains("\"source_identifier\""));

    let map = temp_file("unified-map.covemap");
    fs::write(&map, covemap_bytes()).unwrap();
    let map_validate = run_cove(&["map", "validate", map.to_str().unwrap()]);
    assert!(
        map_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&map_validate.stderr)
    );
    assert!(String::from_utf8_lossy(&map_validate.stdout).contains("\"ok\":true"));

    let map_preview = run_cove(&["map", "preview", map.to_str().unwrap()]);
    assert!(
        map_preview.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&map_preview.stderr)
    );
    assert!(String::from_utf8_lossy(&map_preview.stdout).contains("test/v1"));
}

#[test]
fn delta_cli_commands_validate_plan_and_publish_delta_chains() {
    let workspace = workspace_dir();
    let fixture = workspace.join("conformance/accept/covedelta_object_delta_valid.covedelta");

    let inspect = run_cove(&["delta", "inspect", fixture.to_str().unwrap(), "--json"]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspected: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspected["artifact"], serde_json::json!("covedelta"));
    assert_eq!(inspected["object_delta"]["valid"], serde_json::json!(true));

    let validate = run_cove(&[
        "delta",
        "validate",
        fixture.to_str().unwrap(),
        "--object-delta",
        "--json",
    ]);
    assert!(
        validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let validated: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(validated["ok"], serde_json::json!(true));

    let routed_validate = run_cove(&["validate", "--object-delta", fixture.to_str().unwrap()]);
    assert!(
        routed_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&routed_validate.stderr)
    );

    let dump_summary = run_cove(&["delta", "dump", fixture.to_str().unwrap(), "--summary"]);
    assert!(
        dump_summary.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&dump_summary.stderr)
    );
    assert!(String::from_utf8_lossy(&dump_summary.stdout).contains("temporal_segments"));

    let base = temp_file("delta-cli-base.cove");
    let delta = temp_file("delta-cli-delta.covedelta");
    let manifest = temp_file("delta-cli-chain.covm");
    let dataset_dir = base.parent().expect("temp file has parent").to_path_buf();
    let dataset_arg = dataset_dir.to_str().unwrap();
    let base_bytes = cove_t_events_bytes();
    fs::write(&base, &base_bytes).unwrap();
    let first_delta_bytes = simple_delta_bytes_for_base_with_snapshot(&base_bytes, [0xB0; 16]);
    fs::write(&delta, &first_delta_bytes).unwrap();

    let publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        base.to_str().unwrap(),
        "--delta",
        delta.to_str().unwrap(),
        "--out",
        manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        publish.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&publish.stdout),
        String::from_utf8_lossy(&publish.stderr)
    );
    let published: serde_json::Value = serde_json::from_slice(&publish.stdout).unwrap();
    assert_eq!(published["ok"], serde_json::json!(true));
    assert_eq!(published["delta_count"], serde_json::json!(1));

    let bad_parent_delta = temp_file("delta-cli-bad-parent.covedelta");
    let bad_parent_manifest = temp_file("delta-cli-bad-parent.covm");
    let mut bad_parent_file = CoveDeltaFile::parse(&first_delta_bytes).unwrap();
    bad_parent_file.parent_refs[0].artifact_id = [0xEE; 16];
    fs::write(&bad_parent_delta, bad_parent_file.serialize().unwrap()).unwrap();
    let bad_parent_publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        base.to_str().unwrap(),
        "--delta",
        bad_parent_delta.to_str().unwrap(),
        "--out",
        bad_parent_manifest.to_str().unwrap(),
    ]);
    assert!(!bad_parent_publish.status.success());
    assert!(String::from_utf8_lossy(&bad_parent_publish.stderr).contains("base artifact ID"));

    let chain_inspect = run_cove(&["delta", "chain", "inspect", manifest.to_str().unwrap()]);
    assert!(
        chain_inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&chain_inspect.stderr)
    );
    assert!(String::from_utf8_lossy(&chain_inspect.stdout).contains("delta_chain_required: true"));

    let chain_validate = run_cove(&[
        "delta",
        "chain",
        "validate",
        manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--json",
    ]);
    assert!(
        chain_validate.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&chain_validate.stdout),
        String::from_utf8_lossy(&chain_validate.stderr)
    );
    let chain_validated: serde_json::Value =
        serde_json::from_slice(&chain_validate.stdout).unwrap();
    assert_eq!(chain_validated["ok"], serde_json::json!(true));

    let plan = run_cove(&[
        "delta",
        "chain",
        "plan",
        manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--as-of-csn",
        "1",
        "--json",
    ]);
    assert!(
        plan.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&plan.stdout),
        String::from_utf8_lossy(&plan.stderr)
    );
    let planned: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(planned["selected_chain_ordinals"][0], serde_json::json!(1));

    let graph = run_cove(&[
        "delta",
        "chain",
        "graph",
        manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
    ]);
    assert!(
        graph.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&graph.stderr)
    );
    assert!(String::from_utf8_lossy(&graph.stdout).contains("->"));

    let second_delta = temp_file("delta-cli-delta-2.covedelta");
    let second_delta_bytes = simple_delta_bytes_for_delta_parent(&first_delta_bytes);
    fs::write(&second_delta, &second_delta_bytes).unwrap();

    let bad_second_delta = temp_file("delta-cli-bad-second-parent.covedelta");
    let bad_second_manifest = temp_file("delta-cli-bad-second-parent.covm");
    let mut bad_second_file = CoveDeltaFile::parse(&second_delta_bytes).unwrap();
    bad_second_file.parent_refs[0].artifact_id = [0xEF; 16];
    fs::write(&bad_second_delta, bad_second_file.serialize().unwrap()).unwrap();
    let bad_second_publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        base.to_str().unwrap(),
        "--delta",
        delta.to_str().unwrap(),
        "--delta",
        bad_second_delta.to_str().unwrap(),
        "--out",
        bad_second_manifest.to_str().unwrap(),
    ]);
    assert!(!bad_second_publish.status.success());
    assert!(
        String::from_utf8_lossy(&bad_second_publish.stderr).contains("previous delta artifact ID")
    );

    let extended_manifest = temp_file("delta-cli-chain-extended.covm");
    let extend = run_cove(&[
        "delta",
        "chain",
        "extend",
        "--manifest",
        manifest.to_str().unwrap(),
        "--delta",
        second_delta.to_str().unwrap(),
        "--out",
        extended_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        extend.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&extend.stdout),
        String::from_utf8_lossy(&extend.stderr)
    );
    let extended_validate = run_cove(&[
        "delta",
        "chain",
        "validate",
        extended_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--json",
    ]);
    assert!(
        extended_validate.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&extended_validate.stdout),
        String::from_utf8_lossy(&extended_validate.stderr)
    );

    let reconstructed = temp_file("delta-cli-reconstructed.cove");
    let reconstruct = run_cove(&[
        "delta",
        "reconstruct",
        manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        reconstructed.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        reconstruct.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&reconstruct.stdout),
        String::from_utf8_lossy(&reconstruct.stderr)
    );
    let reconstructed_json: serde_json::Value =
        serde_json::from_slice(&reconstruct.stdout).unwrap();
    assert_eq!(reconstructed_json["ok"], serde_json::json!(true));
    assert!(reconstructed.exists());

    let compacted = temp_file("delta-cli-compacted.cove");
    let compacted_covm = temp_file("delta-cli-compacted.covm");
    let compact = run_cove(&[
        "delta",
        "compact",
        manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        compacted.to_str().unwrap(),
        "--publish-covm",
        compacted_covm.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        compact.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&compact.stdout),
        String::from_utf8_lossy(&compact.stderr)
    );
    let compacted_json: serde_json::Value = serde_json::from_slice(&compact.stdout).unwrap();
    assert_eq!(compacted_json["ok"], serde_json::json!(true));
    assert!(compacted.exists());
    assert!(compacted_covm.exists());

    let object_base = temp_file("delta-cli-object-base.cove");
    let object_delta = temp_file("delta-cli-object-delta.covedelta");
    let object_manifest = temp_file("delta-cli-object-chain.covm");
    let object_bytes = cove_o_thing_bytes();
    fs::write(&object_base, &object_bytes).unwrap();
    fs::write(&object_delta, simple_delta_bytes_for_base(&object_bytes)).unwrap();
    let object_publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        object_base.to_str().unwrap(),
        "--delta",
        object_delta.to_str().unwrap(),
        "--out",
        object_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        object_publish.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&object_publish.stdout),
        String::from_utf8_lossy(&object_publish.stderr)
    );
    let object_published: serde_json::Value =
        serde_json::from_slice(&object_publish.stdout).unwrap();
    let object_chain_digest = hex_decode_exact::<32>(
        object_published["chain_digest"]
            .as_str()
            .expect("publish JSON chain_digest"),
    )
    .unwrap();

    let object_query = run_cove(&[
        "query",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--delta-plan",
        "--perf-report",
        "--format",
        "jsonl",
        "object(Thing).select(active)",
    ]);
    assert!(
        object_query.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&object_query.stdout),
        String::from_utf8_lossy(&object_query.stderr)
    );
    assert!(
        String::from_utf8_lossy(&object_query.stdout).contains(r#""active":true"#),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&object_query.stdout),
        String::from_utf8_lossy(&object_query.stderr)
    );
    assert!(
        String::from_utf8_lossy(&object_query.stderr).contains("Delta snapshot plan"),
        "stderr={}",
        String::from_utf8_lossy(&object_query.stderr)
    );
    assert!(
        String::from_utf8_lossy(&object_query.stderr)
            .contains("delta_execution=direct_object_surface"),
        "stderr={}",
        String::from_utf8_lossy(&object_query.stderr)
    );

    let object_query_export = temp_file("delta-cli-object-query-export.json");
    let object_query_export_output = run_cove(&[
        "export",
        "arrow",
        "--query",
        "object(Thing).select(active)",
        "--format",
        "json",
        "--report",
        "-",
        "--dataset",
        dataset_arg,
        "--perf-report",
        object_manifest.to_str().unwrap(),
        object_query_export.to_str().unwrap(),
    ]);
    assert!(
        object_query_export_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&object_query_export_output.stdout),
        String::from_utf8_lossy(&object_query_export_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&fs::read(&object_query_export).unwrap())
            .contains(r#""active":true"#)
    );
    assert!(
        String::from_utf8_lossy(&object_query_export_output.stdout)
            .contains(r#""delta_execution": "direct_object_surface""#),
        "stdout={}",
        String::from_utf8_lossy(&object_query_export_output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&object_query_export_output.stderr)
            .contains("delta_execution=direct_object_surface"),
        "stderr={}",
        String::from_utf8_lossy(&object_query_export_output.stderr)
    );

    let graph_base = temp_file("delta-cli-graph-base.cove");
    let graph_delta = temp_file("delta-cli-graph-delta.covedelta");
    let graph_manifest = temp_file("delta-cli-graph-chain.covm");
    let graph_base_bytes = fs::read(sample_path("people.cove")).unwrap();
    fs::write(&graph_base, &graph_base_bytes).unwrap();
    fs::write(&graph_delta, simple_delta_bytes_for_base(&graph_base_bytes)).unwrap();
    let graph_publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        graph_base.to_str().unwrap(),
        "--delta",
        graph_delta.to_str().unwrap(),
        "--out",
        graph_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        graph_publish.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&graph_publish.stdout),
        String::from_utf8_lossy(&graph_publish.stderr)
    );
    let graph_query = run_cove(&[
        "query",
        graph_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--perf-report",
        "--format",
        "jsonl",
        "node(Person) as p.connectedComponents().degree(kind: total).select(id: p.goid, component_id, degree).take(2)",
    ]);
    assert!(
        graph_query.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&graph_query.stdout),
        String::from_utf8_lossy(&graph_query.stderr)
    );
    let graph_stdout = String::from_utf8_lossy(&graph_query.stdout);
    assert!(graph_stdout.contains(r#""component_id":0"#));
    assert!(graph_stdout.contains(r#""degree":0"#));
    assert!(
        String::from_utf8_lossy(&graph_query.stderr)
            .contains("delta_execution=direct_object_surface"),
        "stderr={}",
        String::from_utf8_lossy(&graph_query.stderr)
    );
    let graph_table_export = temp_file("delta-cli-graph-table-export.json");
    let graph_table_export_output = run_cove(&[
        "export",
        "arrow",
        "--format",
        "json",
        "--report",
        "-",
        "--dataset",
        dataset_arg,
        "--columns",
        "score,status",
        "--filter",
        "column=score,op=gte,value=20",
        graph_manifest.to_str().unwrap(),
        graph_table_export.to_str().unwrap(),
    ]);
    assert!(
        graph_table_export_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&graph_table_export_output.stdout),
        String::from_utf8_lossy(&graph_table_export_output.stderr)
    );
    let graph_table_export_rows = String::from_utf8_lossy(&fs::read(&graph_table_export).unwrap())
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(
        graph_table_export_rows
            .iter()
            .any(|row| row.contains(r#""score":20"#)),
        "rows={graph_table_export_rows:?}"
    );
    assert!(
        graph_table_export_rows
            .iter()
            .any(|row| row.contains(r#""score":30"#)),
        "rows={graph_table_export_rows:?}"
    );
    assert!(
        String::from_utf8_lossy(&graph_table_export_output.stdout)
            .contains(r#""delta_execution": "direct_projection_surface""#),
        "stdout={}",
        String::from_utf8_lossy(&graph_table_export_output.stdout)
    );

    let source_publish_query = run_cove(&[
        "query",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--source-publish-range",
        "1:2",
        "object(Thing).select(active)",
    ]);
    assert!(!source_publish_query.status.success());
    assert!(
        String::from_utf8_lossy(&source_publish_query.stderr).contains("source-publish"),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&source_publish_query.stdout),
        String::from_utf8_lossy(&source_publish_query.stderr)
    );
    let source_publish_export = temp_file("delta-cli-source-publish-export.json");
    let source_publish_export_output = run_cove(&[
        "export",
        "arrow",
        "--format",
        "json",
        object_manifest.to_str().unwrap(),
        source_publish_export.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--source-publish-range",
        "1:2",
    ]);
    assert!(!source_publish_export_output.status.success());
    assert!(
        String::from_utf8_lossy(&source_publish_export_output.stderr).contains("source-publish"),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&source_publish_export_output.stdout),
        String::from_utf8_lossy(&source_publish_export_output.stderr)
    );
    assert!(!source_publish_export.exists());

    let source_publish_base = temp_file("delta-cli-source-publish-base.cove");
    let source_publish_delta = temp_file("delta-cli-source-publish-delta.covedelta");
    let source_publish_manifest = temp_file("delta-cli-source-publish-chain.covm");
    fs::write(&source_publish_base, &object_bytes).unwrap();
    fs::write(
        &source_publish_delta,
        simple_delta_bytes_for_base_with_source_publish(&object_bytes, 10, 20),
    )
    .unwrap();
    let source_publish_publish = run_cove(&[
        "delta",
        "publish",
        "--base",
        source_publish_base.to_str().unwrap(),
        "--delta",
        source_publish_delta.to_str().unwrap(),
        "--out",
        source_publish_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        source_publish_publish.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&source_publish_publish.stdout),
        String::from_utf8_lossy(&source_publish_publish.stderr)
    );
    let source_publish_scoped_query = run_cove(&[
        "query",
        source_publish_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--source-publish-range",
        "12:18",
        "--perf-report",
        "--format",
        "jsonl",
        "object(Thing).select(active)",
    ]);
    assert!(
        source_publish_scoped_query.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&source_publish_scoped_query.stdout),
        String::from_utf8_lossy(&source_publish_scoped_query.stderr)
    );
    assert!(
        String::from_utf8_lossy(&source_publish_scoped_query.stdout).contains(r#""active":true"#)
    );
    assert!(
        String::from_utf8_lossy(&source_publish_scoped_query.stderr)
            .contains("delta_execution=direct_object_surface"),
        "stderr={}",
        String::from_utf8_lossy(&source_publish_scoped_query.stderr)
    );
    let source_publish_query_export = temp_file("delta-cli-source-publish-query-export.json");
    let source_publish_query_export_output = run_cove(&[
        "export",
        "arrow",
        "--query",
        "object(Thing).select(active)",
        "--format",
        "json",
        "--report",
        "-",
        "--dataset",
        dataset_arg,
        "--source-publish-range",
        "12:18",
        source_publish_manifest.to_str().unwrap(),
        source_publish_query_export.to_str().unwrap(),
    ]);
    assert!(
        source_publish_query_export_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&source_publish_query_export_output.stdout),
        String::from_utf8_lossy(&source_publish_query_export_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&fs::read(&source_publish_query_export).unwrap())
            .contains(r#""active":true"#)
    );
    assert!(
        String::from_utf8_lossy(&source_publish_query_export_output.stdout)
            .contains(r#""delta_execution": "direct_object_surface""#),
        "stdout={}",
        String::from_utf8_lossy(&source_publish_query_export_output.stdout)
    );

    let snapshot_covi = temp_file("delta-cli-snapshot.covi");
    let snapshot_covi_build = run_cove(&[
        "sidecar",
        "build",
        "covi",
        "--snapshot",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        snapshot_covi.to_str().unwrap(),
        "--object-properties",
    ]);
    assert!(
        snapshot_covi_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&snapshot_covi_build.stdout),
        String::from_utf8_lossy(&snapshot_covi_build.stderr)
    );
    let snapshot_covi_inspect = run_cove(&[
        "sidecar",
        "inspect",
        "index",
        snapshot_covi.to_str().unwrap(),
    ]);
    assert!(
        snapshot_covi_inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&snapshot_covi_inspect.stderr)
    );
    let snapshot_covi_bytes = fs::read(&snapshot_covi).unwrap();
    let snapshot_covi_artifact = cove_index::CoviArtifactV2::parse(&snapshot_covi_bytes).unwrap();
    let snapshot_validity = snapshot_covi_artifact
        .snapshot_validity
        .first()
        .expect("snapshot validity row");
    assert_eq!(
        snapshot_validity.delta_chain_digest_algorithm,
        DigestAlgorithm::Sha256 as u16
    );
    assert_eq!(snapshot_validity.delta_chain_digest_len, 32);
    let string_table = snapshot_covi_artifact
        .section_payload_from_bytes(
            &snapshot_covi_bytes,
            snapshot_covi_artifact.header.string_table_section_ref,
        )
        .unwrap();
    let digest_start = snapshot_validity.delta_chain_digest_offset as usize;
    let digest_end = digest_start + usize::from(snapshot_validity.delta_chain_digest_len);
    assert_eq!(
        &string_table[digest_start..digest_end],
        &object_chain_digest
    );
    let referenced_file = &snapshot_covi_artifact.referenced_files[0];
    let base_only_context = cove_index::execution::CoviValidationContextV2::for_file(
        referenced_file.file_id,
        referenced_file.file_len,
        referenced_file.footer_crc32c,
    );
    assert!(
        cove_index::execution::ValidatedCoviArtifactV2::parse_and_validate(
            &snapshot_covi_bytes,
            base_only_context
        )
        .is_err()
    );
    let bound_context = cove_index::execution::CoviValidationContextV2::for_file(
        referenced_file.file_id,
        referenced_file.file_len,
        referenced_file.footer_crc32c,
    )
    .with_delta_chain_digest(DigestAlgorithm::Sha256, object_chain_digest.to_vec());
    cove_index::execution::ValidatedCoviArtifactV2::parse_and_validate(
        &snapshot_covi_bytes,
        bound_context,
    )
    .unwrap();

    let snapshot_covm = temp_file("delta-cli-snapshot.covm");
    let snapshot_covm_build = run_cove(&[
        "sidecar",
        "build",
        "covm",
        "--snapshot",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        snapshot_covm.to_str().unwrap(),
    ]);
    assert!(
        snapshot_covm_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&snapshot_covm_build.stdout),
        String::from_utf8_lossy(&snapshot_covm_build.stderr)
    );
    assert!(snapshot_covm.exists());
    let expected_snapshot_identity = validate_bytes(&object_bytes).unwrap();
    let expected_snapshot_digest = compute_digest(DigestAlgorithm::Sha256, &object_bytes).unwrap();
    let snapshot_covm_file = CovmFile::parse(&fs::read(&snapshot_covm).unwrap()).unwrap();
    assert_eq!(snapshot_covm_file.files.len(), 1);
    let snapshot_covm_entry = &snapshot_covm_file.files[0];
    assert_eq!(
        snapshot_covm_entry.file_id,
        expected_snapshot_identity.header.file_id
    );
    assert_eq!(
        snapshot_covm_entry.file_len,
        expected_snapshot_identity.postscript.file_len
    );
    assert_eq!(
        snapshot_covm_entry.footer_crc32c,
        expected_snapshot_identity.postscript.footer.crc32c
    );
    assert_eq!(snapshot_covm_entry.digest, expected_snapshot_digest);

    let snapshot_covx = temp_file("delta-cli-snapshot.covx");
    let snapshot_covx_build = run_cove(&[
        "sidecar",
        "build",
        "covx",
        "--snapshot",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        snapshot_covx.to_str().unwrap(),
    ]);
    assert!(
        snapshot_covx_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&snapshot_covx_build.stdout),
        String::from_utf8_lossy(&snapshot_covx_build.stderr)
    );
    let snapshot_covx_file = CovxFile::parse(&fs::read(&snapshot_covx).unwrap()).unwrap();
    assert_eq!(snapshot_covx_file.referenced_files.len(), 1);
    let snapshot_covx_entry = &snapshot_covx_file.referenced_files[0];
    assert_eq!(
        snapshot_covx_entry.file_id,
        expected_snapshot_identity.header.file_id
    );
    assert_eq!(
        snapshot_covx_entry.file_len,
        expected_snapshot_identity.postscript.file_len
    );
    assert_eq!(
        snapshot_covx_entry.footer_crc32c,
        expected_snapshot_identity.postscript.footer.crc32c
    );
    assert_eq!(snapshot_covx_entry.digest, expected_snapshot_digest);

    let map_delta_bundle = temp_file("delta-cli-map-bundle");
    let map_delta_build = run_cove(&[
        "map",
        "delta",
        "build",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out-dir",
        map_delta_bundle.to_str().unwrap(),
        "--projection-output",
        "none",
        "--force",
        "--json",
    ]);
    assert!(
        map_delta_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&map_delta_build.stdout),
        String::from_utf8_lossy(&map_delta_build.stderr)
    );
    let map_delta_json: serde_json::Value =
        serde_json::from_slice(&map_delta_build.stdout).unwrap();
    assert_eq!(
        map_delta_json["format"],
        serde_json::json!("cove-map-delta-build-manifest-v1")
    );

    let semantic_map_source = temp_file("people.jsonl");
    fs::write(
        &semantic_map_source,
        r#"{"id":"p4","active":true,"score":40,"rating":45,"status":"pending","nickname":"alan"}"#,
    )
    .unwrap();
    let semantic_map_delta = temp_file("delta-cli-map-semantic.covedelta");
    let semantic_map_delta_build = run_cove(&[
        "map",
        "delta",
        "build",
        "--base",
        graph_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--mapping",
        sample_path("people.covemap").to_str().unwrap(),
        "--out",
        semantic_map_delta.to_str().unwrap(),
        "--force",
        "--json",
        semantic_map_source.to_str().unwrap(),
    ]);
    assert!(
        semantic_map_delta_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_delta_build.stdout),
        String::from_utf8_lossy(&semantic_map_delta_build.stderr)
    );
    let semantic_map_report: serde_json::Value =
        serde_json::from_slice(&semantic_map_delta_build.stdout).unwrap();
    assert_eq!(
        semantic_map_report["format"],
        serde_json::json!("cove-map-semantic-delta-build-report-v1")
    );
    assert!(
        semantic_map_report["object_delta_validation"]["touched_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={semantic_map_report}"
    );
    assert!(semantic_map_delta.exists());

    let semantic_map_validate = run_cove(&[
        "delta",
        "validate",
        semantic_map_delta.to_str().unwrap(),
        "--object-delta",
        "--json",
    ]);
    assert!(
        semantic_map_validate.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_validate.stdout),
        String::from_utf8_lossy(&semantic_map_validate.stderr)
    );
    let semantic_map_inspect = run_cove(&[
        "delta",
        "inspect",
        semantic_map_delta.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        semantic_map_inspect.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_inspect.stdout),
        String::from_utf8_lossy(&semantic_map_inspect.stderr)
    );
    let semantic_map_inspected: serde_json::Value =
        serde_json::from_slice(&semantic_map_inspect.stdout).unwrap();
    assert_eq!(
        semantic_map_inspected["object_delta"]["catalog_patches"],
        serde_json::json!(0)
    );
    assert_eq!(
        semantic_map_inspected["object_delta"]["evidence_patches"],
        serde_json::json!(1)
    );

    fs::write(
        &semantic_map_source,
        r#"{"id":"p1","active":true,"score":55,"rating":65,"status":"reopened","nickname":"ada"}"#,
    )
    .unwrap();
    let semantic_map_update_delta = temp_file("delta-cli-map-semantic-update.covedelta");
    let semantic_map_update_build = run_cove(&[
        "map",
        "delta",
        "build",
        "--base",
        graph_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--mapping",
        sample_path("people.covemap").to_str().unwrap(),
        "--out",
        semantic_map_update_delta.to_str().unwrap(),
        "--force",
        "--json",
        semantic_map_source.to_str().unwrap(),
    ]);
    assert!(
        semantic_map_update_build.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_update_build.stdout),
        String::from_utf8_lossy(&semantic_map_update_build.stderr)
    );
    let semantic_map_update_report: serde_json::Value =
        serde_json::from_slice(&semantic_map_update_build.stdout).unwrap();
    assert_eq!(
        semantic_map_update_report["format"],
        serde_json::json!("cove-map-semantic-delta-build-report-v1")
    );
    assert!(
        semantic_map_update_report["object_delta_validation"]["continuation_anchors"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={semantic_map_update_report}"
    );
    assert!(
        semantic_map_update_report["object_delta_validation"]["touched_object_ranges"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "report={semantic_map_update_report}"
    );
    let semantic_map_update_validate = run_cove(&[
        "delta",
        "validate",
        semantic_map_update_delta.to_str().unwrap(),
        "--object-delta",
        "--json",
    ]);
    assert!(
        semantic_map_update_validate.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_update_validate.stdout),
        String::from_utf8_lossy(&semantic_map_update_validate.stderr)
    );
    let semantic_map_update_manifest = temp_file("delta-cli-map-semantic-update-chain.covm");
    let semantic_map_update_extend = run_cove(&[
        "delta",
        "chain",
        "extend",
        "--manifest",
        graph_manifest.to_str().unwrap(),
        "--delta",
        semantic_map_update_delta.to_str().unwrap(),
        "--out",
        semantic_map_update_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        semantic_map_update_extend.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_update_extend.stdout),
        String::from_utf8_lossy(&semantic_map_update_extend.stderr)
    );
    let semantic_map_update_query = run_cove(&[
        "query",
        semantic_map_update_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--format",
        "jsonl",
        "object(Person).where(score >= 50).select(score, status, nickname)",
    ]);
    assert!(
        semantic_map_update_query.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_update_query.stdout),
        String::from_utf8_lossy(&semantic_map_update_query.stderr)
    );
    let semantic_map_update_stdout = String::from_utf8_lossy(&semantic_map_update_query.stdout);
    assert!(semantic_map_update_stdout.contains(r#""score":55"#));
    assert!(semantic_map_update_stdout.contains(r#""status":"reopened"#));

    let semantic_map_manifest = temp_file("delta-cli-map-semantic-chain.covm");
    let semantic_map_extend = run_cove(&[
        "delta",
        "chain",
        "extend",
        "--manifest",
        graph_manifest.to_str().unwrap(),
        "--delta",
        semantic_map_delta.to_str().unwrap(),
        "--out",
        semantic_map_manifest.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        semantic_map_extend.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_extend.stdout),
        String::from_utf8_lossy(&semantic_map_extend.stderr)
    );
    let semantic_map_chain_validate = run_cove(&[
        "delta",
        "chain",
        "validate",
        semantic_map_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--json",
    ]);
    assert!(
        semantic_map_chain_validate.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_chain_validate.stdout),
        String::from_utf8_lossy(&semantic_map_chain_validate.stderr)
    );
    let semantic_map_query = run_cove(&[
        "query",
        semantic_map_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--format",
        "jsonl",
        "object(Person).where(score >= 40).select(score, status, nickname)",
    ]);
    assert!(
        semantic_map_query.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&semantic_map_query.stdout),
        String::from_utf8_lossy(&semantic_map_query.stderr)
    );
    let semantic_map_query_stdout = String::from_utf8_lossy(&semantic_map_query.stdout);
    assert!(semantic_map_query_stdout.contains(r#""score":40"#));
    assert!(semantic_map_query_stdout.contains(r#""status":"pending"#));

    let checkpoint = temp_file("delta-cli-checkpoint.covedelta");
    let checkpoint_summary = temp_file("delta-cli-checkpoint.cds1");
    let checkpoint_output = run_cove(&[
        "delta",
        "checkpoint",
        object_manifest.to_str().unwrap(),
        "--dataset",
        dataset_arg,
        "--out",
        checkpoint.to_str().unwrap(),
        "--summary-out",
        checkpoint_summary.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        checkpoint_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&checkpoint_output.stdout),
        String::from_utf8_lossy(&checkpoint_output.stderr)
    );
    let checkpoint_json: serde_json::Value =
        serde_json::from_slice(&checkpoint_output.stdout).unwrap();
    assert_eq!(checkpoint_json["ok"], serde_json::json!(true));
    assert_eq!(checkpoint_json["state_count"], serde_json::json!(1));
    assert!(checkpoint.exists());
    assert!(checkpoint_summary.exists());

    let checkpoint_validate = run_cove(&[
        "delta",
        "validate",
        checkpoint.to_str().unwrap(),
        "--object-delta",
    ]);
    assert!(
        checkpoint_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&checkpoint_validate.stderr)
    );

    let same_path_publish = run_cove(&[
        "delta",
        "publish-atomic",
        "--delta",
        delta.to_str().unwrap(),
        "--manifest",
        delta.to_str().unwrap(),
        "--json",
    ]);
    assert!(!same_path_publish.status.success());
    assert!(String::from_utf8_lossy(&same_path_publish.stderr).contains("must differ"));
}

#[test]
fn unified_sidecar_profile_digest_and_canonicalise_commands_work() {
    let file = temp_file("sidecar-events.cove");
    fs::write(&file, cove_t_events_bytes()).unwrap();

    let covi = temp_file("sidecar-events.covi");
    let covi_build = run_cove(&[
        "sidecar",
        "build",
        "covi",
        file.to_str().unwrap(),
        covi.to_str().unwrap(),
        "--all-columns",
    ]);
    assert!(
        covi_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covi_build.stderr)
    );
    let covi_inspect = run_cove(&["sidecar", "inspect", "index", covi.to_str().unwrap()]);
    assert!(
        covi_inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covi_inspect.stderr)
    );
    assert!(String::from_utf8_lossy(&covi_inspect.stdout).contains("valid COVE-I"));

    let covm = temp_file("sidecar-events.covm");
    let covm_build = run_cove(&[
        "sidecar",
        "build",
        "covm",
        covm.to_str().unwrap(),
        file.to_str().unwrap(),
    ]);
    assert!(
        covm_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covm_build.stderr)
    );
    assert!(covm.metadata().unwrap().len() > 0);

    let covx = temp_file("sidecar-events.covx");
    let covx_build = run_cove(&[
        "sidecar",
        "build",
        "covx",
        covx.to_str().unwrap(),
        file.to_str().unwrap(),
    ]);
    assert!(
        covx_build.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&covx_build.stderr)
    );
    assert!(covx.metadata().unwrap().len() > 0);

    for (kind, bytes, expected) in [
        ("coverage", coverage_provider_bytes(), "valid COVE-COVERAGE"),
        ("layout", layout_plan_bytes(), "valid COVE-L"),
        ("cache", cache_bytes(), "valid COVE-CACHE"),
        ("runtime", runtime_hint_bytes(), "valid COVE-R"),
    ] {
        let path = temp_file(&format!("{kind}.bin"));
        fs::write(&path, bytes).unwrap();
        let inspect = run_cove(&["sidecar", "inspect", kind, path.to_str().unwrap()]);
        assert!(
            inspect.status.success(),
            "kind={kind}\nstderr={}",
            String::from_utf8_lossy(&inspect.stderr)
        );
        assert!(
            String::from_utf8_lossy(&inspect.stdout).contains(expected),
            "stdout={}",
            String::from_utf8_lossy(&inspect.stdout)
        );
    }

    let digest = run_cove(&["digest", "verify", file.to_str().unwrap()]);
    assert!(
        digest.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&digest.stderr)
    );
    assert!(String::from_utf8_lossy(&digest.stdout).contains("missing_manifest"));

    let profile_section = temp_file("execution-code.bin");
    let profile = run_cove(&[
        "profile",
        "generate",
        "--kind",
        "execution-code",
        "--out",
        profile_section.to_str().unwrap(),
    ]);
    assert!(
        profile.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&profile.stderr)
    );
    let profile_validate = run_cove(&[
        "profile",
        "validate-section",
        profile_section.to_str().unwrap(),
        "--kind",
        "execution-code",
    ]);
    assert!(
        profile_validate.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&profile_validate.stderr)
    );

    let canonicalise = run_cove(&[
        "canonicalise",
        "validate-payload",
        "--tag",
        "int64",
        "--hex",
        "2a00000000000000",
    ]);
    assert!(
        canonicalise.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&canonicalise.stderr)
    );
    assert!(String::from_utf8_lossy(&canonicalise.stdout).contains("\"valid\": true"));
}

#[test]
fn checked_in_showcase_exercises_reference_workflow() {
    let customers = showcase_path("customers.cove");
    let events = showcase_path("events.cove");
    let map = showcase_path("customer_identity.covemap");
    let crm = showcase_path("crm_people.jsonl");
    let support = showcase_path("support_people.jsonl");

    for path in [&customers, &events, &map, &crm, &support] {
        assert!(
            path.exists(),
            "missing showcase artifact {}",
            path.display()
        );
    }

    let doctor = run_cove(&["doctor", customers.to_str().unwrap()]);
    assert!(
        doctor.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("Queryable: yes"));

    let inspect = run_cove(&[
        "inspect",
        "--queries",
        "--performance",
        customers.to_str().unwrap(),
    ]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_stdout.contains("table=customers"));
    assert!(inspect_stdout.contains("Show evidence rows"));

    let projected = run_cove(&[
        "map",
        "project",
        "--format",
        "json",
        map.to_str().unwrap(),
        crm.to_str().unwrap(),
        support.to_str().unwrap(),
    ]);
    assert!(
        projected.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&projected.stderr)
    );
    let projected_stdout = String::from_utf8_lossy(&projected.stdout);
    assert!(projected_stdout.contains("\"mapping_id\": \"showcase-customer-identity\""));
    assert!(projected_stdout.contains("\"output_table\": \"customers\""));

    let customer_query = run_cove(&[
        "query",
        customers.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(customers).select(full_name, score, status)",
    ]);
    assert!(
        customer_query.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&customer_query.stderr)
    );
    let customer_stdout = String::from_utf8_lossy(&customer_query.stdout);
    assert!(customer_stdout.contains("\"full_name\":\"Ada Lovelace\""));
    assert!(customer_stdout.contains("\"status\":\"dormant\""));

    let evidence = run_cove(&[
        "query",
        customers.to_str().unwrap(),
        "evidence().select(source_id, source_row_identity, rule_id).take(10)",
    ]);
    assert!(
        evidence.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&evidence.stderr)
    );
    assert!(String::from_utf8_lossy(&evidence.stdout).contains("customers_360"));

    let optimized_events = temp_file("showcase-events-for-optimize.cove");
    fs::copy(&events, &optimized_events).unwrap();
    let optimized = run_cove(&["optimize", optimized_events.to_str().unwrap()]);
    assert!(
        optimized.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&optimized.stderr)
    );
    assert!(String::from_utf8_lossy(&optimized.stdout).contains("Generated sidecars"));

    let compare = run_cove(&[
        "query",
        "--engine",
        "compare",
        "--perf-report",
        optimized_events.to_str().unwrap(),
        "table(events).where(score >= 25).select(event_id, person_id, score)",
    ]);
    assert!(
        compare.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&compare.stderr)
    );
    assert!(String::from_utf8_lossy(&compare.stdout).contains("1005"));
    assert!(String::from_utf8_lossy(&compare.stderr).contains("Performance report"));
}

#[test]
fn showcase_generator_rebuilds_valid_artifacts() {
    let out_dir = temp_file("regenerated-showcase");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "run",
            "-q",
            "-p",
            "cove-cli",
            "--example",
            "generate_beginner_samples",
            "--",
            "--showcase",
            out_dir.to_str().unwrap(),
        ])
        .current_dir(workspace_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_customers = out_dir.join("customers.cove");
    let generated_events = out_dir.join("events.cove");
    let validate_customers = run_cove(&[
        "validate",
        "--semantic",
        generated_customers.to_str().unwrap(),
    ]);
    assert!(
        validate_customers.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&validate_customers.stderr)
    );
    let query_events = run_cove(&[
        "query",
        generated_events.to_str().unwrap(),
        "--format",
        "jsonl",
        "table(events).where(score >= 25).select(event_id, person_id, score)",
    ]);
    assert!(
        query_events.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&query_events.stderr)
    );
    assert!(String::from_utf8_lossy(&query_events.stdout).contains("\"event_id\":1005"));
}

#[test]
fn customer360_showcase_command_generates_queryable_artifacts() {
    let out_dir = temp_file("customer360-generated");
    let generated = run_cove(&[
        "showcase",
        "customer360",
        "--profile",
        "quick",
        "--out",
        out_dir.to_str().unwrap(),
        "--force",
    ]);
    assert!(
        generated.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );

    for name in [
        "crm.csv",
        "support.jsonl",
        "billing.parquet",
        "events.jsonl",
        "events.cove",
        "customer360.covemap",
        "customers.cove",
        "customers_projection.cove",
        "evidence_projection.cove",
        "customers_projection.parquet",
        "evidence_projection.parquet",
        "map-build-bundle/map-build-manifest.json",
        "doctor-report.json",
        "proof-size-comparison.json",
        "customer360-manifest.json",
        "notebooks/customer360_analysis.py",
    ] {
        assert!(out_dir.join(name).exists(), "missing generated {name}");
    }

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(out_dir.join("customer360-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["profile"], "quick");
    assert_eq!(manifest["row_counts"]["canonical_customers"], 12);
    assert_eq!(
        manifest["pipeline"]["canonical_readback_source"],
        "customers_360.jsonl"
    );
    assert_eq!(manifest["proof"]["doctor_status"], serde_json::json!("ok"));
    assert_eq!(manifest["proof"]["parity_status"], serde_json::json!("ok"));
    assert!(manifest["recommended_queries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|query| query["title"] == "Customer events through an external JSONL table"));
    assert!(manifest["recommended_queries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|query| query["command"]
            .as_str()
            .unwrap()
            .contains("groupBy(source_id)")));
    let benchmark_cases = manifest["benchmark_cases"].as_array().unwrap();
    assert!(benchmark_cases.contains(&serde_json::json!("customer360_event_filter")));
    assert!(!benchmark_cases.contains(&serde_json::json!("customer360_evidence_aggregate")));
    assert!(!benchmark_cases.contains(&serde_json::json!("customer360_customer_event_join")));

    let inspect = run_cove(&[
        "inspect",
        "--queries",
        out_dir.join("customers.cove").to_str().unwrap(),
    ]);
    assert!(
        inspect.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(inspect_stdout.contains("table=customers"));
    assert!(inspect_stdout.contains("table=customer_evidence"));

    let customers = run_cove(&[
        "query",
        out_dir.join("customers.cove").to_str().unwrap(),
        "--format",
        "jsonl",
        "table(customers).where(score >= 80).select(customer_id, tier, score, status).take(5)",
    ]);
    assert!(
        customers.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&customers.stderr)
    );
    assert!(String::from_utf8_lossy(&customers.stdout).contains("\"customer_id\""));

    let joined = run_cove(&[
        "query",
        out_dir.join("customers.cove").to_str().unwrap(),
        "--external-table",
        &format!("events={}", out_dir.join("events.jsonl").display()),
        "--format",
        "jsonl",
        "table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, event_kind: e.event_kind, event_score: e.score).take(5)",
    ]);
    assert!(
        joined.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&joined.stderr)
    );
    assert!(String::from_utf8_lossy(&joined.stdout).contains("\"event_kind\""));
}

#[test]
fn proof_suite_showcase_command_generates_verified_scenario() {
    let out_dir = temp_file("proof-suite-generated");
    let generated = run_cove(&[
        "showcase",
        "proof-suite",
        "--scenario",
        "claims",
        "--profile",
        "quick",
        "--out",
        out_dir.to_str().unwrap(),
        "--force",
        "--json",
    ]);
    assert!(
        generated.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&generated.stdout),
        String::from_utf8_lossy(&generated.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_slice(&generated.stdout).unwrap();
    assert_eq!(
        manifest["format"],
        serde_json::json!("cove-proof-suite-manifest-v1")
    );
    assert_eq!(
        manifest["scenarios"][0]["scenario"],
        serde_json::json!("claims")
    );
    assert_eq!(
        manifest["scenarios"][0]["doctor_status"],
        serde_json::json!("ok")
    );
    assert_eq!(
        manifest["scenarios"][0]["parity_status"],
        serde_json::json!("ok")
    );
    for name in [
        "proof-suite-manifest.json",
        "claims/claims.covemap",
        "claims/map-build-bundle/map-build-manifest.json",
        "claims/doctor-report.json",
        "claims/proof-size-comparison.json",
        "claims/parity/claims.v1.json",
        "claims/proof-baselines/source-parquet/claims.parquet",
    ] {
        assert!(out_dir.join(name).exists(), "missing proof-suite {name}");
    }
}

#[test]
fn customer360_showcase_requires_force_for_non_empty_output() {
    let missing_out = run_cove(&["showcase", "customer360", "--profile", "quick"]);
    assert!(!missing_out.status.success());
    assert!(String::from_utf8_lossy(&missing_out.stderr).contains("--out is required"));

    let out_dir = temp_file("customer360-force");
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(out_dir.join("keep.txt"), "existing").unwrap();
    let rejected = run_cove(&[
        "showcase",
        "customer360",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--force"));
}

#[test]
fn examples_json_includes_customer360_showcase() {
    let examples = run_cove(&["examples", "--json"]);
    assert!(
        examples.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&examples.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&examples.stdout).unwrap();
    assert_eq!(value["showcases"][0]["name"], "customer360");
    assert!(value["showcases"][0]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["command"]
            .as_str()
            .unwrap()
            .contains("cove showcase customer360")));
}
