use std::{
    fs,
    path::{Path, PathBuf},
};

use cove_core::{
    compression,
    constants::{DigestAlgorithm, SectionKind},
    digest::compute_digest,
    postscript::CovePostscriptV1,
    profile::cove_o::{read_object_surface_from_bytes_with_options, CoveObjectReadOptions},
    reader::{validate_bytes_with_options, ValidationOptions},
    table::TableCatalog,
};
use cove_index::{
    execution::{CoviValidationContextV2, ValidatedCoviArtifactV2},
    CoviArtifactV2, CoviIndexedTargetKindV2,
};
use serde_json::{json, Value};

use crate::{
    project::{project_cove_o_bytes_output, ProjectionFormat},
    sha256_hex,
};

#[derive(Debug, Clone)]
pub(crate) struct VerifiableProjection {
    pub(crate) projection_id: String,
    pub(crate) output_table: Option<String>,
    pub(crate) relative_path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiableIndex {
    pub(crate) index_id: String,
    pub(crate) target: String,
    pub(crate) relative_path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiableBundle {
    pub(crate) object_relative_path: PathBuf,
    pub(crate) object_bytes: Vec<u8>,
    pub(crate) report: Value,
    pub(crate) indexes: Vec<VerifiableIndex>,
    pub(crate) projections: Vec<VerifiableProjection>,
}

pub(crate) fn verify_bundle(bundle: &VerifiableBundle) -> Value {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut checks = Vec::new();

    let object_validation = validate_cove_o(&bundle.object_bytes);
    checks.push(object_validation.check);
    if let Some(error) = object_validation.error {
        errors.push(error);
    }

    let conversion_report = bundle
        .report
        .get("conversion_report")
        .cloned()
        .unwrap_or_else(|| json!({}));
    compare_conversion_counts(&conversion_report, &object_validation.counts, &mut warnings);

    if let Some(skipped) = bundle
        .report
        .get("skipped_projections")
        .and_then(Value::as_array)
    {
        for projection in skipped {
            warnings.push(warning(
                "skipped_projection",
                "projection was declared but not materialized",
                projection.clone(),
            ));
        }
    }

    for projection in &bundle.projections {
        let projection_validation = validate_projection(&bundle.object_bytes, projection);
        checks.push(projection_validation.check);
        errors.extend(projection_validation.errors);
        warnings.extend(projection_validation.warnings);
    }

    for index in &bundle.indexes {
        let index_validation = validate_index(&bundle.object_bytes, index);
        checks.push(index_validation.check);
        errors.extend(index_validation.errors);
        warnings.extend(index_validation.warnings);
    }

    let status = if errors.is_empty() { "ok" } else { "error" };
    json!({
        "format": "cove-map-doctor-report-v1",
        "status": status,
        "object": {
            "path": path_string(&bundle.object_relative_path),
            "sha256": format!("sha256:{}", sha256_hex(&bundle.object_bytes)),
        },
        "checks": checks,
        "warnings": warnings,
        "errors": errors,
    })
}

pub(crate) fn verify_bundle_dir(bundle_dir: &Path) -> Result<Value, String> {
    let manifest_path = bundle_dir.join("map-build-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|err| format!("cannot read {}: {err}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| format!("{} is not valid JSON: {err}", manifest_path.display()))?;
    let report_relative = manifest
        .pointer("/artifacts/report/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "bundle manifest is missing artifacts.report.path".to_string())?;
    let report_path = bundle_dir.join(report_relative);
    let report_bytes = fs::read(&report_path)
        .map_err(|err| format!("cannot read {}: {err}", report_path.display()))?;
    let report: Value = serde_json::from_slice(&report_bytes)
        .map_err(|err| format!("{} is not valid JSON: {err}", report_path.display()))?;
    let object_relative = manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "bundle manifest is missing artifacts.object.path".to_string())?;
    let object_path = bundle_dir.join(object_relative);
    let object_bytes = fs::read(&object_path)
        .map_err(|err| format!("cannot read {}: {err}", object_path.display()))?;

    let mut projections = Vec::new();
    if let Some(values) = manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
    {
        for value in values {
            let relative = value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "projection artifact is missing path".to_string())?;
            let projection_id = value
                .get("projection_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "projection artifact is missing projection_id".to_string())?;
            let path = bundle_dir.join(relative);
            let bytes =
                fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
            projections.push(VerifiableProjection {
                projection_id: projection_id.to_string(),
                output_table: value
                    .get("output_table")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                relative_path: PathBuf::from(relative),
                bytes,
            });
        }
    }

    let mut indexes = Vec::new();
    if let Some(values) = manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
    {
        for value in values {
            let relative = value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "index artifact is missing path".to_string())?;
            let index_id = value
                .get("index_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "index artifact is missing index_id".to_string())?;
            let path = bundle_dir.join(relative);
            let bytes =
                fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
            indexes.push(VerifiableIndex {
                index_id: index_id.to_string(),
                target: value
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                relative_path: PathBuf::from(relative),
                bytes,
            });
        }
    }

    Ok(verify_bundle(&VerifiableBundle {
        object_relative_path: PathBuf::from(object_relative),
        object_bytes,
        report,
        indexes,
        projections,
    }))
}

pub(crate) fn report_has_failures(report: &Value, strict: bool) -> bool {
    let has_errors = report
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty());
    let has_warnings = report
        .get("warnings")
        .and_then(Value::as_array)
        .is_some_and(|warnings| !warnings.is_empty());
    has_errors || (strict && has_warnings)
}

struct ObjectValidation {
    check: Value,
    error: Option<Value>,
    counts: ObjectCounts,
}

#[derive(Debug, Clone, Default)]
struct ObjectCounts {
    record_count: u64,
    object_count: u64,
    association_count: u64,
    evidence_entry_count: u64,
}

fn validate_cove_o(bytes: &[u8]) -> ObjectValidation {
    let mut error = None;
    let mut counts = ObjectCounts::default();
    let validation = validate_bytes_with_options(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    );
    let validation_ok = match validation {
        Ok(_) => true,
        Err(err) => {
            error = Some(json!({
                "code": "invalid_cove_o",
                "message": err.to_string(),
            }));
            false
        }
    };
    let readback = read_object_surface_from_bytes_with_options(
        bytes,
        &CoveObjectReadOptions {
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: true,
            include_records: true,
            include_evidence_index: true,
            ..Default::default()
        },
    );
    let readback_ok = match readback {
        Ok(surface) => {
            counts.record_count = surface.records.len() as u64;
            counts.association_count = surface
                .records
                .iter()
                .filter(|record| record.association.is_some())
                .count() as u64;
            counts.object_count = counts.record_count.saturating_sub(counts.association_count);
            counts.evidence_entry_count = surface
                .evidence_index
                .as_ref()
                .map(|index| index.entries.len() as u64)
                .unwrap_or(0);
            true
        }
        Err(err) => {
            error.get_or_insert_with(|| {
                json!({
                    "code": "object_readback_failed",
                    "message": err.to_string(),
                })
            });
            false
        }
    };
    ObjectValidation {
        check: json!({
            "name": "cove_o_validation",
            "ok": validation_ok && readback_ok,
            "counts": {
                "record_count": counts.record_count,
                "object_count": counts.object_count,
                "association_count": counts.association_count,
                "evidence_entry_count": counts.evidence_entry_count,
            },
        }),
        error,
        counts,
    }
}

struct ProjectionValidation {
    check: Value,
    warnings: Vec<Value>,
    errors: Vec<Value>,
}

struct IndexValidation {
    check: Value,
    warnings: Vec<Value>,
    errors: Vec<Value>,
}

fn validate_projection(
    object_bytes: &[u8],
    projection: &VerifiableProjection,
) -> ProjectionValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut catalog_rows = None;
    let validation = validate_bytes_with_options(
        &projection.bytes,
        ValidationOptions {
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    );
    match validation {
        Ok(report) => match projection_table_row_count(&projection.bytes, &report.validated.footer)
        {
            Ok(row_count) => catalog_rows = Some(row_count),
            Err(err) => errors.push(json!({
                "code": "projection_readback_mismatch",
                "projection_id": projection.projection_id,
                "message": err,
            })),
        },
        Err(err) => errors.push(json!({
            "code": "invalid_cove_t_projection",
            "projection_id": projection.projection_id,
            "message": err.to_string(),
        })),
    }

    let projected_json = project_cove_o_bytes_output(
        object_bytes,
        None,
        ProjectionFormat::Json,
        Some(&projection.projection_id),
        "<doctor>",
    );
    let json_rows = match projected_json {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| value.get("rows").and_then(Value::as_array).map(Vec::len)),
        Err(err) => {
            errors.push(json!({
                "code": "projection_readback_mismatch",
                "projection_id": projection.projection_id,
                "message": err,
            }));
            None
        }
    };
    if let (Some(catalog_rows), Some(json_rows)) = (catalog_rows, json_rows) {
        if catalog_rows != json_rows as u64 {
            warnings.push(warning(
                "projection_readback_mismatch",
                "COVE-T projection row count differs from COVE-O projection readback",
                json!({
                    "projection_id": projection.projection_id,
                    "catalog_rows": catalog_rows,
                    "readback_rows": json_rows,
                }),
            ));
        }
    }

