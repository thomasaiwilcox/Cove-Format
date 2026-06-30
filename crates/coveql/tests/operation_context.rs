#![allow(
    clippy::field_reassign_with_default,
    clippy::manual_repeat_n,
    clippy::too_many_arguments,
    clippy::unnecessary_find_map,
    clippy::useless_vec
)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use arrow_array::{
    types::UInt32Type, Array, BooleanArray, DictionaryArray, FixedSizeBinaryArray, Int64Array,
    StringArray,
};
use arrow_schema::DataType;
use cove_core::{
    artifact::{
        coveai::{
            write_coveai_descriptor_bundle, write_covev_filecode_vectors, AiAssetRefV1,
            AiDescriptorTablesV1, AiDigestEntryV1, AiPayloadEncodingV1, AiPayloadRefEntryV1,
            AiPrivacySummaryEntryV1, AiRequirednessScopeV1, AiStorageKindV1, AssetVectorBindingV1,
            AssociationStateVectorBindingV1, ChunkProfileV1, CoveAiDescriptorBundleBuild,
            CoveAiWritableSection, CoveVecFileCodeVectorBuild, DatasetSplitV1, DedupGroupV1,
            DeviceTransferHintV1, GenerationDecodingProfileV1, GeneratorProvenanceV1,
            HumanReviewEntryV1, ModelActorDescriptorV1, MultimodalSequenceElementV1,
            MultimodalSequencePackV1, MultimodalSequenceVectorBindingV1, PreferencePairEntryV1,
            TensorLayoutDescriptorV1, TextChunkEntryV1, TokenBlockHeaderV1, TokenSequencePackV1,
            TokenizedSpanV1, TokenizerProfileV1, TrainingEpochPlanV1, TrainingLabelEntryV1,
            TrainingProfileV1, TrainingSampleEntryV1, VectorEntryV1, VectorPayloadBlockHeaderV1,
            VectorSpaceDescriptorV1,
        },
        covm::{CovmFile, CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1},
    },
    canonical::CanonicalValue,
    checksum,
    collation::CollationKind,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        PrimaryProfile, SectionKind, StorageClass, FEATURE_ENGINE_PROFILE, FEATURE_FILE_DICTIONARY,
        FEATURE_OBJECT_PROFILE, FEATURE_REDACTIONS, FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        FEATURE_SEMANTIC_MAP,
    },
    dictionary::{FileDictionaryEncoding, FileDictionaryKey},
    digest::compute_digest,
    page::ColumnPageIndexEntryV1,
    page_payload::{
        ColumnPagePayloadHeaderV1, ColumnPagePayloadV1, CoveEncodingNodeV1, PageBufferDescriptorV1,
        PageBufferKind, COLUMN_PAGE_PAYLOAD_HEADER_LEN, COLUMN_PAGE_PAYLOAD_MAGIC,
        COLUMN_PAGE_PAYLOAD_VERSION_MAJOR, COVE_ENCODING_NODE_LEN, PAGE_BUFFER_DESCRIPTOR_LEN,
    },
    profile::cove_e::{
        CodeSpaceDescriptorV1, EngineMountPolicyV1, ExecutionCodeCanonicality,
        ExecutionCodeComparisonScope, ExecutionCodeDescriptorV1, ExecutionCodeKind,
        ExecutionCodeLifetime, ExecutionScopeDescriptorV1, ExecutionScopeKind, FileCodeMappingKind,
        MissingValuePolicy, NullCodePolicy, ReverseLookupPolicy, StaleMappingPolicy,
    },
    profile::cove_o::{
        read_object_surface_from_bytes, read_retained_object_temporal_segments,
        CoveObjectPropertyValue, CoveObjectState, CoveObjectTombstoneStatus, CoveRecordRefV1,
        ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1,
        TemporalSegmentHeaderV1, TemporalSegmentIndex, TemporalSegmentIndexEntryV1,
        OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
        PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    },
    reader::{validate_bytes, OptionalPushdownPolicy, ValidationOptions},
    redaction::{RedactionEntry, RedactionManifest},
    retained_bytes::RetainedBytes,
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    wire,
    writer::{MinimalCoveWriter, SectionPayload},
};
use cove_coverage::{
    coverage_set_payload_checksum, CoverageExactnessV2, CoverageGranularityV2, CoverageProofKindV2,
    CoverageProofRecordV2, CoverageProofStrengthV2, CoverageSetEntryV2, CoverageSetHeaderV2,
    CoverageSetV2,
};
use cove_index::{
    execution::CoviAggregateKindV2, CoviAggregateAnswerBlockHeaderV2, CoviAggregateAnswerBlockV2,
    CoviAggregateAnswerV2, CoviArtifactV2, CoviComparatorKindV2, CoviEntryBlockHeaderV2,
    CoviEntryBlockV2, CoviIndexEntryV2, CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2,
    CoviKeyBlockHeaderV2, CoviKeyBlockV2, CoviKeyEncodingKindV2, CoviPostingsBlockHeaderV2,
    CoviPostingsBlockV2, CoviPostingsHeaderV2, CoviReferencedFileV2, CoviRowRangePostingV2,
    CoviSectionKindV2, CoviSectionPayloadV2, CoviSnapshotValidityV2, IndexCapabilityExactnessV2,
    IndexCapabilityV2, IndexOnlyCapabilityV2,
};
use cove_layout::{
    ZeroCopyBufferMapEntryV2, ZeroCopyBufferMapHeaderV2, ZeroCopyBufferMapV2,
    ZeroCopyDictionarySemanticsV2, ZeroCopyLifetimeScopeV2, ZeroCopyNestedLayoutKindV2,
    ZeroCopyNullBitmapPolarityV2, ZeroCopySourceBufferRoleV2, ZeroCopyTargetBufferRoleV2,
    ZeroCopyTargetV2,
};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeHintKindV2};
use coveql::{
    build_operation_context, build_physical_plan, execute_manifest_physical_planned_query,
    execute_planned_query, execute_planned_query_stream, parse_and_resolve_query,
    parse_resolve_and_plan_query, parse_resolve_plan_and_build_physical_plan,
    parse_resolve_plan_and_execute_query, parse_resolve_plan_build_physical_and_execute_query,
    parse_resolve_plan_build_physical_and_execute_query_retained, render_explain_text,
    AggregateDisclosurePolicy, AssociationEndpointRole, AstCompareOp, CoveQlExecutionResult,
    CoveQlOperationRequest, CoveQlOutputMode, CoveQlRetainedInput, CoveQlSelectedOperation,
    ExecutionOptions, ExplainDisclosurePolicy, ExplainMode, FallbackPolicy, FilterClassification,
    KernelDecisionKind, KernelExecutedQuery, KernelExecutionMode, KernelExecutionOptions,
    KernelFallbackReason, LogicalPlanNodeKind, LogicalPredicateKind, MaterializedChangeDiffKind,
    MetadataDisclosurePolicy, OptionalMetadataKind, OptionalMetadataStatus, OutputGrain,
    ParseOptions, PhysicalPlanNodeKind, PhysicalPlanOptions, PhysicalPredicateFormKind,
    PhysicalRepresentationClass, PhysicalSidecarInputs, PhysicalSidecarStatus, PlanOptions,
    PredicatePlacement, PredicateProofState, PushdownDecisionKind, PushdownOptions,
    PushdownOutcome, RedactionPolicy, RepresentationClass, ResolveOptions, ResolvedExpr,
    ResolvedLiteralValue, ResolvedPredicate, ResolvedRoot, SecurityContext, TemporalRole,
    VisibilityOverlay, VisibilityPolicy,
};
use serde_json::{json, Value};

#[cfg(feature = "datafusion")]
use cove_datafusion::register::df as datafusion;

fn validation_options() -> ValidationOptions {
    ValidationOptions {
        semantic: true,
        verify_digests: false,
        allow_unknown_optional_extensions: true,
        optional_pushdown_policy: OptionalPushdownPolicy::Strict,
    }
}

fn projection_backed_thing_table_contract() -> coveql::TableSurfaceContract {
    coveql::TableSurfaceContract {
        table_id: "projection:thing_projection".into(),
        table_name: "thing_projection".into(),
        contract_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
        authority_kind: coveql::TableSurfaceAuthorityKind::DeterministicProjection,
        authority_fingerprint: "thing_projection".into(),
        schema_fingerprint: "thing_projection".into(),
        logical_column_map: vec![coveql::TableSurfaceColumnContract {
            name: "active".into(),
            logical_type: Some("bool".into()),
            nullable: true,
            source_path: Some("active".into()),
            code_domain: None,
            collation: None,
        }],
        row_grain: "one_row_per_object".into(),
        row_identity: vec!["projection_row_identity".into()],
        canonical_order: vec!["projection_row_identity".into()],
        visibility_authority: "cove_o_visibility".into(),
        redaction_authority: "cove_o_redaction".into(),
        temporal_authority: coveql::TableTemporalAuthority::MaterializedSnapshotOnly,
        evidence_capabilities: vec![
            coveql::AstEvidenceGrain::Row,
            coveql::AstEvidenceGrain::Column,
            coveql::AstEvidenceGrain::Projection,
            coveql::AstEvidenceGrain::Source,
        ],
        null_missing_nan_policy: "projection_contract".into(),
        collation_policy: "projection_contract".into(),
        code_domain_contexts: Vec::new(),
        code_domain_bridges: Vec::new(),
        projection_dependency_contract_id: Some("thing_projection".into()),
        datafusion_interop_contract: Some("coveql_table_projection_provider".into()),
    }
}

fn registered_people_table_authority(
    authority_kind: coveql::TableSurfaceAuthorityKind,
    execution_authority: coveql::TableExecutionAuthority,
) -> coveql::TableSurfaceAuthority {
    coveql::TableSurfaceAuthority {
        contract: coveql::TableSurfaceContract {
            table_id: "table:people".into(),
            table_name: "people".into(),
            contract_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
            authority_kind,
            authority_fingerprint: "people:v1".into(),
            schema_fingerprint: "people-schema:v1".into(),
            logical_column_map: vec![
                coveql::TableSurfaceColumnContract {
                    name: "id".into(),
                    logical_type: Some("utf8".into()),
                    nullable: false,
                    source_path: Some("id".into()),
                    code_domain: None,
                    collation: None,
                },
                coveql::TableSurfaceColumnContract {
                    name: "active".into(),
                    logical_type: Some("bool".into()),
                    nullable: false,
                    source_path: Some("active".into()),
                    code_domain: None,
                    collation: None,
                },
            ],
            row_grain: "registered_table_row".into(),
            row_identity: vec!["id".into()],
            canonical_order: vec!["id".into()],
            visibility_authority: "registered_visibility".into(),
            redaction_authority: "registered_redaction".into(),
            temporal_authority: coveql::TableTemporalAuthority::StaticTableSnapshot,
            evidence_capabilities: vec![coveql::AstEvidenceGrain::Row],
            null_missing_nan_policy: "missing_is_null".into(),
            collation_policy: "binary".into(),
            code_domain_contexts: Vec::new(),
            code_domain_bridges: Vec::new(),
            projection_dependency_contract_id: None,
            datafusion_interop_contract: Some("registered_materialized_rows".into()),
        },
        execution_authority,
    }
}

fn people_rows(rows: &[(&str, bool)]) -> Vec<coveql::TableSurfaceRow> {
    rows.iter()
        .map(|(id, active)| {
            BTreeMap::from([("id".into(), json!(id)), ("active".into(), json!(active))])
        })
        .collect()
}

fn score_table_authority() -> coveql::TableSurfaceAuthority {
    let mut authority = registered_people_table_authority(
        coveql::TableSurfaceAuthorityKind::MaterializedTable,
        coveql::TableExecutionAuthority::MaterializedRows {
            rows: vec![
                BTreeMap::from([
                    ("id".into(), json!("a")),
                    ("active".into(), json!(true)),
                    ("score".into(), json!(1)),
                ]),
                BTreeMap::from([
                    ("id".into(), json!("b")),
                    ("active".into(), json!(false)),
                    ("score".into(), json!(2)),
                ]),
                BTreeMap::from([
                    ("id".into(), json!("c")),
                    ("active".into(), json!(true)),
                    ("score".into(), json!(3)),
                ]),
            ],
        },
    );
    authority.contract.table_id = "table:scores".into();
    authority.contract.table_name = "scores".into();
    authority
        .contract
        .logical_column_map
        .push(coveql::TableSurfaceColumnContract {
            name: "score".into(),
            logical_type: Some("int64".into()),
            nullable: false,
            source_path: Some("score".into()),
            code_domain: None,
            collation: None,
        });
    authority
}

