use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use cove_core::durable;
use serde_json::{json, Value};

use crate::{
    emit::build_cove_o_from_materialized,
    input::{read_source_inputs, validate_source_inputs},
    materialize_with_source_states,
    project::{
        project_cove_o_bytes_output, projection_catalog_from_cove_o_bytes_internal,
        ProjectionFormat,
    },
    sections::mapping_identity,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapBuildOptions {
    pub out_dir: PathBuf,
    pub force: bool,
    pub object_name: Option<String>,
    pub projection_output: MapBuildProjectionOutput,
}

impl MapBuildOptions {
    pub fn new(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            out_dir: out_dir.into(),
            force: false,
            object_name: None,
            projection_output: MapBuildProjectionOutput::CoveT,
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
struct SkippedProjection {
    projection_id: String,
    output_table: Option<String>,
    reason: String,
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
    let object_bytes = build_cove_o_from_materialized(&file, &materialized)?;
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
    let (projection_artifacts, skipped_projections, projection_files) =
        projection_artifacts(&object_bytes, options.projection_output)?;
    let warnings = build_warnings(options.projection_output, &projection_artifacts);
    let mut artifacts = Vec::new();
    artifacts.push(PendingArtifact {
        relative_path: object_relative.clone(),
        bytes: object_bytes.clone(),
    });
    artifacts.extend(projection_files);

    let report_relative = PathBuf::from("map-build-report.json");
    let readme_relative = PathBuf::from("README.md");
    let manifest_relative = PathBuf::from("map-build-manifest.json");
    let report = build_report(
        &materialized,
        options.projection_output,
        &object_relative,
        object_bytes.len(),
        &projection_artifacts,
        &skipped_projections,
        &warnings,
    );
    let report_bytes = json_bytes(&report)?;
    let readme_bytes = readme_bytes(&mapping_id, &object_relative, &projection_artifacts);
    let manifest = build_manifest(
        map,
        sources,
        &materialized,
        &mapping_id,
        &mapping_version,
        options.projection_output,
        &object_relative,
        object_bytes.len(),
        &projection_artifacts,
        &report_relative,
        report_bytes.len(),
        &readme_relative,
        readme_bytes.len(),
        &manifest_relative,
        &warnings,
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
) -> Result<
    (
        Vec<ProjectionArtifact>,
        Vec<SkippedProjection>,
        Vec<PendingArtifact>,
    ),
    String,
> {
    let catalog = projection_catalog_from_cove_o_bytes_internal(object_bytes, None, "<map-build>")?;
    let mut used_names = BTreeMap::<String, String>::new();
    let mut projection_artifacts = Vec::new();
    let mut skipped = Vec::new();
    let mut files = Vec::new();
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
        let bytes = project_cove_o_bytes_output(
            object_bytes,
            None,
            ProjectionFormat::CoveT,
            Some(&projection.projection_id),
            "<map-build>",
        )?;
        let relative_path = PathBuf::from("projections").join(file_name);
        projection_artifacts.push(ProjectionArtifact {
            projection_id: projection.projection_id.clone(),
            output_table: projection.output_table.clone(),
            relative_path: relative_path.clone(),
            byte_size: bytes.len(),
        });
        files.push(PendingArtifact {
            relative_path,
            bytes,
        });
    }
    Ok((projection_artifacts, skipped, files))
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

fn build_report(
    materialized: &MaterializedModel,
    projection_output: MapBuildProjectionOutput,
    object_relative: &Path,
    object_bytes: usize,
    projections: &[ProjectionArtifact],
    skipped: &[SkippedProjection],
    warnings: &[String],
) -> Value {
    json!({
        "format": "cove-map-build-report-v1",
        "projection_output": projection_output.as_str(),
        "conversion_report": materialized.conversion_report,
        "generated_artifacts": generated_artifacts(object_relative, object_bytes, projections),
        "skipped_projections": skipped.iter().map(skipped_projection_json).collect::<Vec<_>>(),
        "warnings": warnings,
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
    object_relative: &Path,
    object_bytes: usize,
    projections: &[ProjectionArtifact],
    report_relative: &Path,
    report_bytes: usize,
    readme_relative: &Path,
    readme_bytes: usize,
    manifest_relative: &Path,
    warnings: &[String],
) -> Value {
    json!({
        "format": "cove-map-build-manifest-v1",
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "projection_output": projection_output.as_str(),
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
            "projections": projections.iter().map(projection_artifact_json).collect::<Vec<_>>(),
            "report": artifact_json("report", report_relative, report_bytes),
            "readme": artifact_json("readme", readme_relative, readme_bytes),
            "manifest": {
                "kind": "manifest",
                "path": path_string(manifest_relative),
            },
        },
        "warnings": warnings,
        "recommended_commands": recommended_commands(object_relative, projections),
    })
}

fn generated_artifacts(
    object_relative: &Path,
    object_bytes: usize,
    projections: &[ProjectionArtifact],
) -> Vec<Value> {
    let mut artifacts = vec![artifact_json("object", object_relative, object_bytes)];
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

fn projection_artifact_json(artifact: &ProjectionArtifact) -> Value {
    json!({
        "kind": "projection",
        "projection_id": artifact.projection_id,
        "output_table": artifact.output_table,
        "path": path_string(&artifact.relative_path),
        "byte_size": artifact.byte_size,
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

fn recommended_commands(object_relative: &Path, projections: &[ProjectionArtifact]) -> Vec<Value> {
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
    commands
}

fn readme_bytes(
    mapping_id: &str,
    object_relative: &Path,
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
    for projection in projections {
        text.push_str(&format!(
            "- `{}` projection `{}`\n",
            path_string(&projection.relative_path),
            projection.projection_id
        ));
    }
    text.push_str("\nSuggested commands:\n\n");
    for command in recommended_commands(object_relative, projections) {
        if let Some(raw) = command.get("command").and_then(Value::as_str) {
            text.push_str("```sh\n");
            text.push_str(raw);
            text.push_str("\n```\n\n");
        }
    }
    text.into_bytes()
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