    ProjectionValidation {
        check: json!({
            "name": "cove_t_projection",
            "projection_id": projection.projection_id,
            "output_table": projection.output_table,
            "path": path_string(&projection.relative_path),
            "ok": errors.is_empty(),
            "catalog_rows": catalog_rows,
            "readback_rows": json_rows,
        }),
        warnings,
        errors,
    }
}

fn validate_index(object_bytes: &[u8], index: &VerifiableIndex) -> IndexValidation {
    let mut errors = Vec::new();
    let warnings = Vec::new();
    let mut root_count = 0usize;
    let mut object_property_root_count = 0usize;
    let mut auxiliary_root_count = 0usize;
    let parsed = CoviArtifactV2::parse(&index.bytes);
    match parsed {
        Ok(artifact) => {
            root_count = artifact.index_roots.len();
            object_property_root_count = artifact
                .index_roots
                .iter()
                .filter(|root| root.indexed_target_kind == CoviIndexedTargetKindV2::ObjectProperty)
                .count();
            auxiliary_root_count = artifact
                .index_roots
                .iter()
                .filter(|root| {
                    matches!(
                        root.indexed_target_kind,
                        CoviIndexedTargetKindV2::ObjectPath
                            | CoviIndexedTargetKindV2::AssociationEndpoint
                            | CoviIndexedTargetKindV2::ProjectionColumn
                    )
                })
                .count();
            if index.target == "cove-o-object-properties"
                && object_property_root_count + auxiliary_root_count != root_count
            {
                errors.push(json!({
                    "code": "invalid_covi_index",
                    "index_id": index.index_id,
                    "message": "object-property index contains unsupported root kinds",
                }));
            }
        }
        Err(err) => errors.push(json!({
            "code": "invalid_covi_index",
            "index_id": index.index_id,
            "message": err.to_string(),
        })),
    }

    match covi_validation_context_for_object(object_bytes) {
        Ok(context) => {
            if let Err(err) = ValidatedCoviArtifactV2::parse_and_validate(&index.bytes, context) {
                errors.push(json!({
                    "code": "invalid_covi_index_reference",
                    "index_id": index.index_id,
                    "message": err.to_string(),
                }));
            }
        }
        Err(err) => errors.push(json!({
            "code": "invalid_covi_index_reference",
            "index_id": index.index_id,
            "message": err,
        })),
    }

    IndexValidation {
        check: json!({
            "name": "covi_index",
            "index_id": index.index_id,
            "target": index.target,
            "path": path_string(&index.relative_path),
            "ok": errors.is_empty(),
            "root_count": root_count,
            "object_property_root_count": object_property_root_count,
            "auxiliary_root_count": auxiliary_root_count,
        }),
        warnings,
        errors,
    }
}

