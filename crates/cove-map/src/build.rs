use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use cove_core::{
    artifact::{
        covedelta::{
            CoveDeltaFile, CoveDeltaFooterV1, CoveDeltaHeaderV1, CoveDeltaObjectValidation,
            CoveDeltaPostscriptV1, CoveDeltaSection, CoveDeltaSectionDirectoryEntryV1,
            CoveDeltaSectionKind, CoveObjectDeltaStateHashPropertyV1, CoveObjectDeltaStateHashV1,
            DeltaContinuationAnchorV1, DeltaDictionaryEntryV1, DeltaInlineValueV1,
            DeltaParentRefV1, DeltaSparsePatchPropertyOpV1, DeltaSparsePatchRecordV1,
            DeltaStateHashDescriptorV1, DeltaTouchedObjectRangeV1,
            DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
            DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE, DELTA_FEATURE_CONTINUATION_ANCHORS,
            DELTA_FEATURE_EXACT_TOMBSTONE_SET, DELTA_FEATURE_EXACT_TOUCHED_SET,
            DELTA_FEATURE_INLINE_DICTIONARY, DELTA_FEATURE_MAP_EVIDENCE_PATCH,
            DELTA_FEATURE_PROJECTION_PATCH, DELTA_FEATURE_SPARSE_PATCH_ROWS,
            DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT, DELTA_OBJECT_STATE_TOMBSTONE_DELETED,
            DELTA_OBJECT_STATE_TOMBSTONE_LIVE, DELTA_OBJECT_STATE_VALUE_NULL,
            DELTA_OBJECT_STATE_VALUE_REDACTED, DELTA_OBJECT_STATE_VALUE_VISIBLE,
            DELTA_PARENT_REF_LINEAGE_PARENT, DELTA_PROPERTY_OP_SET_NULL,
            DELTA_PROPERTY_OP_SET_VALUE, DELTA_REF_NONE,
            DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1, DELTA_TOMBSTONE_KIND_NONE,
        },
        covm::CovmDeltaArtifactRefV1,
    },
    constants::{CoveLogicalType, CovePhysicalKind, DigestAlgorithm, ValueTag},
    dictionary::{FileDictionaryEncoding, FileDictionaryKey},
    digest::compute_digest,
    durable,
    profile::cove_map::{
        compact_evidence_index_bytes, MapEvidenceEntry, MapEvidenceIndex, MapProjectionCatalog,
    },
    profile::cove_o::{
        CoveObjectPropertyValue, CoveObjectState, CoveObjectTombstoneStatus, ObjectTypeCatalog,
        ObjectTypeEntryV1, RecordKind, TemporalRowEntryV1, TemporalSegmentData,
        TemporalSegmentHeaderV1, TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    },
    utility::{build_covm_artifact_from_bytes, hex_encode, CovmInputArtifact},
};
use cove_index::{
    build::{
        build_covi_from_cove_o_bytes, build_projection_columns_covi_for_cove_o_bytes,
        CoviObjectPropertyBuildOptions, CoviProjectionColumnInput,
    },
    CoviArtifactV2,
};
use serde_json::{json, Value};

