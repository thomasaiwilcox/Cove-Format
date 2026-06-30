//! `cove-conformance` — Cove Format conformance corpus runner (Spec §70, §73, §76, §78-§79).
//!
//! Walks a corpus directory of fixtures plus a JSON manifest that
//! maps each fixture to (a) the spec sections it exercises and (b) the
//! expected outcome (accept / reject-with-error-code). Prints a summary and
//! exits non-zero on any mismatch.
//!
//! Corpus layout:
//! ```text
//! conformance/
//!   manifest.jsonl
//!   accept/<fixture>
//!   reject/<fixture>
//! ```
//! By default the runner uses `<corpus-dir>/manifest.jsonl`. A smaller
//! manifest can be supplied with `--manifest <path>` while still resolving
//! fixture paths relative to `<corpus-dir>`.
//!
//! Manifest format (one JSON object per line):
//! ```json
//! {"path":"accept/min.cove","kind":"cove","expect":"accept","sections":["§9","§10"]}
//! {"path":"reject/bad_crc.cove","kind":"cove","expect":"reject","error_code":"COVE_E_CHECKSUM_MISMATCH","sections":["§13"]}
//! ```

mod delta_validation;
mod encoding_validation;
mod extension_validation;
mod feature_runtime_validation;
mod manifest;
mod map_validation;
mod pruning_validation;
mod runner;
mod suite_validation;

use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
    process,
};

use arrow_array::{
    Array, BinaryArray, BinaryViewArray, BooleanArray, Int32Array, StringArray, StringViewArray,
    UInt64Array,
};
use cove_arrow::{
    arrow::{
        arrow_validity_to_cove_null, cove_null_to_arrow_validity, encoded_array_to_arrow,
        encoded_array_to_arrow_with_row_selection_options, ArrowExportOptions, ArrowRowSelection,
        ArrowVarBytesExportPolicy,
    },
    parquet::{
        convert_parquet_bytes, decode_materialized_page_values_with_nulls,
        ParquetConversionOptions, ParquetScalarValue,
    },
};
use serde_json::{json, Value};