fn graph_traversal_contract() -> coveql::GraphTraversalContract {
    coveql::GraphTraversalContract {
        contract_version: coveql::COVEQL_PROFILE_CONTRACT_VERSION.into(),
        allow_variable_length: true,
        supported_modes: vec![
            coveql::GraphTraversalMode::Walk,
            coveql::GraphTraversalMode::Trail,
            coveql::GraphTraversalMode::SimplePath,
        ],
        supported_distinct_policies: vec![
            coveql::GraphTraversalDistinctPolicy::None,
            coveql::GraphTraversalDistinctPolicy::Path,
            coveql::GraphTraversalDistinctPolicy::EndNode,
        ],
        max_depth: 4,
        max_fanout_per_node: 16,
        max_paths: 64,
        max_frontier: 64,
        path_identity: vec!["start_goid".into(), "edge_goids".into(), "end_goid".into()],
        hidden_endpoint_policy: "suppress_hidden_endpoints".into(),
        ordering_policy: "breadth_first_canonical_association_order".into(),
        execution_authority: "visible_materialized_graph_oracle".into(),
    }
}

fn assert_unique_contract_fields(label: &str, fields: &[&str]) {
    let mut seen = BTreeSet::new();
    for field in fields {
        assert!(
            seen.insert(*field),
            "{label} contains duplicate field {field}"
        );
    }
}

#[path = "operation_context/profile_ai_graph.rs"]
mod profile_ai_graph;

fn minimal_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_file_with_runtime_hints(hints: &[RuntimeCompatibilityHintV2]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };

    let mut runtime_payload = Vec::new();
    for hint in hints {
        runtime_payload.extend_from_slice(&hint.serialize().unwrap());
    }

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.optional_features = FEATURE_RUNTIME_COMPATIBILITY_HINTS;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::RuntimeCompatibilityHints as u16,
        profile: PrimaryProfile::RuntimeCompatibility as u8,
        flags: 0,
        item_count: hints.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        data: runtime_payload,
    });
    writer.write().unwrap()
}

fn minimal_object_file_with_id(file_id: [u8; 16]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_incompatible_object_file_with_id(file_id: [u8; 16]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_projection_file_with_id(file_id: [u8; 16], column_name: &str) -> Vec<u8> {
    minimal_object_projection_file_with_id_and_mapping(
        file_id,
        column_name,
        "people-map",
        "2026.05",
    )
}

fn minimal_object_projection_file_with_id_and_mapping(
    file_id: [u8; 16],
    column_name: &str,
    mapping_id: &str,
    mapping_version: &str,
) -> Vec<u8> {
    minimal_object_projection_file_with_id_mapping_and_ordering(
        file_id,
        column_name,
        mapping_id,
        mapping_version,
        &[],
    )
}

fn minimal_object_projection_file_with_id_mapping_and_ordering(
    file_id: [u8; 16],
    column_name: &str,
    mapping_id: &str,
    mapping_version: &str,
    ordering: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    let mut projection = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": column_name,
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });
    if !ordering.is_empty() {
        projection["projections"][0]["ordering"] = json!(ordering);
    }
    push_embedded_map_section(&mut writer, SectionKind::MapProjectionCatalog, projection);
    writer.write().unwrap()
}

fn minimal_object_with_ordered_projection_file() -> Vec<u8> {
    minimal_object_projection_file_with_id_mapping_and_ordering(
        [0xC3; 16],
        "active",
        "people-map",
        "2026.05",
        &["active"],
    )
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
        header: CovmHeaderV1::new([0xC0; 16], 1, files.len() as u32, 1_700_000_000_000_000),
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

fn minimal_binary_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "payload".into(),
                logical_type: CoveLogicalType::Binary,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_string_object_with_function_registry(function_id: &str) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let registry = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapFunctionRegistry as u16,
        "mapping_id": "function-map",
        "mapping_version": "2026.06",
        "functions": [{
            "function_id": function_id,
            "version": "1",
            "deterministic": true,
            "dependency": "unicode:15.1"
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapFunctionRegistry as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&registry).unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_nullable_name_records(values: &[Option<&str>]) -> Vec<u8> {
    object_file_with_nullable_name_records_and_function_registry(values, &[])
}

fn string_function_dependency(function_id: &str) -> &'static str {
    match function_id {
        "startsWith" => "unicode:15.1;prefix-accelerator:startsWith:exact",
        "length" => "unicode:15.1;encoded-logical-length:exact",
        _ => "unicode:15.1",
    }
}

fn object_file_with_nullable_name_records_and_function_registry(
    values: &[Option<&str>],
    function_ids: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::VarBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_utf8_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE
        | if function_ids.is_empty() {
            0
        } else {
            FEATURE_SEMANTIC_MAP
        };
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": string_function_dependency(function_id)
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    writer.write().unwrap()
}

fn object_file_with_bool_records(values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_and_function_registry(values, &[])
}

fn object_file_with_bool_records_and_function_registry(
    values: &[bool],
    function_ids: &[&str],
) -> Vec<u8> {
    object_file_with_bool_records_with_file_id_and_function_registry([0; 16], values, function_ids)
}

fn object_file_with_bool_records_with_file_id(file_id: [u8; 16], values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_with_file_id_and_function_registry(file_id, values, &[])
}

fn object_file_with_bool_records_with_file_id_and_function_registry(
    file_id: [u8; 16],
    values: &[bool],
    function_ids: &[&str],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
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
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE
        | if function_ids.is_empty() {
            0
        } else {
            FEATURE_SEMANTIC_MAP
        };
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": if *function_id == "cast" {
                        "safe-cast:declared"
                    } else {
                        "pure"
                    }
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    writer.write().unwrap()
}

fn object_file_with_nullable_bool_records(values: &[Option<bool>]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_bool_records_and_projection(values: &[bool]) -> Vec<u8> {
    object_file_with_bool_records_and_projection_with_file_id([0; 16], values)
}

fn object_file_with_bool_records_and_projection_with_file_id(
    file_id: [u8; 16],
    values: &[bool],
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
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
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "thing-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "thing_projection",
            "output_table": "thing_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Thing"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": "active",
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_bool_records_on_branches(values: &[bool], branches: &[u64]) -> Vec<u8> {
    assert_eq!(values.len(), branches.len());
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
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
        }],
    };
    let rows = values
        .iter()
        .zip(branches.iter())
        .enumerate()
        .map(|(index, (_, branch))| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: *branch,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_bool_change() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
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
        }],
    };
    let rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [7; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Delta,
            prev_ref: Some(CoveRecordRefV1 {
                segment_id: 7,
                row_index: 0,
                target_kind: 1,
            }),
        },
    ];
    let segment = temporal_segment_with_bool_property(&rows, &[false, true]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn association_file_with_endpoint_change() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };
    let rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [7; 16],
            record_id: [40; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 2,
            branch_key: 0,
            goid: [7; 16],
            record_id: [41; 16],
            record_kind: RecordKind::Delta,
            prev_ref: Some(CoveRecordRefV1 {
                segment_id: 8,
                row_index: 0,
                target_kind: 1,
            }),
        },
    ];
    let segment = temporal_segment_with_association_endpoints(
        &rows,
        &[[1; 16], [1; 16]],
        &[[2; 16], [3; 16]],
    );
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_object_type_rows(
            8,
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_numcode_records(values: &[i64]) -> Vec<u8> {
    for payload_padding in 0..8 {
        let bytes = object_file_with_numcode_records_with_padding(values, payload_padding);
        if retained_metric_values_are_aligned(&bytes) {
            return bytes;
        }
    }
    panic!("could not build aligned retained NumCode object fixture");
}

fn object_file_with_nullable_numcode_records(values: &[Option<i64>]) -> Vec<u8> {
    for payload_padding in 0..8 {
        let bytes = object_file_with_nullable_numcode_records_with_padding(values, payload_padding);
        if retained_metric_values_are_aligned(&bytes) {
            return bytes;
        }
    }
    panic!("could not build aligned retained nullable NumCode object fixture");
}

fn object_file_with_plain_fixed_uuid_records(values: &[[u8; 16]]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "UuidFixedThing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 4,
                property_name: "uid".into(),
                logical_type: CoveLogicalType::Uuid,
                physical_kind: CovePhysicalKind::FixedBytes,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 112; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_plain_fixed_uuid_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_nullable_plain_fixed_uuid_records(values: &[Option<[u8; 16]>]) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "UuidFixedThing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 4,
                property_name: "uid".into(),
                logical_type: CoveLogicalType::Uuid,
                physical_kind: CovePhysicalKind::FixedBytes,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 112; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_plain_fixed_uuid_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_numcode_records_with_padding(
    values: &[i64],
    payload_padding: usize,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "MetricThing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 3,
                property_name: "metric".into(),
                logical_type: CoveLogicalType::Int64,
                physical_kind: CovePhysicalKind::NumCode,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 96; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_numcode_property(&rows, values, payload_padding);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn object_file_with_nullable_numcode_records_with_padding(
    values: &[Option<i64>],
    payload_padding: usize,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "MetricThing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 3,
                property_name: "metric".into(),
                logical_type: CoveLogicalType::Int64,
                physical_kind: CovePhysicalKind::NumCode,
                nullable: true,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 96; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_nullable_numcode_property(&rows, values, payload_padding);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.write().unwrap()
}

fn retained_metric_values_are_aligned(bytes: &[u8]) -> bool {
    let retained = read_retained_object_temporal_segments(
        RetainedBytes::from_vec(bytes.to_vec()),
        validation_options(),
    )
    .unwrap();
    let payload = retained.segments[0].property_columns[0].pages[0]
        .payload
        .as_ref()
        .unwrap();
    let values = payload
        .buffer_bytes(PageBufferKind::Values)
        .unwrap()
        .unwrap();
    (values.as_ptr() as usize).is_multiple_of(std::mem::align_of::<u64>())
}

fn metric_zero_copy_map() -> Vec<u8> {
    metric_zero_copy_map_with_null_polarity(ZeroCopyNullBitmapPolarityV2::NoNullBitmap)
}

fn nullable_metric_zero_copy_map() -> Vec<u8> {
    metric_zero_copy_map_with_null_polarity(ZeroCopyNullBitmapPolarityV2::OneMeansNull)
}

fn uid_fixed_bytes_zero_copy_map() -> Vec<u8> {
    uid_fixed_bytes_zero_copy_map_with_null_polarity(ZeroCopyNullBitmapPolarityV2::NoNullBitmap)
}

fn nullable_uid_fixed_bytes_zero_copy_map() -> Vec<u8> {
    uid_fixed_bytes_zero_copy_map_with_null_polarity(ZeroCopyNullBitmapPolarityV2::OneMeansNull)
}

fn active_bool_zero_copy_map() -> Vec<u8> {
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
        entries: vec![ZeroCopyBufferMapEntryV2 {
            target_id: 1,
            table_id: 1,
            column_id: 1,
            segment_id: 7,
            morsel_id: 0,
            page_ref: 1,
            buffer_id: 0,
            buffer_kind: PageBufferKind::Values as u16,
            logical_type: CoveLogicalType::Bool as u16,
            physical_kind: CovePhysicalKind::Boolean as u8,
            source_endianness: 0,
            required_alignment_log2: 0,
            null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2::NoNullBitmap,
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
        }],
    }
    .serialize()
    .unwrap()
}

fn name_filecode_zero_copy_map() -> Vec<u8> {
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
        entries: vec![ZeroCopyBufferMapEntryV2 {
            target_id: 1,
            table_id: 1,
            column_id: 2,
            segment_id: 7,
            morsel_id: 0,
            page_ref: 1,
            buffer_id: 0,
            buffer_kind: PageBufferKind::Values as u16,
            logical_type: CoveLogicalType::Utf8 as u16,
            physical_kind: CovePhysicalKind::FileCode as u8,
            source_endianness: 0,
            required_alignment_log2: 0,
            null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2::NoNullBitmap,
            source_offset_width_bits: 0,
            target_offset_width_bits: 0,
            dictionary_key_width_bits: 32,
            dictionary_semantics: ZeroCopyDictionarySemanticsV2::FileCodeDictionary,
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
        }],
    }
    .serialize()
    .unwrap()
}

fn uid_fixed_bytes_zero_copy_map_with_null_polarity(
    null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2,
) -> Vec<u8> {
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
        entries: vec![ZeroCopyBufferMapEntryV2 {
            target_id: 1,
            table_id: 1,
            column_id: 4,
            segment_id: 7,
            morsel_id: 0,
            page_ref: 1,
            buffer_id: 0,
            buffer_kind: PageBufferKind::Values as u16,
            logical_type: CoveLogicalType::Uuid as u16,
            physical_kind: CovePhysicalKind::FixedBytes as u8,
            source_endianness: 0,
            required_alignment_log2: 0,
            null_bitmap_polarity,
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
        }],
    }
    .serialize()
    .unwrap()
}