use crate::{
    emit::{
        build_cove_o_from_materialized_with_options, evidence_encoding_summary,
        CoveOCompressionSummary, EvidenceEncodingSummary,
    },
    file_dictionary_key_for_property,
    input::{read_source_inputs, validate_source_inputs},
    materialize_with_source_states,
    project::projection_catalog_json_value,
    project::{
        project_cove_o_bytes_output_with_lineage, projection_catalog_from_cove_o_bytes_internal,
        projection_sidecar_columns_from_cove_o_bytes, ProjectionFormat, ProjectionLineageContext,
        ProjectionSourceCoveO,
    },
    sections::{embedded_sections, mapping_identity},
    verify::{verify_bundle, VerifiableBundle, VerifiableIndex, VerifiableProjection},
    MaterializedModel, NestedShapeByProperty, ObjectRow, TemporalSegmentBuild,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapBuildProjectionOutput {
    CoveT,
    None,
}

impl MapBuildProjectionOutput {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CoveT => "cove-t",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEvidenceEncoding {
    Compact,
    Expanded,
    Both,
}

impl MapEvidenceEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Expanded => "expanded",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapBuildSectionCompression {
    Zstd,
    None,
}

impl MapBuildSectionCompression {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
            Self::None => "none",
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapBuildOptions {
    pub out_dir: PathBuf,
    pub force: bool,
    pub object_name: Option<String>,
    pub projection_output: MapBuildProjectionOutput,
    pub evidence_encoding: MapEvidenceEncoding,
    pub section_compression: MapBuildSectionCompression,
    pub verify: bool,
    pub publish_covm: bool,
    pub reuse_cache: bool,
}

impl MapBuildOptions {
    pub fn new(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            out_dir: out_dir.into(),
            force: false,
            object_name: None,
            projection_output: MapBuildProjectionOutput::CoveT,
            evidence_encoding: MapEvidenceEncoding::Compact,
            section_compression: MapBuildSectionCompression::Zstd,
            verify: false,
            publish_covm: false,
            reuse_cache: true,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct MapBuildResult {
    pub manifest: Value,
    pub report: Value,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSemanticDeltaParent {
    pub dataset_id: [u8; 16],
    pub parent_snapshot_id: [u8; 16],
    pub chain_ordinal: u32,
    pub chain_depth: u32,
    pub parent_ref: CovmDeltaArtifactRefV1,
}

#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct MapSemanticDeltaBuildOptions {
    pub out: PathBuf,
    pub force: bool,
    pub parent: MapSemanticDeltaParent,
    pub parent_object_types: Vec<ObjectTypeEntryV1>,
    pub parent_object_states: Vec<CoveObjectState>,
    pub parent_evidence_entries: Vec<MapEvidenceEntry>,
    pub parent_projection_catalog: Option<MapProjectionCatalog>,
    pub csn_start: u64,
    pub commit_time_start_us: i64,
    pub source_publish_range_us: Option<(i64, i64)>,
}

#[must_use]
#[derive(Debug, Clone, PartialEq)]
pub struct MapSemanticDeltaBuildResult {
    pub report: Value,
    pub bytes_written: usize,
}

#[derive(Debug, Default)]
struct SemanticDeltaInlineValueTable {
    values: Vec<DeltaInlineValueV1>,
    refs_by_key: BTreeMap<FileDictionaryKey, u32>,
}

impl SemanticDeltaInlineValueTable {
    fn seed_file_dictionary(&mut self, dictionary: &FileDictionaryEncoding) -> Result<(), String> {
        let mut by_code = dictionary
            .assignments()
            .iter()
            .map(|(key, code)| (*code, key.clone()))
            .collect::<Vec<_>>();
        by_code.sort_by_key(|(code, _)| *code);
        for (expected_code, (code, key)) in by_code.into_iter().enumerate() {
            let expected_code =
                u32::try_from(expected_code).map_err(|_| "delta dictionary too large")?;
            if code != expected_code {
                return Err("semantic delta file dictionary codes must be dense".into());
            }
            let value_ref = self.intern_key(key)?;
            if value_ref != code {
                return Err(
                    "semantic delta inline value refs must match seeded dictionary codes".into(),
                );
            }
        }
        Ok(())
    }

    fn intern_key(&mut self, key: FileDictionaryKey) -> Result<u32, String> {
        if let Some(value_ref) = self.refs_by_key.get(&key).copied() {
            return Ok(value_ref);
        }
        let value_ref =
            u32::try_from(self.values.len()).map_err(|_| "too many delta inline values")?;
        self.values.push(DeltaInlineValueV1 {
            value_ref,
            value_tag: key.value_tag,
            flags: 0,
            value: key.canonical.clone(),
            checksum: 0,
        });
        self.refs_by_key.insert(key, value_ref);
        Ok(value_ref)
    }

    fn values(&self) -> &[DeltaInlineValueV1] {
        &self.values
    }

    fn dictionary_overlay_entries(
        &self,
        dictionary: &FileDictionaryEncoding,
    ) -> Result<Vec<DeltaDictionaryEntryV1>, String> {
        let mut by_code = dictionary
            .assignments()
            .iter()
            .map(|(key, code)| (*code, key))
            .collect::<Vec<_>>();
        by_code.sort_by_key(|(code, _)| *code);
        let mut entries = Vec::with_capacity(by_code.len());
        for (code, key) in by_code {
            let inline_value_ref =
                self.refs_by_key.get(key).copied().ok_or_else(|| {
                    "semantic delta dictionary value was not interned".to_string()
                })?;
            entries.push(DeltaDictionaryEntryV1 {
                local_dictionary_id: 0,
                local_code: code,
                logical_type: logical_type_for_value_tag(key.value_tag)? as u16,
                collation_id: 0,
                entry_kind: DELTA_DICTIONARY_ENTRY_KIND_INLINE_VALUE,
                flags: 0,
                inline_value_ref,
                parent_ref: DELTA_REF_NONE,
                parent_dictionary_id: 0,
                parent_code: 0,
                parent_dictionary_digest_ref: DELTA_REF_NONE,
                canonical_hash128: [0; 16],
                checksum: 0,
            });
        }
        Ok(entries)
    }
}

fn logical_type_for_value_tag(value_tag: u16) -> Result<CoveLogicalType, String> {
    match ValueTag::from_u16(value_tag).ok_or_else(|| "unknown delta value tag".to_string())? {
        ValueTag::Null => Ok(CoveLogicalType::Null),
        ValueTag::BoolFalse | ValueTag::BoolTrue => Ok(CoveLogicalType::Bool),
        ValueTag::Int64 => Ok(CoveLogicalType::Int64),
        ValueTag::UInt64 => Ok(CoveLogicalType::UInt64),
        ValueTag::Float32Bits => Ok(CoveLogicalType::Float32),
        ValueTag::Float64Bits => Ok(CoveLogicalType::Float64),
        ValueTag::Decimal64 => Ok(CoveLogicalType::Decimal64),
        ValueTag::Decimal128 => Ok(CoveLogicalType::Decimal128),
        ValueTag::DateDays => Ok(CoveLogicalType::DateDays),
        ValueTag::TimestampMicros => Ok(CoveLogicalType::TimestampMicros),
        ValueTag::TimestampNanos => Ok(CoveLogicalType::TimestampNanos),
        ValueTag::Utf8 => Ok(CoveLogicalType::Utf8),
        ValueTag::Binary => Ok(CoveLogicalType::Binary),
        ValueTag::Uuid => Ok(CoveLogicalType::Uuid),
        ValueTag::Json => Ok(CoveLogicalType::Json),
        ValueTag::List => Ok(CoveLogicalType::List),
        ValueTag::Struct => Ok(CoveLogicalType::Struct),
        ValueTag::Map => Ok(CoveLogicalType::Map),
        _ => Err("unsupported future delta value tag".into()),
    }
}

#[derive(Debug, Clone)]
struct PendingArtifact {
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct ProjectionArtifact {
    projection_id: String,
    output_table: Option<String>,
    relative_path: PathBuf,
    byte_size: usize,
}

#[derive(Debug, Clone)]
struct IndexArtifact {
    index_id: String,
    target: String,
    relative_path: PathBuf,
    byte_size: usize,
    root_count: usize,
}

#[derive(Debug, Clone)]
struct BuiltSemanticDelta {
    bytes: Vec<u8>,
    report: Value,
}

#[derive(Debug, Clone)]
struct CovmArtifact {
    relative_path: PathBuf,
    bytes: Vec<u8>,
    byte_size: usize,
    digest: String,
    report: Value,
}

#[derive(Debug, Clone)]
struct SkippedProjection {
    projection_id: String,
    output_table: Option<String>,
    reason: String,
}

struct ProjectionArtifactsResult {
    artifacts: Vec<ProjectionArtifact>,
    skipped: Vec<SkippedProjection>,
    files: Vec<PendingArtifact>,
    verifiable: Vec<VerifiableProjection>,
}

struct IndexArtifactsResult {
    artifacts: Vec<IndexArtifact>,
    files: Vec<PendingArtifact>,
    verifiable: Vec<VerifiableIndex>,
}

struct BuildReportContext<'a> {
    materialized: &'a MaterializedModel,
    projection_output: MapBuildProjectionOutput,
    evidence_encoding: MapEvidenceEncoding,
    section_compression: MapBuildSectionCompression,
    object_relative: &'a Path,
    object_bytes: usize,
    evidence_summary: &'a EvidenceEncodingSummary,
    compression_summary: &'a CoveOCompressionSummary,
    covm: Option<&'a CovmArtifact>,
    indexes: &'a [IndexArtifact],
    projections: &'a [ProjectionArtifact],
    skipped: &'a [SkippedProjection],
    warnings: &'a [String],
    verification: Option<&'a Value>,
}

pub fn build_from_paths(
    map: &Path,
    sources: &[PathBuf],
    options: MapBuildOptions,
) -> Result<MapBuildResult, String> {
    if sources.is_empty() {
        return Err("map build requires at least one source path".into());
    }
    let file = crate::parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    let materialized = materialize_with_source_states(&file, &inputs.rows, &inputs.states)?;
    let object_output = build_cove_o_from_materialized_with_options(
        &file,
        &materialized,
        options.evidence_encoding,
        options.section_compression,
    )?;
    let object_bytes = object_output.bytes;
    let compression_summary = object_output.compression_summary;
    let evidence_summary = evidence_encoding_summary(&materialized, options.evidence_encoding)?;
    let (mapping_id, mapping_version) = mapping_identity(&file).unwrap_or_else(|_| {
        (
            map.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("mapping")
                .to_string(),
            file.mapping_version.clone(),
        )
    });
    let object_name = object_file_name(&options, map, &mapping_id)?;
    let object_relative = PathBuf::from(&object_name);
    let mapping_digest = fs::read(map)
        .map(|bytes| format!("sha256:{}", crate::sha256_hex(&bytes)))
        .map_err(|err| format!("cannot read {}: {err}", map.display()))?;
    let covm_artifact = if options.publish_covm {
        Some(covm_artifact_for_object(&object_relative, &object_bytes)?)
    } else {
        None
    };
    let lineage = ProjectionLineageContext {
        source_cove_o: Some(ProjectionSourceCoveO {
            path: Some(path_string(&object_relative)),
            label: path_string(&object_relative),
            digest: Some(format!("sha256:{}", crate::sha256_hex(&object_bytes))),
        }),
        mapping_artifact_digest: Some(mapping_digest),
        covm_manifest: covm_artifact.as_ref().map(|artifact| {
            format!(
                "{} {}",
                path_string(&artifact.relative_path),
                artifact.digest
            )
        }),
    };
    let projection_result =
        projection_artifacts(&object_bytes, options.projection_output, Some(&lineage))?;
    let index_result = index_artifacts(&object_bytes)?;
    let mut warnings = build_warnings(options.projection_output, &projection_result.artifacts);
    let mut artifacts = Vec::new();
    artifacts.push(PendingArtifact {
        relative_path: object_relative.clone(),
        bytes: object_bytes.clone(),
    });
    if let Some(covm) = &covm_artifact {
        artifacts.push(PendingArtifact {
            relative_path: covm.relative_path.clone(),
            bytes: covm.bytes.clone(),
        });
    }
    artifacts.extend(index_result.files.clone());
    artifacts.extend(projection_result.files.clone());

    let report_relative = PathBuf::from("map-build-report.json");
    let readme_relative = PathBuf::from("README.md");
    let manifest_relative = PathBuf::from("map-build-manifest.json");
    let report = build_report(BuildReportContext {
        materialized: &materialized,
        projection_output: options.projection_output,
        evidence_encoding: options.evidence_encoding,
        section_compression: options.section_compression,
        object_relative: &object_relative,
        object_bytes: object_bytes.len(),
        evidence_summary: &evidence_summary,
        compression_summary: &compression_summary,
        covm: covm_artifact.as_ref(),
        indexes: &index_result.artifacts,
        projections: &projection_result.artifacts,
        skipped: &projection_result.skipped,
        warnings: &warnings,
        verification: None,
    });
    let verification = if options.verify {
        let verification = verify_bundle(&VerifiableBundle {
            object_relative_path: object_relative.clone(),
            object_bytes: object_bytes.clone(),
            report: report.clone(),
            indexes: index_result.verifiable.clone(),
            projections: projection_result.verifiable.clone(),
        });
        if verification
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
        {
            return Err(format!(
                "map build verification failed: {}",
                serde_json::to_string(&verification["errors"]).unwrap_or_default()
            ));
        }
        warnings.extend(verification_warning_strings(&verification));
        Some(verification)
    } else {
        None
    };
    let report = build_report(BuildReportContext {
        materialized: &materialized,
        projection_output: options.projection_output,
        evidence_encoding: options.evidence_encoding,
        section_compression: options.section_compression,
        object_relative: &object_relative,
        object_bytes: object_bytes.len(),
        evidence_summary: &evidence_summary,
        compression_summary: &compression_summary,
        covm: covm_artifact.as_ref(),
        indexes: &index_result.artifacts,
        projections: &projection_result.artifacts,
        skipped: &projection_result.skipped,
        warnings: &warnings,
        verification: verification.as_ref(),
    });
    let report_bytes = json_bytes(&report)?;
    let readme_bytes = readme_bytes(
        &mapping_id,
        &object_relative,
        covm_artifact.as_ref(),
        &index_result.artifacts,
        &projection_result.artifacts,
    );
    let manifest = build_manifest(
        map,
        sources,
        &materialized,
        &mapping_id,
        &mapping_version,
        options.projection_output,
        options.evidence_encoding,
        options.section_compression,
        &object_relative,
        object_bytes.len(),
        &evidence_summary,
        &compression_summary,
        covm_artifact.as_ref(),
        &index_result.artifacts,
        &projection_result.artifacts,
        &report_relative,
        report_bytes.len(),
        &readme_relative,
        readme_bytes.len(),
        &manifest_relative,
        &warnings,
        verification.as_ref(),
    );
    let manifest_bytes = json_bytes(&manifest)?;

    artifacts.push(PendingArtifact {
        relative_path: report_relative,
        bytes: report_bytes,
    });
    artifacts.push(PendingArtifact {
        relative_path: readme_relative,
        bytes: readme_bytes,
    });

    prepare_output_paths(
        &options.out_dir,
        options.force,
        &artifacts,
        &manifest_relative,
    )?;
    write_artifacts(&options.out_dir, &artifacts)?;
    write_one_artifact(&options.out_dir, &manifest_relative, &manifest_bytes)?;
    Ok(MapBuildResult { manifest, report })
}

pub fn build_from_cove_o_bytes(
    source_label: &str,
    object_bytes: Vec<u8>,
    options: MapBuildOptions,
) -> Result<MapBuildResult, String> {
    let object_name = if let Some(name) = &options.object_name {
        validate_object_name(name)?;
        name.clone()
    } else {
        "delta_snapshot.cove".into()
    };
    let object_relative = PathBuf::from(object_name);
    let source_digest = format!("sha256:{}", crate::sha256_hex(&object_bytes));
    let covm_artifact = if options.publish_covm {
        Some(covm_artifact_for_object(&object_relative, &object_bytes)?)
    } else {
        None
    };
    let lineage = ProjectionLineageContext {
        source_cove_o: Some(ProjectionSourceCoveO {
            path: Some(path_string(&object_relative)),
            label: source_label.to_string(),
            digest: Some(source_digest.clone()),
        }),
        mapping_artifact_digest: None,
        covm_manifest: covm_artifact.as_ref().map(|artifact| {
            format!(
                "{} {}",
                path_string(&artifact.relative_path),
                artifact.digest
            )
        }),
    };
    let projection_result = if options.projection_output == MapBuildProjectionOutput::None {
        empty_projection_artifacts_result()
    } else {
        projection_artifacts(&object_bytes, options.projection_output, Some(&lineage))?
    };
    let index_result = index_artifacts(&object_bytes)?;
    let mut warnings = build_warnings(options.projection_output, &projection_result.artifacts);
    let mut artifacts = Vec::new();
    artifacts.push(PendingArtifact {
        relative_path: object_relative.clone(),
        bytes: object_bytes.clone(),
    });
    if let Some(covm) = &covm_artifact {
        artifacts.push(PendingArtifact {
            relative_path: covm.relative_path.clone(),
            bytes: covm.bytes.clone(),
        });
    }
    artifacts.extend(index_result.files.clone());
    artifacts.extend(projection_result.files.clone());

    let report_relative = PathBuf::from("map-delta-build-report.json");
    let readme_relative = PathBuf::from("README.md");
    let manifest_relative = PathBuf::from("map-delta-build-manifest.json");
    let verification = if options.verify {
        let report = delta_build_report(
            source_label,
            &source_digest,
            &object_relative,
            object_bytes.len(),
            covm_artifact.as_ref(),
            &index_result.artifacts,
            &projection_result.artifacts,
            &projection_result.skipped,
            &warnings,
            None,
        );
        let verification = verify_bundle(&VerifiableBundle {
            object_relative_path: object_relative.clone(),
            object_bytes: object_bytes.clone(),
            report,
            indexes: index_result.verifiable.clone(),
            projections: projection_result.verifiable.clone(),
        });
        if verification
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
        {
            return Err(format!(
                "map delta build verification failed: {}",
                serde_json::to_string(&verification["errors"]).unwrap_or_default()
            ));
        }
        warnings.extend(verification_warning_strings(&verification));
        Some(verification)
    } else {
        None
    };
    let report = delta_build_report(
        source_label,
        &source_digest,
        &object_relative,
        object_bytes.len(),
        covm_artifact.as_ref(),
        &index_result.artifacts,
        &projection_result.artifacts,
        &projection_result.skipped,
        &warnings,
        verification.as_ref(),
    );
    let report_bytes = json_bytes(&report)?;
    let readme_bytes = readme_bytes(
        "delta_snapshot",
        &object_relative,
        covm_artifact.as_ref(),
        &index_result.artifacts,
        &projection_result.artifacts,
    );
    let manifest = delta_build_manifest(
        source_label,
        &source_digest,
        options.projection_output,
        &object_relative,
        object_bytes.len(),
        covm_artifact.as_ref(),
        &index_result.artifacts,
        &projection_result.artifacts,
        &report_relative,
        report_bytes.len(),
        &readme_relative,
        readme_bytes.len(),
        &manifest_relative,
        &warnings,
        verification.as_ref(),
    );
    let manifest_bytes = json_bytes(&manifest)?;
    artifacts.push(PendingArtifact {
        relative_path: report_relative,
        bytes: report_bytes,
    });
    artifacts.push(PendingArtifact {
        relative_path: readme_relative,
        bytes: readme_bytes,
    });

    prepare_output_paths(
        &options.out_dir,
        options.force,
        &artifacts,
        &manifest_relative,
    )?;
    write_artifacts(&options.out_dir, &artifacts)?;
    write_one_artifact(&options.out_dir, &manifest_relative, &manifest_bytes)?;
    Ok(MapBuildResult { manifest, report })
}

pub fn build_semantic_delta_from_paths(
    map: &Path,
    sources: &[PathBuf],
    options: MapSemanticDeltaBuildOptions,
) -> Result<MapSemanticDeltaBuildResult, String> {
    if sources.is_empty() {
        return Err("map semantic delta build requires at least one source path".into());
    }
    if options.out.exists() && !options.force {
        return Err(format!(
            "{} already exists; pass --force to replace it",
            options.out.display()
        ));
    }
    let built = build_semantic_delta_bytes_from_paths(map, sources, &options)?;
    durable::durable_replace(&options.out, &built.bytes)
        .map_err(|err| format!("cannot durably publish {}: {err}", options.out.display()))?;
    Ok(MapSemanticDeltaBuildResult {
        report: built.report,
        bytes_written: built.bytes.len(),
    })
}

fn build_semantic_delta_bytes_from_paths(
    map: &Path,
    sources: &[PathBuf],
    options: &MapSemanticDeltaBuildOptions,
) -> Result<BuiltSemanticDelta, String> {
    let file = crate::parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    let mut materialized = materialize_with_source_states(&file, &inputs.rows, &inputs.states)?;
    apply_semantic_delta_identity_migrations(
        &mut materialized,
        &options.parent_evidence_entries,
        &options.parent_object_states,
    )?;
    assign_semantic_delta_record_kinds(&mut materialized, &options.parent_object_states);

    let nested_shapes = crate::nested_shapes_for_model(&file, &materialized)?;
    reject_semantic_delta_unsupported_encodings(&materialized)?;
    let file_dictionary = crate::file_dictionary_for_model(&materialized, &nested_shapes)?;
    let mut inline_values = SemanticDeltaInlineValueTable::default();
    if let Some(file_dictionary) = &file_dictionary {
        inline_values.seed_file_dictionary(file_dictionary)?;
    }
    let segments =
        crate::build_temporal_segments(&materialized, &nested_shapes, file_dictionary.as_ref())?;
    if segments.is_empty() {
        return Err("map semantic delta build produced no object rows".into());
    }

    let object_catalog_patch =
        additive_object_catalog_patch(&options.parent_object_types, &materialized.object_types)?;
    let object_catalog_payload = object_catalog_patch
        .serialize()
        .map_err(|err| format!("cannot serialize semantic delta object catalog patch: {err}"))?;
    let evidence_patch = additive_evidence_patch(&materialized, &options.parent_evidence_entries)?;
    let evidence_payload = evidence_patch
        .as_ref()
        .map(|patch| {
            compact_evidence_index_bytes(patch)
                .map_err(|err| format!("cannot compact semantic delta evidence patch: {err}"))
        })
        .transpose()?;
    let projection_payload =
        semantic_delta_projection_payload(&file, options.parent_projection_catalog.as_ref())?;
    let mut required_features = 0u64;
    if evidence_payload.is_some() {
        required_features |= DELTA_FEATURE_MAP_EVIDENCE_PATCH;
    }
    if projection_payload.is_some() {
        required_features |= DELTA_FEATURE_PROJECTION_PATCH;
    }

    let mut sections = Vec::new();
    if !object_catalog_patch.types.is_empty() {
        sections.push(delta_section(
            CoveDeltaSectionKind::CatalogPatch,
            object_catalog_patch.types.len() as u64,
            0,
            object_catalog_payload.clone(),
        ));
    }
    if let Some(evidence_payload) = evidence_payload.clone() {
        sections.push(delta_section(
            CoveDeltaSectionKind::EvidencePatch,
            evidence_patch
                .as_ref()
                .map_or(0, |patch| patch.entries.len() as u64),
            0,
            evidence_payload,
        ));
    }
    if let Some(projection_payload) = projection_payload.clone() {
        sections.push(delta_section(
            CoveDeltaSectionKind::ProjectionPatch,
            1,
            DELTA_FEATURE_PROJECTION_PATCH,
            projection_payload,
        ));
    }

    let mut next_csn = options.csn_start;
    let mut next_commit_us = options.commit_time_start_us;
    let mut temporal_row_count = 0u64;
    let mut temporal_section_payloads = Vec::new();
    let mut parsed_temporal_segments = Vec::new();
    for segment in &segments {
        let remapped = remap_delta_temporal_payload(
            &segment.payload,
            &mut next_csn,
            &mut next_commit_us,
            options.commit_time_start_us,
        )?;
        let parsed = TemporalSegmentData::parse(&remapped)
            .map_err(|err| format!("generated delta temporal segment is invalid: {err}"))?;
        temporal_row_count = temporal_row_count.saturating_add(parsed.rows.len() as u64);
        parsed_temporal_segments.push(parsed);
        temporal_section_payloads.push(remapped);
    }
    let sparse_patch_records = semantic_delta_sparse_patch_records(
        &parsed_temporal_segments,
        &segments,
        &options.parent_object_states,
        &nested_shapes,
        &mut inline_values,
    )?;
    if !inline_values.values().is_empty() {
        let payload = inline_values
            .values()
            .iter()
            .map(DeltaInlineValueV1::serialize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("cannot serialize semantic delta inline values: {err}"))?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::StringTable,
            inline_values.values().len() as u64,
            0,
            payload,
        ));
    }
    if let Some(file_dictionary) = &file_dictionary {
        let dictionary_overlay_entries =
            inline_values.dictionary_overlay_entries(file_dictionary)?;
        if !dictionary_overlay_entries.is_empty() {
            required_features |= DELTA_FEATURE_INLINE_DICTIONARY;
            let payload = dictionary_overlay_entries
                .iter()
                .map(DeltaDictionaryEntryV1::serialize)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| {
                    format!("cannot serialize semantic delta dictionary overlay: {err}")
                })?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            sections.push(delta_section(
                CoveDeltaSectionKind::DictionaryOverlay,
                dictionary_overlay_entries.len() as u64,
                DELTA_FEATURE_INLINE_DICTIONARY,
                payload,
            ));
        }
    }
    if let Some(sparse_patch_records) = sparse_patch_records {
        required_features |= DELTA_FEATURE_SPARSE_PATCH_ROWS;
        let payload = sparse_patch_records
            .iter()
            .map(DeltaSparsePatchRecordV1::serialize)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("cannot serialize semantic delta sparse property ops: {err}"))?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::PropertyOps,
            sparse_patch_records.len() as u64,
            DELTA_FEATURE_SPARSE_PATCH_ROWS,
            payload,
        ));
    }
    let touched_object_ranges = semantic_delta_object_ranges(&parsed_temporal_segments, false)?;
    if !touched_object_ranges.is_empty() {
        required_features |= DELTA_FEATURE_EXACT_TOUCHED_SET;
        let payload = touched_object_ranges
            .iter()
            .flat_map(|range| range.serialize())
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::TouchedObjectSet,
            touched_object_ranges.len() as u64,
            DELTA_FEATURE_EXACT_TOUCHED_SET,
            payload,
        ));
    }
    let tombstone_object_ranges = semantic_delta_object_ranges(&parsed_temporal_segments, true)?;
    if !tombstone_object_ranges.is_empty() {
        required_features |= DELTA_FEATURE_EXACT_TOMBSTONE_SET;
        let payload = tombstone_object_ranges
            .iter()
            .flat_map(|range| range.serialize())
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::TombstoneSet,
            tombstone_object_ranges.len() as u64,
            DELTA_FEATURE_EXACT_TOMBSTONE_SET,
            payload,
        ));
    }
    let (continuation_anchors, state_hash_descriptors) = semantic_delta_continuation_anchors(
        &parsed_temporal_segments,
        &options.parent_object_states,
    )?;
    if !continuation_anchors.is_empty() {
        required_features |= DELTA_FEATURE_CONTINUATION_ANCHORS;
        let payload = state_hash_descriptors
            .iter()
            .flat_map(|descriptor| descriptor.serialize())
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::StateHashTable,
            state_hash_descriptors.len() as u64,
            0,
            payload,
        ));
        let payload = continuation_anchors
            .iter()
            .flat_map(|anchor| anchor.serialize())
            .collect::<Vec<_>>();
        sections.push(delta_section(
            CoveDeltaSectionKind::ContinuationAnchors,
            continuation_anchors.len() as u64,
            DELTA_FEATURE_CONTINUATION_ANCHORS,
            payload,
        ));
    }
    for payload in temporal_section_payloads {
        sections.push(delta_section(
            CoveDeltaSectionKind::TemporalSegmentData,
            1,
            0,
            payload,
        ));
    }

    let binding_material = semantic_delta_binding_material(
        map,
        sources,
        &options.parent,
        &object_catalog_payload,
        evidence_payload.as_deref().unwrap_or(&[]),
        projection_payload.as_deref(),
        &sections,
    );
    let artifact_id = digest_id(b"cove-map-semantic-delta-artifact", &binding_material)?;
    let snapshot_id = digest_id(b"cove-map-semantic-delta-snapshot", &binding_material)?;
    let mut header = CoveDeltaHeaderV1::new(
        artifact_id,
        options.parent.dataset_id,
        snapshot_id,
        options.parent.parent_snapshot_id,
    );
    header.chain_ordinal = options.parent.chain_ordinal;
    header.chain_depth = options.parent.chain_depth;
    header.csn_min = options.csn_start;
    header.csn_max = next_csn.saturating_sub(1).max(options.csn_start);
    header.commit_time_range_start_us = options.commit_time_start_us;
    header.commit_time_range_end_us = next_commit_us
        .saturating_sub(1)
        .max(options.commit_time_start_us);
    header.created_at_us = options.commit_time_start_us;
    header.required_delta_features = required_features;
    if !object_catalog_patch.types.is_empty() {
        header.object_catalog_fingerprint_ref =
            fingerprint_ref("object-catalog", &object_catalog_payload);
    }
    if let Some(evidence_payload) = evidence_payload.as_deref() {
        header.semantic_map_fingerprint_ref = fingerprint_ref("semantic-map", evidence_payload);
    }
    if let Some(projection_payload) = projection_payload.as_deref() {
        header.projection_fingerprint_ref = fingerprint_ref("projection", projection_payload);
    }
    if let Some((start, end)) = options.source_publish_range_us {
        if start > end {
            return Err("--source-publish-range start must be <= end".into());
        }
        header.flags |= DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT;
        header.source_publish_range_start_us = start;
        header.source_publish_range_end_us = end;
    }

    let parent = delta_lineage_parent_ref(&options.parent.parent_ref);
    let file = CoveDeltaFile {
        header,
        parent_refs: vec![parent],
        sections,
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
            required_delta_features: required_features,
            optional_delta_features: 0,
            file_len: 0,
            footer_offset: 0,
            footer_length: 0,
            checksum: 0,
        },
    };
    let bytes = file
        .serialize()
        .map_err(|err| format!("cannot serialize map semantic delta: {err}"))?;
    let parsed = CoveDeltaFile::parse(&bytes)
        .map_err(|err| format!("serialized map semantic delta is invalid: {err}"))?;
    let validation = parsed
        .validate_object_delta()
        .map_err(|err| format!("serialized map semantic object delta is invalid: {err}"))?;
    let delta_digest = crate::sha256_hex(&bytes);
    let report = semantic_delta_report(
        map,
        sources,
        &materialized,
        &parsed,
        &validation,
        temporal_row_count,
        &object_catalog_payload,
        evidence_payload.as_deref(),
        projection_payload.as_deref(),
        &delta_digest,
    );
    Ok(BuiltSemanticDelta { bytes, report })
}

