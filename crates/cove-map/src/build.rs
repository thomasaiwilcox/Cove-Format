use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use cove_core::{
    durable,
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
    input::{read_source_inputs, validate_source_inputs},
    materialize_with_source_states,
    project::{
        project_cove_o_bytes_output_with_lineage, projection_catalog_from_cove_o_bytes_internal,
        projection_sidecar_columns_from_cove_o_bytes, ProjectionFormat, ProjectionLineageContext,
        ProjectionSourceCoveO,
    },
    sections::mapping_identity,
    verify::{verify_bundle, VerifiableBundle, VerifiableIndex, VerifiableProjection},
    MaterializedModel,
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

#[derive(Debug, Clone, PartialEq)]
pub struct MapBuildResult {
    pub manifest: Value,
    pub report: Value,
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
        projection_sidecar_columns_from_cove_o_bytes(object_bytes, "<map-build>")?
            .into_iter()
            .map(|column| CoviProjectionColumnInput {
                table_id: column.table_id,
                column_id: column.column_id,
                logical_type: column.logical_type,
                physical_kind: column.physical_kind,
                values: column.values,
            })
            .collect::<Vec<_>>();
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