fn metric_zero_copy_map_with_null_polarity(
    null_bitmap_polarity: ZeroCopyNullBitmapPolarityV2,
) -> Vec<u8> {
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
        entries: vec![ZeroCopyBufferMapEntryV2 {
            target_id: 1,
            table_id: 1,
            column_id: 3,
            segment_id: 7,
            morsel_id: 0,
            page_ref: 1,
            buffer_id: 0,
            buffer_kind: PageBufferKind::Values as u16,
            logical_type: CoveLogicalType::Int64 as u16,
            physical_kind: CovePhysicalKind::NumCode as u8,
            source_endianness: 0,
            required_alignment_log2: 3,
            null_bitmap_polarity,
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
        }],
    }
    .serialize()
    .unwrap()
}

fn object_file_with_filecode_records(values: &[&str]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_records_and_function_registry(values, &[])
}

fn object_file_with_filecode_records_with_collation(
    values: &[&str],
    collation_id: u16,
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
        [0; 16],
        "Person",
        "name",
        CoveLogicalType::Utf8,
        collation_id,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_records_with_file_id(
    file_id: [u8; 16],
    values: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        file_id,
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_records_and_function_registry(
    values: &[&str],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_and_function_registry(
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &[],
        function_ids,
    )
}

fn object_file_with_filecode_records_and_redactions(
    values: &[&str],
    redacted_values: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    let redacted_keys = redacted_values
        .iter()
        .map(|value| {
            FileDictionaryKey::from_logical_bytes(CoveLogicalType::Utf8, value.as_bytes()).unwrap()
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_and_function_registry(
        "Person",
        "name",
        CoveLogicalType::Utf8,
        &keys,
        &redacted_keys,
        &[],
    )
}

fn object_file_with_bool_filecode_records(values: &[bool]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::Bool(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records("FlagThing", "active", CoveLogicalType::Bool, &keys, &[])
}

fn object_file_with_int64_filecode_records(values: &[i64]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| {
            file_dictionary_key_from_canonical(CanonicalValue::Int {
                width: 8,
                value: i128::from(*value),
            })
        })
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records(
        "MetricThing",
        "metric",
        CoveLogicalType::Int64,
        &keys,
        &[],
    )
}

fn object_file_with_uuid_filecode_records(values: &[[u8; 16]]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::Uuid(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records("UuidThing", "uid", CoveLogicalType::Uuid, &keys, &[])
}

fn object_file_with_timestamp_filecode_records(values: &[i64]) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::TimestampMicros(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records(
        "EventThing",
        "event_time",
        CoveLogicalType::TimestampMicros,
        &keys,
        &[],
    )
}

fn object_file_with_timestamp_filecode_records_with_file_id(
    file_id: [u8; 16],
    values: &[i64],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let keys = values
        .iter()
        .map(|value| file_dictionary_key_from_canonical(CanonicalValue::TimestampMicros(*value)))
        .collect::<Vec<_>>();
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        file_id,
        "EventThing",
        "event_time",
        CoveLogicalType::TimestampMicros,
        &keys,
        &[],
        &[],
    )
}

fn object_file_with_filecode_key_records(
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_and_function_registry(
        type_name,
        property_name,
        logical_type,
        keys,
        redacted_keys,
        &[],
    )
}

fn object_file_with_filecode_key_records_and_function_registry(
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_with_file_id_and_function_registry(
        [0; 16],
        type_name,
        property_name,
        logical_type,
        keys,
        redacted_keys,
        function_ids,
    )
}

fn object_file_with_filecode_key_records_with_file_id_and_function_registry(
    file_id: [u8; 16],
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
        file_id,
        type_name,
        property_name,
        logical_type,
        0,
        keys,
        redacted_keys,
        function_ids,
    )
}

fn object_file_with_filecode_key_records_with_file_id_collation_and_function_registry(
    file_id: [u8; 16],
    type_name: &str,
    property_name: &str,
    logical_type: CoveLogicalType,
    collation_id: u16,
    keys: &[FileDictionaryKey],
    redacted_keys: &[FileDictionaryKey],
    function_ids: &[&str],
) -> (Vec<u8>, BTreeMap<u32, u64>) {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: type_name.into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 2,
                property_name: property_name.into(),
                logical_type,
                physical_kind: CovePhysicalKind::FileCode,
                nullable: false,
                collation_id,
                flags: 0,
            }],
        }],
    };
    let mut dictionary = FileDictionaryEncoding::from_keys(keys.iter().cloned()).unwrap();
    let file_codes = keys
        .iter()
        .map(|key| dictionary.file_code_for_key(key).unwrap())
        .collect::<Vec<_>>();
    let mut redacted_codes = Vec::new();
    for key in redacted_keys {
        let code = dictionary.file_code_for_key(key).unwrap();
        redacted_codes.push(u64::from(code));
        let entry = &mut dictionary.dictionary.entries[code as usize];
        entry.storage_class = StorageClass::Redacted as u8;
        entry.inline_len = 0;
        entry.inline_data = [0; 16];
        entry.payload_offset = 0;
        entry.payload_length = 0;
        entry.canonical_hash64 = 0;
    }
    let mut execution_map = BTreeMap::new();
    for code in 0..dictionary.dictionary.len() {
        execution_map.insert(code, 10_000 + u64::from(code));
    }
    let rows = keys
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 64; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_filecode_property(logical_type, &rows, &file_codes);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };
    let mut dictionary_index = Vec::new();
    dictionary_index.extend_from_slice(&dictionary.dictionary.header.serialize());
    for entry in &dictionary.dictionary.entries {
        dictionary_index.extend_from_slice(&entry.serialize());
    }

    let mut writer = MinimalCoveWriter::new();
    writer.file_id = file_id;
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features =
        FEATURE_OBJECT_PROFILE | FEATURE_FILE_DICTIONARY | FEATURE_ENGINE_PROFILE;
    if !function_ids.is_empty() {
        writer.required_features |= FEATURE_SEMANTIC_MAP;
    }
    if !redacted_codes.is_empty() {
        writer.required_features |= FEATURE_REDACTIONS;
    }
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::FileDictionaryIndex as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count: dictionary.dictionary.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_FILE_DICTIONARY,
        optional_features: 0,
        data: dictionary_index,
    });
    if !function_ids.is_empty() {
        let registry = json!({
            "schema_id": "org.coveformat.covemap.v2",
            "section_id": SectionKind::MapFunctionRegistry as u16,
            "mapping_id": "function-map",
            "mapping_version": "2026.06",
            "functions": function_ids
                .iter()
                .map(|function_id| json!({
                    "function_id": function_id,
                    "version": "1",
                    "deterministic": true,
                    "dependency": string_function_dependency(function_id)
                }))
                .collect::<Vec<_>>()
        });
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::MapFunctionRegistry as u16,
            profile: PrimaryProfile::SemanticMapping as u8,
            flags: 0,
            item_count: function_ids.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            data: serde_json::to_vec_pretty(&registry).unwrap(),
        });
    }
    if !redacted_codes.is_empty() {
        let manifest = RedactionManifest {
            entries: redacted_codes
                .iter()
                .enumerate()
                .map(|(index, code)| RedactionEntry {
                    redaction_id: index as u64 + 1,
                    section_id: 2,
                    local_ref: *code,
                    reason_code: 0,
                    policy_id: Vec::new(),
                    audit_ref: Vec::new(),
                    created_at_us: 0,
                })
                .collect(),
        };
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::RedactionManifest as u16,
            profile: PrimaryProfile::Mixed as u8,
            flags: 0,
            item_count: redacted_codes.len() as u64,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_REDACTIONS,
            optional_features: 0,
            data: manifest.serialize().unwrap(),
        });
    }
    if !dictionary.dictionary.payload.is_empty() {
        writer.sections.push(SectionPayload {
            section_kind: SectionKind::FileDictionaryPayload as u16,
            profile: PrimaryProfile::Mixed as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: FEATURE_FILE_DICTIONARY,
            optional_features: 0,
            data: dictionary.dictionary.payload.clone(),
        });
    }
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ExecutionCodeDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: ExecutionCodeDescriptorV1 {
            descriptor_id: 1,
            code_kind: ExecutionCodeKind::UnsignedInteger,
            code_width_bits: 64,
            byte_order: 0,
            lifetime: ExecutionCodeLifetime::Scan,
            comparison_scope: ExecutionCodeComparisonScope::File,
            canonicality: ExecutionCodeCanonicality::CanonicalWithinScope,
            null_code_policy: NullCodePolicy::NullBitmapOnly,
            flags: 0,
            scope_ref: 1,
            code_space_ref: 1,
            checksum: 0,
        }
        .serialize()
        .to_vec(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ExecutionScopeDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: ExecutionScopeDescriptorV1 {
            scope_id: 1,
            scope_kind: ExecutionScopeKind::Dataset,
            flags: 0,
            stable_id: b"people".to_vec(),
            display_name: "people".into(),
            private_payload_ref: u32::MAX,
        }
        .serialize()
        .unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::CodeSpaceDescriptor as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: CodeSpaceDescriptorV1 {
            code_space_id: 1,
            namespace: "org.example.coveql".into(),
            stable_id: b"exec-codes".to_vec(),
            epoch: 1,
            flags: 0,
            private_payload_ref: u32::MAX,
        }
        .serialize()
        .unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::EngineMountPolicy as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data: EngineMountPolicyV1 {
            policy_id: 1,
            filecode_mapping_kind: FileCodeMappingKind::MapToExecutionCode,
            missing_value_policy: MissingValuePolicy::Error,
            stale_mapping_policy: StaleMappingPolicy::Reject,
            reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
            flags: 0,
            dictionary_digest_ref: 0,
            code_space_ref: 1,
            cache_key_ref: u32::MAX,
            private_payload_ref: u32::MAX,
            checksum: 0,
        }
        .serialize()
        .to_vec(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    (writer.write().unwrap(), execution_map)
}

fn file_dictionary_key_from_canonical(value: CanonicalValue<'_>) -> FileDictionaryKey {
    FileDictionaryKey {
        value_tag: value.value_tag() as u16,
        canonical: value.encode().unwrap(),
    }
}

fn temporal_segment_entry_for_rows(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
    length: u64,
) -> TemporalSegmentIndexEntryV1 {
    temporal_segment_entry_for_object_type_rows(segment_id, 1, rows, length)
}

fn temporal_segment_entry_for_object_type_rows(
    segment_id: u32,
    object_type_id: u32,
    rows: &[TemporalRowEntryV1],
    length: u64,
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
        length,
        checksum: 0,
    }
}

fn temporal_row_kind_counts(rows: &[TemporalRowEntryV1]) -> (u32, u32, u32, u32) {
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

fn temporal_segment_with_bool_property(rows: &[TemporalRowEntryV1], values: &[bool]) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values.iter().map(|value| u8::from(*value)).collect();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        logical_type: CoveLogicalType::Bool,
        physical_kind: CovePhysicalKind::Boolean,
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
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_nullable_bool_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<bool>],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut null_count = 0u32;
    let value_bytes = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if value.is_none() {
                null_count += 1;
                null_bitmap[index / 8] |= 1 << (index % 8);
            }
            u8::from(value.unwrap_or(false))
        })
        .collect();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        logical_type: CoveLogicalType::Bool,
        physical_kind: CovePhysicalKind::Boolean,
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
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_association_endpoints(
    rows: &[TemporalRowEntryV1],
    sources: &[[u8; 16]],
    targets: &[[u8; 16]],
) -> Vec<u8> {
    assert_eq!(rows.len(), sources.len());
    assert_eq!(rows.len(), targets.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let column_directory_length = 2 * TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_offset = column_directory_offset + column_directory_length;
    let page_index_length = 2 * cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let source_value_bytes = sources
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let target_value_bytes = targets
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let source_payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        source_value_bytes,
    )
    .unwrap();
    let target_payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        target_value_bytes,
    )
    .unwrap();
    let source_data_offset = data_offset;
    let target_data_offset = source_data_offset + source_payload.len() as u64;
    let header = TemporalSegmentHeaderV1 {
        segment_id: 8,
        object_type_id: 7,
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
        column_count: 2,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let source_directory = TableColumnDirectoryEntryV1 {
        column_id: 11,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
        flags: 0,
        page_index_offset,
        page_index_length: cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        data_offset: source_data_offset,
        data_length: source_payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let target_directory = TableColumnDirectoryEntryV1 {
        column_id: 12,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
        flags: 0,
        page_index_offset: page_index_offset + cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        page_index_length: cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
        data_offset: target_data_offset,
        data_length: target_payload.len() as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    };
    let source_page = ColumnPageIndexEntryV1 {
        column_id: 11,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: source_data_offset,
        page_length: source_payload.len() as u64,
        uncompressed_length: source_payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&source_payload),
    };
    let target_page = ColumnPageIndexEntryV1 {
        column_id: 12,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: target_data_offset,
        page_length: target_payload.len() as u64,
        uncompressed_length: target_payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&target_payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&source_directory.serialize());
    bytes.extend_from_slice(&target_directory.serialize());
    bytes.extend_from_slice(&source_page.serialize());
    bytes.extend_from_slice(&target_page.serialize());
    bytes.extend_from_slice(&source_payload);
    bytes.extend_from_slice(&target_payload);
    bytes
}