fn assign_semantic_delta_record_kinds(
    materialized: &mut MaterializedModel,
    parent_states: &[CoveObjectState],
) {
    let parent_keys = parent_states
        .iter()
        .map(|state| (state.object_type_id, state.goid))
        .collect::<BTreeSet<_>>();
    for row in &mut materialized.rows {
        if row.record_kind == RecordKind::Tombstone {
            continue;
        }
        if parent_keys.contains(&(row.object_type_id, row.goid)) {
            row.record_kind = RecordKind::Delta;
        } else {
            row.record_kind = RecordKind::Baseline;
        }
    }
}

fn reject_semantic_delta_unsupported_encodings(
    materialized: &MaterializedModel,
) -> Result<(), String> {
    for object_type in &materialized.object_types {
        for property in &object_type.properties {
            match property.physical_kind {
                CovePhysicalKind::Boolean
                | CovePhysicalKind::NumCode
                | CovePhysicalKind::FixedBytes
                | CovePhysicalKind::VarBytes
                | CovePhysicalKind::FileCode => {}
                CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
                    return Err(format!(
                        "map semantic delta build does not yet emit nested physical pages; property '{}.{}' uses nested physical encoding",
                        object_type.type_name, property.property_name
                    ));
                }
                _ => {
                    return Err(format!(
                        "map semantic delta build does not support future physical kind {:?} on '{}.{}'",
                        property.physical_kind, object_type.type_name, property.property_name
                    ));
                }
            }
        }
    }
    Ok(())
}

