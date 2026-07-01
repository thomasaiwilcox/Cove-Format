//! Generates the conformance corpus referenced by `conformance/manifest.jsonl`.
//! Run with `cargo run -p cove-conformance --bin gen-corpus`.
//!
//! Each fixture maps to one or more Spec §76 error codes; the manifest is
//! written alongside the binaries so the generator stays the source of truth.

#[path = "../gen_corpus_cove_map_support.rs"]
mod gen_corpus_cove_map_support;
#[path = "../gen_corpus_support.rs"]
mod gen_corpus_support;

#[path = "gen_corpus/ai.rs"]
mod gen_corpus_ai;
#[path = "gen_corpus/base.rs"]
mod gen_corpus_base;
#[path = "gen_corpus/catalog_recipes.rs"]
mod gen_corpus_catalog_recipes;
#[path = "gen_corpus/codec_layout.rs"]
mod gen_corpus_codec_layout;
#[path = "gen_corpus/delta_chain.rs"]
mod gen_corpus_delta_chain;
#[path = "gen_corpus/delta_sidecars.rs"]
mod gen_corpus_delta_sidecars;
#[path = "gen_corpus/error_surface.rs"]
mod gen_corpus_error_surface;
#[path = "gen_corpus/feature_scope.rs"]
mod gen_corpus_feature_scope;
#[path = "gen_corpus/metadata_kernel.rs"]
mod gen_corpus_metadata_kernel;
#[path = "gen_corpus/object_table.rs"]
mod gen_corpus_object_table;
#[path = "gen_corpus/object_temporal.rs"]
mod gen_corpus_object_temporal;
#[path = "gen_corpus/payloads.rs"]
mod gen_corpus_payloads;
#[path = "gen_corpus/profiles.rs"]
mod gen_corpus_profiles;
#[path = "gen_corpus/suite_contracts.rs"]
mod gen_corpus_suite_contracts;
#[path = "gen_corpus/trust_chain.rs"]
mod gen_corpus_trust_chain;
#[path = "gen_corpus/validation_surface.rs"]
mod gen_corpus_validation_surface;