fn temporal_segment_with_nullable_utf8_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<&str>],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut value_bytes = Vec::new();
    let mut null_count = 0u32;
    for (index, value) in values.iter().enumerate() {
        if value.is_none() {
            null_count += 1;
            null_bitmap[index / 8] |= 1 << (index % 8);
        }
        let bytes = value.unwrap_or_default().as_bytes();
        value_bytes.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        value_bytes.extend_from_slice(bytes);
    }
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::VarBytes,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        physical_kind: CovePhysicalKind::VarBytes,
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
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::VarBytes as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_numcode_property(
    rows: &[TemporalRowEntryV1],
    values: &[i64],
    payload_padding: usize,
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values
        .iter()
        .flat_map(|value| (*value as u64).to_le_bytes())
        .collect::<Vec<_>>();
    let payload = aligned_numcode_page_payload(rows.len() as u32, value_bytes, payload_padding);
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        column_id: 3,
        logical_type: CoveLogicalType::Int64,
        physical_kind: CovePhysicalKind::NumCode,
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
        column_id: 3,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::NumCode as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_nullable_numcode_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<i64>],
    payload_padding: usize,
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut null_count = 0u32;
    let value_bytes = values
        .iter()
        .enumerate()
        .flat_map(|(index, value)| {
            if value.is_none() {
                null_count += 1;
                null_bitmap[index / 8] |= 1 << (index % 8);
            }
            (value.unwrap_or(0) as u64).to_le_bytes()
        })
        .collect::<Vec<_>>();
    let payload = aligned_numcode_page_payload_with_nulls(
        rows.len() as u32,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
        payload_padding,
    );
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        column_id: 3,
        logical_type: CoveLogicalType::Int64,
        physical_kind: CovePhysicalKind::NumCode,
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
        column_id: 3,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::NumCode as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_plain_fixed_uuid_property(
    rows: &[TemporalRowEntryV1],
    values: &[[u8; 16]],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values
        .iter()
        .flat_map(|value| value.iter().copied())
        .collect::<Vec<_>>();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        column_id: 4,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
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
        column_id: 4,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn temporal_segment_with_nullable_plain_fixed_uuid_property(
    rows: &[TemporalRowEntryV1],
    values: &[Option<[u8; 16]>],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut null_count = 0u32;
    let value_bytes = values
        .iter()
        .enumerate()
        .flat_map(|(index, value)| {
            if value.is_none() {
                null_count += 1;
                null_bitmap[index / 8] |= 1 << (index % 8);
            }
            value.unwrap_or([0; 16])
        })
        .collect::<Vec<_>>();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        (null_count > 0).then_some(null_bitmap),
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        column_id: 4,
        logical_type: CoveLogicalType::Uuid,
        physical_kind: CovePhysicalKind::FixedBytes,
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
        column_id: 4,
        morsel_id: 0,
        row_count: rows.len() as u32,
        non_null_count: rows.len() as u32 - null_count,
        null_count,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset: data_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    };

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn aligned_numcode_page_payload(row_count: u32, value_bytes: Vec<u8>, padding: usize) -> Vec<u8> {
    aligned_numcode_page_payload_with_nulls(row_count, None, value_bytes, padding)
}

fn aligned_numcode_page_payload_with_nulls(
    row_count: u32,
    null_bitmap: Option<Vec<u8>>,
    value_bytes: Vec<u8>,
    padding: usize,
) -> Vec<u8> {
    let buffer_count = 1 + usize::from(null_bitmap.is_some());
    let buffers_offset = COLUMN_PAGE_PAYLOAD_HEADER_LEN
        + COVE_ENCODING_NODE_LEN
        + buffer_count * PAGE_BUFFER_DESCRIPTOR_LEN;
    let null_offset = buffers_offset;
    let values_offset = null_offset + null_bitmap.as_ref().map_or(0, Vec::len) + padding;
    let header = ColumnPagePayloadHeaderV1 {
        magic: COLUMN_PAGE_PAYLOAD_MAGIC,
        version_major: COLUMN_PAGE_PAYLOAD_VERSION_MAJOR,
        header_len: COLUMN_PAGE_PAYLOAD_HEADER_LEN as u16,
        flags: 0,
        root_node_id: 0,
        node_count: 1,
        buffer_count: buffer_count as u16,
        row_count,
        nodes_offset: COLUMN_PAGE_PAYLOAD_HEADER_LEN as u32,
        buffer_directory_offset: (COLUMN_PAGE_PAYLOAD_HEADER_LEN + COVE_ENCODING_NODE_LEN) as u32,
        buffers_offset: buffers_offset as u32,
        reserved: 0,
    };
    let node = CoveEncodingNodeV1 {
        node_id: 0,
        encoding_kind: CoveEncodingKind::NumCode,
        logical_type: CoveLogicalType::Int64,
        physical_kind: CovePhysicalKind::NumCode,
        flags: 0,
        logical_len: row_count,
        child_count: 0,
        buffer_count: buffer_count as u16,
        params_offset: 0,
        params_length: 0,
        stats_id: u32::MAX,
        reserved: 0,
    };
    let mut descriptors = Vec::with_capacity(buffer_count);
    if let Some(null_bitmap) = &null_bitmap {
        descriptors.push(PageBufferDescriptorV1 {
            buffer_id: 0,
            kind: PageBufferKind::NullBitmap,
            flags: 0,
            offset: null_offset as u64,
            length: null_bitmap.len() as u64,
            checksum: checksum::crc32c(null_bitmap),
            reserved: 0,
        });
    }
    descriptors.push(PageBufferDescriptorV1 {
        buffer_id: descriptors.len() as u16,
        kind: PageBufferKind::Values,
        flags: 0,
        offset: values_offset as u64,
        length: value_bytes.len() as u64,
        checksum: checksum::crc32c(&value_bytes),
        reserved: 0,
    });
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&header.serialize());
    bytes.extend_from_slice(&node.serialize());
    for descriptor in descriptors {
        bytes.extend_from_slice(&descriptor.serialize());
    }
    if let Some(null_bitmap) = &null_bitmap {
        bytes.extend_from_slice(null_bitmap);
    }
    bytes.extend(std::iter::repeat(0).take(padding));
    bytes.extend_from_slice(&value_bytes);
    bytes
}

fn temporal_segment_with_filecode_property(
    logical_type: CoveLogicalType,
    rows: &[TemporalRowEntryV1],
    values: &[u32],
) -> Vec<u8> {
    assert_eq!(rows.len(), values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let page_index_offset = column_directory_offset + TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let data_offset = page_index_offset + page_index_length;
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let payload = ColumnPagePayloadV1::build_single_node(
        rows.len() as u32,
        CoveEncodingKind::FileCode,
        logical_type,
        CovePhysicalKind::FileCode,
        None,
        value_bytes,
    )
    .unwrap();
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
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
        column_id: 2,
        logical_type,
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
        column_id: 2,
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

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    bytes.extend_from_slice(&directory.serialize());
    bytes.extend_from_slice(&page.serialize());
    bytes.extend_from_slice(&payload);
    bytes
}

fn object_property_count_covi(bytes: &[u8], row_count: u64) -> Vec<u8> {
    object_property_index_only_covi(bytes, row_count, &[CoviAggregateKindV2::Count])
}

fn object_property_index_only_covi(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
) -> Vec<u8> {
    object_property_index_only_covi_with_target(
        bytes,
        row_count,
        aggregate_kinds,
        IndexOnlyTestTarget {
            object_type_id: 1,
            property_id: 1,
            logical_type: CoveLogicalType::Bool,
            physical_kind: CovePhysicalKind::Boolean,
            aggregate_payloads: Vec::new(),
        },
    )
}

fn object_metric_index_only_covi(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
    sum: i64,
) -> Vec<u8> {
    let mut aggregate_payloads = Vec::new();
    if aggregate_kinds.contains(&CoviAggregateKindV2::Sum) {
        aggregate_payloads.push((CoviAggregateKindV2::Sum, sum.to_le_bytes().to_vec()));
    }
    if aggregate_kinds.contains(&CoviAggregateKindV2::Avg) {
        aggregate_payloads.push((CoviAggregateKindV2::Avg, sum.to_le_bytes().to_vec()));
    }
    object_property_index_only_covi_with_target(
        bytes,
        row_count,
        aggregate_kinds,
        IndexOnlyTestTarget {
            object_type_id: 1,
            property_id: 3,
            logical_type: CoveLogicalType::Int64,
            physical_kind: CovePhysicalKind::NumCode,
            aggregate_payloads,
        },
    )
}

struct IndexOnlyTestTarget {
    object_type_id: u32,
    property_id: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    aggregate_payloads: Vec<(CoviAggregateKindV2, Vec<u8>)>,
}

fn object_property_index_only_covi_with_target(
    bytes: &[u8],
    row_count: u64,
    aggregate_kinds: &[CoviAggregateKindV2],
    target: IndexOnlyTestTarget,
) -> Vec<u8> {
    let context = build_operation_context(
        bytes,
        CoveQlOperationRequest::default(),
        validation_options(),
    )
    .unwrap();
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
        index_kind: CoviIndexKindV2::Sorted,
        coverage_granularity: 0,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: 0,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        object_type_id: target.object_type_id,
        property_id: target.property_id,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: target.logical_type as u16,
        physical_kind: target.physical_kind as u8,
        key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes as u8,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: row_count,
        distinct_count: 0,
        null_count: 0,
        min_key_ref: u32::MAX,
        max_key_ref: u32::MAX,
        key_block_section_id: 1,
        entry_block_section_id: 2,
        postings_block_section_id: 3,
        aggregate_block_section_id: 4,
        coverage_set_ref: u32::MAX,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 1,
        supports_range: 0,
        supports_membership: 0,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::Count)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Exists)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Avg),
        ),
        supports_min: 0,
        supports_max: 0,
        supports_sum: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::Sum)
                || aggregate_kinds.contains(&CoviAggregateKindV2::Avg),
        ),
        supports_distinct_count: u8::from(
            aggregate_kinds.contains(&CoviAggregateKindV2::DistinctCount),
        ),
        supports_join_coverage: 0,
        supports_index_only: 1,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let index_only = aggregate_kinds
        .iter()
        .map(|kind| IndexOnlyCapabilityV2 {
            capability_id: 0,
            aggregate_kind: *kind as u16,
            predicate_supported: 0,
            exactness: IndexCapabilityExactnessV2::Exact,
            null_semantics: 0,
            flags: 0,
            snapshot_validity_ref: 0,
            required_visibility_overlay_ref: u32::MAX,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    let key_block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: 1,
            index_root_id: 0,
            key_count: 0,
            encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: 0,
            checksum: 0,
        },
        key_data: Vec::new(),
    };
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: 2,
            index_root_id: 0,
            entry_count: 0,
            key_block_id: 1,
            postings_block_id: 3,
            aggregate_block_id: 4,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: 0,
            flags: 0,
            checksum: 0,
        },
        entries: Vec::new(),
    };
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: 3,
            index_root_id: 0,
            postings_count: 0,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: 0,
            flags: 0,
            checksum: 0,
        },
        postings: Vec::new(),
        row_ordinal_sets: Vec::new(),
        payload: Vec::new(),
    };
    let mut aggregate_payload = Vec::new();
    let aggregate_answers = aggregate_kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let value_ref = match kind {
                CoviAggregateKindV2::DistinctCount => {
                    let offset = u32::try_from(aggregate_payload.len()).unwrap();
                    aggregate_payload.extend_from_slice(&0u64.to_le_bytes());
                    offset
                }
                CoviAggregateKindV2::Sum | CoviAggregateKindV2::Avg => {
                    let payload = target
                        .aggregate_payloads
                        .iter()
                        .find(|(payload_kind, _)| payload_kind == kind)
                        .map(|(_, payload)| payload)
                        .expect("sum/avg test sidecar requires a sum payload");
                    let offset = u32::try_from(aggregate_payload.len()).unwrap();
                    aggregate_payload.extend_from_slice(payload);
                    offset
                }
                _ => u32::MAX,
            };
            CoviAggregateAnswerV2 {
                aggregate_answer_ref: index as u32,
                index_root_id: 0,
                aggregate_kind: *kind as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count,
                null_count: 0,
                non_null_count: row_count,
                value_ref,
                predicate_form_ref: u32::MAX,
                snapshot_validity_ref: 0,
                checksum: 0,
            }
        })
        .collect::<Vec<_>>();
    let aggregate_block = CoviAggregateAnswerBlockV2 {
        header: CoviAggregateAnswerBlockHeaderV2 {
            magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
            aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
            aggregate_block_id: 4,
            index_root_id: 0,
            aggregate_answer_count: aggregate_answers.len() as u32,
            aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
            aggregate_payload_offset: 0,
            aggregate_payload_length: aggregate_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        answers: aggregate_answers,
        payload: aggregate_payload,
    };
    let index_only_payload = index_only
        .iter()
        .flat_map(|capability| capability.serialize().unwrap())
        .collect::<Vec<_>>();
    CoviArtifactV2::serialize_with_sections(
        [1; 16],
        [2; 16],
        &[CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: context.file.file_id,
            file_len: context.file.file_len,
            footer_crc32c: context.file.footer_crc32c,
            digest_algorithm: DigestAlgorithm::None as u16,
            digest_len: 0,
            digest_offset: 0,
            uri_ref: u32::MAX,
            schema_fingerprint_ref: u32::MAX,
            checksum: 0,
        }],
        &[CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id: [1; 16],
            snapshot_id: [2; 16],
            schema_fingerprint_ref: u32::MAX,
            semantic_map_fingerprint_ref: u32::MAX,
            external_visibility_ref: u32::MAX,
            data_checksum_root_ref: u32::MAX,
            delta_chain_digest_algorithm: DigestAlgorithm::None as u16,
            delta_chain_digest_len: 0,
            delta_chain_digest_offset: 0,
            valid_from_us: 0,
            valid_until_us: i64::MAX,
            flags: 0,
            checksum: 0,
        }],
        &[root],
        &[capability],
        &[
            CoviSectionPayloadV2 {
                section_id: 1,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: key_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 2,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: entry_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 3,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: postings_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 4,
                section_kind: CoviSectionKindV2::AggregateAnswerBlock,
                payload: aggregate_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 5,
                section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
                payload: index_only_payload,
                item_count: aggregate_kinds.len() as u64,
                required_features: 0,
                optional_features: 0,
            },
        ],
    )
    .unwrap()
}