fn additive_object_catalog_patch(
    parent_types: &[ObjectTypeEntryV1],
    generated_types: &[ObjectTypeEntryV1],
) -> Result<ObjectTypeCatalog, String> {
    let parent_by_id = parent_types
        .iter()
        .map(|object_type| (object_type.object_type_id, object_type))
        .collect::<BTreeMap<_, _>>();
    let mut patch_types = Vec::new();
    for generated in generated_types {
        let Some(parent) = parent_by_id.get(&generated.object_type_id).copied() else {
            patch_types.push(generated.clone());
            continue;
        };
        if parent.type_name != generated.type_name || parent.flags != generated.flags {
            return Err(format!(
                "mapping object type '{}' reuses parent object_type_id {} with different semantics",
                generated.type_name, generated.object_type_id
            ));
        }
        let parent_props = parent
            .properties
            .iter()
            .map(|property| (property.property_id, property))
            .collect::<BTreeMap<_, _>>();
        let mut new_properties = Vec::new();
        for generated_property in &generated.properties {
            match parent_props.get(&generated_property.property_id).copied() {
                Some(parent_property) if parent_property != generated_property => {
                    return Err(format!(
                        "mapping property '{}.{}' reuses parent property_id {} with different semantics",
                        generated.type_name,
                        generated_property.property_name,
                        generated_property.property_id
                    ));
                }
                Some(_) => {}
                None => new_properties.push(generated_property.clone()),
            }
        }
        if !new_properties.is_empty() {
            patch_types.push(ObjectTypeEntryV1 {
                object_type_id: generated.object_type_id,
                type_name: generated.type_name.clone(),
                flags: generated.flags,
                properties: new_properties,
            });
        }
    }
    Ok(ObjectTypeCatalog {
        flags: 0,
        types: patch_types,
    })
}

fn additive_evidence_patch(
    materialized: &MaterializedModel,
    parent_entries: &[MapEvidenceEntry],
) -> Result<Option<MapEvidenceIndex>, String> {
    let mut generated = crate::emit::evidence_index_from_materialized(materialized)?;
    let parent_keys = parent_entries
        .iter()
        .map(evidence_entry_key)
        .collect::<BTreeSet<_>>();
    generated
        .entries
        .retain(|entry| !parent_keys.contains(&evidence_entry_key(entry)));
    if generated.entries.is_empty() {
        Ok(None)
    } else {
        Ok(Some(generated))
    }
}

type EvidenceIdentityWithoutOutput = (String, String, String, String);

fn apply_semantic_delta_identity_migrations(
    materialized: &mut MaterializedModel,
    parent_entries: &[MapEvidenceEntry],
    parent_states: &[CoveObjectState],
) -> Result<(), String> {
    if materialized.evidence_entries.is_empty() || parent_entries.is_empty() {
        return Ok(());
    }

    let generated = crate::emit::evidence_index_from_materialized(materialized)?;
    let mut parent_outputs_by_identity =
        BTreeMap::<EvidenceIdentityWithoutOutput, BTreeSet<String>>::new();
    for entry in parent_entries {
        parent_outputs_by_identity
            .entry(evidence_entry_key_without_output(entry))
            .or_default()
            .insert(entry.output_object_id.clone());
    }

    let mut generated_outputs_by_identity =
        BTreeMap::<EvidenceIdentityWithoutOutput, BTreeSet<String>>::new();
    for entry in &generated.entries {
        generated_outputs_by_identity
            .entry(evidence_entry_key_without_output(entry))
            .or_default()
            .insert(entry.output_object_id.clone());
    }

    let parent_states_by_output_id = parent_states
        .iter()
        .map(|state| (hex_encode(&state.goid), state))
        .collect::<BTreeMap<_, _>>();
    let live_parent_states_by_output_id = parent_states_by_output_id
        .iter()
        .filter(|(_, state)| state.tombstone_status == CoveObjectTombstoneStatus::Live)
        .map(|(output_id, state)| (output_id.clone(), *state))
        .collect::<BTreeMap<_, _>>();

    let mut tombstoned = BTreeSet::<(u32, u64, [u8; 16])>::new();
    let mut tombstone_rows = Vec::new();
    for (identity_key, generated_outputs) in generated_outputs_by_identity {
        let Some(parent_outputs) = parent_outputs_by_identity.get(&identity_key) else {
            continue;
        };
        if *parent_outputs == generated_outputs {
            continue;
        }
        for parent_output in parent_outputs.difference(&generated_outputs) {
            let Some(parent_state) = live_parent_states_by_output_id.get(parent_output) else {
                if crate::hex_decode_16(parent_output).is_ok()
                    && !parent_states_by_output_id.contains_key(parent_output)
                {
                    return Err(format!(
                        "map semantic delta build cannot migrate parent evidence output object '{}' because its parent object state was not provided",
                        parent_output
                    ));
                }
                continue;
            };
            let key = (
                parent_state.object_type_id,
                parent_state.branch_key,
                parent_state.goid,
            );
            if tombstoned.insert(key) {
                tombstone_rows.push(semantic_delta_identity_migration_tombstone_row(
                    parent_state,
                )?);
            }
        }
    }

    materialized.rows.extend(tombstone_rows);
    Ok(())
}

fn semantic_delta_identity_migration_tombstone_row(
    parent_state: &CoveObjectState,
) -> Result<ObjectRow, String> {
    let mut material = Vec::new();
    material.extend_from_slice(&parent_state.object_type_id.to_le_bytes());
    material.extend_from_slice(&parent_state.branch_key.to_le_bytes());
    material.extend_from_slice(&parent_state.goid);
    material.extend_from_slice(&parent_state.latest_record_id);
    material.extend_from_slice(&parent_state.csn.to_le_bytes());
    let record_id = digest_id(b"cove-map-semantic-delta-identity-tombstone", &material)?;
    Ok(ObjectRow {
        goid: parent_state.goid,
        record_id,
        object_type_id: parent_state.object_type_id,
        object_type: parent_state.object_type_name.clone(),
        source_id: "__identity_migration__".into(),
        source_row_index: usize::MAX,
        record_kind: RecordKind::Tombstone,
        properties: BTreeMap::new(),
    })
}

fn evidence_entry_key(entry: &MapEvidenceEntry) -> (String, String, String, String, String) {
    (
        entry.source_id.clone(),
        entry.source_row_identity.clone(),
        entry.rule_id.clone(),
        entry.assertion_id.clone(),
        entry.output_object_id.clone(),
    )
}

fn evidence_entry_key_without_output(entry: &MapEvidenceEntry) -> EvidenceIdentityWithoutOutput {
    (
        entry.source_id.clone(),
        entry.source_row_identity.clone(),
        entry.rule_id.clone(),
        entry.assertion_id.clone(),
    )
}

fn semantic_delta_projection_payload(
    file: &cove_core::artifact::covemap::CovemapFile,
    parent_catalog: Option<&MapProjectionCatalog>,
) -> Result<Option<Vec<u8>>, String> {
    let mut merged: Option<MapProjectionCatalog> = None;
    for section in embedded_sections(file)? {
        let cove_core::profile::cove_map::EmbeddedMapSection::ProjectionCatalog(catalog) = section
        else {
            continue;
        };
        match &mut merged {
            Some(existing) => {
                if existing.mapping_id != catalog.mapping_id
                    || existing.mapping_version != catalog.mapping_version
                {
                    return Err(
                        "projection catalogs in one COVE-MAP must share mapping identity".into(),
                    );
                }
                existing.projections.extend(catalog.projections);
            }
            None => merged = Some(catalog),
        }
    }
    let Some(catalog) = merged else {
        return Ok(None);
    };
    let mut catalog = catalog;
    if let Some(parent_catalog) = parent_catalog {
        if parent_catalog.mapping_id != catalog.mapping_id
            || parent_catalog.mapping_version != catalog.mapping_version
        {
            return Err(
                "map semantic delta projection patch cannot change mapping identity".into(),
            );
        }
        let parent_by_id = parent_catalog
            .projections
            .iter()
            .map(|projection| (projection.projection_id.as_str(), projection))
            .collect::<BTreeMap<_, _>>();
        catalog.projections.retain(|projection| {
            parent_by_id
                .get(projection.projection_id.as_str())
                .is_none_or(|parent| *parent != projection)
        });
    }
    if catalog.projections.is_empty() {
        return Ok(None);
    }
    let value = projection_catalog_json_value(&catalog);
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|err| format!("cannot serialize MAP_PROJECTION_CATALOG patch: {err}"))?;
    Ok(Some(crate::ensure_covemap_payload_envelope(
        cove_core::constants::SectionKind::MapProjectionCatalog,
        bytes,
    )))
}

fn semantic_delta_object_ranges(
    segments: &[TemporalSegmentData],
    tombstones_only: bool,
) -> Result<Vec<DeltaTouchedObjectRangeV1>, String> {
    let mut grouped = BTreeMap::<(u32, u32), BTreeSet<[u8; 16]>>::new();
    for segment in segments {
        for row in &segment.rows {
            if tombstones_only && row.record_kind != RecordKind::Tombstone {
                continue;
            }
            let branch_identity_ref = u32::try_from(row.branch_key).map_err(|_| {
                "semantic delta branch key exceeds exact object range ref width".to_string()
            })?;
            grouped
                .entry((segment.header.object_type_id, branch_identity_ref))
                .or_default()
                .insert(row.goid);
        }
    }

    let mut ranges = Vec::with_capacity(grouped.len());
    for ((object_type_id, branch_identity_ref), goids) in grouped {
        let touched_count = u32::try_from(goids.len()).map_err(|_| {
            "semantic delta exact object range touched count exceeds u32".to_string()
        })?;
        let min_goid = *goids
            .first()
            .ok_or_else(|| "semantic delta exact object range is empty".to_string())?;
        let max_goid = *goids
            .last()
            .ok_or_else(|| "semantic delta exact object range is empty".to_string())?;
        ranges.push(DeltaTouchedObjectRangeV1 {
            scope_kind: 0,
            scope_id: [0; 16],
            object_type_id,
            branch_identity_ref,
            min_goid,
            max_goid,
            touched_count,
            property_bitmap_ref: DELTA_REF_NONE,
            object_set_ref: DELTA_REF_NONE,
            checksum: 0,
        });
    }
    Ok(ranges)
}