use std::{
    collections::BTreeSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{
    builder::{ListBuilder, Time32MillisecondBuilder},
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array, Int64Array, RecordBatch,
    StringArray, TimestampMicrosecondArray,
};
use parquet::arrow::ArrowWriter;

use cove_cache::{CoveCoverageCacheHeaderV2, CoverageCacheEntryV2, CoverageCacheV2};
use cove_codec::{
    CodecExtensionDescriptorV2, CodecFallbackPolicyV2, CodecRequirementV2,
    CodecSpecificationStatusV2, RegisteredEncodingEnvelopeV2,
};
use cove_core::{
    artifact::{
        coveai::{
            write_coveai_artifact, write_coveai_descriptor_bundle, write_covev_filecode_vectors,
            AiAssetRefV1, AiCompanionArtifactRefV1, AiDescriptorTablesV1, AiDigestEntryV1,
            AiPayloadEncodingV1, AiPayloadRefEntryV1, AiPolicyRefEntryV1, AiPrivacySummaryEntryV1,
            AiRequirednessScopeV1, AiSectionFeatureBindingV1, AiSourceBindingV1,
            AiSourceSpanEntryV1, AiStorageKindV1, AiStringEntryV1, AiTransformEntryV1,
            AssetVectorBindingV1, AssociationStateVectorBindingV1, ChunkProfileV1,
            ChunkVectorBindingV1, CoveAiArtifactKind, CoveAiDescriptorBundleBuild,
            CoveAiWritableSection, CoveVecFileCodeVectorBuild, DatasetSplitV1, DedupGroupV1,
            DeviceTransferHintV1, GenerationDecodingProfileV1, GeneratorProvenanceV1,
            HumanReviewEntryV1, ModelActorDescriptorV1, MultimodalSequenceElementV1,
            MultimodalSequencePackV1, MultimodalSequenceVectorBindingV1, PreferencePairEntryV1,
            TensorLayoutDescriptorV1, TextChunkEntryV1, TokenBlockHeaderV1, TokenSequencePackV1,
            TokenizedSpanV1, TokenizerProfileV1, TrainingEpochPlanV1, TrainingLabelEntryV1,
            TrainingProfileV1, TrainingSampleEntryV1, VectorEntryV1, VectorPayloadBlockHeaderV1,
            VectorSpaceDescriptorV1, AI_COMPANION_ARTIFACT_KIND_CVV2,
            AI_FLAG_PAYLOAD_CRC32C_PRESENT, AI_FLAG_REQUIRED_RECORD, AI_POLICY_KIND_VISIBILITY,
            AI_SOURCE_KIND_COVE_FILE, AI_TRANSFORM_KIND_VECTORIZER, COVEAI_HEADER_LEN,
            COVEAI_POSTSCRIPT_LEN, COVEAI_POSTSCRIPT_TAIL_SIZE, COVEAI_SECTION_ENTRY_LEN,
        },
        covedelta::{
            CoveDeltaFile, CoveDeltaFooterV1, CoveDeltaHeaderV1, CoveDeltaPostscriptV1,
            CoveDeltaSection, CoveDeltaSectionDirectoryEntryV1, CoveDeltaSectionKind,
            CoveObjectDeltaStateHashPropertyV1, CoveObjectDeltaStateHashV1, DeltaBranchIdentityV1,
            DeltaContinuationAnchorV1, DeltaDictionaryEntryV1, DeltaInlineValueV1,
            DeltaParentRefV1, DeltaScopeDescriptorV1, DeltaSidecarHintV1,
            DeltaSparsePatchPropertyOpV1, DeltaSparsePatchRecordV1, DeltaStateHashDescriptorV1,
            DeltaSummaryDescriptorV1, DeltaTouchedObjectRangeV1, COVEDELTA_FOOTER_LEN,
            COVEDELTA_HEADER_LEN, COVEDELTA_POSTSCRIPT_LEN, COVEDELTA_POSTSCRIPT_TAIL_SIZE,
            DELTA_ANCHOR_STRENGTH_KEY_AND_RECORD_ID, DELTA_ANCHOR_STRENGTH_KEY_ONLY,
            DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
            DELTA_BRANCH_IDENTITY_KIND_CANONICAL_VALUE_REF,
            DELTA_DICTIONARY_ENTRY_KIND_CANONICAL_HASH_HINT,
            DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE,
            DELTA_DICTIONARY_ENTRY_KIND_PARENT_DICTIONARY_ALIAS,
            DELTA_FEATURE_CHECKPOINT_BASELINES, DELTA_FEATURE_CONTINUATION_ANCHORS,
            DELTA_FEATURE_COVERAGE_PATCH, DELTA_FEATURE_EXACT_TOMBSTONE_SET,
            DELTA_FEATURE_EXACT_TOUCHED_SET, DELTA_FEATURE_INDEX_HINTS,
            DELTA_FEATURE_INLINE_DICTIONARY, DELTA_FEATURE_MAP_EVIDENCE_PATCH,
            DELTA_FEATURE_PARENT_DICTIONARY_ALIASES, DELTA_FEATURE_PROJECTION_PATCH,
            DELTA_FEATURE_SPARSE_PATCH_ROWS, DELTA_FLAG_SINGLE_SCOPE,
            DELTA_OBJECT_STATE_TOMBSTONE_LIVE, DELTA_OBJECT_STATE_VALUE_NULL,
            DELTA_OBJECT_STATE_VALUE_REDACTED, DELTA_OBJECT_STATE_VALUE_VISIBLE,
            DELTA_PARENT_REF_LINEAGE_PARENT, DELTA_PROPERTY_OP_CLEAR, DELTA_PROPERTY_OP_REDACT,
            DELTA_PROPERTY_OP_SET_NULL, DELTA_PROPERTY_OP_SET_VALUE, DELTA_PROPERTY_OP_TOMBSTONE,
            DELTA_REF_NONE, DELTA_SIDECAR_HINT_KIND_COVERAGE_PATCH,
            DELTA_SIDECAR_HINT_KIND_COVI_INDEX, DELTA_SIDECAR_HINT_KIND_LAYOUT_HINTS,
            DELTA_SPARSE_PATCH_PROPERTY_OP_LEN, DELTA_SPARSE_PATCH_RECORD_HEADER_LEN,
            DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1,
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_RANGE_SET,
            DELTA_SUMMARY_DESCRIPTOR_KIND_EXACT_SORTED_SET,
            DELTA_SUMMARY_DESCRIPTOR_KIND_PROPERTY_BITMAP,
            DELTA_SUMMARY_DESCRIPTOR_KIND_TEMPORAL_ROLE_RANGE, DELTA_TOMBSTONE_KIND_NONE,
            DELTA_TOMBSTONE_KIND_PROPERTY,
        },
        covemap::{
            CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
            CovemapSection, CovemapSectionEntryV1, COVEMAP_HEADER_LEN, COVEMAP_POSTSCRIPT_LEN,
            COVEMAP_POSTSCRIPT_TAIL_SIZE,
        },
        covm::{
            CovmDeltaArtifactRefV1, CovmDeltaChainExtensionV1, CovmDeltaChainSummaryV1, CovmFile,
            CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1, DeltaChainSummaryEntryV1,
            COVM_DELTA_ARTIFACT_REF_LEN, COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN,
            COVM_DELTA_CHAIN_SUMMARY_ENTRY_LEN, COVM_DELTA_CHAIN_SUMMARY_HEADER_LEN,
            COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1, COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED,
            DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
            DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT,
            DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        },
        covx::{CovxFile, CovxHeaderV1, CovxPostscriptV1, CovxReferencedFileV1},
    },
    canonical::{CanonicalField, CanonicalValue},
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        PrimaryProfile, SectionKind, StorageClass, ValueTag,
        AI_FEATURE_EXTERNAL_ASSET_DIGEST_REQUIRED, AI_FEATURE_MODEL_INPUT_IDENTITY,
        AI_FEATURE_VECTOR, FEATURE_CODEC_EXTENSION_REGISTRY, FEATURE_CODEC_LZ4, FEATURE_CODEC_ZSTD,
        FEATURE_COLUMN_DOMAINS, FEATURE_ENGINE_PROFILE, FEATURE_EXTENDED_FEATURE_SET,
        FEATURE_FILE_DICTIONARY, FEATURE_HARBOR_PROFILE, FEATURE_INDEX_ONLY_CAPABILITY,
        FEATURE_LAYOUT_PLAN, FEATURE_OBJECT_PROFILE, FEATURE_PAGE_PAYLOAD_ELISION,
        FEATURE_REGISTERED_ENCODINGS, FEATURE_RUNTIME_COMPATIBILITY_HINTS,
        FEATURE_SECONDARY_INDEX_ARTIFACT, FEATURE_SEMANTIC_MAP, FEATURE_TABLE_PROFILE,
        FEATURE_TRUST_CHAIN, FEATURE_ZERO_COPY_BUFFER_MAP, MAGIC_COVI, POSTSCRIPT_VERSION_V1,
        VERSION_MAJOR_V1,
    },
    dictionary::{FileDictionary, FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
    digest::{compute_digest, DigestEntry, DigestManifest, DigestScope, DigestTargetKind},
    domain::{ColumnDomain, ColumnDomainHeaderV1, COLUMN_DOMAIN_HEADER_LEN},
    encoding::{
        bit_packed::BitPackedPayload,
        constant::ConstantPayload,
        delta::DeltaPayload,
        frame_of_reference::ForPayload,
        local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
        nested::{
            ListLayout, ListLayoutPayload, MapLayout, MapLayoutPayload, StructLayout,
            StructLayoutPayload,
        },
        plain::{PlainFixedPayload, PlainVarintPayload},
        rle::RlePayload,
    },
    extensions::{
        ExtensionFalseNegativePolicy, ExtensionIndexDescriptorV1, ExtensionKind,
        ExtensionLogicalTypeV1, ExtensionProofCapability, ExtensionRegistry,
        ExtensionRegistryEntry,
    },
    feature_binding::{
        FeatureScopeV2, OperationKindV2, SectionFeatureBindingPayloadKindV2,
        SectionFeatureBindingPayloadRefV2, SectionFeatureBindingSectionHeaderV2,
        SectionFeatureBindingSectionV2, SectionFeatureBindingV2,
    },
    feature_scope::{
        cove_column_page_target_ref, ExtendedFeatureSetHeaderV2, ExtendedFeatureSetV2,
        ProfileCapabilityEntryV2, ProfileCapabilityMatrixHeaderV2, ProfileCapabilityMatrixV2,
    },
    footer::{CoveFooterHeaderV1, CoveSectionEntryV1, FOOTER_HEADER_SIZE, SECTION_ENTRY_SIZE},
    header::{CoveHeaderV1, HEADER_SIZE},
    index::{
        aggregate::{
            AggregateEntry, AggregatePayloadV2, AggregateSynopsis, HistogramBucket,
            NumericAggregateOverflowPolicy, SynopsisAccuracy, SynopsisKind, TaggedCanonicalValue,
            DEFAULT_HLL_PRECISION, DEFAULT_KLL_K,
        },
        bloom::{
            BloomAlgorithm, BloomGranularity, BloomHashDomain, BloomIndexHeaderV1,
            BLOOM_INDEX_HEADER_LEN,
        },
        composite::{
            CompositeTransformKind, CompositeZoneIndexHeaderV1, COMPOSITE_ZONE_INDEX_HEADER_LEN,
        },
        exact_set::{
            ExactSetGranularity, ExactSetIndexHeaderV1, ExactSetKeyKind, ExactSetRepresentation,
            EXACT_SET_HEADER_LEN,
        },
        inverted::{
            InvertedEntry, InvertedKeyKind, InvertedMorselIndexHeaderV1, INVERTED_MORSEL_ENTRY_LEN,
            INVERTED_MORSEL_INDEX_HEADER_LEN,
        },
        lookup::{
            LookupIndexHeaderV1, LookupIndexKind, LookupKeyKind, LookupUniqueness,
            LOOKUP_INDEX_HEADER_LEN,
        },
        topn::{TopNDirection, TopNSummary, TOPN_ZONE_SUMMARY_LEN},
    },
    interop::lakehouse::{LakehouseHints, LakehouseVisibilityOverlayRef},
    io_hints::defaults_object_store,
    kernel::{KernelCapabilities, KernelCapabilityEntry},
    nested_schema::{NestedSchemaEntryV1, NestedSchemaNodeV1, NestedSchemaSectionV1},
    page::{
        ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN, PAGE_FLAG_ALL_NON_NULL,
        PAGE_FLAG_ALL_NULL, PAGE_FLAG_STATS_ONLY_CONSTANT, PAGE_FLAG_VALUE_STREAM_ELIDED,
    },
    page_payload::{
        ColumnPagePayloadV1, CoveEncodingNodeV1, PageBufferKind, COLUMN_PAGE_PAYLOAD_HEADER_LEN,
        COVE_ENCODING_NODE_LEN, PAGE_BUFFER_DESCRIPTOR_LEN,
    },
    postscript::{CovePostscriptV1, POSTSCRIPT_SIZE, POSTSCRIPT_TOTAL_SIZE},
    profile::{
        cove_e::{
            CodeSpaceDescriptorV1, EngineMountPolicyV1, EngineProfileEntryV1,
            EngineProfileRegistry, ExecutionCodeCanonicality, ExecutionCodeComparisonScope,
            ExecutionCodeDescriptorV1, ExecutionCodeKind, ExecutionCodeLifetime,
            ExecutionScopeDescriptorV1, ExecutionScopeKind, FileCodeMappingKind,
            MissingValuePolicy, NullCodePolicy, ReverseLookupPolicy, StaleMappingPolicy,
        },
        cove_h::HarborMountHintsV1,
        cove_o::{
            temporal_row_trust_payload, CoveRecordRefV1, ObjectTypeCatalog, ObjectTypeEntryV1,
            PropertyEntryV1, RecordKind, TemporalBloomEntryV1, TemporalBloomIndex,
            TemporalRowEntryV1, TemporalSegmentData, TemporalSegmentHeaderV1, TemporalSegmentIndex,
            TemporalSegmentIndexEntryV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            OBJECT_TYPE_FLAG_LINK_OBJECT, TEMPORAL_BLOOM_ENTRY_LEN, TEMPORAL_ROW_ENTRY_LEN,
            TEMPORAL_SEGMENT_HEADER_LEN,
        },
    },
    reader,
    row_ref::RowRef,
    segment::{
        RowMorselDirectory, RowMorselEntryV1, TableColumnDirectoryEntryV1, TableSegmentHeaderV1,
        TableSegmentIndex, TableSegmentIndexEntryV1, TableSegmentPayloadV1,
        TABLE_COLUMN_DIRECTORY_ENTRY_LEN, TABLE_SEGMENT_HEADER_LEN,
    },
    sort::{ClusteringKeyEntryV1, ClusteringStrength, NullOrder, SortDirection, SortKeyEntryV1},
    table::{ColumnEntry, TableCatalog, TableEntry, COLUMN_FLAG_BOOL_DECLARED_NUMERIC},
    writer::{MinimalCoveWriter, ScanPageSpec, ScanProfileCoveWriter, ScanSegment, SectionPayload},
    zone_stats::{
        StatKind, StatScalar, ZoneScope, ZoneStatFlags, ZoneStats, ZoneStatsEntry,
        ZoneStatsSection, STAT_SCALAR_ENCODED_LEN, ZONE_STATS_ENTRY_LEN,
    },
    CoveError,
};
use cove_coverage::{
    coverage_set_payload_checksum, CoverageExactnessV2, CoverageFallbackPolicyV2,
    CoverageGranularityV2, CoveragePlanCandidateV2, CoverageProofKindV2, CoverageProofRecordV2,
    CoverageProofStrengthV2, CoverageProviderDescriptorV2, CoverageSetEntryV2, CoverageSetHeaderV2,
    IntervalBoundKindV2, IntervalNullPolicyV2, IntervalPredicateV2, PredicateFormKindV2,
    PredicateNormalFormV2, COVERAGE_PLAN_FLAG_MAY_UNDER_INCLUDE,
    COVERAGE_PLAN_FLAG_PRUNING_CANDIDATE,
};
use cove_index::{
    CoviAggregateAnswerBlockHeaderV2, CoviAggregateAnswerBlockV2, CoviAggregateAnswerV2,
    CoviArtifactV2, CoviByteRangePostingV2, CoviComparatorKindV2, CoviDimensionalBucketPostingV2,
    CoviEntryBlockHeaderV2, CoviEntryBlockV2, CoviFileRefPostingV2, CoviHeaderV2, CoviIndexEntryV2,
    CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2, CoviKeyBlockHeaderV2,
    CoviKeyBlockV2, CoviKeyEncodingKindV2, CoviMorselRefPostingV2, CoviObjectPathPostingV2,
    CoviPageRefPostingV2, CoviPostingRepresentationV2, CoviPostingsBlockHeaderV2,
    CoviPostingsBlockV2, CoviPostingsHeaderV2, CoviPostscriptV2, CoviReferencedFileV2,
    CoviRowOrdinalSetHeaderV2, CoviRowRangePostingV2, CoviSectionEntryV2, CoviSectionKindV2,
    CoviSectionPayloadV2, CoviSegmentRefPostingV2, CoviSnapshotValidityV2,
    IndexCapabilityExactnessV2, IndexCapabilityV2, IndexOnlyCapabilityV2, COVI_HEADER_LEN,
    COVI_POSTSCRIPT_LEN, COVI_SECTION_ENTRY_LEN, COVI_TAIL_LEN,
};
use cove_layout::{
    build_default_layout_plan, build_default_scan_split_index, FastMetadataIndexEntryV2,
    FastMetadataIndexHeaderV2, FastMetadataIndexV2, LayoutPlanHeaderV2, LayoutPlanNodeV2,
    PageClusterDirectoryHeaderV2, PageClusterDirectoryV2, PageClusterEntryV2, ScanSplitEntryV2,
    ScanSplitIndexHeaderV2, ZeroCopyBufferMapEntryV2, ZeroCopyBufferMapHeaderV2,
    ZeroCopyBufferMapV2, ZeroCopyDictionarySemanticsV2, ZeroCopyLifetimeScopeV2,
    ZeroCopyNestedLayoutKindV2, ZeroCopyNullBitmapPolarityV2, ZeroCopySourceBufferRoleV2,
    ZeroCopyTargetBufferRoleV2, ZeroCopyTargetV2,
};
use cove_runtime::{RuntimeCompatibilityHintV2, RuntimeHintKindV2};
use serde_json::{json, Value};

use gen_corpus_codec_layout::*;
use gen_corpus_cove_map_support::{
    cove_map_evidence_invalid_file, cove_map_function_undeclared_file,
    cove_map_identity_conflict_file, cove_map_invalid_file, cove_map_source_stale_file,
    cove_map_valid_file, write_cove_map_execution_cases,
};
use gen_corpus_delta_chain::*;
use gen_corpus_metadata_kernel::*;
use gen_corpus_object_table::*;
use gen_corpus_payloads::*;
use gen_corpus_profiles::*;
use gen_corpus_support::{
    check_mode, fixture, json_fixture_bytes, with_collation_count, with_expect_can_skip,
    with_morsel_count, write_auxiliary_file, write_fixture,
};

struct CorpusWriter<'a> {
    root: &'a Path,
    entries: &'a mut Vec<Value>,
}