fn object_property_bool_lookup_covi(
    bytes: &[u8],
    type_name: &str,
    property_name: &str,
    value: bool,
) -> Vec<u8> {
    let context = build_operation_context(
        bytes,
        CoveQlOperationRequest::default(),
        validation_options(),
    )
    .unwrap();
    let surface = read_object_surface_from_bytes(bytes).unwrap();
    let object_type = surface
        .object_types
        .iter()
        .find(|object_type| object_type.type_name == type_name)
        .unwrap();
    let property = object_type
        .properties
        .iter()
        .find(|property| property.property_name == property_name)
        .unwrap();
    let mut key_data = Vec::new();
    wire::append_u64_leb128(
        &mut key_data,
        CanonicalValue::Bool(value).value_tag() as u64,
    );
    key_data.extend_from_slice(&CanonicalValue::Bool(value).encode().unwrap());
    let row_ranges = surface
        .records
        .iter()
        .filter(|record| record.object_type_id == object_type.object_type_id)
        .filter(|record| {
            record.properties.iter().any(|record_property| {
                record_property.property_id == property.property_id
                    && record_property.value == json!(value)
            })
        })
        .map(|record| CoviRowRangePostingV2 {
            file_ref: 0,
            table_id: u32::MAX,
            segment_id: record.segment_id,
            morsel_id: 0,
            row_start: u64::from(record.row_index),
            row_count: 1,
            flags: 0,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    assert!(!row_ranges.is_empty());
    let postings_payload = row_ranges
        .iter()
        .flat_map(|range| range.serialize().unwrap())
        .collect::<Vec<_>>();
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: CoviIndexedTargetKindV2::ObjectProperty,
        index_kind: CoviIndexKindV2::Sorted,
        coverage_granularity: 0,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: IndexCapabilityExactnessV2::Exact as u8,
        flags: 0,
        table_id: u32::MAX,
        column_id: u32::MAX,
        object_type_id: object_type.object_type_id,
        property_id: property.property_id,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: CoveLogicalType::Bool as u16,
        physical_kind: CovePhysicalKind::Boolean as u8,
        key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes as u8,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: row_ranges.len() as u64,
        distinct_count: 1,
        null_count: 0,
        min_key_ref: 0,
        max_key_ref: 0,
        key_block_section_id: 1,
        entry_block_section_id: 2,
        postings_block_section_id: 3,
        aggregate_block_section_id: u32::MAX,
        coverage_set_ref: u32::MAX,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 1,
        supports_range: 0,
        supports_membership: 1,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 0,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 0,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: 0,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let key_block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: 1,
            index_root_id: 0,
            key_count: 1,
            encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: key_data.len() as u64,
            checksum: 0,
        },
        key_data: key_data.clone(),
    };
    let entry = CoviIndexEntryV2 {
        entry_ref: 0,
        index_root_id: 0,
        entry_id: 0,
        key_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
        flags: 0,
        key_offset: 0,
        key_length: key_data.len() as u32,
        key_hash64: 0,
        postings_ref: 0,
        coverage_set_ref: u32::MAX,
        aggregate_answer_ref: u32::MAX,
        next_duplicate_ref: u32::MAX,
        checksum: 0,
    };
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: 2,
            index_root_id: 0,
            entry_count: 1,
            key_block_id: 1,
            postings_block_id: 3,
            aggregate_block_id: u32::MAX,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: CoviIndexEntryV2::LEN as u64,
            flags: 0,
            checksum: 0,
        },
        entries: vec![entry],
    };
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: 3,
            index_root_id: 0,
            postings_count: 1,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: postings_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        postings: vec![CoviPostingsHeaderV2 {
            postings_ref: 0,
            index_root_id: 0,
            representation: cove_index::CoviPostingRepresentationV2::RowRangeList,
            target_granularity: 0,
            flags: 0,
            item_count: row_ranges.len() as u64,
            payload_offset: 0,
            payload_length: postings_payload.len() as u64,
            coverage_set_ref: u32::MAX,
            checksum: 0,
        }],
        row_ordinal_sets: Vec::new(),
        payload: postings_payload,
    };
    CoviArtifactV2::serialize_with_sections(
        [1; 16],
        [2; 16],
        &[CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: context.file.file_id,
            file_len: context.file.file_len,
            footer_crc32c: context.file.footer_crc32c,
            digest_algorithm: DigestAlgorithm::None as u16,
            digest_len: 0,
            digest_offset: 0,
            uri_ref: u32::MAX,
            schema_fingerprint_ref: u32::MAX,
            checksum: 0,
        }],
        &[CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id: [1; 16],
            snapshot_id: [2; 16],
            schema_fingerprint_ref: u32::MAX,
            semantic_map_fingerprint_ref: u32::MAX,
            external_visibility_ref: u32::MAX,
            data_checksum_root_ref: u32::MAX,
            delta_chain_digest_algorithm: DigestAlgorithm::None as u16,
            delta_chain_digest_len: 0,
            delta_chain_digest_offset: 0,
            valid_from_us: 0,
            valid_until_us: i64::MAX,
            flags: 0,
            checksum: 0,
        }],
        &[root],
        &[capability],
        &[
            CoviSectionPayloadV2 {
                section_id: 1,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: key_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 2,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: entry_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 3,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: postings_block.serialize().unwrap(),
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
        ],
    )
    .unwrap()
}

fn object_property_bool_coverage(
    bytes: &[u8],
    type_name: &str,
    property_name: &str,
    value: bool,
    predicate_form_ref: u32,
) -> (Vec<u8>, Vec<u8>) {
    let surface = read_object_surface_from_bytes(bytes).unwrap();
    let object_type = surface
        .object_types
        .iter()
        .find(|object_type| object_type.type_name == type_name)
        .unwrap();
    let property = object_type
        .properties
        .iter()
        .find(|property| property.property_name == property_name)
        .unwrap();
    let entries = surface
        .records
        .iter()
        .filter(|record| record.object_type_id == object_type.object_type_id)
        .filter(|record| {
            record.properties.iter().any(|record_property| {
                record_property.property_id == property.property_id
                    && record_property.value == json!(value)
            })
        })
        .map(|record| CoverageSetEntryV2 {
            target_kind: CoverageGranularityV2::RowRange,
            flags: 0,
            file_ref: 0,
            table_id: 0,
            segment_id: record.segment_id,
            morsel_id: u32::MAX,
            page_ref: u32::MAX,
            object_type_id: record.object_type_id,
            path_ref: u32::MAX,
            dimensional_bucket_ref: u32::MAX,
            row_start: u64::from(record.row_index),
            row_count: 1,
            row_ordinal_bitmap_ref: u32::MAX,
            byte_range_ref: u32::MAX,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    assert!(!entries.is_empty());
    let set = CoverageSetV2 {
        header: CoverageSetHeaderV2 {
            coverage_set_id: 1,
            provider_id: 1,
            granularity: CoverageGranularityV2::RowRange,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            predicate_form_ref,
            snapshot_validity_ref: 1,
            total_fragment_count: surface.records.len() as u64,
            covered_fragment_count: entries.len() as u64,
            required_fragment_count_estimate: entries.len() as u64,
            coverage_degree_ppm: 1_000_000,
            tightness_degree_ppm: 1_000_000,
            entries_offset: CoverageSetHeaderV2::LEN as u64,
            entries_length: (entries.len() * CoverageSetEntryV2::LEN) as u64,
            checksum: 0,
        },
        entries,
    };
    let set_bytes = set.serialize().unwrap();
    let proof = CoverageProofRecordV2 {
        proof_id: 1,
        provider_id: 1,
        coverage_set_id: 1,
        predicate_form_ref,
        proof_kind: CoverageProofKindV2::ExactSet,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::RowRange,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref: 1,
        coverage_set_checksum: coverage_set_payload_checksum(&set_bytes),
        proof_payload_ref: u32::MAX,
        checksum: 0,
    };
    (set_bytes, proof.serialize().unwrap().to_vec())
}

fn minimal_map_file() -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::SemanticMapping as u8;
    writer.required_features = FEATURE_SEMANTIC_MAP;
    writer.write().unwrap()
}

fn minimal_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn association_file_with_evidence_entries(evidence_entries: Vec<Value>) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 7,
            type_name: "CustomerPlacedOrder".into(),
            flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 11,
                    property_name: "source_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                },
                PropertyEntryV1 {
                    property_id: 12,
                    property_name: "target_goid".into(),
                    logical_type: CoveLogicalType::Uuid,
                    physical_kind: CovePhysicalKind::FixedBytes,
                    nullable: false,
                    collation_id: 0,
                    flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                },
            ],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    let assertions = evidence_entries
        .iter()
        .map(|entry| {
            json!({
                "assertion_id": entry
                    .get("assertion_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has assertion id"),
                "output_object_id": entry
                    .get("output_object_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has output object id"),
            })
        })
        .collect::<Vec<_>>();
    let output_assertion_ids = assertions
        .iter()
        .filter_map(|assertion| assertion.get("assertion_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.orders",
                "schema_fingerprint": "orders-schema-v1",
                "snapshot_digest": "orders-digest-v1",
                "row_identity_rules": ["order_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "customer_order_identity",
                "object_type": "CustomerPlacedOrder",
                "semantic_role": "association",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "order_id",
                    "source_column": "order_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "link_customer_order",
                "source_id": "crm.orders",
                "identity_rule_id": "customer_order_identity",
                "row_semantics_kind": "AssociationOnly",
                "assertion_kinds": ["association", "evidence"],
                "function_ids": [],
                "output_assertion_ids": output_assertion_ids,
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "assertions": assertions,
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "association-map",
            "mapping_version": "2026.05",
            "entries": evidence_entries,
        }),
    );
    writer.write().unwrap()
}