use cove_cache::CoverageCacheV2;
use cove_codec::{CodecExtensionDescriptorV2, RegisteredEncodingEnvelopeV2};
use cove_core::{
    array::{CoveArrayValue, EncodedArray},
    artifact::{
        coveai::CoveAiFile,
        covedelta::CoveDeltaFile,
        covedelta::{
            reconstruct_sparse_patch_state_table, CoveObjectDeltaStateHashPropertyV1,
            CoveObjectDeltaStateHashV1, DeltaBranchIdentityV1, DeltaContinuationAnchorV1,
            DeltaExactObjectSetMembershipV1, DeltaObjectPointLookupV1, DeltaSparseObjectKeyV1,
            DeltaSparseObjectTombstoneStatusV1, DeltaSparsePatchPropertyStateV1,
            DeltaSparsePatchRecordV1, DeltaStateHashDescriptorV1, DeltaTouchedObjectRangeV1,
        },
        covemap::CovemapFile,
        covm::{
            validate_selected_delta_chain_with_base,
            validate_selected_delta_chain_with_summary_bytes, CovmDeltaChainExtensionV1,
            CovmDeltaChainSummaryV1, CovmDeltaPruneMetrics, CovmDeltaPruneReason,
            CovmDeltaPruneRequest, CovmDeltaPruneSkip, CovmFile,
        },
        covx::CovxFile,
    },
    checksum,
    collation::CollationRegistry,
    compression::{column_page_payload, encode_page_payload, section_payload},
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, DigestAlgorithm,
        SectionKind,
    },
    dictionary::FileDictionary,
    digest::DigestManifest,
    domain::ColumnDomain,
    durable,
    encoding::{
        assert_parity,
        bit_packed::{BitPacked, BitPackedPayload},
        constant::{Constant, ConstantPayload},
        delta::{Delta, DeltaPayload},
        frame_of_reference::{ForPayload, FrameOfReference},
        local_codebook::{LocalCodebook, LocalCodebookPayload},
        nested::{ListLayout, MapLayout, StructLayout},
        patched_base::{PatchedBase, PatchedBasePayload},
        plain::{PlainFixed, PlainFixedPayload, PlainVarint, PlainVarintPayload},
        rle::{Rle, RlePayload},
        run_end::{RunEnd, RunEndPayload},
        sparse::{Sparse, SparsePayload},
        Encoding,
    },
    extensions::{
        ExtensionIndexDescriptorV1, ExtensionLogicalTypeV1, ExtensionRegistry,
        ExtensionValidationContext,
    },
    feature_binding::{OperationKindV2, SectionFeatureBindingSectionV2},
    feature_scope::FeatureUseRequestV2,
    index::{
        aggregate::AggregateSynopsis,
        bloom::BloomFilterIndex,
        composite::CompositeIndex,
        exact_set::{
            ExactSetGranularity, ExactSetIndex, ExactSetIndexHeaderV1, ExactSetKeyKind,
            ExactSetRepresentation,
        },
        inverted::InvertedMorselIndex,
        lookup::LookupIndex,
        topn::TopNSummary,
    },
    interop::lakehouse::{LakehouseHints, LakehouseMetadataUse, LakehouseOverlayDecision},
    io_hints::IoHints,
    kernel::KernelCapabilities,
    metadata::MetadataJson,
    mount::{
        mount_cove_file, mount_cove_h_file, ExecutionCodeRequest, ExecutionCodeResolver,
        ExecutionCodeValue, HarborMountOptions, MountOptions, SidecarValidationStatus,
    },
    page::{ColumnPageIndex, ColumnPageIndexEntryV1, PageIndex},
    page_payload::{ColumnPagePayloadV1, PageBufferKind},
    profile::{
        cove_e::{
            CodeSpaceDescriptorV1, EngineMountPolicyV1, EngineProfileRegistry,
            ExecutionCodeDescriptorV1, ExecutionScopeDescriptorV1,
        },
        cove_h::HarborMountHintsV1,
        cove_map::EmbeddedMapSection,
        cove_o::{
            read_object_surface_from_base_and_delta_files_with_options,
            read_object_surface_from_bytes, reconstruct_object_states,
            reconstruct_object_states_from_base_and_delta_files, CoveObjectReadOptions,
            CoveObjectReconstructionOptions, CoveObjectState, CoveObjectSurface,
            CoveObjectTombstoneStatus, ObjectTypeCatalog, RecordKind, TemporalBloomIndex,
            TemporalSegmentIndex, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            OBJECT_TYPE_FLAG_LINK_OBJECT, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
            PROPERTY_FLAG_EVIDENCE_REF, PROPERTY_FLAG_MAPPING_RULE_REF,
        },
    },
    pruning::{
        explain_aggregate_synopsis, explain_bloom_membership, explain_composite_zone,
        explain_file_code_equality, explain_inverted_morsel_lookup, explain_is_not_null,
        explain_is_null, explain_lookup_index_point, explain_numcode_range,
        explain_resolved_domain_rank_range, PruningEvidence, PruningExplanation,
    },
    reader::{self, OptionalPushdownPolicy, ValidationOptions},
    redaction::RedactionManifest,
    row_ref::RowRef,
    segment::{RowMorselDirectory, TableSegmentHeaderV1, TableSegmentIndex, TableSegmentPayloadV1},
    sort::{ClusteringKeyEntryV1, SortKeyEntryV1},
    table::TableCatalog,
    utility::hex_encode,
    wire::{
        decode_u64_leb128, encode_u64_leb128, parse_bool_strict, read_u32_le_checked,
        zigzag_decode_i64, zigzag_encode_i64,
    },
    zone_stats::{
        NumericStatValue, StatKind, StatScalar, ZoneScope, ZoneStatFlags, ZoneStats, ZoneStatsEntry,
    },
    CoveError,
};
use cove_coverage::{
    can_use_proof_for_pruning, CoveragePlanCandidateV2, CoverageProofRecordV2,
    CoverageProviderDescriptorV2, CoverageSetV2, IntervalPredicateV2, PredicateNormalFormV2,
};
use cove_index::{
    execution::{
        CoviAggregateKindV2, CoviIndexOnlyRequestV2, CoviLookupKeyV2, CoviLookupRequestV2,
        CoviLookupTargetV2, CoviValidationContextV2, ValidatedCoviArtifactV2,
    },
    CoviArtifactV2, IndexCapabilityV2, IndexOnlyCapabilityV2,
};
use cove_layout::{
    FastMetadataIndexV2, LayoutPlanV2, PageClusterDirectoryV2, ScanSplitIndexV2,
    ZeroCopyBufferMapV2, ZeroCopyCompatibilityContext, ZeroCopyCompatibilityV2,
    ZeroCopyMaterializationReasonV2,
};
use cove_map::ProjectionFormat;
use cove_profile_validation::EmbeddedOptionalProfileValidator;
use cove_runtime::{
    unsupported_required_hints, validate_hints, RuntimeCompatibilityHintV2, RuntimeHintKindV2,
};
use delta_validation::*;
use encoding_validation::*;
use extension_validation::*;
use feature_runtime_validation::*;
use manifest::Entry;
use map_validation::*;
use pruning_validation::*;
use suite_validation::*;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() < 2 {
        eprintln!("Usage: cove-conformance <corpus-dir> [--manifest <manifest.jsonl>]");
        process::exit(2);
    }
    let corpus = Path::new(&args[1]);
    let manifest_path = match parse_manifest_arg(&args[2..]) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    };
    let entries = match manifest_path {
        Some(path) => manifest::load_manifest_path(&path),
        None => manifest::load_manifest(corpus),
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    };
    let all_ok = runner::run_entries(corpus, &entries, validate_fixture);
    process::exit(if all_ok { 0 } else { 1 });
}