fn semantic_delta_sparse_patch_records(
    parsed_segments: &[TemporalSegmentData],
    built_segments: &[TemporalSegmentBuild],
    parent_states: &[CoveObjectState],
    nested_shapes: &NestedShapeByProperty,
    inline_values: &mut SemanticDeltaInlineValueTable,
) -> Result<Option<Vec<DeltaSparsePatchRecordV1>>, String> {
    if parsed_segments.len() != built_segments.len() {
        return Err("semantic delta sparse patch segment count mismatch".into());
    }
    let parent_by_key = parent_states
        .iter()
        .map(|state| ((state.object_type_id, state.branch_key, state.goid), state))
        .collect::<BTreeMap<_, _>>();

    let mut records = Vec::new();
    let mut delta_row_count = 0usize;
    for (parsed_segment, built_segment) in parsed_segments.iter().zip(built_segments) {
        if parsed_segment.rows.len() != built_segment.rows.len() {
            return Err("semantic delta sparse patch row count mismatch".into());
        }
        for (row_index, row) in parsed_segment.rows.iter().enumerate() {
            if row.record_kind != RecordKind::Delta {
                continue;
            }
            delta_row_count += 1;
            let generated_row = &built_segment.rows[row_index];
            let Some(parent) = parent_by_key.get(&(
                parsed_segment.header.object_type_id,
                row.branch_key,
                row.goid,
            )) else {
                return Err(format!(
                    "semantic delta sparse patch row for goid {} has no parent state",
                    hex_encode(&row.goid)
                ));
            };
            let parent_properties = parent
                .properties
                .iter()
                .map(|property| (property.property_id, &property.value))
                .collect::<BTreeMap<_, _>>();
            let mut changed_properties = Vec::new();
            for (property_id, property) in &generated_row.properties {
                if parent_properties
                    .get(property_id)
                    .is_some_and(|value| *value == &property.value)
                {
                    continue;
                }
                let (property_op, value_ref) = if property.value.is_null() {
                    (DELTA_PROPERTY_OP_SET_NULL, DELTA_REF_NONE)
                } else {
                    let key = file_dictionary_key_for_property(
                        property.entry.logical_type,
                        &property.value,
                        nested_shapes.get(&(generated_row.object_type_id, *property_id)),
                    )?;
                    (DELTA_PROPERTY_OP_SET_VALUE, inline_values.intern_key(key)?)
                };
                changed_properties.push(DeltaSparsePatchPropertyOpV1 {
                    property_id: *property_id,
                    property_op,
                    tombstone_kind: DELTA_TOMBSTONE_KIND_NONE,
                    value_ref,
                    redaction_ref: DELTA_REF_NONE,
                    flags: 0,
                });
            }
            if changed_properties.is_empty() {
                return Ok(None);
            }
            let branch_identity_ref = u32::try_from(row.branch_key).map_err(|_| {
                "semantic delta branch key exceeds sparse patch ref width".to_string()
            })?;
            records.push(DeltaSparsePatchRecordV1 {
                scope_kind: 0,
                scope_id: [0; 16],
                branch_identity_ref,
                object_type_id: parsed_segment.header.object_type_id,
                goid: row.goid,
                record_id: row.record_id,
                timestamp_us: row.timestamp_us,
                csn: row.csn,
                record_kind: row.record_kind,
                flags: 0,
                changed_properties,
                checksum: 0,
            });
        }
    }

    if delta_row_count == 0 || records.len() != delta_row_count {
        Ok(None)
    } else {
        Ok(Some(records))
    }
}

fn delta_section(
    kind: CoveDeltaSectionKind,
    item_count: u64,
    required_features: u64,
    payload: Vec<u8>,
) -> CoveDeltaSection {
    CoveDeltaSection {
        entry: CoveDeltaSectionDirectoryEntryV1 {
            section_id: 0,
            section_kind: kind as u16,
            flags: 0,
            offset: 0,
            length: 0,
            uncompressed_length: 0,
            item_count,
            compression: 0,
            encryption: 0,
            alignment_log2: 0,
            reserved0: 0,
            required_delta_features: required_features,
            optional_delta_features: 0,
            crc32c: 0,
            checksum: 0,
        },
        payload,
    }
}

fn remap_delta_temporal_payload(
    payload: &[u8],
    next_csn: &mut u64,
    next_commit_us: &mut i64,
    fallback_commit_us: i64,
) -> Result<Vec<u8>, String> {
    let mut out = payload.to_vec();
    let mut header = TemporalSegmentHeaderV1::parse(&out)
        .map_err(|err| format!("cannot parse generated temporal segment header: {err}"))?;
    let row_count = usize::try_from(header.row_count)
        .map_err(|_| "temporal row_count does not fit usize".to_string())?;
    let row_start = usize::try_from(header.row_directory_offset)
        .map_err(|_| "temporal row_directory_offset does not fit usize".to_string())?;
    let row_bytes = row_count
        .checked_mul(TEMPORAL_ROW_ENTRY_LEN)
        .ok_or_else(|| "temporal row bytes overflow".to_string())?;
    let row_end = row_start
        .checked_add(row_bytes)
        .ok_or_else(|| "temporal row end overflow".to_string())?;
    if out.len() < row_end || row_start < TEMPORAL_SEGMENT_HEADER_LEN {
        return Err("temporal row directory is outside generated payload".into());
    }

    let mut min_csn = None::<u64>;
    let mut max_csn = None::<u64>;
    let mut min_time = None::<i64>;
    let mut max_time = None::<i64>;
    for row_index in 0..row_count {
        let start = row_start + row_index * TEMPORAL_ROW_ENTRY_LEN;
        let end = start + TEMPORAL_ROW_ENTRY_LEN;
        let mut row = TemporalRowEntryV1::parse(&out[start..end])
            .map_err(|err| format!("cannot parse generated temporal row: {err}"))?;
        row.csn = *next_csn;
        row.timestamp_us = *next_commit_us;
        *next_csn = next_csn.saturating_add(1);
        *next_commit_us = next_commit_us.saturating_add(1);
        min_csn = Some(min_csn.map_or(row.csn, |current| current.min(row.csn)));
        max_csn = Some(max_csn.map_or(row.csn, |current| current.max(row.csn)));
        min_time = Some(min_time.map_or(row.timestamp_us, |current| current.min(row.timestamp_us)));
        max_time = Some(max_time.map_or(row.timestamp_us, |current| current.max(row.timestamp_us)));
        out[start..end].copy_from_slice(&row.serialize());
    }
    header.csn_min = min_csn.unwrap_or(*next_csn);
    header.csn_max = max_csn.unwrap_or(*next_csn);
    header.time_range_start_us = min_time.unwrap_or(fallback_commit_us);
    header.time_range_end_us = max_time.unwrap_or(fallback_commit_us);
    out[..TEMPORAL_SEGMENT_HEADER_LEN].copy_from_slice(&header.serialize());
    Ok(out)
}

fn semantic_delta_continuation_anchors(
    segments: &[TemporalSegmentData],
    parent_states: &[CoveObjectState],
) -> Result<
    (
        Vec<DeltaContinuationAnchorV1>,
        Vec<DeltaStateHashDescriptorV1>,
    ),
    String,
> {
    let parent_by_key = parent_states
        .iter()
        .map(|state| ((state.object_type_id, state.branch_key, state.goid), state))
        .collect::<BTreeMap<_, _>>();
    let mut anchors = Vec::new();
    let mut descriptors = Vec::new();
    let mut descriptor_by_parent = BTreeMap::<(u32, u64, [u8; 16]), u32>::new();

    for segment in segments {
        for row in &segment.rows {
            if !matches!(row.record_kind, RecordKind::Delta | RecordKind::Tombstone) {
                continue;
            }
            let parent = parent_by_key
                .get(&(segment.header.object_type_id, row.branch_key, row.goid))
                .copied()
                .ok_or_else(|| {
                    format!(
                        "semantic delta row for object_type_id {} goid {} is a {:?} without a parent state",
                        segment.header.object_type_id,
                        hex_encode(&row.goid),
                        row.record_kind
                    )
                })?;
            if parent.csn >= row.csn || parent.timestamp_us > row.timestamp_us {
                return Err(format!(
                    "semantic delta row for goid {} does not advance parent temporal position",
                    hex_encode(&row.goid)
                ));
            }
            let state_hash_ref = match descriptor_by_parent.get(&parent_state_key(parent)).copied()
            {
                Some(existing) => existing,
                None => {
                    let state_hash_ref = u32::try_from(descriptors.len())
                        .map_err(|_| "state hash descriptor ref overflows".to_string())?;
                    let material = semantic_delta_state_hash_material(parent)?;
                    let digest = material
                        .compute_hash(DigestAlgorithm::Sha256)
                        .map_err(|err| format!("cannot compute parent state hash: {err}"))?;
                    descriptors.push(DeltaStateHashDescriptorV1 {
                        state_hash_ref,
                        state_hash_kind: DELTA_STATE_HASH_KIND_COVE_OBJECT_DELTA_STATE_HASH_V1,
                        hash_algorithm: DigestAlgorithm::Sha256 as u16,
                        hash_len: u16::try_from(digest.len())
                            .map_err(|_| "state hash length overflows".to_string())?,
                        hash_payload_ref: state_hash_ref,
                        flags: 0,
                        checksum: 0,
                    });
                    descriptor_by_parent.insert(parent_state_key(parent), state_hash_ref);
                    state_hash_ref
                }
            };
            let branch_identity_ref = u32::try_from(row.branch_key)
                .map_err(|_| "semantic delta branch_key exceeds anchor ref width".to_string())?;
            anchors.push(DeltaContinuationAnchorV1 {
                scope_kind: 0,
                scope_id: [0; 16],
                object_type_id: segment.header.object_type_id,
                branch_identity_ref,
                goid: row.goid,
                parent_ref: 0,
                predecessor_csn: parent.csn,
                predecessor_timestamp_us: parent.timestamp_us,
                predecessor_record_id: parent.latest_record_id,
                predecessor_state_hash_ref: state_hash_ref,
                predecessor_trust_hash_ref: DELTA_REF_NONE,
                anchor_strength: DELTA_ANCHOR_STRENGTH_KEY_RECORD_AND_STATE_HASH,
                flags: 0,
                checksum: 0,
            });
        }
    }

    Ok((anchors, descriptors))
}

fn parent_state_key(state: &CoveObjectState) -> (u32, u64, [u8; 16]) {
    (state.object_type_id, state.branch_key, state.goid)
}

fn semantic_delta_state_hash_material(
    state: &CoveObjectState,
) -> Result<CoveObjectDeltaStateHashV1, String> {
    let mut properties = state
        .properties
        .iter()
        .map(semantic_delta_state_hash_property)
        .collect::<Result<Vec<_>, _>>()?;
    properties.sort_by_key(|property| property.property_id);
    Ok(CoveObjectDeltaStateHashV1 {
        scope_kind: 0,
        scope_id: [0; 16],
        canonical_branch_identity: state.branch_key.to_le_bytes().to_vec(),
        object_type_id: state.object_type_id,
        goid: state.goid,
        predecessor_record_id: state.latest_record_id,
        predecessor_csn: state.csn,
        predecessor_timestamp_us: state.timestamp_us,
        record_kind: state.record_kind,
        tombstone_state: if state.tombstone_status == CoveObjectTombstoneStatus::Tombstoned {
            DELTA_OBJECT_STATE_TOMBSTONE_DELETED
        } else {
            DELTA_OBJECT_STATE_TOMBSTONE_LIVE
        },
        properties,
    })
}

fn semantic_delta_state_hash_property(
    property: &CoveObjectPropertyValue,
) -> Result<CoveObjectDeltaStateHashPropertyV1, String> {
    let encoded = serde_json::to_vec(&property.value)
        .map_err(|err| format!("cannot encode state-hash property value: {err}"))?;
    let (value_state, canonical_value, redaction_commitment) = if property.redacted {
        let commitment = compute_digest(DigestAlgorithm::Sha256, &encoded)
            .map_err(|err| format!("cannot hash redaction commitment: {err}"))?;
        (DELTA_OBJECT_STATE_VALUE_REDACTED, Vec::new(), commitment)
    } else if property.value.is_null() {
        (DELTA_OBJECT_STATE_VALUE_NULL, Vec::new(), Vec::new())
    } else {
        (DELTA_OBJECT_STATE_VALUE_VISIBLE, encoded, Vec::new())
    };
    Ok(CoveObjectDeltaStateHashPropertyV1 {
        property_id: property.property_id,
        logical_type: property.logical_type as u16,
        collation_id: 0,
        value_state,
        canonical_value,
        redaction_commitment,
        hidden_value_commitment: None,
    })
}

fn semantic_delta_binding_material(
    map: &Path,
    sources: &[PathBuf],
    parent: &MapSemanticDeltaParent,
    object_catalog_payload: &[u8],
    evidence_payload: &[u8],
    projection_payload: Option<&[u8]>,
    sections: &[CoveDeltaSection],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"COVE_MAP_SEMANTIC_DELTA_BINDING_V1\0");
    out.extend_from_slice(&parent.dataset_id);
    out.extend_from_slice(&parent.parent_snapshot_id);
    out.extend_from_slice(&parent.chain_ordinal.to_le_bytes());
    out.extend_from_slice(&parent.chain_depth.to_le_bytes());
    out.extend_from_slice(&parent.parent_ref.artifact_id);
    out.extend_from_slice(&parent.parent_ref.snapshot_id);
    out.extend_from_slice(map.display().to_string().as_bytes());
    for source in sources {
        out.extend_from_slice(source.display().to_string().as_bytes());
        out.push(0);
    }
    out.extend_from_slice(object_catalog_payload);
    out.extend_from_slice(evidence_payload);
    if let Some(projection_payload) = projection_payload {
        out.extend_from_slice(projection_payload);
    }
    for section in sections {
        out.extend_from_slice(&(section.entry.section_kind).to_le_bytes());
        out.extend_from_slice(&section.payload);
    }
    out
}