fn minimal_object_with_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
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
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn object_file_with_person_and_association_record() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
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
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 10,
        csn: 1,
        branch_key: 0,
        goid: [0; 16],
        record_id: [32; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let association_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 2,
        branch_key: 0,
        goid: [7; 16],
        record_id: [40; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true]);
    let association_segment =
        temporal_segment_with_association_endpoints(&association_rows, &[[0; 16]], &[[2; 16]]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 2,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn object_file_with_two_person_association_record() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
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
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 11,
            csn: 2,
            branch_key: 0,
            goid: [2; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let association_rows = vec![TemporalRowEntryV1 {
        timestamp_us: 20,
        csn: 3,
        branch_key: 0,
        goid: [7; 16],
        record_id: [40; 16],
        record_kind: RecordKind::Baseline,
        prev_ref: None,
    }];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true, false]);
    let association_segment =
        temporal_segment_with_association_endpoints(&association_rows, &[[0; 16]], &[[2; 16]]);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 3,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn object_file_with_three_person_two_association_records() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
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
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    let person_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 10,
            csn: 1,
            branch_key: 0,
            goid: [0; 16],
            record_id: [32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 11,
            csn: 2,
            branch_key: 0,
            goid: [2; 16],
            record_id: [33; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 12,
            csn: 3,
            branch_key: 0,
            goid: [3; 16],
            record_id: [34; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let association_rows = vec![
        TemporalRowEntryV1 {
            timestamp_us: 20,
            csn: 4,
            branch_key: 0,
            goid: [7; 16],
            record_id: [40; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
        TemporalRowEntryV1 {
            timestamp_us: 21,
            csn: 5,
            branch_key: 0,
            goid: [8; 16],
            record_id: [41; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        },
    ];
    let person_segment = temporal_segment_with_bool_property(&person_rows, &[true, false, true]);
    let association_segment = temporal_segment_with_association_endpoints(
        &association_rows,
        &[[0; 16], [2; 16]],
        &[[2; 16], [3; 16]],
    );
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![
            temporal_segment_entry_for_object_type_rows(
                7,
                1,
                &person_rows,
                person_segment.len() as u64,
            ),
            temporal_segment_entry_for_object_type_rows(
                8,
                7,
                &association_rows,
                association_segment.len() as u64,
            ),
        ],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 2,
        row_count: 5,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: person_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: person_segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: association_rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: association_segment,
    });
    writer.write().unwrap()
}

fn minimal_filecode_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 2,
                property_name: "name".into(),
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_with_projection_file() -> Vec<u8> {
    minimal_object_with_projection_file_with_evidence_index(false)
}

fn minimal_object_with_projection_and_evidence_index_file() -> Vec<u8> {
    minimal_object_with_projection_file_with_evidence_index(true)
}

fn minimal_object_with_projection_file_with_evidence_index(
    include_evidence_index: bool,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "customer-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [{
                "name": "active",
                "value": "property.active",
                "logical_type": "bool"
            }],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    if include_evidence_index {
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "sources": [{
                    "source_id": "crm.customers",
                    "schema_fingerprint": "schema-v1",
                    "snapshot_digest": "digest-v1",
                    "row_identity_rules": ["customer_id"],
                    "replay_claimed": true
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "identity_rules": [{
                    "rule_id": "projection_identity",
                    "object_type": "Person",
                    "semantic_role": "projection_row",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "function_ids": [],
                    "join_keys": [{
                        "role_id": "customer_id",
                        "source_column": "customer_id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "asc"
                    }]
                }],
                "do_not_merge": []
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "rules": [{
                    "rule_id": "projection_people",
                    "source_id": "crm.customers",
                    "identity_rule_id": "projection_identity",
                    "row_semantics_kind": "ProjectionOnly",
                    "assertion_kinds": ["projection", "evidence"],
                    "function_ids": [],
                    "output_assertion_ids": ["assert_people_projection_row"],
                    "association_endpoints": []
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapAssertionLog,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "assertions": [{
                    "assertion_id": "assert_people_projection_row",
                    "output_object_id": "projection:people:1"
                }]
            }),
        );
        push_embedded_map_section(
            &mut writer,
            SectionKind::MapEvidenceIndex,
            json!({
                "mapping_id": "customer-map",
                "mapping_version": "2026.05",
                "entries": [{
                    "source_id": "crm.customers",
                    "source_row_identity": "customer_id=1",
                    "rule_id": "projection_people",
                    "assertion_id": "assert_people_projection_row",
                    "output_object_id": "projection:people:1",
                    "operation_target": "projection"
                }]
            }),
        );
    }
    writer.write().unwrap()
}

fn minimal_object_with_two_column_projection_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
                PropertyEntryV1 {
                    property_id: 2,
                    property_name: "enabled".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "customer-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "people_projection",
            "output_table": "people_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Person"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [
                {
                    "name": "active",
                    "value": "property.active",
                    "logical_type": "bool"
                },
                {
                    "name": "enabled",
                    "value": "property.enabled",
                    "logical_type": "bool"
                }
            ],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).unwrap(),
    });
    writer.write().unwrap()
}

fn minimal_object_with_evidence_index_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v1",
                "snapshot_digest": "digest-v1",
                "row_identity_rules": ["customer_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "person_identity",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "upsert_person",
                "source_id": "crm.customers",
                "identity_rule_id": "person_identity",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": [],
                "output_assertion_ids": ["assert_person"],
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "assertions": [{
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1"
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": [{
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1",
                "observed_schema_fingerprint": "schema-v1",
                "observed_snapshot_digest": "digest-v1",
                "operation_target": "object",
                "object_type": "Person"
            }]
        }),
    );
    writer.write().unwrap()
}

fn person_file_with_bool_records_and_evidence_entries(
    values: &[bool],
    evidence_entries: Vec<Value>,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
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
        }],
    };
    let rows = values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: [index as u8; 16],
            record_id: [index as u8 + 32; 16],
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_property(&rows, values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().unwrap(),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    let assertions = evidence_entries
        .iter()
        .map(|entry| {
            json!({
                "assertion_id": entry
                    .get("assertion_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has assertion id"),
                "output_object_id": entry
                    .get("output_object_id")
                    .and_then(Value::as_str)
                    .expect("test evidence entry has output object id"),
            })
        })
        .collect::<Vec<_>>();
    let output_assertion_ids = assertions
        .iter()
        .filter_map(|assertion| assertion.get("assertion_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v1",
                "snapshot_digest": "digest-v1",
                "row_identity_rules": ["customer_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "person_identity",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "upsert_person",
                "source_id": "crm.customers",
                "identity_rule_id": "person_identity",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": [],
                "output_assertion_ids": output_assertion_ids,
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "assertions": assertions,
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": evidence_entries,
        }),
    );
    writer.write().unwrap()
}

fn push_embedded_map_section(
    writer: &mut MinimalCoveWriter,
    section_kind: SectionKind,
    mut value: Value,
) {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".into(),
            Value::String("org.coveformat.covemap.v2".into()),
        );
        object.insert(
            "section_id".into(),
            Value::Number((section_kind as u16).into()),
        );
    }
    writer.sections.push(SectionPayload {
        section_kind: section_kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&value).unwrap(),
    });
}

#[path = "operation_context/manifest.rs"]
mod manifest;

#[path = "operation_context/parser_logical.rs"]
mod parser_logical;

#[path = "operation_context/association_evidence.rs"]
mod association_evidence;
#[path = "operation_context/datafusion_integration.rs"]
mod datafusion_integration;
#[path = "operation_context/execution_materialized.rs"]
mod execution_materialized;
#[path = "operation_context/kernel_native.rs"]
mod kernel_native;
#[path = "operation_context/physical_planning.rs"]
mod physical_planning;

#[test]
fn directionless_object_relative_association_rejects_when_endpoint_role_is_ambiguous() {
    let err = parse_and_resolve_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(association(CustomerPlacedOrder))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_ASSOCIATION_ROLE");
}

#[test]
fn association_role_from_and_to_disambiguate_object_relative_associations() {
    for query in [
        "Person.where(exists(association(CustomerPlacedOrder, from: customer))).select(active)",
        "Person.where(exists(association(CustomerPlacedOrder, to: order))).select(active)",
    ] {
        parse_and_resolve_query(
            &minimal_object_with_association_file(),
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            validation_options(),
        )
        .unwrap();
    }
}

#[test]
fn free_form_association_role_rejects_without_unique_metadata_proof() {
    let err = parse_and_resolve_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(association(CustomerPlacedOrder, role: customer))).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_ASSOCIATION_ROLE");
}

#[test]
fn association_valid_time_resolves_without_commit_time_cut() {
    let planned = parse_resolve_and_plan_query(
        &minimal_association_file(),
        "association(CustomerPlacedOrder).asOf(association_valid_time: \"2026-01-01T00:00:00Z\").select(source_goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        planned.resolved.temporal.role,
        TemporalRole::AssociationValidTime
    );
}

#[test]
fn source_event_time_without_binding_rejects_when_no_unambiguous_property() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(source_event_time: \"2026-01-01T00:00:00Z\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_TEMPORAL_ROLE");
}

#[test]
fn source_event_time_infers_event_time_property_binding() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        planned.resolved.temporal.role,
        TemporalRole::SourceEventTime
    );
    assert_eq!(
        planned.resolved.temporal.role_binding.as_deref(),
        Some("event_time")
    );
    assert!(planned
        .dependencies
        .temporal_role_bindings
        .contains("event_time"));

    let executed = execute_planned_query(&bytes, planned, ExecutionOptions::default()).unwrap();
    let CoveQlExecutionResult::JsonRows(rows) = executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"event_time": 1}), json!({"event_time": 2})]
    );
}

#[test]
fn kernel_source_event_time_as_of_direct_projection_executes_with_exact_native_authority() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let query =
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#;

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(!kernel.executed.authority.residual_required);
    assert!(kernel.kernel_report.compared_with_materialized);
    assert!(kernel.executed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "W_KERNEL_NATIVE_DIRECT_PROJECTION_EXECUTED"
            && diagnostic.safe_details["root_kind"] == json!("object")
            && diagnostic.safe_details["rows_projected"] == json!(2)
            && diagnostic.safe_details["residual_verification"] == json!(false)
    }));
    let contracts = kernel.kernel_report.decision.safe_details["kernel_shape"]
        ["operator_contracts"]
        .as_array()
        .expect("kernel shape reports coded operator contracts");
    let temporal_contract = contracts
        .iter()
        .find(|contract| contract["operator"] == "temporal_role_bound_as_of")
        .expect("role-bound temporal contract is present");
    assert_eq!(temporal_contract["exact"], json!(true));
    assert_eq!(temporal_contract["residual_required"], json!(false));
    assert_eq!(temporal_contract["fallback_boundary"], json!(null));
    let CoveQlExecutionResult::JsonRows(rows) = kernel.executed.result else {
        panic!("expected JSON rows");
    };
    assert_eq!(
        rows,
        vec![json!({"event_time": 1}), json!({"event_time": 2})]
    );
}

#[test]
fn kernel_source_event_time_as_of_direct_projection_can_return_arrow_batches() {
    let (bytes, _) = object_file_with_timestamp_filecode_records(&[1, 2, 3]);
    let query =
        r#"EventThing.asOf(source_event_time: "1970-01-01T00:00:00.000002Z").select(event_time)"#;
    let resolve_options = ResolveOptions {
        output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
            zero_copy_requested: false,
        }),
        ..json_resolve_options()
    };

    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options.clone(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .unwrap();
    let materialized = parse_resolve_plan_and_execute_query(
        &bytes,
        query,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        kernel.executed.output_fingerprint,
        materialized.output_fingerprint
    );
    assert!(!kernel.executed.authority.residual_required);
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = &kernel.executed.result else {
        panic!("expected Arrow batches");
    };
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.schema().field(0).name(), "event_time");
    assert_eq!(batch.schema().field(0).data_type(), &DataType::Int64);
    let values = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("event_time is int64");
    assert_eq!(values.value(0), 1);
    assert_eq!(values.value(1), 2);
}

#[test]
fn source_event_time_resolves_with_explicit_binding() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.asOf(source_event_time: \"2026-01-01T00:00:00Z\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            temporal_role_bindings: BTreeMap::from([(
                TemporalRole::SourceEventTime,
                "source_event_time".into(),
            )]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap();

    assert_eq!(resolved.temporal.role, TemporalRole::SourceEventTime);
}

#[test]
fn branch_string_selector_resolves_through_aliases() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            branch_aliases: BTreeMap::from([("main".into(), 7)]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap();

    assert_eq!(
        resolved.branch.selector,
        coveql::BranchSelector::BranchKey(7)
    );
}

#[test]
fn missing_branch_alias_rejects_named_selector() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNKNOWN_BRANCH");
}