fn parse_manifest_arg(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--manifest" => Ok(Some(PathBuf::from(path))),
        _ => Err("Usage: cove-conformance <corpus-dir> [--manifest <manifest.jsonl>]".into()),
    }
}

fn validate_fixture(entry: &Entry, corpus: &Path, bytes: &[u8]) -> Result<(), CoveError> {
    match entry.kind.as_str() {
        "cove" => validate_cove_fixture(bytes),
        "covemap" => CovemapFile::parse(bytes).and_then(|file| file.validate_map_sections()),
        "covx" => CovxFile::parse(bytes).map(|_| ()),
        "covm" => CovmFile::parse(bytes).map(|_| ()),
        "coveai" | "covev" => CoveAiFile::parse(bytes).map(|_| ()),
        "covm_delta_chain_extension" => CovmDeltaChainExtensionV1::parse(bytes).map(|_| ()),
        "covm_delta_chain_summary" => CovmDeltaChainSummaryV1::parse(bytes).map(|_| ()),
        "covm_delta_chain_selection_case" => validate_covm_delta_chain_selection_fixture(bytes),
        "covm_delta_pruning_case" => validate_covm_delta_pruning_fixture(bytes),
        "covedelta" => CoveDeltaFile::parse(bytes).map(|_| ()),
        "covedelta_object_delta" => CoveDeltaFile::parse(bytes)
            .and_then(|file| file.validate_object_delta_sections().map(|_| ())),
        "covedelta_layout_hints" => {
            CoveDeltaFile::parse(bytes).and_then(|file| file.validate_layout_hints().map(|_| ()))
        }
        "covedelta_continuation_anchor" => DeltaContinuationAnchorV1::parse(bytes).map(|_| ()),
        "covedelta_existing_patch_anchor" => DeltaContinuationAnchorV1::parse(bytes)
            .and_then(|anchor| anchor.validate_for_existing_object_patch()),
        "covedelta_branch_identity" => DeltaBranchIdentityV1::parse(bytes).map(|_| ()),
        "covedelta_state_hash_descriptor" => DeltaStateHashDescriptorV1::parse(bytes).map(|_| ()),
        "covedelta_state_hash_case" => validate_covedelta_state_hash_fixture(bytes),
        "covedelta_sparse_patch_record" => DeltaSparsePatchRecordV1::parse(bytes).map(|_| ()),
        "covedelta_sparse_patch_state_case" => validate_covedelta_sparse_patch_state_fixture(bytes),
        "covedelta_object_membership_case" => validate_covedelta_object_membership_fixture(bytes),
        "covedelta_reconstruction_case" => validate_covedelta_reconstruction_fixture(bytes),
        "covedelta_covi_tombstone_overlay_case" => {
            validate_covedelta_covi_tombstone_overlay_fixture(bytes)
        }
        "covedelta_touched_object_range" => DeltaTouchedObjectRangeV1::parse(bytes).map(|_| ()),
        "covi" => CoviArtifactV2::parse(bytes).map(|_| ()),
        "covi_validation_case" => validate_covi_validation_fixture(bytes),
        "cove_codec_descriptors" => CodecExtensionDescriptorV2::parse_many(bytes).map(|_| ()),
        "cove_codec_envelopes" => RegisteredEncodingEnvelopeV2::parse_many(bytes).map(|_| ()),
        "section_feature_binding" => SectionFeatureBindingSectionV2::parse(bytes).map(|_| ()),
        "feature_scope_use_case" => validate_feature_scope_use_fixture(entry, bytes),
        "fast_metadata_index" => FastMetadataIndexV2::parse(bytes).map(|_| ()),
        "page_cluster_directory" => PageClusterDirectoryV2::parse(bytes).map(|_| ()),
        "cove_layout_plan" => LayoutPlanV2::parse(bytes).map(|_| ()),
        "cove_layout_scan_split" => ScanSplitIndexV2::parse(bytes).map(|_| ()),
        "zero_copy_map" => ZeroCopyBufferMapV2::parse(bytes).map(|_| ()),
        "zero_copy_compat_case" => validate_zero_copy_compat_fixture(bytes),
        "cove_runtime_hints" => {
            let hints = RuntimeCompatibilityHintV2::parse_many(bytes)?;
            validate_hints(&hints)
        }
        "runtime_operation_case" => validate_runtime_operation_fixture(bytes),
        "cove_coverage_providers" => CoverageProviderDescriptorV2::parse_many(bytes).map(|_| ()),
        "cove_coverage_set" => CoverageSetV2::parse(bytes).map(|_| ()),
        "coverage_proof_records" => CoverageProofRecordV2::parse_many(bytes).map(|_| ()),
        "coverage_proof_case" => validate_coverage_proof_fixture(bytes),
        "predicate_normal_form" => PredicateNormalFormV2::parse_many(bytes).map(|_| ()),
        "interval_predicate" => IntervalPredicateV2::parse_many(bytes).map(|_| ()),
        "coverage_plan_candidates" => CoveragePlanCandidateV2::parse_many(bytes).map(|_| ()),
        "cove_index_capabilities" => IndexCapabilityV2::parse_many(bytes).map(|_| ()),
        "cove_index_only_capabilities" => IndexOnlyCapabilityV2::parse_many(bytes).map(|_| ()),
        "cove_cache" => CoverageCacheV2::parse(bytes).map(|_| ()),
        "extension_registry" => validate_extension_registry_fixture(bytes),
        "extension_logical_type" => validate_extension_logical_type_fixture(entry, bytes),
        "extension_index_descriptor" => validate_extension_index_descriptor_fixture(entry, bytes),
        "durable_publish_case" => validate_durable_publish_fixture(bytes),
        "metadata_json" => MetadataJson::parse(bytes).map(|_| ()),
        "encoding_case" => validate_encoding_fixture(bytes),
        "encoded_array_decode_case" => validate_encoded_array_decode_fixture(bytes),
        "nested_case" => validate_nested_fixture(bytes),
        "arrow_bitmap_case" => validate_arrow_bitmap_fixture(bytes),
        "arrow_export_case" => validate_arrow_export_fixture(bytes),
        "arrow_view_materialization_case" => validate_arrow_view_materialization_fixture(bytes),
        "parquet_conversion_case" => validate_parquet_conversion_fixture(entry, bytes),
        "error_surface_case" => validate_error_surface_fixture(bytes),
        "suite_contract_case" => validate_suite_contract_fixture(corpus, bytes),
        "file_dictionary" => validate_file_dictionary_fixture(bytes),
        "collation_registry" => CollationRegistry::parse(bytes).map(|_| ()),
        "digest_manifest" => DigestManifest::parse(bytes).map(|_| ()),
        "redaction_manifest" => RedactionManifest::parse(bytes).map(|_| ()),
        "io_hints" => IoHints::parse(bytes).map(|_| ()),
        "lakehouse_hints" => LakehouseHints::parse(bytes).map(|_| ()),
        "lakehouse_overlay_guard_case" => validate_lakehouse_overlay_guard_fixture(bytes),
        "kernel_capabilities" => KernelCapabilities::parse(bytes).map(|_| ()),
        "page_index" => PageIndex::parse(bytes).map(|_| ()),
        "column_domain" => ColumnDomain::parse(bytes).map(|_| ()),
        "table_catalog" => TableCatalog::parse(bytes).map(|_| ()),
        "table_segment_index" => TableSegmentIndex::parse(bytes).map(|_| ()),
        "table_segment_header" => TableSegmentHeaderV1::parse(bytes).map(|_| ()),
        "row_morsel_directory" => RowMorselDirectory::parse(
            bytes,
            entry.morsel_count.ok_or_else(|| {
                CoveError::BadSection("row_morsel_directory fixture missing morsel_count".into())
            })?,
        )
        .map(|_| ()),
        "exact_set_index" => ExactSetIndex::parse(bytes).map(|_| ()),
        "bloom_index" => BloomFilterIndex::parse(bytes).map(|_| ()),
        "inverted_morsel_index" => InvertedMorselIndex::parse(bytes).map(|_| ()),
        "lookup_index" => LookupIndex::parse(bytes).map(|_| ()),
        "row_ref" => RowRef::decode(bytes).map(|_| ()),
        "aggregate_synopsis" => AggregateSynopsis::parse(bytes).map(|_| ()),
        "composite_zone_index" => CompositeIndex::parse(bytes).map(|_| ()),
        "topn_summary" => TopNSummary::parse(bytes).map(|_| ()),
        "sort_key" => SortKeyEntryV1::parse(bytes).map(|_| ()),
        "clustering_key" => ClusteringKeyEntryV1::parse(bytes).map(|_| ()),
        "cove_e_engine_registry" => EngineProfileRegistry::parse(bytes).map(|_| ()),
        "cove_e_execution_code" => ExecutionCodeDescriptorV1::parse(bytes).map(|_| ()),
        "cove_e_execution_scope" => ExecutionScopeDescriptorV1::parse(bytes).map(|_| ()),
        "cove_e_code_space" => CodeSpaceDescriptorV1::parse(bytes).map(|_| ()),
        "cove_e_mount_policy" => EngineMountPolicyV1::parse(bytes).map(|_| ()),
        "cove_h_mount_hints" => HarborMountHintsV1::parse(bytes).map(|_| ()),
        "cove_h_mount_case" => validate_harbor_mount_fixture(bytes),
        "cove_o_object_catalog" => ObjectTypeCatalog::parse(bytes).map(|_| ()),
        "cove_o_temporal_segment_index" => TemporalSegmentIndex::parse(bytes).map(|_| ()),
        "cove_o_temporal_bloom_index" => TemporalBloomIndex::parse(bytes).map(|_| ()),
        "cove_map_convert_case" => validate_cove_map_convert_fixture(corpus, bytes),
        "cove_map_candidates_case" => validate_cove_map_candidates_fixture(corpus, bytes),
        "cove_map_replay_case" => validate_cove_map_replay_fixture(corpus, bytes),
        "cove_map_project_case" => validate_cove_map_project_fixture(corpus, bytes),
        "pruning_case" => validate_pruning_fixture(bytes),
        "page_codec_case" => validate_page_codec_fixture(bytes),
        "wire_primitive_case" => validate_wire_primitive_fixture(bytes),
        "sidecar_freshness_case" => validate_sidecar_freshness_fixture(bytes),
        "sidecar_validity_case" => validate_sidecar_validity_fixture(bytes),
        "visibility_safety_case" => validate_visibility_safety_fixture(bytes),
        other => Err(CoveError::BadSection(format!(
            "unknown conformance fixture kind {other}"
        ))),
    }
}

fn validate_cove_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let report = reader::validate_bytes_with_options(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            optional_pushdown_policy: OptionalPushdownPolicy::FailOpen,
        },
    )?;
    EmbeddedOptionalProfileValidator::default_builtins()
        .validate_embedded_optional_profile_sections(
            bytes,
            &report,
            OptionalPushdownPolicy::FailOpen,
            None,
            false,
        )
}