fn digest_id(label: &[u8], material: &[u8]) -> Result<[u8; 16], String> {
    let mut seed = Vec::with_capacity(label.len() + material.len());
    seed.extend_from_slice(label);
    seed.extend_from_slice(material);
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed)
        .map_err(|err| format!("cannot compute semantic delta ID: {err}"))?;
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    Ok(id)
}

fn fingerprint_ref(label: &str, payload: &[u8]) -> u32 {
    let mut material = Vec::with_capacity(label.len() + payload.len());
    material.extend_from_slice(label.as_bytes());
    material.push(0);
    material.extend_from_slice(payload);
    let digest = compute_digest(DigestAlgorithm::Sha256, &material).unwrap_or_default();
    let value = if digest.len() >= 4 {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&digest[..4]);
        u32::from_le_bytes(raw)
    } else {
        1
    };
    if value == 0 {
        1
    } else {
        value
    }
}

fn delta_lineage_parent_ref(parent: &CovmDeltaArtifactRefV1) -> DeltaParentRefV1 {
    DeltaParentRefV1 {
        parent_ref: 0,
        parent_kind: 0,
        flags: DELTA_PARENT_REF_LINEAGE_PARENT,
        artifact_id: parent.artifact_id,
        snapshot_id: parent.snapshot_id,
        file_len: parent.file_len,
        footer_crc32c: parent.footer_crc32c,
        digest_algorithm: parent.digest_algorithm,
        digest_len: parent.digest_len,
        digest_ref: 0,
        uri_ref: parent.uri_ref,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn semantic_delta_report(
    map: &Path,
    sources: &[PathBuf],
    materialized: &MaterializedModel,
    file: &CoveDeltaFile,
    validation: &CoveDeltaObjectValidation,
    temporal_row_count: u64,
    object_catalog_payload: &[u8],
    evidence_payload: Option<&[u8]>,
    projection_payload: Option<&[u8]>,
    delta_digest: &str,
) -> Value {
    json!({
        "format": "cove-map-semantic-delta-build-report-v1",
        "mapping_id": materialized.conversion_report.get("mapping_id").cloned().unwrap_or(Value::Null),
        "mapping_version": materialized.conversion_report.get("mapping_version").cloned().unwrap_or(Value::Null),
        "mapping_path": map.display().to_string(),
        "source_paths": sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "delta": {
            "artifact_id": hex16(&file.header.delta_artifact_id),
            "snapshot_id": hex16(&file.header.snapshot_id),
            "parent_snapshot_id": hex16(&file.header.parent_snapshot_id),
            "chain_ordinal": file.header.chain_ordinal,
            "chain_depth": file.header.chain_depth,
            "csn_min": file.header.csn_min,
            "csn_max": file.header.csn_max,
            "commit_time_range_start_us": file.header.commit_time_range_start_us,
            "commit_time_range_end_us": file.header.commit_time_range_end_us,
            "sha256": delta_digest,
        },
        "counts": {
            "source_rows": materialized.conversion_report.get("row_count").cloned().unwrap_or(Value::Null),
            "object_rows": materialized.rows.len(),
            "temporal_rows": temporal_row_count,
            "evidence_entries": materialized.evidence_entries.len(),
            "catalog_patch_object_types": validation.catalog_patches.iter().map(|patch| patch.types.len()).sum::<usize>(),
            "evidence_patches": validation.evidence_patches.len(),
            "projection_patches": validation.projection_patches.len(),
        },
        "fingerprints": {
            "object_catalog_ref": file.header.object_catalog_fingerprint_ref,
            "object_catalog_sha256": crate::sha256_hex(object_catalog_payload),
            "semantic_map_ref": file.header.semantic_map_fingerprint_ref,
            "semantic_map_sha256": evidence_payload.map(crate::sha256_hex),
            "projection_ref": file.header.projection_fingerprint_ref,
            "projection_sha256": projection_payload.map(crate::sha256_hex),
        },
        "object_delta_validation": {
            "temporal_segments": validation.temporal_segments.len(),
            "catalog_patches": validation.catalog_patches.len(),
            "evidence_patches": validation.evidence_patches.len(),
            "projection_patches": validation.projection_patches.len(),
            "sparse_patch_rows": validation.sparse_patch_records.len(),
            "touched_object_ranges": validation.touched_object_ranges.len(),
            "tombstone_object_ranges": validation.tombstone_object_ranges.len(),
            "continuation_anchors": validation.continuation_anchors.len(),
            "state_hash_descriptors": validation.state_hash_descriptors.len(),
        },
        "unsupported": [],
    })
}

fn hex16(bytes: &[u8; 16]) -> String {
    hex_encode(bytes)
}

fn projection_artifacts(
    object_bytes: &[u8],
    output: MapBuildProjectionOutput,
    lineage: Option<&ProjectionLineageContext>,
) -> Result<ProjectionArtifactsResult, String> {
    let catalog = projection_catalog_from_cove_o_bytes_internal(object_bytes, None, "<map-build>")?;
    let mut used_names = BTreeMap::<String, String>::new();
    let mut projection_artifacts = Vec::new();
    let mut skipped = Vec::new();
    let mut files = Vec::new();
    let mut verifiable = Vec::new();
    for projection in &catalog.projections {
        if output == MapBuildProjectionOutput::None {
            skipped.push(SkippedProjection {
                projection_id: projection.projection_id.clone(),
                output_table: projection.output_table.clone(),
                reason: "projection output disabled".into(),
            });
            continue;
        }
        if !projection
            .output_modes
            .iter()
            .any(|mode| mode == ProjectionFormat::CoveT.as_str())
        {
            skipped.push(SkippedProjection {
                projection_id: projection.projection_id.clone(),
                output_table: projection.output_table.clone(),
                reason: "projection does not declare cove-t output".into(),
            });
            continue;
        }
        let stem = projection
            .output_table
            .as_deref()
            .unwrap_or(&projection.projection_id);
        let file_name = format!("{}.cove", sanitize_file_stem(stem));
        if let Some(existing) =
            used_names.insert(file_name.clone(), projection.projection_id.clone())
        {
            return Err(format!(
                "projection '{}' and '{}' both map to output file '{}'",
                existing, projection.projection_id, file_name
            ));
        }
        let bytes = project_cove_o_bytes_output_with_lineage(
            object_bytes,
            None,
            ProjectionFormat::CoveT,
            Some(&projection.projection_id),
            "<map-build>",
            lineage,
        )?;
        let relative_path = PathBuf::from("projections").join(file_name);
        projection_artifacts.push(ProjectionArtifact {
            projection_id: projection.projection_id.clone(),
            output_table: projection.output_table.clone(),
            relative_path: relative_path.clone(),
            byte_size: bytes.len(),
        });
        files.push(PendingArtifact {
            relative_path: relative_path.clone(),
            bytes: bytes.clone(),
        });
        verifiable.push(VerifiableProjection {
            projection_id: projection.projection_id.clone(),
            output_table: projection.output_table.clone(),
            relative_path,
            bytes,
        });
    }
    Ok(ProjectionArtifactsResult {
        artifacts: projection_artifacts,
        skipped,
        files,
        verifiable,
    })
}

fn empty_projection_artifacts_result() -> ProjectionArtifactsResult {
    ProjectionArtifactsResult {
        artifacts: Vec::new(),
        skipped: Vec::new(),
        files: Vec::new(),
        verifiable: Vec::new(),
    }
}

fn index_artifacts(object_bytes: &[u8]) -> Result<IndexArtifactsResult, String> {
    let object_property_bytes =
        build_covi_from_cove_o_bytes(object_bytes, &CoviObjectPropertyBuildOptions::default())
            .map_err(|err| format!("cannot build COVE-I object-property index: {err}"))?;
    let object_property_artifact = CoviArtifactV2::parse(&object_property_bytes)
        .map_err(|err| format!("generated COVE-I object-property index is invalid: {err}"))?;
    let object_property_relative = PathBuf::from("indexes").join("object_properties.covi");
    let object_property_index = IndexArtifact {
        index_id: "object_properties".into(),
        target: "cove-o-object-properties".into(),
        relative_path: object_property_relative.clone(),
        byte_size: object_property_bytes.len(),
        root_count: object_property_artifact.index_roots.len(),
    };
    let mut indexes = vec![object_property_index.clone()];
    let mut files = vec![PendingArtifact {
        relative_path: object_property_relative.clone(),
        bytes: object_property_bytes.clone(),
    }];
    let mut verifiable = vec![VerifiableIndex {
        index_id: object_property_index.index_id,
        target: object_property_index.target,
        relative_path: object_property_relative,
        bytes: object_property_bytes,
    }];

    let projection_columns =
        match projection_sidecar_columns_from_cove_o_bytes(object_bytes, "<map-build>") {
            Ok(columns) => columns
                .into_iter()
                .map(|column| CoviProjectionColumnInput {
                    table_id: column.table_id,
                    column_id: column.column_id,
                    logical_type: column.logical_type,
                    physical_kind: column.physical_kind,
                    values: column.values,
                })
                .collect::<Vec<_>>(),
            Err(error) if error.contains("embedded MAP_PROJECTION_CATALOG") => Vec::new(),
            Err(error) => return Err(error),
        };
    if !projection_columns.is_empty() {
        let projection_bytes =
            build_projection_columns_covi_for_cove_o_bytes(object_bytes, &projection_columns)
                .map_err(|err| format!("cannot build COVE-I projection-column index: {err}"))?;
        let projection_artifact = CoviArtifactV2::parse(&projection_bytes)
            .map_err(|err| format!("generated COVE-I projection-column index is invalid: {err}"))?;
        let projection_relative = PathBuf::from("indexes").join("projection_columns.covi");
        let projection_index = IndexArtifact {
            index_id: "projection_columns".into(),
            target: "cove-o-projection-columns".into(),
            relative_path: projection_relative.clone(),
            byte_size: projection_bytes.len(),
            root_count: projection_artifact.index_roots.len(),
        };
        indexes.push(projection_index.clone());
        files.push(PendingArtifact {
            relative_path: projection_relative.clone(),
            bytes: projection_bytes.clone(),
        });
        verifiable.push(VerifiableIndex {
            index_id: projection_index.index_id,
            target: projection_index.target,
            relative_path: projection_relative,
            bytes: projection_bytes,
        });
    }
    Ok(IndexArtifactsResult {
        artifacts: indexes,
        files,
        verifiable,
    })
}

fn covm_artifact_for_object(
    object_relative: &Path,
    object_bytes: &[u8],
) -> Result<CovmArtifact, String> {
    let relative_path = PathBuf::from("dataset.covm");
    let output = path_string(&relative_path);
    let input = CovmInputArtifact {
        uri: path_string(object_relative),
        bytes: object_bytes,
    };
    let (bytes, report) = build_covm_artifact_from_bytes(&output, &[input])
        .map_err(|err| format!("cannot build COVM publication manifest: {err}"))?;
    let digest = format!(
        "sha256:{}",
        hex_encode(
            &cove_core::digest::compute_digest(
                cove_core::constants::DigestAlgorithm::Sha256,
                &bytes,
            )
            .map_err(|err| format!("cannot digest generated COVM: {err}"))?
        )
    );
    Ok(CovmArtifact {
        relative_path,
        byte_size: bytes.len(),
        bytes,
        digest,
        report: report.to_json_value(),
    })
}

fn prepare_output_paths(
    out_dir: &Path,
    force: bool,
    artifacts: &[PendingArtifact],
    manifest_relative: &Path,
) -> Result<(), String> {
    if out_dir.exists() && !out_dir.is_dir() {
        return Err(format!(
            "{} exists and is not a directory",
            out_dir.display()
        ));
    }
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        let path = out_dir.join(&artifact.relative_path);
        if !paths.insert(path.clone()) {
            return Err(format!(
                "map build would write duplicate artifact path {}",
                path.display()
            ));
        }
        if path.exists() && !force {
            return Err(format!(
                "{} already exists; pass --force to replace command-owned outputs",
                path.display()
            ));
        }
    }
    let manifest_path = out_dir.join(manifest_relative);
    if !paths.insert(manifest_path.clone()) {
        return Err(format!(
            "map build would write duplicate artifact path {}",
            manifest_path.display()
        ));
    }
    if manifest_path.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to replace command-owned outputs",
            manifest_path.display()
        ));
    }
    Ok(())
}

