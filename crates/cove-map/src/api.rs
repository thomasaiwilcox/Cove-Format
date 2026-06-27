use std::{
    fs,
    path::{Path, PathBuf},
};

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use cove_core::artifact::covemap::CovemapFile;
use cove_core::profile::cove_map::MapProjectionCatalog;
use serde_json::{json, Value};

use crate::{
    candidate_match_id, candidate_matches,
    emit::build_cove_o_with_source_states,
    hex_encode,
    input::{read_source_inputs, validate_source_inputs, SourceRow},
    materialize_with_source_states, plan_identities,
    project::{
        project_cove_o_bytes_output, project_cove_o_bytes_record_batch,
        project_cove_o_bytes_record_batches, project_cove_o_bytes_record_batches_with_catalog,
        project_cove_o_path, project_cove_o_path_output, project_rows_with_source_states,
        project_rows_with_source_states_output, projection_catalog_from_cove_o_bytes_internal,
        projection_catalog_from_cove_o_path, projection_read_requirements,
        projection_schema_from_descriptor, ProjectionBatchOptions, ProjectionFilter,
        ProjectionFilterLiteral, ProjectionFilterOp, ProjectionFormat, ProjectionReadRequirements,
    },
    section_kind, MaterializedModel,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionDescriptor {
    pub projection_id: String,
    pub output_table: Option<String>,
    pub output_modes: Vec<String>,
    pub columns: Vec<ProjectionColumnDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionColumnDescriptor {
    pub name: String,
    pub logical_type: String,
    pub nested_shape: Option<String>,
    pub lineage: Option<ProjectionColumnLineageDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionColumnLineageDescriptor {
    pub source: String,
    pub object_type_id: u32,
    pub object_type_name: String,
    pub property_id: u32,
    pub property_name: String,
    pub projection_table_id: u32,
    pub projection_column_id: u32,
    pub expression: String,
    pub transform: String,
    pub filter_pushdown: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionCoviFilterPlan {
    pub lookups: Vec<ProjectionCoviFilterLookup>,
    pub unsupported_filters: Vec<String>,
    pub diagnostics: Vec<ProjectionCoviFilterDiagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionCoviFilterLookup {
    pub column: String,
    pub projection_table_id: u32,
    pub projection_column_id: u32,
    pub logical_type: String,
    pub filter: ProjectionFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionCoviFilterDiagnostic {
    pub column: String,
    pub op: String,
    pub lineage_status: String,
    pub logical_type: Option<String>,
    pub eligible: bool,
    pub projection_table_id: Option<u32>,
    pub projection_column_id: Option<u32>,
    pub reason: String,
}

pub fn conversion_report_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Value, String> {
    Ok(materialize_from_paths(map, sources)?.conversion_report)
}

pub fn verify_replay_report_from_paths(map: &Path, report: &Value) -> Result<Value, String> {
    let file = parse_map(map)?;
    crate::verify_replay_report(&file, report)
}

pub fn conversion_summary_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Value, String> {
    let materialized = materialize_from_paths(map, sources)?;
    Ok(json!({
        "report": materialized.conversion_report,
        "materialized_row_count": materialized.rows.len(),
        "evidence_entry_count": materialized.evidence_entries.len(),
        "assertion_count": materialized.assertions.len(),
        "identity_equivalence_index": materialized.identity_equivalence_index,
        "evidence_entries": materialized.evidence_entries,
    }))
}

pub fn candidate_matches_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Value, String> {
    let file = parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    candidate_matches(&file, &inputs.rows)
}

pub fn cove_o_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Vec<u8>, String> {
    let file = parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    build_cove_o_with_source_states(&file, &inputs.rows, &inputs.states)
}

pub fn projected_rows_from_paths(map: &Path, sources: &[PathBuf]) -> Result<Value, String> {
    let file = parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    project_rows_with_source_states(&file, &inputs.rows, &inputs.states)
}

pub fn projected_output_from_paths(
    map: &Path,
    sources: &[PathBuf],
    format: ProjectionFormat,
    projection_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    let file = parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    project_rows_with_source_states_output(
        &file,
        &inputs.rows,
        &inputs.states,
        format,
        projection_id,
    )
}

pub fn projected_rows_from_cove_o_path(
    object: &Path,
    mapping: Option<&Path>,
) -> Result<Value, String> {
    project_cove_o_path(object, mapping)
}

pub fn projected_output_from_cove_o_path(
    object: &Path,
    mapping: Option<&Path>,
    format: ProjectionFormat,
    projection_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    project_cove_o_path_output(object, mapping, format, projection_id)
}

pub fn projected_output_from_cove_o_bytes(
    object: &[u8],
    mapping: Option<&Path>,
    format: ProjectionFormat,
    projection_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    project_cove_o_bytes_output(object, mapping, format, projection_id, "<bytes>")
}

pub fn projected_record_batch_from_cove_o_bytes(
    object: &[u8],
    mapping: Option<&Path>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<RecordBatch, String> {
    project_cove_o_bytes_record_batch(object, mapping, projection_id, options, "<bytes>")
}

pub fn projected_record_batches_from_cove_o_bytes(
    object: &[u8],
    mapping: Option<&Path>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    project_cove_o_bytes_record_batches(object, mapping, projection_id, options, "<bytes>")
}

pub fn projected_record_batches_from_cove_o_bytes_with_catalog(
    object: &[u8],
    mapping: Option<&Path>,
    catalog: &MapProjectionCatalog,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    project_cove_o_bytes_record_batches_with_catalog(
        object,
        mapping,
        catalog,
        projection_id,
        options,
        "<bytes>",
    )
}

pub fn projection_catalog_from_cove_o_bytes(
    object: &[u8],
    mapping: Option<&Path>,
) -> Result<MapProjectionCatalog, String> {
    projection_catalog_from_cove_o_bytes_internal(object, mapping, "<bytes>")
}

pub fn projection_descriptors_from_cove_o_path(
    object: &Path,
    mapping: Option<&Path>,
) -> Result<Vec<ProjectionDescriptor>, String> {
    let catalog = projection_catalog_from_cove_o_path(object, mapping)?;
    Ok(catalog
        .projections
        .into_iter()
        .map(|projection| ProjectionDescriptor {
            columns: projection
                .columns
                .into_iter()
                .map(|column| ProjectionColumnDescriptor {
                    name: column.name,
                    logical_type: column.logical_type.unwrap_or_else(|| "utf8".to_string()),
                    nested_shape: column.nested_shape,
                    lineage: column
                        .lineage
                        .map(|lineage| ProjectionColumnLineageDescriptor {
                            source: lineage.source,
                            object_type_id: lineage.object_type_id,
                            object_type_name: lineage.object_type_name,
                            property_id: lineage.property_id,
                            property_name: lineage.property_name,
                            projection_table_id: lineage.projection_table_id,
                            projection_column_id: lineage.projection_column_id,
                            expression: lineage.expression,
                            transform: lineage.transform,
                            filter_pushdown: lineage.filter_pushdown,
                        }),
                })
                .collect(),
            projection_id: projection.projection_id,
            output_table: projection.output_table,
            output_modes: projection.output_modes,
        })
        .collect())
}

pub fn projection_arrow_schema(descriptor: &ProjectionDescriptor) -> Result<SchemaRef, String> {
    projection_schema_from_descriptor(descriptor)
}

pub fn projection_covi_filter_plan(
    descriptor: &ProjectionDescriptor,
    filters: &[ProjectionFilter],
) -> ProjectionCoviFilterPlan {
    let columns = descriptor
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut lookups = Vec::new();
    let mut unsupported_filters = Vec::new();
    let mut diagnostics = Vec::new();
    for filter in filters {
        let column_name = projection_filter_column(filter);
        let Some(column) = columns.get(column_name) else {
            let reason = "column_not_found";
            unsupported_filters.push(format!("{column_name}: {reason}"));
            diagnostics.push(projection_covi_filter_diagnostic(
                filter,
                ProjectionCoviFilterDiagnosticParams {
                    column: column_name,
                    lineage_status: "column_not_found",
                    logical_type: None,
                    eligible: false,
                    projection_table_id: None,
                    projection_column_id: None,
                    reason,
                },
            ));
            continue;
        };
        let Some(lineage) = &column.lineage else {
            let reason = "missing_lineage";
            unsupported_filters.push(format!("{column_name}: {reason}"));
            diagnostics.push(projection_covi_filter_diagnostic(
                filter,
                ProjectionCoviFilterDiagnosticParams {
                    column: column_name,
                    lineage_status: "missing",
                    logical_type: Some(column.logical_type.clone()),
                    eligible: false,
                    projection_table_id: None,
                    projection_column_id: None,
                    reason,
                },
            ));
            continue;
        };
        if lineage.source != "object_property"
            || lineage.transform != "identity"
            || lineage.filter_pushdown != "projection_covi_prefilter"
        {
            let reason = "lineage_not_covi_eligible";
            unsupported_filters.push(format!("{column_name}: {reason}"));
            diagnostics.push(projection_covi_filter_diagnostic(
                filter,
                ProjectionCoviFilterDiagnosticParams {
                    column: column_name,
                    lineage_status: "ineligible",
                    logical_type: Some(column.logical_type.clone()),
                    eligible: false,
                    projection_table_id: Some(lineage.projection_table_id),
                    projection_column_id: Some(lineage.projection_column_id),
                    reason,
                },
            ));
            continue;
        }
        if let Some(reason) = projection_filter_shape_unsupported_reason(filter) {
            unsupported_filters.push(format!("{column_name}: {reason}"));
            diagnostics.push(projection_covi_filter_diagnostic(
                filter,
                ProjectionCoviFilterDiagnosticParams {
                    column: column_name,
                    lineage_status: "present",
                    logical_type: Some(column.logical_type.clone()),
                    eligible: false,
                    projection_table_id: Some(lineage.projection_table_id),
                    projection_column_id: Some(lineage.projection_column_id),
                    reason,
                },
            ));
            continue;
        }
        diagnostics.push(projection_covi_filter_diagnostic(
            filter,
            ProjectionCoviFilterDiagnosticParams {
                column: column_name,
                lineage_status: "present",
                logical_type: Some(column.logical_type.clone()),
                eligible: true,
                projection_table_id: Some(lineage.projection_table_id),
                projection_column_id: Some(lineage.projection_column_id),
                reason: "eligible",
            },
        ));
        lookups.push(ProjectionCoviFilterLookup {
            column: column_name.to_string(),
            projection_table_id: lineage.projection_table_id,
            projection_column_id: lineage.projection_column_id,
            logical_type: column.logical_type.clone(),
            filter: filter.clone(),
        });
    }
    ProjectionCoviFilterPlan {
        lookups,
        unsupported_filters,
        diagnostics,
    }
}

fn projection_filter_column(filter: &ProjectionFilter) -> &str {
    match filter {
        ProjectionFilter::Compare { column, .. }
        | ProjectionFilter::InList { column, .. }
        | ProjectionFilter::IsNull { column, .. } => column,
    }
}

fn projection_filter_shape_unsupported_reason(filter: &ProjectionFilter) -> Option<&'static str> {
    match filter {
        ProjectionFilter::Compare { op, literal, .. } => {
            if matches!(op, ProjectionFilterOp::Ne) {
                Some("not_equal")
            } else if matches!(literal, ProjectionFilterLiteral::Null) {
                Some("null_literal")
            } else {
                None
            }
        }
        ProjectionFilter::InList { literals, .. } => {
            if literals.is_empty() {
                Some("empty_in_list")
            } else if literals
                .iter()
                .any(|literal| matches!(literal, ProjectionFilterLiteral::Null))
            {
                Some("null_literal")
            } else {
                None
            }
        }
        ProjectionFilter::IsNull { .. } => Some("is_null"),
    }
}

fn projection_covi_filter_diagnostic(
    filter: &ProjectionFilter,
    params: ProjectionCoviFilterDiagnosticParams,
) -> ProjectionCoviFilterDiagnostic {
    ProjectionCoviFilterDiagnostic {
        column: params.column.to_string(),
        op: projection_filter_op(filter).to_string(),
        lineage_status: params.lineage_status.to_string(),
        logical_type: params.logical_type,
        eligible: params.eligible,
        projection_table_id: params.projection_table_id,
        projection_column_id: params.projection_column_id,
        reason: params.reason.to_string(),
    }
}

struct ProjectionCoviFilterDiagnosticParams<'a> {
    column: &'a str,
    lineage_status: &'a str,
    logical_type: Option<String>,
    eligible: bool,
    projection_table_id: Option<u32>,
    projection_column_id: Option<u32>,
    reason: &'a str,
}

fn projection_filter_op(filter: &ProjectionFilter) -> &'static str {
    match filter {
        ProjectionFilter::Compare { op, .. } => match op {
            ProjectionFilterOp::Eq => "eq",
            ProjectionFilterOp::Ne => "ne",
            ProjectionFilterOp::Lt => "lt",
            ProjectionFilterOp::LtEq => "lte",
            ProjectionFilterOp::Gt => "gt",
            ProjectionFilterOp::GtEq => "gte",
        },
        ProjectionFilter::InList { .. } => "in",
        ProjectionFilter::IsNull { .. } => "is_null",
    }
}

pub fn projection_read_requirements_for_catalog(
    catalog: &cove_core::profile::cove_map::MapProjectionCatalog,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<ProjectionReadRequirements, String> {
    projection_read_requirements(catalog, projection_id, options)
}

pub(crate) fn parse_map(path: &Path) -> Result<CovemapFile, String> {
    let bytes = fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    CovemapFile::parse_validated(&bytes).map_err(|err| format!("{}: {err}", path.display()))
}

pub(crate) fn preview(file: &CovemapFile) -> Value {
    json!({
        "mapping_version": file.mapping_version,
        "section_count": file.sections.len(),
        "sections": file.sections.iter().map(|section| {
            let kind = section_kind(section.entry.section_id);
            json!({
                "section_id": section.entry.section_id,
                "kind": kind,
                "required": section.entry.required,
                "payload_len": section.payload.len(),
            })
        }).collect::<Vec<_>>(),
    })
}

pub(crate) fn plan_keys(file: &CovemapFile, rows: &[SourceRow]) -> Value {
    let planned = match plan_identities(file, rows) {
        Ok(planned) => planned,
        Err(message) => return json!({"error": message}),
    };
    json!({
        "rows": planned.canonical.iter().map(|identity| {
            json!({
                "source_id": identity.source_id,
                "row_index": identity.row_index,
                "source_row_identity": identity.source_row_identity,
                "row_digest": identity.row_digest,
                "row_rule_id": identity.row_rule_id,
                "identity_rule_id": identity.identity_rule_id,
                "object_type": identity.object_type,
                "join_key_sha256": identity.join_key_sha256,
                "identity_alias": identity.identity_alias,
                "equivalence_id": identity.equivalence_id,
                "canonical_anchor": identity.canonical_anchor,
                "goid": hex_encode(&identity.goid),
                "resolution": identity.resolution_metadata.iter().map(|metadata| json!({
                    "role_id": metadata.role_id,
                    "resolution_kind": metadata.resolution_kind,
                    "resolver_id": metadata.resolver_id,
                    "normalized_value": metadata.normalized_value,
                    "resolved_identity_value": metadata.resolved_identity_value,
                    "canonical_key": metadata.canonical_key,
                    "alias_hit": metadata.alias_hit,
                    "alias_miss": metadata.alias_miss,
                    "alias_ambiguous": metadata.alias_ambiguous,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "candidate_matches": planned.candidates.iter().map(|candidate| {
            json!({
                "source_id": candidate.source_id,
                "row_index": candidate.row_index,
                "source_row_identity": candidate.source_row_identity,
                "row_digest": candidate.row_digest,
                "row_rule_id": candidate.row_rule_id,
                "identity_rule_id": candidate.identity_rule_id,
                "object_type": candidate.object_type,
                "join_key_sha256": candidate.join_key_sha256,
                "identity_alias": candidate.identity_alias,
                "candidate_match_id": candidate_match_id(candidate),
                "resolution": candidate.resolution_metadata.iter().map(|metadata| json!({
                    "role_id": metadata.role_id,
                    "resolution_kind": metadata.resolution_kind,
                    "resolver_id": metadata.resolver_id,
                    "normalized_value": metadata.normalized_value,
                    "resolved_identity_value": metadata.resolved_identity_value,
                    "canonical_key": metadata.canonical_key,
                    "alias_hit": metadata.alias_hit,
                    "alias_miss": metadata.alias_miss,
                    "alias_ambiguous": metadata.alias_ambiguous,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>()
    })
}

fn materialize_from_paths(map: &Path, sources: &[PathBuf]) -> Result<MaterializedModel, String> {
    let file = parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    validate_source_inputs(&file, &inputs.states)?;
    materialize_with_source_states(&file, &inputs.rows, &inputs.states)
}