#[test]
fn ambiguous_branch_alias_rejects_named_selector() {
    let err = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.branch(\"main\").select(active)",
        ParseOptions::default(),
        ResolveOptions {
            ambiguous_branch_aliases: BTreeMap::from([("main".into(), vec![1, 2])]),
            ..ResolveOptions::default()
        },
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_AMBIGUOUS_BRANCH");
}

#[test]
fn logical_plan_grouping_rejects_ungrouped_raw_fields() {
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.groupBy(active).select(goid)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();

    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_GROUPING");
}

#[test]
fn logical_plan_accepts_grouped_fields_and_aggregates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.groupBy(active).select(active, n: count(*))",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::Aggregate { .. })));
}

#[test]
fn logical_plan_rejects_aggregates_when_disclosure_policy_rejects() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::Reject;
    let resolved = parse_and_resolve_query(
        &minimal_object_file(),
        "Person.select(n: count(*))",
        ParseOptions::default(),
        resolve_options,
        validation_options(),
    )
    .unwrap();

    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_AGGREGATE_DISCLOSURE_FORBIDDEN");
}

#[test]
fn logical_plan_rejects_aggregate_filters() {
    let err = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(count(*) > 0).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap_err();

    assert_eq!(err.diagnostics[0].code, "E_UNSUPPORTED_AGGREGATE_FILTER");
}

#[test]
fn logical_plan_explicit_ordering_defaults_nulls_by_direction() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).orderBy(active, desc)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();
    assert_eq!(sort[0].nulls, coveql::AstNullOrdering::NullsFirst);
    assert!(!planned.logical_plan.default_ordering_applied);
}

#[test]
fn logical_plan_rejects_non_string_filecode_ordering() {
    let (bytes, _) = object_file_with_bool_filecode_records(&[true, false]);
    let resolved = parse_and_resolve_query(
        &bytes,
        "FlagThing.select(active).orderBy(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        validation_options(),
    )
    .unwrap();
    let err = coveql::build_logical_plan(resolved, PlanOptions::default()).unwrap_err();
    assert_eq!(err.diagnostics[0].code, "E_UNSAFE_CODE_ORDERING");
}

#[test]
fn logical_plan_accepts_filecode_ordering_with_default_string_collation() {
    let (bytes, _) = object_file_with_filecode_records(&["Zoë", "Ada", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name).orderBy(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();

    assert_eq!(sort[0].representation, RepresentationClass::DecodeBoundary);
    assert_eq!(sort[0].collation_id, Some(CollationKind::None.id()));
}

#[test]
fn logical_plan_accepts_filecode_ordering_with_declared_collation_contract() {
    let (bytes, _) = object_file_with_filecode_records_with_collation(
        &["Zoë", "Ada", "Bob"],
        CollationKind::Utf8Bytewise.id(),
    );
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.select(name).orderBy(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let sort = planned
        .logical_plan
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            LogicalPlanNodeKind::Sort { keys, .. } => Some(keys),
            _ => None,
        })
        .unwrap();

    assert_eq!(sort[0].representation, RepresentationClass::DecodeBoundary);
    assert_eq!(sort[0].collation_id, Some(CollationKind::Utf8Bytewise.id()));
}

#[test]
fn logical_plan_marks_function_predicates_as_residual_boundaries() {
    let (bytes, _) = object_file_with_filecode_records(&["Ada", "Bob"]);
    let planned = parse_resolve_and_plan_query(
        &bytes,
        "Person.where(startsWith(name, \"A\")).select(name)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(!planned.logical_plan.residual_predicates.is_empty());
}

#[test]
fn logical_plan_marks_coded_safe_function_predicates_as_exact_pre_reconstruction() {
    let bytes = object_file_with_nullable_name_records_and_function_registry(
        &[Some("Ada"), Some("Åsa"), None],
        &["startsWith", "length", "lower"],
    );
    for query in [
        r#"Person.where(startsWith(name, "A")).select(name)"#,
        "Person.where(length(name) == 3).select(name)",
        r#"Person.where(lower(name) == "åsa").select(name)"#,
    ] {
        let planned = parse_resolve_and_plan_query(
            &bytes,
            query,
            ParseOptions::default(),
            ResolveOptions::default(),
            PlanOptions::default(),
            validation_options(),
        )
        .unwrap();

        assert!(
            planned.logical_plan.residual_predicates.is_empty(),
            "{query} should not leave residual function predicates"
        );
        assert!(
            planned
                .logical_plan
                .predicate_forms
                .iter()
                .any(|form| serde_json::to_value(form.placement).unwrap()
                    == json!("pre_reconstruction")
                    && form.representation.exact
                    && form.residual_reason.is_none()
                    && serde_json::to_value(form.representation.proof_state).unwrap()
                        == json!("proven_exact")),
            "{query} should report a proven-exact pre-reconstruction function predicate"
        );
    }
}

#[test]
fn logical_plan_json_redacts_protected_details_by_default() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json().to_string();
    assert!(explain.contains("<redacted>"));
    assert!(!explain.contains("\"active\""));
    assert_eq!(
        planned.explain_json()["fingerprints"]["logical_plan"],
        planned.logical_plan_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["predicate_ast"],
        planned.predicate_ast_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["predicate_cnf"],
        planned.predicate_cnf_fingerprint
    );
    assert_eq!(
        planned.explain_json()["fingerprints"]["projection_dependency"],
        planned.projection_dependency_fingerprint
    );
    assert_eq!(planned.predicate_ast_fingerprint.len(), 64);
    assert_eq!(planned.predicate_cnf_fingerprint.len(), 64);
    assert_eq!(planned.projection_dependency_fingerprint.len(), 64);
}

#[test]
fn logical_plan_json_can_disclose_protected_details_when_allowed() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    assert!(explain.to_string().contains("active"));
    assert!(explain["predicate_forms"]
        .as_array()
        .unwrap()
        .iter()
        .all(|form| form["representation"]["contract_version"]
            == coveql::PREDICATE_REPRESENTATION_CONTRACT_VERSION));
}

#[test]
fn stable_explain_json_has_ordered_top_level_schema() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    let keys = explain
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            "schema_version",
            "mode",
            "coveql_version",
            "core_version",
            "primary_profile",
            "profiles",
            "profile_contracts",
            "root",
            "grain",
            "operation",
            "temporal_mode",
            "canonical_order",
            "visibility_applied",
            "redaction_applied",
            "fingerprints",
            "operation_context",
            "logical_plan",
            "physical_plan",
            "resolved_dependencies",
            "predicate_forms",
            "trusted_metadata",
            "ignored_metadata",
            "fallbacks",
            "rejections",
            "decode_boundaries",
            "residual_predicates",
            "visibility",
            "redactions",
            "warnings",
            "diagnostics",
            "execution",
        ]
    );
    assert_eq!(explain["schema_version"], "0.1");
    assert_eq!(explain["coveql_version"], coveql::COVEQL_LANGUAGE_VERSION);
    assert_eq!(explain["core_version"], coveql::COVEQL_CORE_VERSION);
    assert_eq!(explain["primary_profile"], "object");
    assert_eq!(explain["profiles"], json!(["object"]));
    assert_eq!(explain["root"], "object");
    assert_eq!(explain["grain"], "latest_state");
    assert_eq!(explain["operation"], "explain_object");
    assert_eq!(explain["temporal_mode"], "latest");
    assert!(explain["canonical_order"]
        .as_array()
        .unwrap()
        .contains(&json!("goid")));
    assert_eq!(explain["physical_plan"], json!([]));
    assert!(explain["fingerprints"]["physical_plan"].is_null());
    assert!(explain["fingerprints"]["predicate_ast"].is_string());
    assert!(explain["fingerprints"]["predicate_cnf"].is_string());
    assert!(explain["fingerprints"]["projection_dependency"].is_string());
    assert_eq!(explain["execution"]["completed"], false);
    assert_eq!(explain["execution"]["kind"], "plan_explanation");
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reason"],
        "physical_plan_required"
    );
    assert!(explain["execution"]["coded_execution"]["operator_contracts"].is_array());
    assert!(
        explain["execution"]["coded_execution"]["operator_contracts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|contract| contract["contract_version"]
                == coveql::CODED_OPERATOR_CONTRACT_VERSION)
    );
}

#[test]
fn stable_explain_modes_are_policy_clamped() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::PublicOnly;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active).explain(\"forensic\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();

    assert_eq!(explain["mode"], "public");
    let diagnostic = explain["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["code"] == "E_SECURITY_DISCLOSURE_FORBIDDEN"
                && item["severity"] == "warning"
                && item["redacted"] == true
        })
        .expect("policy clamp diagnostic is present");
    for field in coveql::conformance_profile().required_diagnostic_fields {
        assert!(
            diagnostic.get(*field).is_some(),
            "diagnostic JSON missing required field {field}: {diagnostic}"
        );
    }
}

#[test]
fn stable_explain_supports_all_policy_modes() {
    for (mode, policy) in [
        ("public", ExplainDisclosurePolicy::PublicOnly),
        ("developer", ExplainDisclosurePolicy::Developer),
        ("proof", ExplainDisclosurePolicy::Proof),
        ("coded", ExplainDisclosurePolicy::Proof),
        ("forensic", ExplainDisclosurePolicy::Forensic),
    ] {
        let mut resolve_options = ResolveOptions::default();
        resolve_options.security.explain_policy = policy;
        resolve_options.security.metadata_disclosure_policy =
            MetadataDisclosurePolicy::AllowProtected;

        let planned = parse_resolve_and_plan_query(
            &minimal_object_file(),
            &format!("Person.select(active).explain(\"{mode}\")"),
            ParseOptions::default(),
            resolve_options,
            PlanOptions::default(),
            validation_options(),
        )
        .unwrap();

        assert_eq!(planned.explain_json()["mode"], mode);
    }
}

#[test]
fn coded_explain_mode_reports_coded_execution_contracts() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();

    assert_eq!(explain["mode"], "coded");
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reason"],
        "physical_plan_required"
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["coded_suitability"],
        "physical_plan_required"
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["fallback_reasons"],
        json!(["physical_plan_required"])
    );
    assert_eq!(
        explain["execution"]["coded_execution"]["residual_verification"],
        true
    );
    assert!(explain["execution"]["coded_execution"]["pushed_filters"].is_array());
    assert!(explain["execution"]["coded_execution"]["pushed_columns"].is_array());
    assert!(explain["execution"]["coded_execution"]["operator_contracts"].is_array());
}

#[test]
fn coded_explain_reports_projection_pushed_columns() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();
    let pushed_columns = explain["execution"]["coded_execution"]["pushed_columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(pushed_columns, vec!["active"]);
}

#[test]
fn coded_explain_reports_projection_pushed_filters() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active).explain(\"coded\")",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let pushed_filters = planned.explain_json()["execution"]["coded_execution"]["pushed_filters"]
        .as_array()
        .unwrap()
        .clone();

    assert!(pushed_filters.iter().any(|filter| {
        filter["kind"] == "projection_filter"
            && filter["placement"] == "projection_readback"
            && filter["projection_id"] == "people_projection"
            && filter["predicate"]
                .as_str()
                .is_some_and(|predicate| predicate.contains("compare:Eq:active"))
    }));
}

#[test]
fn coded_explain_reports_execution_code_domain_bridge_decision() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue"]);
    let mut resolve_options = ResolveOptions::default();
    resolve_options.execution_code_mapping_requested = true;
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name == "red").select(name).explain("coded")"#,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let explain = planned.explain_json();
    let decisions = explain["execution"]["coded_execution"]["bridge_decisions"]
        .as_array()
        .unwrap();

    assert!(decisions.iter().any(|decision| {
        decision.as_str().is_some_and(|decision| {
            decision.contains("execution_code_domain")
                && decision.contains("scope=File")
                && decision.contains("lifetime=Scan")
                && decision.contains("runtime remap proof")
        })
    }));
}