fn write_artifacts(out_dir: &Path, artifacts: &[PendingArtifact]) -> Result<(), String> {
    for artifact in artifacts {
        write_one_artifact(out_dir, &artifact.relative_path, &artifact.bytes)?;
    }
    Ok(())
}

fn write_one_artifact(out_dir: &Path, relative_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let path = out_dir.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    }
    durable::durable_replace(&path, bytes)
        .map(|_| ())
        .map_err(|err| format!("cannot durably publish {}: {err}", path.display()))
}

fn build_report(ctx: BuildReportContext<'_>) -> Value {
    json!({
        "format": "cove-map-build-report-v1",
        "projection_output": ctx.projection_output.as_str(),
        "evidence_encoding": ctx.evidence_encoding.as_str(),
        "section_compression": ctx.section_compression.as_str(),
        "evidence": evidence_summary_json(ctx.evidence_summary),
        "compression_summary": compression_summary_json(ctx.compression_summary),
        "expanded_evidence_index": if ctx.evidence_encoding == MapEvidenceEncoding::Both {
            ctx.materialized.evidence_index.clone()
        } else {
            Value::Null
        },
        "conversion_report": ctx.materialized.conversion_report,
        "generated_artifacts": generated_artifacts(ctx.object_relative, ctx.object_bytes, ctx.covm, ctx.indexes, ctx.projections),
        "covm": ctx.covm.map(covm_artifact_json),
        "cache": {
            "format": "cove-map-build-cache-v1",
            "reuse_enabled": true,
            "status": "recorded",
        },
        "sidecar_readiness": sidecar_readiness(ctx.indexes),
        "skipped_projections": ctx.skipped.iter().map(skipped_projection_json).collect::<Vec<_>>(),
        "warnings": ctx.warnings,
        "verification": ctx.verification,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_manifest(
    map: &Path,
    sources: &[PathBuf],
    materialized: &MaterializedModel,
    mapping_id: &str,
    mapping_version: &str,
    projection_output: MapBuildProjectionOutput,
    evidence_encoding: MapEvidenceEncoding,
    section_compression: MapBuildSectionCompression,
    object_relative: &Path,
    object_bytes: usize,
    evidence_summary: &EvidenceEncodingSummary,
    compression_summary: &CoveOCompressionSummary,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
    report_relative: &Path,
    report_bytes: usize,
    readme_relative: &Path,
    readme_bytes: usize,
    manifest_relative: &Path,
    warnings: &[String],
    verification: Option<&Value>,
) -> Value {
    json!({
        "format": "cove-map-build-manifest-v1",
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "projection_output": projection_output.as_str(),
        "evidence_encoding": evidence_encoding.as_str(),
        "section_compression": section_compression.as_str(),
        "evidence": evidence_summary_json(evidence_summary),
        "compression_summary": compression_summary_json(compression_summary),
        "covm_published": covm.is_some(),
        "mapping_path": map.display().to_string(),
        "sources": materialized.conversion_report.get("sources").cloned().unwrap_or_else(|| json!([])),
        "source_paths": sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        "counts": {
            "source_count": materialized.conversion_report.get("source_count").cloned().unwrap_or(Value::Null),
            "row_count": materialized.conversion_report.get("row_count").cloned().unwrap_or(Value::Null),
            "object_count": materialized.conversion_report.get("object_count").cloned().unwrap_or(Value::Null),
            "association_count": materialized.conversion_report.get("association_count").cloned().unwrap_or(Value::Null),
            "property_value_count": materialized.conversion_report.get("property_value_count").cloned().unwrap_or(Value::Null),
            "candidate_match_count": materialized.conversion_report.get("candidate_match_count").cloned().unwrap_or(Value::Null),
            "evidence_entry_count": materialized.evidence_entries.len(),
            "assertion_count": materialized.assertions.len(),
        },
        "artifacts": {
            "object": artifact_json("object", object_relative, object_bytes),
            "covm": covm.map(covm_artifact_json),
            "indexes": indexes.iter().map(index_artifact_json).collect::<Vec<_>>(),
            "projections": projections.iter().map(projection_artifact_json).collect::<Vec<_>>(),
            "report": artifact_json("report", report_relative, report_bytes),
            "readme": artifact_json("readme", readme_relative, readme_bytes),
            "manifest": {
                "kind": "manifest",
                "path": path_string(manifest_relative),
            },
        },
        "cache": cache_manifest(map, sources, projection_output, evidence_encoding, section_compression, object_relative),
        "sidecar_readiness": sidecar_readiness(indexes),
        "warnings": warnings,
        "verification": verification,
        "recommended_commands": recommended_commands(object_relative, covm, indexes, projections),
    })
}

#[allow(clippy::too_many_arguments)]
fn delta_build_report(
    source_label: &str,
    source_digest: &str,
    object_relative: &Path,
    object_bytes: usize,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
    skipped: &[SkippedProjection],
    warnings: &[String],
    verification: Option<&Value>,
) -> Value {
    json!({
        "format": "cove-map-delta-build-report-v1",
        "source": {
            "kind": "cove-o-delta-snapshot",
            "label": source_label,
            "digest": source_digest,
        },
        "generated_artifacts": generated_artifacts(object_relative, object_bytes, covm, indexes, projections),
        "covm": covm.map(covm_artifact_json),
        "sidecar_readiness": sidecar_readiness(indexes),
        "skipped_projections": skipped.iter().map(skipped_projection_json).collect::<Vec<_>>(),
        "warnings": warnings,
        "verification": verification,
    })
}

#[allow(clippy::too_many_arguments)]
fn delta_build_manifest(
    source_label: &str,
    source_digest: &str,
    projection_output: MapBuildProjectionOutput,
    object_relative: &Path,
    object_bytes: usize,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
    report_relative: &Path,
    report_bytes: usize,
    readme_relative: &Path,
    readme_bytes: usize,
    manifest_relative: &Path,
    warnings: &[String],
    verification: Option<&Value>,
) -> Value {
    json!({
        "format": "cove-map-delta-build-manifest-v1",
        "source": {
            "kind": "cove-o-delta-snapshot",
            "label": source_label,
            "digest": source_digest,
        },
        "mapping_id": "delta_snapshot",
        "projection_output": projection_output.as_str(),
        "covm_published": covm.is_some(),
        "artifacts": {
            "object": artifact_json("object", object_relative, object_bytes),
            "covm": covm.map(covm_artifact_json),
            "indexes": indexes.iter().map(index_artifact_json).collect::<Vec<_>>(),
            "projections": projections.iter().map(projection_artifact_json).collect::<Vec<_>>(),
            "report": artifact_json("report", report_relative, report_bytes),
            "readme": artifact_json("readme", readme_relative, readme_bytes),
            "manifest": {
                "kind": "manifest",
                "path": path_string(manifest_relative),
            },
        },
        "sidecar_readiness": sidecar_readiness(indexes),
        "warnings": warnings,
        "verification": verification,
        "recommended_commands": recommended_commands(object_relative, covm, indexes, projections),
    })
}

fn evidence_summary_json(summary: &EvidenceEncodingSummary) -> Value {
    json!({
        "format": "cove-map-evidence-encoding-summary-v1",
        "encoding": summary.encoding.as_str(),
        "logical_entry_count": summary.logical_entry_count,
        "expanded_json_bytes": summary.expanded_json_bytes,
        "emitted_index_bytes": summary.emitted_index_bytes,
        "compact_binary_bytes": summary.compact_binary_bytes,
        "estimated_saved_bytes": summary.estimated_saved_bytes,
        "savings_ratio": if summary.expanded_json_bytes == 0 {
            0.0
        } else {
            summary.estimated_saved_bytes as f64 / summary.expanded_json_bytes as f64
        },
    })
}

fn compression_summary_json(summary: &CoveOCompressionSummary) -> Value {
    json!({
        "format": "cove-map-section-compression-summary-v1",
        "mode": summary.mode.as_str(),
        "threshold_bytes": summary.threshold_bytes,
        "uncompressed_section_bytes": summary.uncompressed_section_bytes,
        "emitted_section_bytes": summary.emitted_section_bytes,
        "saved_bytes": summary.saved_bytes,
        "savings_ratio": if summary.uncompressed_section_bytes == 0 {
            0.0
        } else {
            summary.saved_bytes as f64 / summary.uncompressed_section_bytes as f64
        },
        "compressed_section_count": summary.compressed_section_count,
        "sections": summary.sections.iter().map(|section| json!({
            "section_kind": section.section_kind,
            "section_name": section.section_name,
            "profile": section.profile,
            "compression": section.compression,
            "uncompressed_bytes": section.uncompressed_bytes,
            "emitted_bytes": section.emitted_bytes,
            "saved_bytes": section.saved_bytes,
        })).collect::<Vec<_>>(),
    })
}

fn generated_artifacts(
    object_relative: &Path,
    object_bytes: usize,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
) -> Vec<Value> {
    let mut artifacts = vec![artifact_json("object", object_relative, object_bytes)];
    if let Some(covm) = covm {
        artifacts.push(covm_artifact_json(covm));
    }
    artifacts.extend(indexes.iter().map(index_artifact_json));
    artifacts.extend(projections.iter().map(projection_artifact_json));
    artifacts
}

fn artifact_json(kind: &str, relative_path: &Path, byte_size: usize) -> Value {
    json!({
        "kind": kind,
        "path": path_string(relative_path),
        "byte_size": byte_size,
    })
}

fn covm_artifact_json(artifact: &CovmArtifact) -> Value {
    json!({
        "kind": "covm",
        "format": "covm",
        "path": path_string(&artifact.relative_path),
        "byte_size": artifact.byte_size,
        "digest": artifact.digest,
        "report": artifact.report,
    })
}

fn projection_artifact_json(artifact: &ProjectionArtifact) -> Value {
    json!({
        "kind": "projection",
        "projection_id": artifact.projection_id,
        "output_table": artifact.output_table,
        "path": path_string(&artifact.relative_path),
        "byte_size": artifact.byte_size,
    })
}

fn index_artifact_json(artifact: &IndexArtifact) -> Value {
    json!({
        "kind": "index",
        "format": "covi",
        "index_id": artifact.index_id,
        "target": artifact.target,
        "path": path_string(&artifact.relative_path),
        "byte_size": artifact.byte_size,
        "root_count": artifact.root_count,
    })
}

fn cache_manifest(
    map: &Path,
    sources: &[PathBuf],
    projection_output: MapBuildProjectionOutput,
    evidence_encoding: MapEvidenceEncoding,
    section_compression: MapBuildSectionCompression,
    object_relative: &Path,
) -> Value {
    json!({
        "format": "cove-map-build-cache-v1",
        "reuse_enabled": true,
        "key_material": {
            "mapping_path": map.display().to_string(),
            "source_paths": sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "projection_output": projection_output.as_str(),
            "evidence_encoding": evidence_encoding.as_str(),
            "section_compression": section_compression.as_str(),
            "object_path": path_string(object_relative),
        },
        "status": "recorded",
    })
}

fn sidecar_readiness(indexes: &[IndexArtifact]) -> Value {
    json!({
        "format": "cove-map-sidecar-readiness-v1",
        "covi": {
            "available": !indexes.is_empty(),
            "index_count": indexes.len(),
            "targets": indexes.iter().map(|index| index.target.clone()).collect::<Vec<_>>(),
            "generated_root_families": [
                "object_properties",
                "object_paths",
                "association_endpoints",
                "evidence_lookup",
                "projection_fragments",
                "projection_columns"
            ],
        },
        "authority": "materialized-readback",
        "fallback": "required-on-stale-or-unsupported-sidecar",
    })
}

fn skipped_projection_json(skipped: &SkippedProjection) -> Value {
    json!({
        "projection_id": skipped.projection_id,
        "output_table": skipped.output_table,
        "reason": skipped.reason,
    })
}

fn build_warnings(
    output: MapBuildProjectionOutput,
    projections: &[ProjectionArtifact],
) -> Vec<String> {
    if output == MapBuildProjectionOutput::CoveT && projections.is_empty() {
        vec!["no COVE-T projections were generated".into()]
    } else {
        Vec::new()
    }
}

fn verification_warning_strings(verification: &Value) -> Vec<String> {
    verification
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|warning| {
            let code = warning.get("code").and_then(Value::as_str)?;
            let message = warning
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("verification warning");
            Some(format!("{code}: {message}"))
        })
        .collect()
}

pub(crate) fn verify_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Value, String> {
    if sources.is_empty() {
        return Err("map doctor requires at least one source path".into());
    }
    let file = crate::parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    let materialized = materialize_with_source_states(&file, &inputs.rows, &inputs.states)?;
    let evidence_encoding = MapEvidenceEncoding::Compact;
    let section_compression = MapBuildSectionCompression::Zstd;
    let object_output = build_cove_o_from_materialized_with_options(
        &file,
        &materialized,
        evidence_encoding,
        section_compression,
    )?;
    let object_bytes = object_output.bytes;
    let compression_summary = object_output.compression_summary;
    let evidence_summary = evidence_encoding_summary(&materialized, evidence_encoding)?;
    let (mapping_id, _mapping_version) = mapping_identity(&file).unwrap_or_else(|_| {
        (
            map.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("mapping")
                .to_string(),
            file.mapping_version.clone(),
        )
    });
    let object_relative = PathBuf::from(format!("{}.cove", sanitize_file_stem(&mapping_id)));
    let mapping_digest = fs::read(map)
        .map(|bytes| format!("sha256:{}", crate::sha256_hex(&bytes)))
        .map_err(|err| format!("cannot read {}: {err}", map.display()))?;
    let lineage = ProjectionLineageContext {
        source_cove_o: Some(ProjectionSourceCoveO {
            path: Some(path_string(&object_relative)),
            label: path_string(&object_relative),
            digest: Some(format!("sha256:{}", crate::sha256_hex(&object_bytes))),
        }),
        mapping_artifact_digest: Some(mapping_digest),
        covm_manifest: None,
    };
    let projection_result = projection_artifacts(
        &object_bytes,
        MapBuildProjectionOutput::CoveT,
        Some(&lineage),
    )?;
    let index_result = index_artifacts(&object_bytes)?;
    let warnings = build_warnings(
        MapBuildProjectionOutput::CoveT,
        &projection_result.artifacts,
    );
    let report = build_report(BuildReportContext {
        materialized: &materialized,
        projection_output: MapBuildProjectionOutput::CoveT,
        evidence_encoding,
        section_compression,
        object_relative: &object_relative,
        object_bytes: object_bytes.len(),
        evidence_summary: &evidence_summary,
        compression_summary: &compression_summary,
        covm: None,
        indexes: &index_result.artifacts,
        projections: &projection_result.artifacts,
        skipped: &projection_result.skipped,
        warnings: &warnings,
        verification: None,
    });
    Ok(verify_bundle(&VerifiableBundle {
        object_relative_path: object_relative,
        object_bytes,
        report,
        indexes: index_result.verifiable,
        projections: projection_result.verifiable,
    }))
}

fn recommended_commands(
    object_relative: &Path,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
) -> Vec<Value> {
    let object = path_string(object_relative);
    let mut commands = vec![
        json!({
            "description": "validate the mapped COVE-O artifact",
            "command": format!("cove validate --semantic {object}"),
        }),
        json!({
            "description": "inspect query and performance surfaces",
            "command": format!("cove inspect --queries --performance {object}"),
        }),
        json!({
            "description": "preview projection readback as JSON",
            "command": format!("cove map project-cove-o --format json {object}"),
        }),
    ];
    if let Some(projection) = projections.first() {
        commands.push(json!({
            "description": "query the first generated COVE-T projection",
            "command": format!(
                "cove query {} 'table({}).take(10)'",
                path_string(&projection.relative_path),
                projection.output_table.as_deref().unwrap_or(&projection.projection_id)
            ),
        }));
    }
    if let Some(index) = indexes.first() {
        commands.push(json!({
            "description": "inspect the generated COVE-I acceleration index",
            "command": format!(
                "cove sidecar inspect index {}",
                path_string(&index.relative_path)
            ),
        }));
    }
    if let Some(covm) = covm {
        commands.push(json!({
            "description": "query through the generated COVM publication manifest",
            "command": format!("cove query {} 'tables()'", path_string(&covm.relative_path)),
        }));
    }
    commands
}

fn readme_bytes(
    mapping_id: &str,
    object_relative: &Path,
    covm: Option<&CovmArtifact>,
    indexes: &[IndexArtifact],
    projections: &[ProjectionArtifact],
) -> Vec<u8> {
    let mut text = String::new();
    text.push_str("# COVE-MAP Build Bundle\n\n");
    text.push_str(&format!("Mapping: `{mapping_id}`\n\n"));
    text.push_str("Generated artifacts:\n\n");
    text.push_str(&format!(
        "- `{}` mapped COVE-O object\n",
        path_string(object_relative)
    ));
    text.push_str("- `map-build-report.json` conversion and generation report\n");
    text.push_str("- `map-build-manifest.json` stable bundle manifest\n");
    if let Some(covm) = covm {
        text.push_str(&format!(
            "- `{}` normative COVM publication manifest\n",
            path_string(&covm.relative_path)
        ));
    }
    for index in indexes {
        text.push_str(&format!(
            "- `{}` COVE-I index `{}` ({})\n",
            path_string(&index.relative_path),
            index.index_id,
            index.target
        ));
    }
    for projection in projections {
        text.push_str(&format!(
            "- `{}` projection `{}`\n",
            path_string(&projection.relative_path),
            projection.projection_id
        ));
    }
    text.push_str("\nSuggested commands:\n\n");
    for command in recommended_commands(object_relative, covm, indexes, projections) {
        if let Some(raw) = command.get("command").and_then(Value::as_str) {
            text.push_str("```sh\n");
            text.push_str(raw);
            text.push_str("\n```\n\n");
        }
    }
    text.into_bytes()
}

pub fn publish_covm_from_bundle(
    bundle_dir: &Path,
    output: &Path,
    force: bool,
) -> Result<Value, String> {
    if output.exists() && !force {
        return Err(format!(
            "{} already exists; pass --force to replace it",
            output.display()
        ));
    }
    let manifest_path = bundle_dir.join("map-build-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|err| format!("cannot read {}: {err}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| format!("cannot parse {}: {err}", manifest_path.display()))?;
    let mut artifacts = Vec::<(String, Vec<u8>)>::new();
    if let Some(path) = manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
    {
        artifacts.push(read_bundle_cove_artifact(bundle_dir, path)?);
    }
    for projection in manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = projection.get("path").and_then(Value::as_str) {
            artifacts.push(read_bundle_cove_artifact(bundle_dir, path)?);
        }
    }
    if artifacts.is_empty() {
        return Err("bundle manifest does not reference any COVE artifacts".into());
    }
    let inputs = artifacts
        .iter()
        .map(|(uri, bytes)| CovmInputArtifact {
            uri: uri.clone(),
            bytes,
        })
        .collect::<Vec<_>>();
    let (bytes, report) = build_covm_artifact_from_bytes(output.display().to_string(), &inputs)
        .map_err(|err| format!("cannot build COVM publication manifest: {err}"))?;
    durable::durable_replace(output, &bytes)
        .map_err(|err| format!("cannot durably publish {}: {err}", output.display()))?;
    Ok(json!({
        "format": "cove-map-publish-report-v1",
        "bundle_dir": bundle_dir.display().to_string(),
        "output": output.display().to_string(),
        "byte_size": bytes.len(),
        "digest": format!("sha256:{}", crate::sha256_hex(&bytes)),
        "covm_report": report.to_json_value(),
    }))
}

fn read_bundle_cove_artifact(
    bundle_dir: &Path,
    relative: &str,
) -> Result<(String, Vec<u8>), String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "bundle artifact path '{relative}' is not a safe relative path"
        ));
    }
    let path = bundle_dir.join(relative_path);
    let bytes = fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    Ok((relative.to_string(), bytes))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| format!("cannot serialize map build JSON: {err}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn object_file_name(
    options: &MapBuildOptions,
    map: &Path,
    mapping_id: &str,
) -> Result<String, String> {
    if let Some(name) = &options.object_name {
        validate_object_name(name)?;
        return Ok(name.clone());
    }
    let stem = sanitize_file_stem(mapping_id);
    if stem == "mapping" {
        let fallback = map
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(sanitize_file_stem)
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "mapping".into());
        return Ok(format!("{fallback}.cove"));
    }
    Ok(format!("{stem}.cove"))
}