fn covi_validation_context_for_object(
    object_bytes: &[u8],
) -> Result<CoviValidationContextV2, String> {
    let validation = validate_bytes_with_options(
        object_bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .map_err(|err| err.to_string())?;
    let postscript =
        CovePostscriptV1::parse_from_tail(object_bytes).map_err(|err| err.to_string())?;
    let digest =
        compute_digest(DigestAlgorithm::Sha256, object_bytes).map_err(|err| err.to_string())?;
    Ok(CoviValidationContextV2::for_file(
        validation.validated.header.file_id,
        object_bytes.len() as u64,
        postscript.footer.crc32c,
    )
    .with_file_digest(DigestAlgorithm::Sha256, digest))
}

fn projection_table_row_count(
    bytes: &[u8],
    footer: &cove_core::footer::CoveFooter,
) -> Result<u64, String> {
    let entry = footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::TableCatalog as u16)
        .ok_or_else(|| "projection has no TABLE_CATALOG section".to_string())?;
    let payload = compression::section_payload(bytes, entry)
        .map_err(|err| format!("cannot read TABLE_CATALOG: {err}"))?;
    let catalog = TableCatalog::parse(&payload)
        .map_err(|err| format!("cannot parse TABLE_CATALOG: {err}"))?;
    Ok(catalog.tables.iter().map(|table| table.row_count).sum())
}

fn compare_conversion_counts(report: &Value, counts: &ObjectCounts, warnings: &mut Vec<Value>) {
    let row_count = report.get("row_count").and_then(Value::as_u64).unwrap_or(0);
    let object_count = report
        .get("object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let association_count = report
        .get("association_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_count = counts.evidence_entry_count;
    if object_count != counts.object_count || association_count != counts.association_count {
        warnings.push(warning(
            "count_mismatch",
            "conversion report counts differ from COVE-O readback counts",
            json!({
                "report_object_count": object_count,
                "readback_object_count": counts.object_count,
                "report_association_count": association_count,
                "readback_association_count": counts.association_count,
            }),
        ));
    }
    if row_count > 0 && evidence_count == 0 {
        warnings.push(warning(
            "missing_evidence",
            "source rows were converted but no evidence entries were read back",
            json!({"row_count": row_count}),
        ));
    }
    if row_count > evidence_count && evidence_count > 0 {
        warnings.push(warning(
            "unmapped_rows",
            "fewer evidence entries than source rows were read back",
            json!({"row_count": row_count, "evidence_entry_count": evidence_count}),
        ));
    }
    if row_count > object_count.saturating_add(association_count) && object_count > 0 {
        warnings.push(warning(
            "duplicate_identity",
            "multiple source rows mapped onto fewer materialized objects or associations",
            json!({
                "row_count": row_count,
                "object_count": object_count,
                "association_count": association_count,
            }),
        ));
    }
}

fn warning(code: &str, message: &str, details: Value) -> Value {
    json!({
        "code": code,
        "message": message,
        "details": details,
    })
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