#[test]
fn coded_explain_requires_exact_manifest_bridge_for_multifile_filecode_contracts() {
    let (bytes, _) = object_file_with_filecode_records(&["red", "blue"]);
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.explain_policy = ExplainDisclosurePolicy::Proof;
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;

    let mut planned = parse_resolve_and_plan_query(
        &bytes,
        r#"Person.where(name == "red").select(name).explain("coded")"#,
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let mut second = planned.resolved.operation_context.dataset.files[0].clone();
    second.ordinal = 1;
    second.source = "right.cove".into();
    second.file_id[0] ^= 0xff;
    planned
        .resolved
        .operation_context
        .dataset
        .files
        .push(second);
    planned
        .resolved
        .operation_context
        .dataset
        .cross_file_ordering = coveql::CrossFileOrderingPolicy::CanonicalDatasetOrder;
    planned.resolved.operation_context.dataset.object_identity =
        coveql::CrossFileObjectIdentityPolicy::DatasetFileIdAndGoid;
    planned
        .resolved
        .operation_context
        .dataset
        .association_identity =
        coveql::CrossFileAssociationIdentityPolicy::DatasetFileQualifiedEndpoints;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges = vec![coveql::CodeDomainBridgeContext {
        domain_id: "name_domain".into(),
        bridge_kind: "manifest_candidate_requires_canonical_remap".into(),
        epoch: None,
        security_scope_id: None,
        exact: false,
        reason: "raw local FileCode dictionaries remain file scoped".into(),
    }];

    let explain = planned.explain_json();
    let contracts = explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "root_scan"
            && contract["representation_class"] == "cross_source_code_bridge"
            && contract["exact"] == false
            && contract["residual_required"] == true
    }));
    assert!(contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["exact"] == false
            && contract["residual_required"] == true
    }));
    assert!(explain["execution"]["coded_execution"]["decode_boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|boundary| boundary
            .as_str()
            .is_some_and(|boundary| boundary.contains("multi-file FileCode identity requires"))));

    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .bridge_kind = "materialized_canonical_value".into();
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .exact = true;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .reason = "materialized canonical value path validated by manifest".into();

    let materialized_value_explain = planned.explain_json();
    let materialized_value_contracts = materialized_value_explain["execution"]["coded_execution"]
        ["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(
        materialized_value_contracts.iter().any(|contract| {
            contract["operator"] == "predicate_compare"
                && contract["representation_class"] == "cross_source_code_bridge"
                && contract["exact"] == false
                && contract["residual_required"] == true
        }),
        "materialized canonical value proofs must not authorize raw coded comparison"
    );
    assert!(
        materialized_value_explain["execution"]["coded_execution"]["bridge_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision.as_str().is_some_and(|decision| {
                decision.contains("materialized_canonical_value") && decision.contains("exact=true")
            }))
    );

    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .bridge_kind = "manifest_validated_canonical_remap".into();
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .exact = true;
    planned
        .resolved
        .operation_context
        .dataset
        .code_domain_bridges[0]
        .reason = "epoch-bound canonical remap validated by manifest".into();

    let exact_explain = planned.explain_json();
    let exact_contracts = exact_explain["execution"]["coded_execution"]["operator_contracts"]
        .as_array()
        .unwrap();
    assert!(exact_contracts.iter().any(|contract| {
        contract["operator"] == "root_scan"
            && contract["representation_class"] == "cross_source_code_bridge"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
    assert!(exact_contracts.iter().any(|contract| {
        contract["operator"] == "predicate_compare"
            && contract["exact"] == true
            && contract["residual_required"] == false
    }));
    assert!(
        exact_explain["execution"]["coded_execution"]["bridge_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision
                .as_str()
                .is_some_and(|decision| decision.contains("exact=true")))
    );
}

#[test]
fn stable_explain_text_is_derived_from_json() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active).explain()",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let explain = planned.explain_json();
    let text = planned.explain_text();
    assert_eq!(text, render_explain_text(&explain));
    assert_eq!(text, planned.explain_text());
    assert!(text.contains("schema_version: 0.1"));
    assert!(text.contains("execution: completed=false kind=plan_explanation"));
}

#[test]
fn logical_plan_fingerprint_is_stable_for_harmless_quoting() {
    let a = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "`Person`.select(`active`)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    assert_eq!(a.logical_plan_fingerprint, b.logical_plan_fingerprint);
    assert_eq!(a.predicate_ast_fingerprint, b.predicate_ast_fingerprint);
    assert_eq!(a.predicate_cnf_fingerprint, b.predicate_cnf_fingerprint);
    assert_eq!(
        a.projection_dependency_fingerprint,
        b.projection_dependency_fingerprint
    );
}

#[test]
fn predicate_fingerprints_change_with_predicate_semantics() {
    let a = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(active == false).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_ne!(a.predicate_ast_fingerprint, b.predicate_ast_fingerprint);
    assert_ne!(a.predicate_cnf_fingerprint, b.predicate_cnf_fingerprint);
}

#[test]
fn projection_dependency_fingerprint_changes_with_projection_contract() {
    let bytes = minimal_object_with_two_column_projection_file();
    let a = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();
    let b = parse_resolve_and_plan_query(
        &bytes,
        "projection(people_projection).select(enabled)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_ne!(
        a.projection_dependency_fingerprint,
        b.projection_dependency_fingerprint
    );
    assert_eq!(
        a.explain_json()["fingerprints"]["projection_dependency"],
        a.projection_dependency_fingerprint
    );
}

#[test]
fn logical_plan_projection_roots_produce_projection_read_contract() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.metadata_disclosure_policy = MetadataDisclosurePolicy::AllowProtected;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert!(planned
        .dependencies
        .projection_ids
        .contains("people_projection"));
    assert!(planned
        .logical_plan
        .nodes
        .iter()
        .any(|node| matches!(node.kind, LogicalPlanNodeKind::ProjectionRead { .. })));
    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(
        contract.contract_version,
        coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION
    );
    assert_eq!(contract.projection_id, "people_projection");
    assert_eq!(contract.projection_version.as_deref(), Some("2026.05"));
    assert_eq!(contract.mapping_version.as_deref(), Some("2026.05"));
    assert_eq!(contract.row_grain.as_deref(), Some("one_row_per_object"));
    assert_eq!(contract.anchor_object_type.as_deref(), Some("Person"));
    assert_eq!(contract.map_columns.len(), 1);
    assert_eq!(contract.map_columns[0].name, "active");
    assert_eq!(contract.map_columns[0].value, "property.active");
    assert_eq!(contract.columns.len(), 1);
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.source_properties.contains(&1));
    assert!(contract.pushed_predicates.is_empty());
    assert!(contract.residual_predicates.is_empty());
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::FullyPushdownSafe
    );
    assert_eq!(contract.visibility_policy, "all_rows");
    assert_eq!(contract.redaction_policy, "protected_values_redacted");
    assert_eq!(
        contract.domain_policy,
        "same_projection_contract_or_materialized"
    );
    assert_eq!(
        contract.collation_policy,
        "declared_collation_or_materialized_sort"
    );
    assert_eq!(contract.null_policy, "cove_null_semantics_preserved");
    assert!(contract.residual_required_fields.is_empty());
    assert_eq!(contract.output_compatibility, vec!["json", "arrow"]);
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);

    let explain_contract =
        &planned.explain_json()["resolved_dependencies"]["projection_contracts"][0];
    assert_eq!(
        explain_contract["contract_version"],
        json!(coveql::PROJECTION_DEPENDENCY_CONTRACT_VERSION)
    );
    assert_eq!(explain_contract["map_columns"][0]["name"], "active");
    assert_eq!(
        explain_contract["map_columns"][0]["value"],
        "property.active"
    );
}

#[test]
fn projection_contract_records_source_properties_for_pushed_columns() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_two_column_projection_file(),
        "projection(people_projection).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.source_properties.contains(&1));
    assert!(!contract.source_properties.contains(&2));
}

#[test]
fn projection_contract_treats_omitted_select_as_all_projection_columns() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_two_column_projection_file(),
        "projection(people_projection)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(
        contract.selected_columns,
        BTreeSet::from(["active".into(), "enabled".into()])
    );
    assert_eq!(
        contract.pushed_columns,
        BTreeSet::from(["active".into(), "enabled".into()])
    );
    assert!(contract.source_properties.contains(&1));
    assert!(contract.source_properties.contains(&2));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::FullyPushdownSafe
    );
}

#[test]
fn projection_contract_reports_pushed_filter_predicates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Eq:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_root_execution_reports_provider_column_and_filter_pushdown() {
    let executed = parse_resolve_plan_and_execute_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        ExecutionOptions::default(),
        validation_options(),
    )
    .unwrap();

    assert_eq!(executed.pushdown_report.outcome, PushdownOutcome::Applied);
    assert_eq!(
        executed.pushdown_report.counters.property_columns_requested,
        1
    );
    assert_eq!(
        executed
            .pushdown_report
            .counters
            .property_predicate_candidates,
        1
    );
    assert_eq!(executed.pushdown_report.counters.residual_predicates, 0);
    assert!(executed.pushdown_report.residual_predicates.is_empty());
    assert!(executed.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::ProjectionColumnPrune
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["columns"] == json!(["active"])
    }));
    assert!(executed.pushdown_report.decisions.iter().any(|decision| {
        decision.kind == PushdownDecisionKind::ProjectionFilterCandidate
            && decision.outcome == PushdownOutcome::Applied
            && decision.safe_details["predicate"]
                .as_str()
                .is_some_and(|predicate| predicate.contains("compare:Eq:active"))
    }));
}

#[test]
fn projection_contract_reports_not_equal_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active != true).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Ne:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_negated_equal_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!(active == true)).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates.len(), 1);
    assert!(contract.pushed_predicates[0].contains("compare:Ne:active"));
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_bool_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["bool:active"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_negated_bool_filter_as_pushed() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!active).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["not_bool:active"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_reports_same_column_or_filter_as_pushed_in_list() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true || active in [false]).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert_eq!(contract.pushed_predicates, vec!["in:active:2 literals"]);
    assert!(contract.residual_predicates.is_empty());
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.pushdown_safe);
    assert!(!contract.residual_required);
}

#[test]
fn projection_contract_keeps_negated_in_with_null_residual() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(!(active in [true, null])).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_predicates.is_empty());
    assert_eq!(
        contract.residual_predicates,
        vec!["not:in:active:2 literals"]
    );
    assert!(contract.residual_required);
    assert!(!contract.pushdown_safe);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
}

#[test]
fn projection_contract_reports_residual_filter_predicates() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).where(active == true || active.isNull()).select(active)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.pushed_predicates.is_empty());
    assert_eq!(contract.residual_predicates, vec!["or:2 terms"]);
    assert!(contract.residual_required);
    assert!(!contract.pushdown_safe);
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
}

#[test]
fn projection_order_by_default_nulls_marks_dependency_contract_residual() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(active).orderBy(active, desc)",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
    assert!(contract.residual_required_fields.contains("active"));
    assert!(!contract.pushdown_safe);
    assert!(contract.residual_required);
    assert!(contract
        .residual_reasons
        .iter()
        .any(|reason| reason.contains("projection ordering requires residual")));
}

#[test]
fn projection_contract_pushes_columns_required_by_residual_select_expressions() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(value: coalesce(active, false))",
        ParseOptions::default(),
        ResolveOptions::default(),
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.selected_columns.contains("active"));
    assert!(contract.pushed_columns.contains("active"));
    assert!(contract.deterministic_functions.contains("coalesce"));
    assert_eq!(contract.function_requirements.len(), 1);
    assert_eq!(contract.function_requirements[0].id, "coalesce");
    assert!(contract.function_requirements[0]
        .input_columns
        .contains("active"));
    assert!(contract.function_requirements[0].residual_required);
    assert!(!contract.function_requirements[0].pushdown_safe);
    assert!(contract.function_requirements[0]
        .reason
        .contains("residual verification"));
    assert_eq!(
        contract.pushdown_status,
        coveql::ProjectionPushdownStatus::PartiallyPushdownSafe
    );
    assert!(contract.residual_required);
    assert!(contract.residual_required_fields.contains("active"));
    assert!(contract
        .residual_reasons
        .iter()
        .any(|reason| reason.contains("residual output evaluation required")));
}

#[test]
fn projection_contract_records_aggregate_requirements() {
    let mut resolve_options = ResolveOptions::default();
    resolve_options.security.aggregate_disclosure_policy = AggregateDisclosurePolicy::AllowExact;
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_projection_file(),
        "projection(people_projection).select(n: count(active))",
        ParseOptions::default(),
        resolve_options,
        PlanOptions::default(),
        validation_options(),
    )
    .unwrap();

    let contract = planned.dependencies.projection_contracts.first().unwrap();
    assert!(contract.aggregate_kinds.contains("count"));
    assert_eq!(contract.aggregate_requirements.len(), 1);
    assert_eq!(contract.aggregate_requirements[0].id, "count");
    assert!(contract.aggregate_requirements[0]
        .input_columns
        .contains("active"));
    assert!(contract.aggregate_requirements[0].residual_required);
    assert!(!contract.aggregate_requirements[0].pushdown_safe);
    assert!(contract.aggregate_requirements[0]
        .reason
        .contains("duplicate-row"));
}

fn json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        ..ResolveOptions::default()
    }
}

fn protected_json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        security: SecurityContext {
            metadata_disclosure_policy: MetadataDisclosurePolicy::AllowProtected,
            ..SecurityContext::default()
        },
        ..ResolveOptions::default()
    }
}
