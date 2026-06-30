//! Stable facade for building, validating, replaying, and projecting COVE-MAP artifacts.
//!
//! Public helpers return typed map errors at API and CLI boundaries while keeping
//! the COVE wire sections, projection JSON, and command-line diagnostics stable.

use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    artifact::covemap::CovemapFile,
    canonical::CanonicalValue,
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile,
        SectionKind, FEATURE_FILE_DICTIONARY, FEATURE_OBJECT_PROFILE, FEATURE_SEMANTIC_MAP,
        FEATURE_TRUST_CHAIN,
    },
    dictionary::{FileDictionary, FileDictionaryEncoding},
    durable,
    page::{ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN},
    page_payload::ColumnPagePayloadV1,
    profile::{
        cove_map::{
            MapAliasEntry, MapIdentityRule, MapJoinKeyComponent, MapNormalizationPipeline,
            MapPropertyBinding, MapResolver, MapRowSemanticRule, SourceOperationKind,
        },
        cove_o::{
            temporal_row_trust_payload, CoveObjectState, ObjectTypeCatalog, ObjectTypeEntryV1,
            PropertyEntryV1, RecordKind, TemporalRowEntryV1, TemporalSegmentData,
            TemporalSegmentHeaderV1, TemporalSegmentIndex, TemporalSegmentIndexEntryV1,
            TrustManifest, TrustManifestEntryV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
            OBJECT_TYPE_FLAG_ENTITY_OBJECT, OBJECT_TYPE_FLAG_LINK_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            PROPERTY_FLAG_ASSOCIATION_TYPE, PROPERTY_FLAG_EVIDENCE_REF,
            PROPERTY_FLAG_MAPPING_RULE_REF, TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
        },
    },
    reader::{validate_bytes_with_options, ValidationOptions},
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    trust_chain,
    writer::{MinimalCoveWriter, SectionPayload},
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

mod alias_import;
mod api;
mod build;
mod candidates;
mod cli;
mod context;
mod emit;
mod identity;
mod identity_resolution;
mod input;
mod map_sections;
mod materialization;
mod materialize_execution;
mod object_reconstruction;
mod parity;
mod project;
mod replay;
mod review;
mod sections;
mod suggest;
mod support;
mod ui;
mod verify;

#[cfg(test)]
use crate::cli::{parse_args, Command, OutputFormat};
pub use api::{
    candidate_matches_from_paths, conversion_report_from_paths, conversion_summary_from_paths,
    cove_o_from_paths, projected_output_from_cove_o_bytes, projected_output_from_cove_o_path,
    projected_output_from_paths, projected_record_batch_from_cove_o_bytes,
    projected_record_batches_from_cove_o_bytes,
    projected_record_batches_from_cove_o_bytes_with_catalog,
    projected_record_batches_from_cove_o_surface_with_catalog, projected_rows_from_cove_o_path,
    projected_rows_from_paths, projection_arrow_schema, projection_catalog_from_cove_o_bytes,
    projection_covi_filter_plan, projection_descriptors_from_cove_o_path,
    projection_read_requirements_for_catalog, verify_replay_report_from_paths, MapApiError,
    MapApiResult, ProjectionColumnDescriptor, ProjectionColumnLineageDescriptor,
    ProjectionCoviFilterDiagnostic, ProjectionCoviFilterLookup, ProjectionCoviFilterPlan,
    ProjectionDescriptor,
};
pub(crate) use api::{parse_map, plan_keys, preview};
use build::verify_from_paths;
pub use build::{
    build_from_cove_o_bytes, build_from_paths, build_semantic_delta_from_paths,
    publish_covm_from_bundle, MapBuildOptions, MapBuildProjectionOutput, MapBuildResult,
    MapBuildSectionCompression, MapEvidenceEncoding, MapSemanticDeltaBuildOptions,
    MapSemanticDeltaBuildResult, MapSemanticDeltaParent,
};
pub(crate) use candidates::candidate_matches;
pub(crate) use context::{mapping_context, MappingContext};
#[cfg(test)]
use emit::build_cove_o;
use emit::build_cove_o_with_source_states;
pub(crate) use identity::{plan_identities, CandidateMatch, PlannedIdentity};
#[cfg(test)]
pub(crate) use identity_resolution::{apply_canonicalization, join_key_tuple, JoinKeyComponent};
pub(crate) use identity_resolution::{
    apply_resolution_pipeline, canonical_component_bytes, goid16_parts,
    join_key_tuple_from_rule_with_context, mapped_goid,
};
#[cfg(test)]
use input::read_csv;
use input::{
    read_source_inputs, read_sources, validate_source_inputs, ObservedSourceState, SourceRow,
};
pub(crate) use map_sections::map_passthrough_sections;
pub use materialization::CoveObjectCheckpointTemporalSection;
pub(crate) use materialization::{
    append_property_value_bytes, file_dictionary_for_model, file_dictionary_index_bytes,
    file_dictionary_key_for_property, nested_shapes_for_model, temporal_segment_index,
    temporal_segment_payload, trust_manifest, MaterializedModel, MaterializedProperty,
    NestedShapeByProperty, ObjectRow, ReconstructedTemporalSegmentBuild, TemporalSegmentBuild,
};
#[cfg(test)]
pub(crate) use materialize_execution::identity_equivalence_index;
pub(crate) use materialize_execution::{
    materialize_with_source_states, object_types_from_mapping, reviewed_decision_replay_binding,
    JoinKeyEvaluation, ResolutionMetadata,
};
pub(crate) use object_reconstruction::{
    build_temporal_segments, dictionary_section, ensure_covemap_payload_envelope, map_section,
    object_section,
};
pub use object_reconstruction::{
    checkpoint_temporal_sections_from_object_states, compact_cove_o_from_object_states,
};
use parity::{parity_from_cove_o_path, parity_from_paths, parity_has_failures, ParityOptions};
use project::{
    diff_maps, project_cove_o_path_output, project_rows_with_source_states_output, run_fixture_path,
};
#[cfg(test)]
use project::{project_cove_o_path, project_rows, property_by_name};
pub use project::{
    ProjectionBatchOptions, ProjectionCandidateRows, ProjectionFilter, ProjectionFilterLiteral,
    ProjectionFilterOp, ProjectionFormat, ProjectionReadRequirements,
};
pub(crate) use replay::verify_replay_report;
pub(crate) use review::review_worklist_from_candidate_matches;
pub(crate) use sections::{embedded_sections, mapping_identity, section_kind};
#[cfg(test)]
use std::path::PathBuf;
use suggest::suggest_from_paths;
pub(crate) use support::*;
pub(crate) use ui::{
    candidate_assertion_id, candidate_match_id, evidence_entry_for_candidate,
    evidence_entry_for_identity, explain, identity_assertion_id, print_json, print_usage,
    write_or_print,
};
use verify::{report_has_failures, verify_bundle_dir};

pub use cli::{run_cli, MapCliError};

#[cfg(test)]
mod tests;