fn validate_object_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err("--object-name must be a file name, not a path".into());
    }
    if !name.ends_with(".cove") {
        return Err("--object-name must end with .cove".into());
    }
    Ok(())
}

fn sanitize_file_stem(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_separator = false;
    for byte in value.bytes() {
        let ch = if byte.is_ascii_alphanumeric() {
            byte as char
        } else {
            '_'
        };
        if ch == '_' {
            if !last_was_separator && !out.is_empty() {
                out.push(ch);
            }
            last_was_separator = true;
        } else {
            out.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "mapping".into()
    } else {
        out
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_delta_object_ranges_cover_touched_and_tombstone_rows() {
        let segment = TemporalSegmentData {
            header: TemporalSegmentHeaderV1 {
                segment_id: 0,
                object_type_id: 7,
                time_range_start_us: 10,
                time_range_end_us: 12,
                csn_min: 1,
                csn_max: 2,
                row_count: 2,
                morsel_count: 1,
                morsel_row_count: 2,
                column_count: 0,
                row_directory_offset: 0,
                column_directory_offset: 0,
                page_index_offset: 0,
                data_offset: 0,
                flags: 0,
                checksum: 0,
            },
            rows: vec![
                TemporalRowEntryV1 {
                    timestamp_us: 10,
                    csn: 1,
                    branch_key: 3,
                    goid: [0x10; 16],
                    record_id: [0x20; 16],
                    record_kind: RecordKind::Baseline,
                    prev_ref: None,
                },
                TemporalRowEntryV1 {
                    timestamp_us: 12,
                    csn: 2,
                    branch_key: 3,
                    goid: [0x30; 16],
                    record_id: [0x40; 16],
                    record_kind: RecordKind::Tombstone,
                    prev_ref: None,
                },
            ],
            property_columns: Vec::new(),
        };

        let touched = semantic_delta_object_ranges(std::slice::from_ref(&segment), false).unwrap();
        assert_eq!(touched.len(), 1);
        assert_eq!(touched[0].object_type_id, 7);
        assert_eq!(touched[0].branch_identity_ref, 3);
        assert_eq!(touched[0].touched_count, 2);
        assert_eq!(touched[0].min_goid, [0x10; 16]);
        assert_eq!(touched[0].max_goid, [0x30; 16]);
        assert_eq!(touched[0].property_bitmap_ref, DELTA_REF_NONE);
        assert_eq!(touched[0].object_set_ref, DELTA_REF_NONE);

        let tombstones = semantic_delta_object_ranges(&[segment], true).unwrap();
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].touched_count, 1);
        assert_eq!(tombstones[0].min_goid, [0x30; 16]);
        assert_eq!(tombstones[0].max_goid, [0x30; 16]);
    }
}