impl<'a> CorpusWriter<'a> {
    fn new(root: &'a Path, entries: &'a mut Vec<Value>) -> Self {
        Self { root, entries }
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("conformance");
    fs::create_dir_all(root.join("accept")).unwrap();
    fs::create_dir_all(root.join("reject")).unwrap();
    for family in [
        "feature-scope",
        "coverage",
        "covi",
        "codecs",
        "layout",
        "zerocopy",
        "runtime",
        "cache",
        "sidecars",
        "visibility",
        "ai",
    ] {
        fs::create_dir_all(root.join(family)).unwrap();
    }

    let mut entries = Vec::new();
    write_v2_profile_fixtures(&root, &mut entries);

    let base_fixtures =
        gen_corpus_base::write_base_fixtures(&mut CorpusWriter::new(&root, &mut entries));

    gen_corpus_ai::write_ai_fixtures(&mut CorpusWriter::new(&root, &mut entries));

    gen_corpus_delta_sidecars::write_delta_and_sidecar_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    let covemap_bytes = gen_corpus_catalog_recipes::write_catalog_and_runtime_fixtures(
        &mut CorpusWriter::new(&root, &mut entries),
    );

    gen_corpus_validation_surface::write_validation_surface_fixtures(
        &mut CorpusWriter::new(&root, &mut entries),
        &base_fixtures,
        covemap_bytes,
    );

    gen_corpus_feature_scope::write_feature_scope_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    gen_corpus_object_temporal::write_object_temporal_reject_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    gen_corpus_trust_chain::write_trust_chain_reject_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    gen_corpus_error_surface::write_error_surface_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    gen_corpus_suite_contracts::write_suite_contract_fixtures(&mut CorpusWriter::new(
        &root,
        &mut entries,
    ));

    assert_error_code_coverage(&entries);

    let manifest = root.join("manifest.jsonl");
    let manifest_content = entries
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if check_mode() {
        let existing = fs::read(&manifest).unwrap_or_else(|err| {
            panic!("cannot read {} during --check: {err}", manifest.display())
        });
        assert_eq!(
            existing,
            manifest_content.as_bytes(),
            "{} is not up to date; run cargo run -p cove-conformance --bin gen-corpus",
            manifest.display()
        );
        println!(
            "conformance corpus is up to date ({} fixtures in {})",
            entries.len(),
            root.display()
        );
    } else {
        fs::write(&manifest, manifest_content).unwrap();

        println!("wrote {} fixtures to {}", entries.len(), root.display());
    }
}
