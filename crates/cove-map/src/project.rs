use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{
    builder::StringBuilder, new_empty_array, Array, ArrayRef, BinaryArray, BooleanArray,
    Date32Array, Decimal128Array, Decimal64Array, FixedSizeBinaryArray, Float32Array, Float64Array,
    Int16Array, Int32Array, Int64Array, Int8Array, RecordBatch, RecordBatchOptions, StringArray,
    TimestampMicrosecondArray, TimestampNanosecondArray, UInt16Array, UInt32Array, UInt64Array,
    UInt8Array,
};
use arrow_ipc::writer::FileWriter;
use arrow_json::ReaderBuilder as JsonReaderBuilder;
use arrow_schema::{DataType, Field, Fields, Schema, SchemaRef, TimeUnit};
use cove_core::artifact::covemap::{
    CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1, CovemapSection,
    CovemapSectionEntryV1,
};
use cove_core::profile::{
    cove_map::{
        EmbeddedMapSection, MapEvidenceEntry, MapProjectionCatalog, MapProjectionColumn,
        MapProjectionColumnLineage, MapProjectionEntry,
    },
    cove_o::{
        read_object_surface_from_bytes_with_options, CoveObjectReadOptions, CoveObjectRecord,
        CoveObjectSurface, CoveRecordRefV1, ObjectTypeEntryV1, RecordKind,
        OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_LINK_OBJECT,
        PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT,
        PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
        PROPERTY_FLAG_ASSOCIATION_VALID_FROM, PROPERTY_FLAG_ASSOCIATION_VALID_TO,
        PROPERTY_FLAG_EVIDENCE_REF, PROPERTY_FLAG_MAPPING_RULE_REF,
    },
};
use cove_core::{
    constants::{CoveLogicalType, CovePhysicalKind},
    encoding::nested::{
        ListLayout, ListLayoutPayload, MapLayout, MapLayoutPayload, StructLayout,
        StructLayoutPayload,
    },
    nested_schema::{NestedSchemaEntryV1, NestedSchemaNodeV1, NestedSchemaSectionV1},
    page_payload::{CoveEncodingNodeV1, PageBufferKind},
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use serde_json::{json, Map, Value};

use super::*;

mod encoding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionFormat {
    Json,
    CoveO,
    Arrow,
    CoveT,
    Sql,
}

impl ProjectionFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::CoveO => "cove-o",
            Self::Arrow => "arrow",
            Self::CoveT => "cove-t",
            Self::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectionBatchOptions {
    pub max_rows: Option<usize>,
    pub output_columns: Option<Vec<String>>,
    pub pushed_filters: Vec<ProjectionFilter>,
    pub batch_size: Option<usize>,
    pub candidate_projection_rows: Option<ProjectionCandidateRows>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionCandidateRows {
    pub row_ordinals: BTreeSet<u64>,
}

impl ProjectionCandidateRows {
    pub fn from_ordinals(row_ordinals: impl IntoIterator<Item = u64>) -> Self {
        Self {
            row_ordinals: row_ordinals.into_iter().collect(),
        }
    }

    fn contains(&self, ordinal: u64) -> bool {
        self.row_ordinals.contains(&ordinal)
    }

    fn is_empty(&self) -> bool {
        self.row_ordinals.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionFilterOp {
    Eq,
    Ne,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionFilterLiteral {
    Null,
    Boolean(bool),
    Int64(i64),
    UInt64(u64),
    Float64(f64),
    Utf8(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionFilter {
    Compare {
        column: String,
        op: ProjectionFilterOp,
        literal: ProjectionFilterLiteral,
    },
    InList {
        column: String,
        literals: Vec<ProjectionFilterLiteral>,
    },
    IsNull {
        column: String,
        negated: bool,
    },
}

#[derive(Debug, Clone)]
struct ProjectionAccessPlan {
    requested_property_names: Vec<String>,
    requested_object_type_names: Vec<String>,
    requested_evidence_metadata_keys: Vec<String>,
    include_association_object_types: bool,
    include_records: bool,
    include_evidence_index: bool,
    needs_reconstructed_rows: bool,
    needs_history_rows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionReadRequirements {
    pub requested_object_type_names: Vec<String>,
    pub include_association_object_types: bool,
    pub include_records: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectionSidecarColumnValues {
    pub table_id: u32,
    pub column_id: u32,
    pub logical_type: CoveLogicalType,
    pub physical_kind: CovePhysicalKind,
    pub values: Vec<Value>,
}

pub(crate) use encoding::nested_schema_node_from_shape;

#[derive(Debug, Clone)]
struct ProjectedColumn {
    name: String,
    logical: CoveLogicalType,
    nested_shape: Option<String>,
}

#[derive(Debug, Clone)]
struct ProjectedTable {
    mapping_id: String,
    mapping_version: String,
    projection_id: String,
    output_table: String,
    temporal_cut: Option<String>,
    columns: Vec<ProjectedColumn>,
    rows: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectionLineageContext {
    pub(crate) source_cove_o: Option<ProjectionSourceCoveO>,
    pub(crate) mapping_artifact_digest: Option<String>,
    pub(crate) covm_manifest: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectionSourceCoveO {
    pub(crate) path: Option<String>,
    pub(crate) label: String,
    pub(crate) digest: Option<String>,
}

const DEFAULT_ARROW_BATCH_SIZE: usize = 1024;

pub(crate) fn diff_maps(left: &CovemapFile, right: &CovemapFile) -> Value {
    let left_sections = section_set(left);
    let right_sections = section_set(right);
    let added = right_sections
        .difference(&left_sections)
        .cloned()
        .collect::<Vec<_>>();
    let removed = left_sections
        .difference(&right_sections)
        .cloned()
        .collect::<Vec<_>>();
    let changed = left
        .sections
        .iter()
        .filter_map(|left_section| {
            right
                .sections
                .iter()
                .find(|right_section| {
                    right_section.entry.section_id == left_section.entry.section_id
                })
                .and_then(|right_section| {
                    (sha256_hex(&left_section.payload) != sha256_hex(&right_section.payload))
                        .then(|| section_kind(left_section.entry.section_id))
                })
        })
        .collect::<Vec<_>>();
    json!({
        "mapping_version_changed": left.mapping_version != right.mapping_version,
        "added_sections": added,
        "removed_sections": removed,
        "changed_sections": changed,
    })
}

pub(crate) fn projection_schema_from_descriptor(
    descriptor: &ProjectionDescriptor,
) -> Result<SchemaRef, String> {
    let columns = descriptor
        .columns
        .iter()
        .map(projected_column_from_descriptor)
        .collect::<Result<Vec<_>, _>>()?;
    encoding::arrow_schema_from_projected_columns(&columns)
}

pub(crate) fn project_rows(file: &CovemapFile, rows: &[SourceRow]) -> Result<Value, String> {
    project_rows_with_source_states(file, rows, &[])
}

pub(crate) fn project_rows_with_source_states(
    file: &CovemapFile,
    rows: &[SourceRow],
    source_states: &[ObservedSourceState],
) -> Result<Value, String> {
    let bytes = project_rows_with_source_states_output(
        file,
        rows,
        source_states,
        ProjectionFormat::Json,
        None,
    )?;
    serde_json::from_slice(&bytes).map_err(|err| format!("projection JSON encoding failed: {err}"))
}

pub(crate) fn project_rows_with_source_states_output(
    file: &CovemapFile,
    rows: &[SourceRow],
    source_states: &[ObservedSourceState],
    format: ProjectionFormat,
    projection_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    let materialized = materialize_with_source_states(file, rows, source_states)?;
    let model = ProjectionModel::from_materialized(&materialized);
    let projection_catalog = projection_catalog(file)?
        .ok_or_else(|| "project requires a MAP_PROJECTION_CATALOG section".to_string())?;
    let function_ids = function_registry(file)?;
    let tables = project_tables(
        &model,
        &projection_catalog,
        &function_ids,
        projection_id,
        format,
        &ProjectionBatchOptions::default(),
    )?;
    encode_projection_output(format, &projection_catalog, &tables, None)
}

pub(crate) fn project_cove_o_path(object: &Path, mapping: Option<&Path>) -> Result<Value, String> {
    let bytes = project_cove_o_path_output(object, mapping, ProjectionFormat::Json, None)?;
    serde_json::from_slice(&bytes).map_err(|err| format!("projection JSON encoding failed: {err}"))
}

pub(crate) fn project_cove_o_path_output(
    object: &Path,
    mapping: Option<&Path>,
    format: ProjectionFormat,
    projection_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    let bytes =
        fs::read(object).map_err(|err| format!("cannot read {}: {err}", object.display()))?;
    let lineage = ProjectionLineageContext {
        source_cove_o: Some(ProjectionSourceCoveO {
            path: Some(object.display().to_string()),
            label: object.display().to_string(),
            digest: Some(format!("sha256:{}", sha256_hex(&bytes))),
        }),
        mapping_artifact_digest: mapping
            .map(|path| {
                fs::read(path)
                    .map(|bytes| format!("sha256:{}", sha256_hex(&bytes)))
                    .map_err(|err| format!("cannot read {}: {err}", path.display()))
            })
            .transpose()?,
        covm_manifest: None,
    };
    project_cove_o_bytes_output_with_lineage(
        &bytes,
        mapping,
        format,
        projection_id,
        &object.display().to_string(),
        Some(&lineage),
    )
}

pub(crate) fn projection_catalog_from_cove_o_path(
    object: &Path,
    mapping: Option<&Path>,
) -> Result<MapProjectionCatalog, String> {
    let bytes =
        fs::read(object).map_err(|err| format!("cannot read {}: {err}", object.display()))?;
    projection_catalog_from_cove_o_bytes_internal(&bytes, mapping, &object.display().to_string())
}

pub(crate) fn project_cove_o_bytes_output(
    bytes: &[u8],
    mapping: Option<&Path>,
    format: ProjectionFormat,
    projection_id: Option<&str>,
    object_label: &str,
) -> Result<Vec<u8>, String> {
    project_cove_o_bytes_output_with_lineage(
        bytes,
        mapping,
        format,
        projection_id,
        object_label,
        None,
    )
}

pub(crate) fn project_cove_o_bytes_output_with_lineage(
    bytes: &[u8],
    mapping: Option<&Path>,
    format: ProjectionFormat,
    projection_id: Option<&str>,
    object_label: &str,
    lineage: Option<&ProjectionLineageContext>,
) -> Result<Vec<u8>, String> {
    let projection_catalog =
        projection_catalog_from_cove_o_bytes_internal(bytes, mapping, object_label)?;
    let execution_options = ProjectionBatchOptions::default();
    let surface = read_surface_for_projection(
        bytes,
        mapping,
        object_label,
        &projection_catalog,
        projection_id,
        format,
        &execution_options,
    )?;
    let function_ids = projection_function_ids(mapping, &surface)?;
    let tables = project_tables_from_surface(
        &surface,
        &projection_catalog,
        &function_ids,
        projection_id,
        format,
        &execution_options,
    )?;
    encode_projection_output(format, &projection_catalog, &tables, lineage)
}

pub(crate) fn project_cove_o_bytes_record_batch(
    bytes: &[u8],
    mapping: Option<&Path>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
    object_label: &str,
) -> Result<RecordBatch, String> {
    let projection_catalog =
        projection_catalog_from_cove_o_bytes_internal(bytes, mapping, object_label)?;
    let surface = read_surface_for_projection(
        bytes,
        mapping,
        object_label,
        &projection_catalog,
        Some(projection_id),
        ProjectionFormat::Arrow,
        options,
    )?;
    let function_ids = projection_function_ids(mapping, &surface)?;
    let tables = project_tables_from_surface(
        &surface,
        &projection_catalog,
        &function_ids,
        Some(projection_id),
        ProjectionFormat::Arrow,
        options,
    )?;
    let table = single_projection_table(&tables, "Arrow")?;
    encoding::arrow_record_batch(table)
}

pub(crate) fn project_cove_o_bytes_record_batches(
    bytes: &[u8],
    mapping: Option<&Path>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
    object_label: &str,
) -> Result<Vec<RecordBatch>, String> {
    let projection_catalog =
        projection_catalog_from_cove_o_bytes_internal(bytes, mapping, object_label)?;
    project_cove_o_bytes_record_batches_with_catalog(
        bytes,
        mapping,
        &projection_catalog,
        projection_id,
        options,
        object_label,
    )
}

pub(crate) fn project_cove_o_bytes_record_batches_with_catalog(
    bytes: &[u8],
    mapping: Option<&Path>,
    projection_catalog: &MapProjectionCatalog,
    projection_id: &str,
    options: &ProjectionBatchOptions,
    object_label: &str,
) -> Result<Vec<RecordBatch>, String> {
    let surface = read_surface_for_projection(
        bytes,
        mapping,
        object_label,
        projection_catalog,
        Some(projection_id),
        ProjectionFormat::Arrow,
        options,
    )?;
    let function_ids = projection_function_ids(mapping, &surface)?;
    project_arrow_record_batches_from_surface(
        &surface,
        projection_catalog,
        &function_ids,
        projection_id,
        options,
    )
}

pub(crate) fn project_cove_o_surface_record_batches_with_catalog(
    surface: &CoveObjectSurface,
    projection_catalog: &MapProjectionCatalog,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    let catalog =
        enrich_projection_catalog_lineage(projection_catalog.clone(), &surface.object_types);
    project_arrow_record_batches_from_surface(
        surface,
        &catalog,
        &surface.embedded_function_ids,
        projection_id,
        options,
    )
}

pub(crate) fn projection_sidecar_columns_from_cove_o_bytes(
    bytes: &[u8],
    object_label: &str,
) -> Result<Vec<ProjectionSidecarColumnValues>, String> {
    let catalog = projection_catalog_from_cove_o_bytes_internal(bytes, None, object_label)?;
    let mut out = Vec::new();
    let options = ProjectionBatchOptions::default();
    for projection in &catalog.projections {
        if !projection
            .columns
            .iter()
            .any(|column| column.lineage.is_some())
        {
            continue;
        }
        let surface = read_surface_for_projection(
            bytes,
            None,
            object_label,
            &catalog,
            Some(&projection.projection_id),
            ProjectionFormat::Json,
            &options,
        )?;
        let function_ids = projection_function_ids(None, &surface)?;
        let access_plan = compile_projection_access_plan(
            &catalog,
            Some(&projection.projection_id),
            ProjectionFormat::Json,
            &options,
        )?;
        let model = ProjectionModel::from_surface_with_access_plan(&surface, &access_plan)
            .map_err(|err| err.to_string())?;
        validate_executable_projection(projection, &model, &function_ids)?;
        let rows = project_one(&model, projection, &options)?
            .into_iter()
            .map(|value| match value {
                Value::Object(row) => Ok(projected_table_row(projection, row)),
                _ => Err("projection produced a non-object row".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        for column in &projection.columns {
            let Some(lineage) = &column.lineage else {
                continue;
            };
            let logical_type =
                projection_column_logical_type(column.logical_type.as_deref().unwrap_or("utf8"))?;
            out.push(ProjectionSidecarColumnValues {
                table_id: lineage.projection_table_id,
                column_id: lineage.projection_column_id,
                physical_kind: physical_for_logical(logical_type),
                logical_type,
                values: rows
                    .iter()
                    .map(|row| row.get(&column.name).cloned().unwrap_or(Value::Null))
                    .collect(),
            });
        }
    }
    Ok(out)
}

pub(crate) fn projection_catalog_from_cove_o_bytes_internal(
    bytes: &[u8],
    mapping: Option<&Path>,
    object_label: &str,
) -> Result<MapProjectionCatalog, String> {
    let catalog_surface = read_object_surface_from_bytes_with_options(
        bytes,
        &CoveObjectReadOptions {
            include_projection_catalog: true,
            include_function_registry: false,
            include_records: false,
            include_evidence_index: false,
            ..Default::default()
        },
    )
    .map_err(|err| format!("{object_label}: {err}"))?;
    let catalog = match &catalog_surface.projection_catalog {
        Some(catalog) => Ok(catalog.clone()),
        None => {
            let mapping = mapping.ok_or_else(|| {
                "project-cove-o requires embedded MAP_PROJECTION_CATALOG or --mapping <mapping.covemap>"
                    .to_string()
            })?;
            let file = parse_map(mapping)?;
            projection_catalog(&file)?.ok_or_else(|| {
                "fallback mapping requires a MAP_PROJECTION_CATALOG section".to_string()
            })
        }
    }?;
    Ok(enrich_projection_catalog_lineage(
        catalog,
        &catalog_surface.object_types,
    ))
}

pub(crate) fn projection_read_requirements(
    catalog: &MapProjectionCatalog,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<ProjectionReadRequirements, String> {
    let access_plan = compile_projection_access_plan(
        catalog,
        Some(projection_id),
        ProjectionFormat::Arrow,
        options,
    )?;
    Ok(ProjectionReadRequirements {
        requested_object_type_names: access_plan.requested_object_type_names,
        include_association_object_types: access_plan.include_association_object_types,
        include_records: access_plan.include_records,
    })
}

fn read_surface_for_projection(
    bytes: &[u8],
    mapping: Option<&Path>,
    object_label: &str,
    catalog: &MapProjectionCatalog,
    projection_id: Option<&str>,
    format: ProjectionFormat,
    options: &ProjectionBatchOptions,
) -> Result<CoveObjectSurface, String> {
    let access_plan = compile_projection_access_plan(catalog, projection_id, format, options)?;
    let mut read_options = access_plan.read_options();
    read_options.include_function_registry = mapping.is_none();
    read_object_surface_from_bytes_with_options(bytes, &read_options)
        .map_err(|err| format!("{object_label}: {err}"))
}

fn projection_function_ids(
    mapping: Option<&Path>,
    surface: &CoveObjectSurface,
) -> Result<std::collections::BTreeSet<String>, String> {
    match mapping {
        Some(mapping) => function_registry(&parse_map(mapping)?),
        None => Ok(surface.embedded_function_ids.clone()),
    }
}

fn projection_catalog(file: &CovemapFile) -> Result<Option<MapProjectionCatalog>, String> {
    for section in embedded_sections(file)? {
        if let EmbeddedMapSection::ProjectionCatalog(catalog) = section {
            return Ok(Some(catalog));
        }
    }
    Ok(None)
}

pub(crate) fn enrich_projection_catalog_lineage(
    mut catalog: MapProjectionCatalog,
    object_types: &[ObjectTypeEntryV1],
) -> MapProjectionCatalog {
    let object_types_by_name = object_types
        .iter()
        .map(|object_type| (object_type.type_name.as_str(), object_type))
        .collect::<BTreeMap<_, _>>();
    for (projection_index, projection) in catalog.projections.iter_mut().enumerate() {
        let projection_table_id = (projection_index as u32).saturating_add(1);
        let projection_shape = projection.clone();
        for (column_index, column) in projection.columns.iter_mut().enumerate() {
            let projection_column_id = (column_index as u32).saturating_add(1);
            column.lineage = projection_column_object_property_lineage(
                &projection_shape,
                column,
                projection_table_id,
                projection_column_id,
                &object_types_by_name,
            );
        }
    }
    catalog
}

fn projection_column_object_property_lineage(
    projection: &MapProjectionEntry,
    column: &MapProjectionColumn,
    projection_table_id: u32,
    projection_column_id: u32,
    object_types_by_name: &BTreeMap<&str, &ObjectTypeEntryV1>,
) -> Option<MapProjectionColumnLineage> {
    let row_grain = projection.row_grain.as_deref()?;
    if !matches!(
        row_grain,
        "one_row_per_object" | "one_row_per_event_object" | "one_row_per_object_as_of_time"
    ) {
        return None;
    }
    if column.conflict_policy != "canonical_value" || column.nested_shape.is_some() {
        return None;
    }
    let anchor = projection.anchor.as_ref()?;
    if anchor.association_type.is_some() {
        return None;
    }
    let object_type_name = anchor.object_type.as_ref()?;
    let object_type = object_types_by_name.get(object_type_name.as_str())?;
    let expression = column.value.trim();
    if !direct_property_expression(expression) {
        return None;
    }
    let property_name = expression.rsplit('.').next()?;
    let property = object_type
        .properties
        .iter()
        .find(|property| property.property_name == property_name)?;
    if !projection_property_lineage_type_supported(property.logical_type) {
        return None;
    }
    if let Some(logical_type) = &column.logical_type {
        if projection_column_logical_type(logical_type).ok()? != property.logical_type {
            return None;
        }
    }
    Some(MapProjectionColumnLineage {
        source: "object_property".into(),
        object_type_id: object_type.object_type_id,
        object_type_name: object_type.type_name.clone(),
        property_id: property.property_id,
        property_name: property.property_name.clone(),
        projection_table_id,
        projection_column_id,
        expression: expression.to_string(),
        transform: "identity".into(),
        filter_pushdown: "projection_covi_prefilter".into(),
    })
}

fn direct_property_expression(expression: &str) -> bool {
    if expression.is_empty()
        || expression.contains('(')
        || expression.contains(')')
        || expression.starts_with("association.")
        || expression.starts_with("evidence.")
        || literal_value(expression).is_some()
        || known_projection_path(expression)
        || split_comparison_expression(expression).is_some()
        || parse_association_traversal(expression).is_some()
    {
        return false;
    }
    expression
        .rsplit('.')
        .next()
        .is_some_and(|property_name| !property_name.is_empty())
}

fn projection_property_lineage_type_supported(logical_type: CoveLogicalType) -> bool {
    !matches!(
        logical_type,
        CoveLogicalType::Null
            | CoveLogicalType::List
            | CoveLogicalType::Struct
            | CoveLogicalType::Map
    )
}

pub(crate) fn projection_catalog_json_value(catalog: &MapProjectionCatalog) -> Value {
    json!({
        "mapping_id": catalog.mapping_id,
        "mapping_version": catalog.mapping_version,
        "projections": catalog.projections.iter().map(projection_entry_json_value).collect::<Vec<_>>()
    })
}

fn projection_entry_json_value(projection: &MapProjectionEntry) -> Value {
    let mut value = json!({
        "projection_id": projection.projection_id,
        "assertion_ids": projection.assertion_ids,
        "output_table": projection.output_table,
        "row_grain": projection.row_grain,
        "anchor": projection.anchor.as_ref().map(|anchor| json!({
            "object_type": anchor.object_type,
            "association_type": anchor.association_type,
        })),
        "temporal_mode": projection.temporal_mode,
        "columns": projection.columns.iter().map(projection_column_json_value).collect::<Vec<_>>(),
        "multi_value_policy": projection.multi_value_policy,
        "missing_policy": projection.missing_policy,
        "ordering": projection.ordering,
        "evidence_policy": projection.evidence_policy,
        "output_modes": projection.output_modes,
    });
    strip_null_json_fields(&mut value);
    value
}

fn projection_column_json_value(column: &MapProjectionColumn) -> Value {
    let mut value = json!({
        "name": column.name,
        "value": column.value,
        "logical_type": column.logical_type,
        "nested_shape": column.nested_shape,
        "conflict_policy": column.conflict_policy,
        "missing_policy": column.missing_policy,
        "lineage": column.lineage.as_ref().map(projection_column_lineage_json_value),
    });
    strip_null_json_fields(&mut value);
    value
}

fn projection_column_lineage_json_value(lineage: &MapProjectionColumnLineage) -> Value {
    json!({
        "source": lineage.source,
        "object_type_id": lineage.object_type_id,
        "object_type_name": lineage.object_type_name,
        "property_id": lineage.property_id,
        "property_name": lineage.property_name,
        "projection_table_id": lineage.projection_table_id,
        "projection_column_id": lineage.projection_column_id,
        "expression": lineage.expression,
        "transform": lineage.transform,
        "filter_pushdown": lineage.filter_pushdown,
    })
}

fn strip_null_json_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for value in object.values_mut() {
                strip_null_json_fields(value);
            }
            object.retain(|_, value| !value.is_null());
        }
        Value::Array(values) => {
            for value in values {
                strip_null_json_fields(value);
            }
        }
        _ => {}
    }
}

fn function_registry(file: &CovemapFile) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(embedded_function_registry(&embedded_sections(file)?))
}

fn embedded_function_registry(
    sections: &[EmbeddedMapSection],
) -> std::collections::BTreeSet<String> {
    let mut ids = std::collections::BTreeSet::new();
    for section in sections {
        if let EmbeddedMapSection::FunctionRegistry(registry) = section {
            ids.extend(
                registry
                    .functions
                    .iter()
                    .map(|function| function.function_id.clone()),
            );
        }
    }
    ids
}

impl ProjectionAccessPlan {
    fn read_options(&self) -> CoveObjectReadOptions {
        CoveObjectReadOptions {
            requested_property_ids: Vec::new(),
            requested_property_names: self.requested_property_names.clone(),
            requested_object_type_names: self.requested_object_type_names.clone(),
            requested_evidence_metadata_keys: self.requested_evidence_metadata_keys.clone(),
            include_projection_catalog: true,
            include_function_registry: true,
            include_association_object_types: self.include_association_object_types,
            include_records: self.include_records,
            include_evidence_index: self.include_evidence_index,
            redaction_read_policy: Default::default(),
        }
    }
}

fn compile_projection_access_plan(
    catalog: &MapProjectionCatalog,
    projection_id: Option<&str>,
    format: ProjectionFormat,
    options: &ProjectionBatchOptions,
) -> Result<ProjectionAccessPlan, String> {
    let selected = select_projections(catalog, projection_id, format)?;
    let output_columns = required_projection_columns(options);
    let mut requested_property_names = BTreeSet::new();
    let mut requested_object_type_names = BTreeSet::new();
    let mut requested_evidence_metadata_keys = BTreeSet::new();
    let mut include_association_object_types = false;
    let mut can_prune_object_types = true;
    let mut include_records = false;
    let mut include_evidence_index = false;
    let mut needs_reconstructed_rows = false;
    let mut needs_history_rows = false;

    for projection in &selected {
        let projection = trim_projection_columns(projection, output_columns.as_ref())?;
        let row_grain = projection.row_grain.as_deref().unwrap_or_default();
        let mut uses_association_rows = false;
        if let Some(object_type) = projection
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.object_type.as_ref())
        {
            requested_object_type_names.insert(object_type.clone());
        } else if projection
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.association_type.as_ref())
            .is_some()
        {
            include_association_object_types = true;
        } else {
            can_prune_object_types = false;
        }
        for column in &projection.columns {
            if expression_requires_association_rows(&column.value) {
                uses_association_rows = true;
            }
            collect_projection_expression_requirements(
                &column.value,
                &mut requested_property_names,
                &mut requested_evidence_metadata_keys,
                &mut include_evidence_index,
            );
        }
        for ordering in &projection.ordering {
            let expression = ordering_expression(ordering);
            if expression != "value" {
                if expression_requires_association_rows(expression) {
                    uses_association_rows = true;
                }
                collect_projection_expression_requirements(
                    expression,
                    &mut requested_property_names,
                    &mut requested_evidence_metadata_keys,
                    &mut include_evidence_index,
                );
            }
        }
        if projection
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.association_type.as_ref())
            .is_some()
        {
            include_association_object_types = true;
            requested_property_names.insert("association_type".into());
        }
        if uses_association_rows {
            include_association_object_types = true;
            requested_property_names.insert("association_type".into());
        }
        match row_grain {
            "one_row_per_object"
            | "one_row_per_event_object"
            | "one_row_per_object_as_of_time"
            | "one_row_per_association"
            | "one_row_per_link_object" => {
                include_records = true;
                let temporal_mode = projection
                    .temporal_mode
                    .as_deref()
                    .unwrap_or("latest_committed");
                if matches!(
                    parse_projection_temporal_mode(temporal_mode),
                    Some(
                        ProjectionTemporalMode::LatestCommitted | ProjectionTemporalMode::ValidTime
                    )
                ) {
                    needs_reconstructed_rows = true;
                } else {
                    needs_history_rows = true;
                }
            }
            "one_row_per_property_version" => {
                include_records = true;
                needs_history_rows = true;
            }
            "one_row_per_evidence_assertion" => {
                include_evidence_index = true;
            }
            _ => {
                include_records = true;
                needs_reconstructed_rows = true;
            }
        }
    }
    if !can_prune_object_types {
        requested_object_type_names.clear();
        include_association_object_types = false;
    }

    Ok(ProjectionAccessPlan {
        requested_property_names: requested_property_names.into_iter().collect(),
        requested_object_type_names: requested_object_type_names.into_iter().collect(),
        requested_evidence_metadata_keys: requested_evidence_metadata_keys.into_iter().collect(),
        include_association_object_types,
        include_records,
        include_evidence_index,
        needs_reconstructed_rows,
        needs_history_rows,
    })
}

fn select_projections<'a>(
    catalog: &'a MapProjectionCatalog,
    projection_id: Option<&str>,
    format: ProjectionFormat,
) -> Result<Vec<&'a MapProjectionEntry>, String> {
    let selected = catalog
        .projections
        .iter()
        .filter(|projection| {
            projection_id
                .map(|requested| projection.projection_id == requested)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(match projection_id {
            Some(id) => format!("projection_id '{id}' was not found"),
            None => "projection catalog contains no projections".to_string(),
        });
    }
    if matches!(format, ProjectionFormat::Arrow | ProjectionFormat::CoveT)
        && projection_id.is_none()
        && selected.len() != 1
    {
        return Err("--projection-id is required for Arrow or COVE-T output when a catalog contains multiple projections".into());
    }
    Ok(selected)
}

fn collect_projection_expression_requirements(
    expression: &str,
    property_names: &mut BTreeSet<String>,
    evidence_metadata_keys: &mut BTreeSet<String>,
    include_evidence_index: &mut bool,
) {
    let expression = expression.trim();
    if expression.is_empty() || literal_value(expression).is_some() {
        return;
    }
    if known_projection_path(expression) {
        if let Some(property_name) = association_metadata_property_name(expression) {
            property_names.insert(property_name.to_string());
        }
        return;
    }
    if expression.starts_with("evidence.") {
        *include_evidence_index = true;
        if let Some(key) = expression.strip_prefix("evidence.") {
            if !is_builtin_evidence_field(key) {
                evidence_metadata_keys.insert(key.to_string());
            }
        }
        return;
    }
    if let Some(resolution) = parse_resolution_expression(expression) {
        *include_evidence_index = true;
        for key in [
            "identity_rule_id",
            "resolution_role_id",
            "alias_hit",
            resolution.field,
        ] {
            if !is_builtin_evidence_field(key) {
                evidence_metadata_keys.insert(key.to_string());
            }
        }
        return;
    }
    if let Some(traversal) = parse_association_traversal(expression) {
        add_association_access_requirements(property_names);
        property_names.insert(traversal.property_name.to_string());
        return;
    }
    if let Some((function, args)) = parse_function_call(expression) {
        if function == "association" {
            add_association_access_requirements(property_names);
        }
        for arg in args {
            collect_projection_expression_requirements(
                &arg,
                property_names,
                evidence_metadata_keys,
                include_evidence_index,
            );
        }
        return;
    }
    if let Some((left, right)) = split_comparison_expression(expression) {
        collect_projection_expression_requirements(
            left,
            property_names,
            evidence_metadata_keys,
            include_evidence_index,
        );
        collect_projection_expression_requirements(
            right,
            property_names,
            evidence_metadata_keys,
            include_evidence_index,
        );
        return;
    }
    if let Some(property_name) = expression.rsplit('.').next() {
        if !property_name.is_empty() {
            property_names.insert(property_name.to_string());
        }
    }
}

fn is_builtin_evidence_field(key: &str) -> bool {
    matches!(
        key,
        "source_id"
            | "source_row_identity"
            | "rule_id"
            | "assertion_id"
            | "output_object_id"
            | "observed_schema_fingerprint"
            | "observed_snapshot_digest"
    )
}

fn add_association_access_requirements(property_names: &mut BTreeSet<String>) {
    for property_name in [
        "association_type",
        "source_goid",
        "target_goid",
        "source_role",
        "target_role",
    ] {
        property_names.insert(property_name.to_string());
    }
}

fn association_metadata_property_name(expression: &str) -> Option<&'static str> {
    match expression {
        "association.source_goid" => Some("source_goid"),
        "association.target_goid" => Some("target_goid"),
        "association.association_type" => Some("association_type"),
        "association.mapping_rule_id" => Some("mapping_rule_id"),
        "association.source_evidence_id" => Some("source_evidence_id"),
        "association.source_role" => Some("source_role"),
        "association.target_role" => Some("target_role"),
        "association.valid_from" => Some("valid_from"),
        "association.valid_to" => Some("valid_to"),
        "association.observed_at" => Some("observed_at"),
        "association.cardinality_policy" => Some("cardinality_policy"),
        _ => None,
    }
}

fn expression_requires_association_rows(expression: &str) -> bool {
    let expression = expression.trim();
    if expression.starts_with("association.") {
        return true;
    }
    if let Some(traversal) = parse_association_traversal(expression) {
        return !traversal.association_type.is_empty();
    }
    if let Some((left, right)) = split_comparison_expression(expression) {
        return expression_requires_association_rows(left)
            || expression_requires_association_rows(right);
    }
    if let Some((function, args)) = parse_function_call(expression) {
        return function == "association"
            || args
                .iter()
                .any(|arg| expression_requires_association_rows(arg));
    }
    false
}

fn trim_projection_columns(
    projection: &MapProjectionEntry,
    output_columns: Option<&BTreeSet<String>>,
) -> Result<MapProjectionEntry, String> {
    let Some(output_columns) = output_columns else {
        return Ok(projection.clone());
    };
    let mut trimmed = projection.clone();
    trimmed.columns = projection
        .columns
        .iter()
        .filter(|column| output_columns.contains(&column.name))
        .cloned()
        .collect();
    let missing = output_columns
        .iter()
        .filter(|name| {
            !projection
                .columns
                .iter()
                .any(|column| &column.name == *name)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "projection '{}' does not contain requested columns: {}",
            projection.projection_id,
            missing.join(", ")
        ));
    }
    Ok(trimmed)
}

fn project_tables_from_surface(
    surface: &CoveObjectSurface,
    catalog: &MapProjectionCatalog,
    function_ids: &std::collections::BTreeSet<String>,
    projection_id: Option<&str>,
    format: ProjectionFormat,
    options: &ProjectionBatchOptions,
) -> Result<Vec<ProjectedTable>, String> {
    let access_plan = compile_projection_access_plan(catalog, projection_id, format, options)?;
    let model = ProjectionModel::from_surface_with_access_plan(surface, &access_plan)
        .map_err(|err| err.to_string())?;
    project_tables(
        &model,
        catalog,
        function_ids,
        projection_id,
        format,
        options,
    )
}

fn project_tables(
    model: &ProjectionModel,
    catalog: &MapProjectionCatalog,
    function_ids: &std::collections::BTreeSet<String>,
    projection_id: Option<&str>,
    format: ProjectionFormat,
    options: &ProjectionBatchOptions,
) -> Result<Vec<ProjectedTable>, String> {
    let output_columns = required_projection_columns(options);
    let selected = select_projections(catalog, projection_id, format)?;

    let mut tables = Vec::new();
    for projection in selected {
        ensure_projection_declares_format(projection, format)?;
        let projection = trim_projection_columns(projection, output_columns.as_ref())?;
        validate_executable_projection(&projection, model, function_ids)?;
        let rows = project_one(model, &projection, options)?
            .into_iter()
            .map(|value| match value {
                Value::Object(row) => Ok(projected_table_row(&projection, row)),
                _ => Err("projection produced a non-object row".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        tables.push(ProjectedTable {
            mapping_id: catalog.mapping_id.clone(),
            mapping_version: catalog.mapping_version.clone(),
            projection_id: projection.projection_id.clone(),
            output_table: projection
                .output_table
                .clone()
                .unwrap_or_else(|| projection.projection_id.clone()),
            temporal_cut: projection.temporal_mode.clone(),
            columns: projection
                .columns
                .iter()
                .map(projected_column_from_entry)
                .collect::<Result<Vec<_>, _>>()?,
            rows,
        });
    }
    Ok(tables)
}

fn required_projection_columns(options: &ProjectionBatchOptions) -> Option<BTreeSet<String>> {
    let mut columns = options
        .output_columns
        .as_ref()
        .map(|columns| columns.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    for filter in &options.pushed_filters {
        columns.insert(filter.column_name().to_string());
    }
    (!columns.is_empty()).then_some(columns)
}

fn output_projection_columns(options: &ProjectionBatchOptions) -> Option<BTreeSet<String>> {
    options
        .output_columns
        .as_ref()
        .map(|columns| columns.iter().cloned().collect::<BTreeSet<_>>())
        .filter(|columns| !columns.is_empty())
}

fn project_arrow_record_batches_from_surface(
    surface: &CoveObjectSurface,
    catalog: &MapProjectionCatalog,
    function_ids: &std::collections::BTreeSet<String>,
    projection_id: &str,
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    let access_plan = compile_projection_access_plan(
        catalog,
        Some(projection_id),
        ProjectionFormat::Arrow,
        options,
    )?;
    let model = ProjectionModel::from_surface_with_access_plan(surface, &access_plan)
        .map_err(|err| err.to_string())?;
    let selected = select_projections(catalog, Some(projection_id), ProjectionFormat::Arrow)?;
    let projection = match selected.as_slice() {
        [projection] => *projection,
        _ => return Err("Arrow projection output requires exactly one projection".into()),
    };
    ensure_projection_declares_format(projection, ProjectionFormat::Arrow)?;
    let access_columns = required_projection_columns(options);
    let output_columns = output_projection_columns(options);
    let access_projection = trim_projection_columns(projection, access_columns.as_ref())?;
    validate_executable_projection(&access_projection, &model, function_ids)?;
    let output_projection = trim_projection_columns(projection, output_columns.as_ref())?;
    let output_projected_columns = output_projection
        .columns
        .iter()
        .map(projected_column_from_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(plan) = object_projection_arrow_fast_path_plan(
        &access_projection,
        &output_projection,
        &output_projected_columns,
        options,
    ) {
        return project_arrow_object_record_batches_fast(
            &model,
            &output_projection,
            &output_projected_columns,
            &plan,
            options,
        );
    }
    if evidence_projection_arrow_fast_path_supported(&output_projection, &output_projected_columns)
        && projection_contains_filter_columns(&output_projection, options)
    {
        return project_arrow_evidence_record_batches_fast(
            &model,
            &output_projection,
            &output_projected_columns,
            options,
        );
    }
    let mut sink = ArrowRecordBatchSink::new(
        &catalog.mapping_id,
        &catalog.mapping_version,
        &output_projection,
        output_projected_columns,
        options
            .batch_size
            .unwrap_or(DEFAULT_ARROW_BATCH_SIZE)
            .max(1),
        options.max_rows,
    );
    emit_projection_rows(&model, &access_projection, options, |row| sink.push(row))?;
    sink.finish()
}

#[derive(Debug, Clone)]
enum ObjectProjectionArrowAccessor {
    ObjectGoid,
    Property(String),
}

#[derive(Debug, Clone)]
struct ObjectProjectionArrowFastPathPlan {
    output_accessors: Vec<ObjectProjectionArrowAccessor>,
    filter_accessors: BTreeMap<String, ObjectProjectionArrowAccessor>,
}

fn object_projection_arrow_fast_path_plan(
    access_projection: &MapProjectionEntry,
    output_projection: &MapProjectionEntry,
    output_projected_columns: &[ProjectedColumn],
    options: &ProjectionBatchOptions,
) -> Option<ObjectProjectionArrowFastPathPlan> {
    if !access_projection.ordering.is_empty()
        || access_projection
            .multi_value_policy
            .as_deref()
            .unwrap_or("reject")
            != "reject"
    {
        return None;
    }
    if !matches!(
        access_projection.row_grain.as_deref(),
        Some("one_row_per_object" | "one_row_per_event_object" | "one_row_per_object_as_of_time")
    ) {
        return None;
    }
    let anchor = access_projection.anchor.as_ref()?;
    if anchor.object_type.is_none() || anchor.association_type.is_some() {
        return None;
    }

    let output_accessors = output_projection
        .columns
        .iter()
        .zip(output_projected_columns.iter())
        .map(|(column, projected)| object_projection_arrow_accessor_for_column(column, projected))
        .collect::<Option<Vec<_>>>()?;

    let mut filter_accessors = BTreeMap::new();
    for filter in &options.pushed_filters {
        let column_name = filter.column_name();
        let column = access_projection
            .columns
            .iter()
            .find(|candidate| candidate.name == column_name)?;
        let projected = projected_column_from_entry(column).ok()?;
        if !object_projection_fast_path_filter_column_supported(&projected) {
            return None;
        }
        let accessor = object_projection_arrow_accessor_for_column(column, &projected)?;
        filter_accessors.insert(column_name.to_string(), accessor);
    }

    Some(ObjectProjectionArrowFastPathPlan {
        output_accessors,
        filter_accessors,
    })
}

fn object_projection_arrow_accessor_for_column(
    column: &MapProjectionColumn,
    projected: &ProjectedColumn,
) -> Option<ObjectProjectionArrowAccessor> {
    match column.value.as_str() {
        "goid" | "object.goid" | "Object.goid" => matches!(
            projected.logical,
            CoveLogicalType::Uuid | CoveLogicalType::Utf8
        )
        .then_some(ObjectProjectionArrowAccessor::ObjectGoid),
        value => direct_object_property_fast_path_property_name(value)
            .map(ObjectProjectionArrowAccessor::Property),
    }
}

fn object_projection_fast_path_filter_column_supported(projected: &ProjectedColumn) -> bool {
    projected.nested_shape.is_none()
        && !matches!(
            projected.logical,
            CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map
        )
}

fn direct_object_property_fast_path_property_name(expression: &str) -> Option<String> {
    let expression = expression.trim();
    let property_name = expression.strip_prefix("property.").unwrap_or(expression);
    if !property_name.is_empty()
        && !property_name.contains('.')
        && !expression.contains(char::is_whitespace)
        && !expression.chars().any(|ch| {
            matches!(
                ch,
                '(' | ')' | '[' | ']' | '{' | '}' | '+' | '-' | '*' | '/' | '?' | ':'
            )
        })
    {
        Some(property_name.to_string())
    } else {
        None
    }
}

fn project_arrow_object_record_batches_fast(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    projected_columns: &[ProjectedColumn],
    plan: &ObjectProjectionArrowFastPathPlan,
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    let schema = encoding::arrow_schema_from_projected_columns(projected_columns)?;
    let batch_size = options
        .batch_size
        .unwrap_or(DEFAULT_ARROW_BATCH_SIZE)
        .max(1);
    let max_rows = options.max_rows.unwrap_or(usize::MAX);
    let object_type = projection
        .anchor
        .as_ref()
        .and_then(|anchor| anchor.object_type.as_ref())
        .ok_or_else(|| "object projection anchor object_type is required".to_string())?;
    let rows = model.rows_for_projection(projection)?;
    let mut batches = Vec::new();
    let mut chunk = Vec::new();
    let mut emitted_rows = 0usize;
    let mut projection_row_ordinal = 0u64;

    for row in rows.iter() {
        if emitted_rows >= max_rows {
            break;
        }
        if &row.object_type != object_type {
            continue;
        }
        let row_ordinal = projection_row_ordinal;
        projection_row_ordinal = projection_row_ordinal.saturating_add(1);
        if !candidate_projection_row_allowed(options, row_ordinal) {
            continue;
        }
        if !object_row_matches_fast_path_filters(
            row,
            &options.pushed_filters,
            &plan.filter_accessors,
            &projection.projection_id,
        )? {
            continue;
        }
        chunk.push(row);
        emitted_rows += 1;
        if chunk.len() >= batch_size {
            batches.push(build_object_record_batch(
                &schema,
                projected_columns,
                &plan.output_accessors,
                &chunk,
            )?);
            chunk.clear();
        }
    }

    if !chunk.is_empty() || batches.is_empty() {
        batches.push(build_object_record_batch(
            &schema,
            projected_columns,
            &plan.output_accessors,
            &chunk,
        )?);
    }

    Ok(batches)
}

fn object_row_matches_fast_path_filters(
    row: &ProjectionRow,
    filters: &[ProjectionFilter],
    filter_accessors: &BTreeMap<String, ObjectProjectionArrowAccessor>,
    projection_id: &str,
) -> Result<bool, String> {
    for filter in filters {
        let column = filter.column_name();
        let accessor = filter_accessors.get(column).ok_or_else(|| {
            format!(
                "object projection '{projection_id}' fast path is missing filter column '{column}'"
            )
        })?;
        let value = object_projection_accessor_value(row, accessor, projection_id, column)?;
        if !projection_filter_matches_value(filter, value.as_ref()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn projection_contains_filter_columns(
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
) -> bool {
    options.pushed_filters.iter().all(|filter| {
        projection
            .columns
            .iter()
            .any(|column| column.name == filter.column_name())
    })
}

fn evidence_projection_arrow_fast_path_supported(
    projection: &MapProjectionEntry,
    projected_columns: &[ProjectedColumn],
) -> bool {
    projection.row_grain.as_deref() == Some("one_row_per_evidence_assertion")
        && projection.columns.len() == projected_columns.len()
        && projection.columns.iter().all(|column| {
            column.value.starts_with("evidence.")
                && !matches!(
                    projected_columns
                        .iter()
                        .find(|projected| projected.name == column.name)
                        .map(|projected| projected.logical),
                    Some(CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map)
                )
        })
}

fn project_arrow_evidence_record_batches_fast(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    projected_columns: &[ProjectedColumn],
    options: &ProjectionBatchOptions,
) -> Result<Vec<RecordBatch>, String> {
    let schema = encoding::arrow_schema_from_projected_columns(projected_columns)?;
    let batch_size = options
        .batch_size
        .unwrap_or(DEFAULT_ARROW_BATCH_SIZE)
        .max(1);
    let filter_keys = projection_filter_evidence_keys(projection, &options.pushed_filters)?;
    let mut batches = Vec::new();
    let mut chunk = Vec::new();
    let max_rows = options.max_rows.unwrap_or(usize::MAX);
    let mut emitted_rows = 0usize;
    let mut projection_row_ordinal = 0u64;

    for entry in &model.evidence_entries {
        if emitted_rows >= max_rows {
            break;
        }
        let row_ordinal = projection_row_ordinal;
        projection_row_ordinal = projection_row_ordinal.saturating_add(1);
        if !candidate_projection_row_allowed(options, row_ordinal) {
            continue;
        }
        if !evidence_matches_projection_filters(entry, &filter_keys, &options.pushed_filters) {
            continue;
        }
        chunk.push(entry);
        emitted_rows += 1;
        if chunk.len() >= batch_size {
            batches.push(build_evidence_record_batch(
                &schema,
                projected_columns,
                projection,
                &chunk,
            )?);
            chunk.clear();
        }
    }

    if !chunk.is_empty() || batches.is_empty() {
        batches.push(build_evidence_record_batch(
            &schema,
            projected_columns,
            projection,
            &chunk,
        )?);
    }

    Ok(batches)
}

fn projection_filter_evidence_keys(
    projection: &MapProjectionEntry,
    filters: &[ProjectionFilter],
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for filter in filters {
        let column = filter.column_name();
        let projection_column = projection
            .columns
            .iter()
            .find(|candidate| candidate.name == column)
            .ok_or_else(|| {
                format!(
                    "evidence projection '{}' is missing filter column '{}'",
                    projection.projection_id, column
                )
            })?;
        let key = projection_column
            .value
            .strip_prefix("evidence.")
            .ok_or_else(|| {
                format!(
                    "evidence projection '{}' uses unsupported filtered expression '{}'",
                    projection.projection_id, projection_column.value
                )
            })?;
        out.insert(column.to_string(), key.to_string());
    }
    Ok(out)
}

fn evidence_matches_projection_filters(
    entry: &ProjectionEvidenceEntry,
    filter_keys: &BTreeMap<String, String>,
    filters: &[ProjectionFilter],
) -> bool {
    if filters.is_empty() {
        return true;
    }
    let row = filter_keys
        .iter()
        .map(|(column, key)| (column.clone(), projection_evidence_value(entry, key)))
        .collect::<Map<_, _>>();
    row_matches_projection_filters(&row, filters)
}

fn build_evidence_record_batch(
    schema: &SchemaRef,
    projected_columns: &[ProjectedColumn],
    projection: &MapProjectionEntry,
    entries: &[&ProjectionEvidenceEntry],
) -> Result<RecordBatch, String> {
    let arrays = projection
        .columns
        .iter()
        .zip(projected_columns.iter())
        .map(|(column, projected)| {
            let key = column
                .value
                .strip_prefix("evidence.")
                .ok_or_else(|| format!("unsupported evidence expression '{}'", column.value))?;
            build_evidence_record_batch_column(entries, key, projected)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let options = RecordBatchOptions::new().with_row_count(Some(entries.len()));
    RecordBatch::try_new_with_options(Arc::clone(schema), arrays, &options)
        .map_err(|err| format!("cannot build Arrow evidence record batch: {err}"))
}

fn build_object_record_batch(
    schema: &SchemaRef,
    projected_columns: &[ProjectedColumn],
    accessors: &[ObjectProjectionArrowAccessor],
    rows: &[&ProjectionRow],
) -> Result<RecordBatch, String> {
    let arrays = projected_columns
        .iter()
        .zip(accessors.iter())
        .map(|(projected, accessor)| build_object_record_batch_column(rows, accessor, projected))
        .collect::<Result<Vec<_>, _>>()?;
    let options = RecordBatchOptions::new().with_row_count(Some(rows.len()));
    RecordBatch::try_new_with_options(Arc::clone(schema), arrays, &options)
        .map_err(|err| format!("cannot build Arrow object record batch: {err}"))
}

fn build_object_record_batch_column(
    rows: &[&ProjectionRow],
    accessor: &ObjectProjectionArrowAccessor,
    projected: &ProjectedColumn,
) -> Result<ArrayRef, String> {
    match accessor {
        ObjectProjectionArrowAccessor::ObjectGoid => build_object_goid_array(rows, projected),
        ObjectProjectionArrowAccessor::Property(property_name) => {
            let owned = rows
                .iter()
                .map(|row| {
                    object_projection_accessor_value(
                        row,
                        accessor,
                        "<arrow-fast-path>",
                        property_name,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let values = owned.iter().map(Cow::as_ref).collect::<Vec<_>>();
            encoding::encode_arrow_values(&values, projected)
        }
    }
}

fn object_projection_accessor_value<'a>(
    row: &'a ProjectionRow,
    accessor: &ObjectProjectionArrowAccessor,
    projection_id: &str,
    column_name: &str,
) -> Result<Cow<'a, Value>, String> {
    match accessor {
        ObjectProjectionArrowAccessor::ObjectGoid => {
            Ok(Cow::Owned(Value::String(hex_encode(&row.goid))))
        }
        ObjectProjectionArrowAccessor::Property(property_name) => {
            let Some(value) = projection_property_ref_by_name(row, property_name) else {
                return Ok(Cow::Owned(Value::Null));
            };
            let Some(values) = value.as_array() else {
                return Ok(Cow::Borrowed(value));
            };
            match values.as_slice() {
                [] => Ok(Cow::Owned(Value::Null)),
                [value] => Ok(Cow::Borrowed(value)),
                _ => Err(format!(
                    "projection '{projection_id}' column '{column_name}' produced {} values with multi_value_policy='reject'",
                    values.len()
                )),
            }
        }
    }
}

fn build_object_goid_array(
    rows: &[&ProjectionRow],
    projected: &ProjectedColumn,
) -> Result<ArrayRef, String> {
    match projected.logical {
        CoveLogicalType::Uuid => {
            let borrowed = rows
                .iter()
                .map(|row| Some(row.goid.as_slice()))
                .collect::<Vec<_>>();
            Ok(Arc::new(FixedSizeBinaryArray::from(borrowed)) as ArrayRef)
        }
        CoveLogicalType::Utf8 => {
            let mut builder = StringBuilder::new();
            for row in rows {
                builder.append_value(hex_encode(&row.goid));
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        logical => Err(format!(
            "object.goid fast path does not support Arrow logical type {:?}",
            logical
        )),
    }
}

fn build_evidence_record_batch_column(
    entries: &[&ProjectionEvidenceEntry],
    key: &str,
    projected: &ProjectedColumn,
) -> Result<ArrayRef, String> {
    match projected.logical {
        CoveLogicalType::Utf8 => {
            let mut builder = StringBuilder::new();
            for entry in entries {
                match evidence_utf8_value(entry, key) {
                    Some(value) => builder.append_value(value.as_ref()),
                    None => builder.append_null(),
                }
            }
            Ok(Arc::new(builder.finish()) as ArrayRef)
        }
        CoveLogicalType::Uuid => {
            let owned = entries
                .iter()
                .map(|entry| evidence_uuid_value(entry, key))
                .collect::<Result<Vec<_>, _>>()?;
            let borrowed = owned
                .iter()
                .map(|value| value.as_ref().map(|value| value.as_slice()))
                .collect::<Vec<_>>();
            Ok(Arc::new(FixedSizeBinaryArray::from(borrowed)) as ArrayRef)
        }
        _ => {
            let owned = entries
                .iter()
                .map(|entry| projection_evidence_value(entry, key))
                .collect::<Vec<_>>();
            let values = owned.iter().collect::<Vec<_>>();
            encoding::encode_arrow_values(&values, projected)
        }
    }
}

fn evidence_utf8_value<'a>(entry: &'a ProjectionEvidenceEntry, key: &str) -> Option<Cow<'a, str>> {
    match entry {
        ProjectionEvidenceEntry::Json(value) => value
            .as_object()
            .and_then(|object| object.get(key))
            .and_then(encoding::json_value_to_output_cow),
        ProjectionEvidenceEntry::Parsed(entry) => match key {
            "source_id" => Some(Cow::Borrowed(entry.source_id.as_str())),
            "source_row_identity" => Some(Cow::Borrowed(entry.source_row_identity.as_str())),
            "rule_id" => Some(Cow::Borrowed(entry.rule_id.as_str())),
            "assertion_id" => Some(Cow::Borrowed(entry.assertion_id.as_str())),
            "output_object_id" => Some(Cow::Borrowed(entry.output_object_id.as_str())),
            "observed_schema_fingerprint" => entry
                .observed_schema_fingerprint
                .as_deref()
                .map(Cow::Borrowed),
            "observed_snapshot_digest" => {
                entry.observed_snapshot_digest.as_deref().map(Cow::Borrowed)
            }
            other => entry
                .operation_metadata
                .get(other)
                .and_then(encoding::json_value_to_output_cow),
        },
    }
}

fn evidence_uuid_value(
    entry: &ProjectionEvidenceEntry,
    key: &str,
) -> Result<Option<[u8; 16]>, String> {
    let Some(text) = evidence_utf8_value(entry, key) else {
        return Ok(None);
    };
    hex_decode_16(text.as_ref()).map(Some)
}

struct ArrowRecordBatchSink {
    mapping_id: String,
    mapping_version: String,
    projection_id: String,
    output_table: String,
    columns: Vec<ProjectedColumn>,
    batch_size: usize,
    max_rows: Option<usize>,
    emitted_rows: usize,
    buffered_rows: Vec<Map<String, Value>>,
    batches: Vec<RecordBatch>,
}

impl ArrowRecordBatchSink {
    fn new(
        mapping_id: &str,
        mapping_version: &str,
        projection: &MapProjectionEntry,
        columns: Vec<ProjectedColumn>,
        batch_size: usize,
        max_rows: Option<usize>,
    ) -> Self {
        Self {
            mapping_id: mapping_id.to_string(),
            mapping_version: mapping_version.to_string(),
            projection_id: projection.projection_id.clone(),
            output_table: projection
                .output_table
                .clone()
                .unwrap_or_else(|| projection.projection_id.clone()),
            columns,
            batch_size,
            max_rows,
            emitted_rows: 0,
            buffered_rows: Vec::new(),
            batches: Vec::new(),
        }
    }

    fn push(&mut self, row: Map<String, Value>) -> Result<bool, String> {
        if self
            .max_rows
            .is_some_and(|max_rows| self.emitted_rows >= max_rows)
        {
            return Ok(false);
        }
        self.buffered_rows.push(row);
        self.emitted_rows += 1;
        if self.buffered_rows.len() >= self.batch_size {
            self.flush()?;
        }
        Ok(self
            .max_rows
            .map(|max_rows| self.emitted_rows < max_rows)
            .unwrap_or(true))
    }

    fn finish(mut self) -> Result<Vec<RecordBatch>, String> {
        if !self.buffered_rows.is_empty() || self.batches.is_empty() {
            self.flush()?;
        }
        Ok(self.batches)
    }

    fn flush(&mut self) -> Result<(), String> {
        let table = ProjectedTable {
            mapping_id: self.mapping_id.clone(),
            mapping_version: self.mapping_version.clone(),
            projection_id: self.projection_id.clone(),
            output_table: self.output_table.clone(),
            temporal_cut: None,
            columns: self.columns.clone(),
            rows: std::mem::take(&mut self.buffered_rows),
        };
        self.batches.push(encoding::arrow_record_batch(&table)?);
        Ok(())
    }
}

fn emit_projection_rows<F>(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
    mut emit: F,
) -> Result<(), String>
where
    F: FnMut(Map<String, Value>) -> Result<bool, String>,
{
    let row_grain = projection
        .row_grain
        .as_deref()
        .ok_or_else(|| "projection row_grain is required".to_string())?;
    match row_grain {
        "one_row_per_object" | "one_row_per_event_object" | "one_row_per_object_as_of_time" => {
            emit_object_projection_rows(model, projection, false, options, &mut emit)
        }
        "one_row_per_association" | "one_row_per_link_object" => {
            emit_object_projection_rows(model, projection, true, options, &mut emit)
        }
        "one_row_per_property_version" => {
            emit_property_version_projection_rows(model, projection, options, &mut emit)
        }
        "one_row_per_evidence_assertion" => {
            emit_evidence_projection_rows(model, projection, options, &mut emit)
        }
        other => Err(format!("unsupported projection row_grain '{other}'")),
    }
}

fn emit_object_projection_rows<F>(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    associations: bool,
    options: &ProjectionBatchOptions,
    emit: &mut F,
) -> Result<(), String>
where
    F: FnMut(Map<String, Value>) -> Result<bool, String>,
{
    let anchor = projection
        .anchor
        .as_ref()
        .ok_or_else(|| "projection anchor is required".to_string())?;
    let rows = model.rows_for_projection(projection)?;
    let mut projection_row_ordinal = 0u64;
    for row in rows.iter() {
        if associations {
            let Some(association_type) = &anchor.association_type else {
                continue;
            };
            if !row_matches_association(row, association_type) {
                continue;
            }
        } else {
            let Some(object_type) = &anchor.object_type else {
                continue;
            };
            if &row.object_type != object_type {
                continue;
            }
        }
        let mut base = Map::new();
        base.insert("projection_id".into(), json!(projection.projection_id));
        if let Some(output_table) = &projection.output_table {
            base.insert("output_table".into(), json!(output_table));
        }
        for projected in project_columns_for_row(model, projection, row, base)? {
            let Value::Object(row) = projected else {
                return Err("projection produced a non-object row".into());
            };
            let row_ordinal = projection_row_ordinal;
            projection_row_ordinal = projection_row_ordinal.saturating_add(1);
            if !candidate_projection_row_allowed(options, row_ordinal) {
                continue;
            }
            if !row_matches_projection_filters(&row, &options.pushed_filters) {
                continue;
            }
            if !emit(projected_table_row(projection, row))? {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn emit_property_version_projection_rows<F>(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
    emit: &mut F,
) -> Result<(), String>
where
    F: FnMut(Map<String, Value>) -> Result<bool, String>,
{
    let rows = model.rows_for_projection(projection)?;
    let mut projection_row_ordinal = 0u64;
    for row in rows.iter() {
        for property in &row.properties {
            let mut out = Map::new();
            out.insert("projection_id".into(), json!(projection.projection_id));
            out.insert("object_goid".into(), json!(hex_encode(&row.goid)));
            out.insert("property_id".into(), json!(property.property_id));
            out.insert("property_name".into(), json!(property.property_name));
            out.insert("value".into(), property.value.clone());
            let row_ordinal = projection_row_ordinal;
            projection_row_ordinal = projection_row_ordinal.saturating_add(1);
            if !candidate_projection_row_allowed(options, row_ordinal) {
                continue;
            }
            if !row_matches_projection_filters(&out, &options.pushed_filters) {
                continue;
            }
            if !emit(projected_table_row(projection, out))? {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn emit_evidence_projection_rows<F>(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
    emit: &mut F,
) -> Result<(), String>
where
    F: FnMut(Map<String, Value>) -> Result<bool, String>,
{
    let mut projection_row_ordinal = 0u64;
    for evidence in &model.evidence_entries {
        let mut out = Map::new();
        out.insert("projection_id".into(), json!(projection.projection_id));
        for column in &projection.columns {
            let key = column
                .value
                .strip_prefix("evidence.")
                .ok_or_else(|| format!("unsupported evidence expression '{}'", column.value))?;
            out.insert(
                column.name.clone(),
                projection_evidence_value(evidence, key),
            );
        }
        let row_ordinal = projection_row_ordinal;
        projection_row_ordinal = projection_row_ordinal.saturating_add(1);
        if !candidate_projection_row_allowed(options, row_ordinal) {
            continue;
        }
        if !row_matches_projection_filters(&out, &options.pushed_filters) {
            continue;
        }
        if !emit(projected_table_row(projection, out))? {
            return Ok(());
        }
    }
    Ok(())
}

fn projected_column_from_entry(column: &MapProjectionColumn) -> Result<ProjectedColumn, String> {
    let logical = projection_column_logical_type(column.logical_type.as_deref().unwrap_or("utf8"))
        .map_err(|err| format!("projection column '{}' declares {err}", column.name))?;
    projected_column(column.name.clone(), logical, column.nested_shape.clone())
}

fn projected_column_from_descriptor(
    column: &ProjectionColumnDescriptor,
) -> Result<ProjectedColumn, String> {
    let logical = projection_column_logical_type(&column.logical_type)
        .map_err(|err| format!("projection column '{}' declares {err}", column.name))?;
    projected_column(column.name.clone(), logical, column.nested_shape.clone())
}

fn projected_column(
    name: String,
    logical: CoveLogicalType,
    nested_shape: Option<String>,
) -> Result<ProjectedColumn, String> {
    if matches!(logical, CoveLogicalType::Null) {
        return Err(format!(
            "projection column '{}' declares null logical type; use a concrete scalar logical_type",
            name
        ));
    }
    if matches!(
        logical,
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map
    ) && nested_shape.is_none()
    {
        return Err(format!(
            "projection column '{}' declares nested logical type {:?} without nested_shape",
            name, logical
        ));
    }
    Ok(ProjectedColumn {
        name,
        logical,
        nested_shape,
    })
}

fn projection_column_logical_type(logical_type: &str) -> Result<CoveLogicalType, String> {
    match logical_type {
        "null" => Ok(CoveLogicalType::Null),
        "bool" | "boolean" => Ok(CoveLogicalType::Bool),
        "int8" => Ok(CoveLogicalType::Int8),
        "int16" => Ok(CoveLogicalType::Int16),
        "int32" => Ok(CoveLogicalType::Int32),
        "int64" | "int" => Ok(CoveLogicalType::Int64),
        "uint8" => Ok(CoveLogicalType::UInt8),
        "uint16" => Ok(CoveLogicalType::UInt16),
        "uint32" => Ok(CoveLogicalType::UInt32),
        "uint64" | "uint" => Ok(CoveLogicalType::UInt64),
        "float32" => Ok(CoveLogicalType::Float32),
        "float64" | "float" => Ok(CoveLogicalType::Float64),
        "decimal64" => Ok(CoveLogicalType::Decimal64),
        "decimal128" | "decimal" => Ok(CoveLogicalType::Decimal128),
        "date_days" | "date32" | "date" => Ok(CoveLogicalType::DateDays),
        "timestamp_micros" | "timestamp_us" => Ok(CoveLogicalType::TimestampMicros),
        "timestamp_nanos" | "timestamp_ns" => Ok(CoveLogicalType::TimestampNanos),
        "utf8" | "string" => Ok(CoveLogicalType::Utf8),
        "binary" => Ok(CoveLogicalType::Binary),
        "uuid" => Ok(CoveLogicalType::Uuid),
        "json" => Ok(CoveLogicalType::Json),
        "list" => Ok(CoveLogicalType::List),
        "struct" => Ok(CoveLogicalType::Struct),
        "map" => Ok(CoveLogicalType::Map),
        other => Err(format!("unsupported logical_type '{other}'")),
    }
}

fn projected_table_row(
    projection: &MapProjectionEntry,
    mut row: Map<String, Value>,
) -> Map<String, Value> {
    let mut ordered = Map::new();
    for column in &projection.columns {
        ordered.insert(
            column.name.clone(),
            row.remove(&column.name).unwrap_or(Value::Null),
        );
    }
    ordered
}

fn ensure_projection_declares_format(
    projection: &MapProjectionEntry,
    format: ProjectionFormat,
) -> Result<(), String> {
    let mode = format.as_str();
    if projection.output_modes.iter().any(|value| value == mode) {
        Ok(())
    } else {
        Err(format!(
            "projection '{}' does not declare executable output mode '{mode}'",
            projection.projection_id
        ))
    }
}

fn encode_projection_output(
    format: ProjectionFormat,
    catalog: &MapProjectionCatalog,
    tables: &[ProjectedTable],
    lineage: Option<&ProjectionLineageContext>,
) -> Result<Vec<u8>, String> {
    match format {
        ProjectionFormat::Json => serde_json::to_vec_pretty(&json!({
            "format": "json",
            "mapping_id": catalog.mapping_id,
            "mapping_version": catalog.mapping_version,
            "rows": tables.iter()
                .flat_map(|table| table.rows.iter().map(|row| {
                    let mut out = Map::new();
                    out.insert("projection_id".into(), json!(table.projection_id));
                    out.insert("output_table".into(), json!(table.output_table));
                    for (key, value) in row {
                        out.insert(key.clone(), value.clone());
                    }
                    Value::Object(out)
                }))
                .collect::<Vec<_>>(),
        }))
        .map_err(|err| format!("cannot encode projection JSON: {err}")),
        ProjectionFormat::CoveO => encode_cove_o_projection(tables),
        ProjectionFormat::Sql => encoding::encode_sql_projection(tables),
        ProjectionFormat::Arrow => {
            let table = single_projection_table(tables, "Arrow")?;
            encoding::encode_arrow_projection(table)
        }
        ProjectionFormat::CoveT => {
            let table = single_projection_table(tables, "COVE-T")?;
            encoding::encode_cove_t_projection(table, lineage)
        }
    }
}

fn single_projection_table<'a>(
    tables: &'a [ProjectedTable],
    label: &str,
) -> Result<&'a ProjectedTable, String> {
    match tables {
        [table] => Ok(table),
        _ => Err(format!(
            "{label} projection output requires exactly one projection"
        )),
    }
}

fn encode_cove_o_projection(tables: &[ProjectedTable]) -> Result<Vec<u8>, String> {
    if tables.is_empty() {
        return Err("COVE-O projection output requires at least one projection".into());
    }
    let (mapping_id, mapping_version) = (&tables[0].mapping_id, &tables[0].mapping_version);
    let mut sources = Vec::new();
    let mut identity_rules = Vec::new();
    let mut row_rules = Vec::new();
    let mut rows = Vec::new();
    let mut states = Vec::new();

    for table in tables {
        let source_id = projection_source_id(table);
        sources.push(json!({
            "source_id": source_id,
            "schema_fingerprint": projection_schema_fingerprint(table),
            "snapshot_digest": projection_snapshot_digest(table),
            "row_identity_rules": [projection_identity_rule_id(table)],
            "replay_claimed": true
        }));
        identity_rules.push(json!({
            "rule_id": projection_identity_rule_id(table),
            "object_type": table.output_table,
            "semantic_role": "projection_row",
            "confidence_class": "synthetic",
            "candidate_only": false,
            "property_conflicts_declared": true,
            "function_ids": ["identity"],
            "join_keys": [{
                "role_id": "projection_row",
                "source_column": "__projection_key",
                "logical_type": "utf8",
                "canonicalization": "identity",
                "null_policy": "reject",
                "ordering": "asc"
            }]
        }));
        row_rules.push(json!({
            "rule_id": projection_row_rule_id(table),
            "source_id": source_id,
            "identity_rule_id": projection_identity_rule_id(table),
            "row_semantics_kind": "Object",
            "source_operation_kind": "Upsert",
            "assertion_kinds": ["object", "property", "evidence"],
            "property_bindings": table.columns.iter().map(|column| json!({
                "assertion_id": format!("assert_{}_{}", table.projection_id, column.name),
                "property_id": column.name,
                "property_name": column.name,
                "source_column": column.name,
                "logical_type": cove_o_projection_logical_name(column),
                "physical_kind": "auto",
                "nullable": true,
                "missing_policy": "null",
                "conflict_policy": "reject_conflict"
            })).collect::<Vec<_>>()
        }));

        for (ordinal, row) in table.rows.iter().enumerate() {
            let mut values = row.clone().into_iter().collect::<BTreeMap<_, _>>();
            values.insert(
                "__projection_key".into(),
                json!(format!(
                    "{}:{}:{}:{}:{}",
                    table.mapping_id,
                    table.mapping_version,
                    table.projection_id,
                    table.output_table,
                    ordinal
                )),
            );
            rows.push(SourceRow {
                source_id: source_id.clone(),
                row_index: ordinal,
                values,
            });
        }
        states.push(ObservedSourceState {
            source_id,
            source_kind: "cove-map-projection".into(),
            schema_fingerprint: projection_schema_fingerprint(table),
            snapshot_digest: projection_snapshot_digest(table),
        });
    }

    let file = CovemapFile {
        header: CovemapHeaderV1::new(first_16(&sha256_array(mapping_id.as_bytes())), 0),
        mapping_version: mapping_version.clone(),
        sections: vec![
            projection_covemap_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "sources": sources
                }),
            )?,
            projection_covemap_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "functions": [{
                        "function_id": "identity",
                        "version": "1.0.0",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            )?,
            projection_covemap_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "identity_rules": identity_rules,
                    "do_not_merge": []
                }),
            )?,
            projection_covemap_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "rules": row_rules
                }),
            )?,
            projection_covemap_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "projections": tables.iter().map(projected_table_catalog_entry).collect::<Vec<_>>()
                }),
            )?,
        ],
        postscript: CovemapPostscriptV1 {
            required_features: 0,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    };
    build_cove_o_with_source_states(&file, &rows, &states)
}

fn projection_source_id(table: &ProjectedTable) -> String {
    format!("projection.{}", table.projection_id)
}

fn projection_identity_rule_id(table: &ProjectedTable) -> String {
    format!("identity_{}", table.projection_id)
}

fn projection_row_rule_id(table: &ProjectedTable) -> String {
    format!("materialize_{}", table.projection_id)
}

fn projection_schema_fingerprint(table: &ProjectedTable) -> String {
    format!(
        "cove-map-projection-schema-v1:{}",
        sha256_hex(
            serde_json::to_string(&projected_table_catalog_entry(table))
                .unwrap_or_default()
                .as_bytes()
        )
    )
}

fn projection_snapshot_digest(table: &ProjectedTable) -> String {
    let rows = table
        .rows
        .iter()
        .cloned()
        .map(Value::Object)
        .collect::<Vec<_>>();
    format!(
        "sha256:{}",
        sha256_hex(serde_json::to_string(&rows).unwrap_or_default().as_bytes())
    )
}

fn projected_table_catalog_entry(table: &ProjectedTable) -> Value {
    json!({
        "projection_id": table.projection_id,
        "output_table": table.output_table,
        "row_grain": "one_row_per_object",
        "multi_value_policy": "reject",
        "columns": table.columns.iter().map(|column| {
            let mut value = json!({
                "name": column.name,
                "value": format!("property.{}", column.name),
                "logical_type": projection_logical_type_name(column.logical),
                "missing_policy": "null"
            });
            if let (Some(object), Some(shape)) = (value.as_object_mut(), &column.nested_shape) {
                object.insert("nested_shape".into(), json!(shape));
            }
            value
        }).collect::<Vec<_>>(),
        "output_modes": ["cove-o", "json"]
    })
}

fn projection_covemap_section(
    kind: SectionKind,
    mut value: Value,
) -> Result<CovemapSection, String> {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".to_string(),
            Value::String("org.coveformat.covemap.v2".to_string()),
        );
        object.insert(
            "section_id".to_string(),
            Value::Number((kind as u16).into()),
        );
    }
    let payload = serde_json::to_vec_pretty(&value)
        .map_err(|err| format!("cannot encode synthetic projection COVE-MAP section: {err}"))?;
    Ok(CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: 0,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    })
}

fn cove_o_projection_logical_name(column: &ProjectedColumn) -> &'static str {
    projection_logical_type_name(column.logical)
}

fn projection_logical_type_name(logical: CoveLogicalType) -> &'static str {
    cove_core::types::logical_type_name(logical)
}

#[derive(Debug, Clone)]
struct ProjectionModel {
    rows: Vec<ProjectionRow>,
    reconstructed_rows: Vec<ProjectionRow>,
    evidence_entries: Vec<ProjectionEvidenceEntry>,
}

#[derive(Debug, Clone)]
struct ProjectionRow {
    object_type_id: u32,
    object_type: String,
    object_type_flags: u32,
    goid: [u8; 16],
    record_id: [u8; 16],
    branch_key: u64,
    record_kind: RecordKind,
    timestamp_us: i64,
    csn: u64,
    segment_id: u32,
    row_index: u32,
    prev_ref: Option<CoveRecordRefV1>,
    properties: Vec<ProjectionProperty>,
}

#[derive(Debug, Clone)]
struct ProjectionProperty {
    property_id: u32,
    property_name: String,
    flags: u32,
    value: Value,
}

#[derive(Debug, Clone)]
enum ProjectionEvidenceEntry {
    Json(Value),
    Parsed(MapEvidenceEntry),
}

impl ProjectionModel {
    fn from_materialized(materialized: &MaterializedModel) -> Self {
        let type_flags = materialized
            .object_types
            .iter()
            .map(|ty| (ty.object_type_id, ty.flags))
            .collect::<BTreeMap<_, _>>();
        let mut rows = materialized
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| ProjectionRow {
                object_type_id: row.object_type_id,
                object_type: row.object_type.clone(),
                object_type_flags: type_flags
                    .get(&row.object_type_id)
                    .copied()
                    .unwrap_or_default(),
                goid: row.goid,
                record_id: row.record_id,
                branch_key: 0,
                record_kind: row.record_kind,
                timestamp_us: 0,
                csn: index as u64,
                segment_id: 0,
                row_index: index as u32,
                prev_ref: None,
                properties: row
                    .properties
                    .values()
                    .map(|property| ProjectionProperty {
                        property_id: property.entry.property_id,
                        property_name: property.entry.property_name.clone(),
                        flags: property.entry.flags,
                        value: property.value.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(temporal_sort_key);
        let reconstructed_rows = reconstruct_projection_rows_at_cut(&rows, |_| true)
            .expect("materialized projection rows should not contain prev_ref chains");
        Self {
            rows,
            reconstructed_rows,
            evidence_entries: materialized
                .evidence_entries
                .iter()
                .cloned()
                .map(ProjectionEvidenceEntry::Json)
                .collect(),
        }
    }

    fn from_surface_with_access_plan(
        surface: &CoveObjectSurface,
        access_plan: &ProjectionAccessPlan,
    ) -> Result<Self, cove_core::CoveError> {
        let mut rows = if access_plan.needs_history_rows || access_plan.needs_reconstructed_rows {
            surface
                .records
                .iter()
                .map(row_from_surface_record)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        rows.sort_by_key(temporal_sort_key);
        let mut reconstructed_rows = if access_plan.needs_reconstructed_rows {
            reconstruct_projection_rows_at_cut(&rows, |_| true)
                .map_err(cove_core::CoveError::BadSchema)?
        } else {
            Vec::new()
        };
        reconstructed_rows.sort_by_key(temporal_sort_key);
        if !access_plan.needs_history_rows {
            rows.clear();
        }
        let evidence_entries = if access_plan.include_evidence_index {
            surface
                .evidence_index
                .as_ref()
                .map(|index| {
                    index
                        .entries
                        .iter()
                        .cloned()
                        .map(ProjectionEvidenceEntry::Parsed)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self {
            rows,
            reconstructed_rows,
            evidence_entries,
        })
    }

    fn rows_for_projection(
        &self,
        projection: &MapProjectionEntry,
    ) -> Result<Cow<'_, [ProjectionRow]>, String> {
        match parse_projection_temporal_mode(
            projection
                .temporal_mode
                .as_deref()
                .unwrap_or("latest_committed"),
        )
        .ok_or_else(|| {
            format!(
                "projection '{}' uses unsupported temporal_mode '{}'",
                projection.projection_id,
                projection.temporal_mode.as_deref().unwrap_or_default()
            )
        })? {
            ProjectionTemporalMode::LatestCommitted => Ok(Cow::Borrowed(&self.reconstructed_rows)),
            ProjectionTemporalMode::FullHistory | ProjectionTemporalMode::CommitOrder => {
                Ok(Cow::Borrowed(&self.rows))
            }
            ProjectionTemporalMode::ValidTime => {
                ensure_temporal_surface_fields(
                    &self.reconstructed_rows,
                    PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
                    "valid_from",
                    "valid_time",
                )?;
                Ok(Cow::Borrowed(&self.reconstructed_rows))
            }
            ProjectionTemporalMode::ObservedTime => {
                ensure_temporal_surface_fields(
                    &self.reconstructed_rows,
                    PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT,
                    "observed_at",
                    "observed_time",
                )?;
                Ok(Cow::Borrowed(&self.reconstructed_rows))
            }
            ProjectionTemporalMode::AsOfTimestamp(timestamp_us) => Ok(Cow::Owned(
                reconstruct_projection_rows_at_cut(&self.rows, |row| {
                    row.timestamp_us <= timestamp_us
                })?,
            )),
            ProjectionTemporalMode::AsOfCsn(csn) => Ok(Cow::Owned(
                reconstruct_projection_rows_at_cut(&self.rows, |row| row.csn <= csn)?,
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionTemporalMode {
    LatestCommitted,
    FullHistory,
    CommitOrder,
    ValidTime,
    ObservedTime,
    AsOfTimestamp(i64),
    AsOfCsn(u64),
}

fn parse_projection_temporal_mode(value: &str) -> Option<ProjectionTemporalMode> {
    match value {
        "latest_committed" => Some(ProjectionTemporalMode::LatestCommitted),
        "full_history" => Some(ProjectionTemporalMode::FullHistory),
        "commit_order" => Some(ProjectionTemporalMode::CommitOrder),
        "valid_time" => Some(ProjectionTemporalMode::ValidTime),
        "observed_time" => Some(ProjectionTemporalMode::ObservedTime),
        _ => parse_temporal_cut_value(value),
    }
}

fn parse_temporal_cut_value(value: &str) -> Option<ProjectionTemporalMode> {
    for prefix in [
        "as_of_timestamp_us:",
        "as_of_timestamp_us=",
        "timestamp_us:",
        "timestamp_us=",
        "as_of_time:",
        "as_of_time=",
    ] {
        if let Some(raw) = value.strip_prefix(prefix) {
            return raw.parse().ok().map(ProjectionTemporalMode::AsOfTimestamp);
        }
    }
    for prefix in ["as_of_csn:", "as_of_csn=", "csn:", "csn="] {
        if let Some(raw) = value.strip_prefix(prefix) {
            return raw.parse().ok().map(ProjectionTemporalMode::AsOfCsn);
        }
    }
    None
}

fn ensure_temporal_surface_fields(
    rows: &[ProjectionRow],
    flag: u32,
    name: &str,
    mode: &str,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    if rows.iter().any(|row| {
        row.properties
            .iter()
            .any(|property| property.flags & flag != 0 || property.property_name == name)
    }) {
        return Ok(());
    }
    Err(format!(
        "temporal_mode '{mode}' requires declared '{name}' fields on the projected surface"
    ))
}

fn reconstruct_projection_rows_at_cut(
    rows: &[ProjectionRow],
    visible: impl Fn(&ProjectionRow) -> bool,
) -> Result<Vec<ProjectionRow>, String> {
    let mut current_by_object = BTreeMap::<(u32, u64, [u8; 16]), ProjectionRow>::new();
    for row in rows.iter().filter(|row| visible(row)) {
        let key = (row.object_type_id, row.branch_key, row.goid);
        if let Some(prev_ref) = row.prev_ref {
            let Some(current) = current_by_object.get(&key) else {
                return Err(format!(
                    "projection reconstruction encountered missing prev_ref target {}:{}",
                    prev_ref.segment_id, prev_ref.row_index
                ));
            };
            if current.segment_id != prev_ref.segment_id || current.row_index != prev_ref.row_index
            {
                return Err(format!(
                    "projection reconstruction encountered mismatched prev_ref target {}:{}",
                    prev_ref.segment_id, prev_ref.row_index
                ));
            }
        }
        match row.record_kind {
            RecordKind::Baseline | RecordKind::Snapshot => {
                current_by_object.insert(key, row.clone());
            }
            RecordKind::Delta => match current_by_object.get_mut(&key) {
                Some(state) => apply_projection_delta(state, row),
                None => {
                    current_by_object.insert(key, row.clone());
                }
            },
            RecordKind::Tombstone => {
                current_by_object.remove(&key);
            }
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    let mut out = current_by_object.into_values().collect::<Vec<_>>();
    out.sort_by_key(temporal_sort_key);
    Ok(out)
}

fn apply_projection_delta(state: &mut ProjectionRow, delta: &ProjectionRow) {
    state.record_id = delta.record_id;
    state.record_kind = delta.record_kind;
    state.timestamp_us = delta.timestamp_us;
    state.csn = delta.csn;
    state.segment_id = delta.segment_id;
    state.row_index = delta.row_index;
    for property in &delta.properties {
        match state
            .properties
            .iter_mut()
            .find(|existing| existing.property_id == property.property_id)
        {
            Some(existing) => *existing = property.clone(),
            None => state.properties.push(property.clone()),
        }
    }
}

fn row_from_surface_record(record: &CoveObjectRecord) -> ProjectionRow {
    ProjectionRow {
        object_type_id: record.object_type_id,
        object_type: record.object_type_name.clone(),
        object_type_flags: record.object_type_flags,
        goid: record.goid,
        record_id: record.record_id,
        branch_key: record.branch_key,
        record_kind: record.record_kind,
        timestamp_us: record.timestamp_us,
        csn: record.csn,
        segment_id: record.segment_id,
        row_index: record.row_index,
        prev_ref: record.prev_ref,
        properties: record
            .properties
            .iter()
            .map(|property| ProjectionProperty {
                property_id: property.property_id,
                property_name: property.property_name.clone(),
                flags: property.flags,
                value: property.value.clone(),
            })
            .collect(),
    }
}

fn projection_evidence_value(entry: &ProjectionEvidenceEntry, key: &str) -> Value {
    match entry {
        ProjectionEvidenceEntry::Json(value) => value
            .as_object()
            .and_then(|object| object.get(key))
            .cloned()
            .unwrap_or(Value::Null),
        ProjectionEvidenceEntry::Parsed(entry) => match key {
            "source_id" => Value::String(entry.source_id.clone()),
            "source_row_identity" => Value::String(entry.source_row_identity.clone()),
            "rule_id" => Value::String(entry.rule_id.clone()),
            "assertion_id" => Value::String(entry.assertion_id.clone()),
            "output_object_id" => Value::String(entry.output_object_id.clone()),
            "observed_schema_fingerprint" => entry
                .observed_schema_fingerprint
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            "observed_snapshot_digest" => entry
                .observed_snapshot_digest
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            other => entry
                .operation_metadata
                .get(other)
                .cloned()
                .unwrap_or(Value::Null),
        },
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolutionProjectionExpression<'a> {
    identity_rule_id: &'a str,
    role_id: &'a str,
    field: &'a str,
}

fn parse_resolution_expression(expression: &str) -> Option<ResolutionProjectionExpression<'_>> {
    let expression = expression.trim();
    let rest = expression.strip_prefix("identity(")?;
    let (identity_rule_id, rest) = rest.split_once(").resolution(")?;
    let (role_id, field) = rest.split_once(").")?;
    let identity_rule_id = identity_rule_id.trim();
    let role_id = role_id.trim();
    let field = field.trim();
    if identity_rule_id.is_empty() || role_id.is_empty() || field.is_empty() {
        return None;
    }
    Some(ResolutionProjectionExpression {
        identity_rule_id,
        role_id,
        field,
    })
}

fn resolution_expression_field_allowed(field: &str) -> bool {
    matches!(
        field,
        "canonical_key"
            | "canonical_label"
            | "normalized_value"
            | "raw_observed_value"
            | "resolved_identity_value"
    )
}

fn resolution_expression_value(
    model: &ProjectionModel,
    row: &ProjectionRow,
    expression: ResolutionProjectionExpression<'_>,
    expression_text: &str,
) -> Result<Value, String> {
    if !resolution_expression_field_allowed(expression.field) {
        return Err(format!(
            "unsupported resolution expression field '{}'",
            expression.field
        ));
    }
    let row_goid = hex_encode(&row.goid);
    let mut values = Vec::new();
    for entry in &model.evidence_entries {
        if projection_evidence_value(entry, "output_object_id").as_str() != Some(row_goid.as_str())
            || projection_evidence_value(entry, "identity_rule_id").as_str()
                != Some(expression.identity_rule_id)
        {
            continue;
        }

        let metadata = projection_evidence_value(entry, "resolution_metadata");
        if let Some(items) = metadata.as_array() {
            for item in items {
                if item
                    .as_object()
                    .and_then(|object| object.get("resolution_role_id"))
                    .and_then(Value::as_str)
                    != Some(expression.role_id)
                    || item
                        .as_object()
                        .and_then(|object| object.get("alias_hit"))
                        .and_then(Value::as_bool)
                        != Some(true)
                {
                    continue;
                }
                let value = item
                    .as_object()
                    .and_then(|object| object.get(expression.field))
                    .cloned()
                    .unwrap_or(Value::Null);
                push_unique_resolution_expression_value(&mut values, value);
            }
        } else if projection_evidence_value(entry, "resolution_role_id").as_str()
            == Some(expression.role_id)
            && projection_evidence_value(entry, "alias_hit").as_bool() == Some(true)
        {
            let value = projection_evidence_value(entry, expression.field);
            push_unique_resolution_expression_value(&mut values, value);
        }
    }
    match values.len() {
        0 => Err(format!(
            "resolution expression '{expression_text}' found no resolver hit"
        )),
        1 => Ok(values.remove(0)),
        _ => Ok(Value::Array(values)),
    }
}

fn push_unique_resolution_expression_value(values: &mut Vec<Value>, value: Value) {
    if !value.is_null() && !values.contains(&value) {
        values.push(value);
    }
}

fn temporal_sort_key(row: &ProjectionRow) -> (i64, u64, u32, u32, [u8; 16]) {
    (
        row.timestamp_us,
        row.csn,
        row.segment_id,
        row.row_index,
        row.record_id,
    )
}

fn validate_executable_projection(
    projection: &MapProjectionEntry,
    model: &ProjectionModel,
    function_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    if projection.output_table.is_none()
        || projection.row_grain.is_none()
        || projection.anchor.is_none()
        || projection.temporal_mode.is_none()
        || projection.multi_value_policy.is_none()
        || projection.columns.is_empty()
        || projection.output_modes.is_empty()
    {
        return Err(format!(
            "projection '{}' uses the legacy preview schema; add output_table, row_grain, anchor, temporal_mode, multi_value_policy, columns, and output_modes",
            projection.projection_id
        ));
    }
    let temporal_mode = projection.temporal_mode.as_deref().unwrap_or_default();
    if parse_projection_temporal_mode(temporal_mode).is_none() {
        return Err(format!(
            "projection '{}' uses unsupported temporal_mode '{temporal_mode}'",
            projection.projection_id
        ));
    }
    let policy = projection.multi_value_policy.as_deref().unwrap_or_default();
    let row_grain = projection.row_grain.as_deref().unwrap_or_default();
    match policy {
        "first" | "last" if projection.ordering.is_empty() => {
            return Err(format!(
                "projection '{}' multi_value_policy '{policy}' requires explicit ordering",
                projection.projection_id
            ));
        }
        "reject" | "explode" | "aggregate" | "first" | "last" | "list" => {}
        _ => {
            return Err(format!(
                "projection '{}' uses unsupported multi_value_policy '{policy}' for row_grain '{row_grain}'",
                projection.projection_id
            ));
        }
    }
    for column in &projection.columns {
        validate_projection_expression(model, projection, function_ids, &column.value)?;
    }
    for ordering in &projection.ordering {
        let expression = ordering_expression(ordering);
        if expression != "value" {
            validate_projection_expression(model, projection, function_ids, expression)?;
        }
    }
    Ok(())
}

fn validate_projection_expression(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    function_ids: &std::collections::BTreeSet<String>,
    expression: &str,
) -> Result<(), String> {
    let expression = expression.trim();
    if expression.is_empty()
        || literal_value(expression).is_some()
        || known_projection_path(expression)
    {
        return Ok(());
    }
    if expression.starts_with("evidence.") {
        return Ok(());
    }
    if let Some(resolution) = parse_resolution_expression(expression) {
        if resolution_expression_field_allowed(resolution.field) {
            return Ok(());
        }
        return Err(format!(
            "unsupported resolution expression field '{}'",
            resolution.field
        ));
    }
    if let Some(traversal) = parse_association_traversal(expression) {
        let association_type = traversal.association_type;
        let property_name = traversal.property_name;
        if association_type.is_empty() || property_name.is_empty() {
            return Err(format!("invalid association traversal '{expression}'"));
        }
        return Ok(());
    }
    if let Some((left, right)) = split_comparison_expression(expression) {
        validate_projection_expression(model, projection, function_ids, left)?;
        validate_projection_expression(model, projection, function_ids, right)?;
        return Ok(());
    }
    if let Some((function, args)) = parse_function_call(expression) {
        if !projection_builtin_operator(function) && !function_ids.contains(function) {
            return Err(format!("undeclared projection function '{function}'"));
        }
        if !runtime_projection_function(function) {
            if function_ids.contains(function) {
                return Err(format!(
                    "projection function '{function}' is declared but has no reference executor"
                ));
            }
            return Err(format!("undeclared projection function '{function}'"));
        }
        if matches!(
            function,
            "if" | "ifelse"
                | "count"
                | "min"
                | "max"
                | "sum"
                | "avg"
                | "distinct_count"
                | "list"
                | "identity"
                | "trim"
                | "lower"
                | "lowercase"
                | "upper"
                | "uppercase"
                | "exists"
                | "coalesce"
                | "association"
        ) {
            if projection_aggregate_operator(function) {
                validate_projection_aggregate_policy(projection, function)?;
            }
            if function == "association" {
                if args.len() != 1 || args[0].trim().is_empty() {
                    return Err("projection function 'association' expects one argument".into());
                }
                return Ok(());
            }
            for arg in args {
                validate_projection_expression(model, projection, function_ids, &arg)?;
            }
            return Ok(());
        }
    }
    validate_projection_path(model, projection, expression)
}

fn validate_projection_aggregate_policy(
    projection: &MapProjectionEntry,
    function: &str,
) -> Result<(), String> {
    if projection.multi_value_policy.as_deref() != Some("aggregate") {
        return Err(format!(
            "projection '{}' aggregate '{function}' requires multi_value_policy='aggregate'",
            projection.projection_id
        ));
    }
    if projection.temporal_mode.is_none() {
        return Err(format!(
            "projection '{}' aggregate '{function}' requires temporal_mode",
            projection.projection_id
        ));
    }
    if projection.missing_policy.trim().is_empty() {
        return Err(format!(
            "projection '{}' aggregate '{function}' requires missing_policy",
            projection.projection_id
        ));
    }
    Ok(())
}

fn runtime_projection_function(function: &str) -> bool {
    matches!(
        function,
        "if" | "ifelse"
            | "count"
            | "min"
            | "max"
            | "sum"
            | "avg"
            | "distinct_count"
            | "list"
            | "identity"
            | "trim"
            | "lower"
            | "lowercase"
            | "upper"
            | "uppercase"
            | "exists"
            | "coalesce"
            | "association"
    )
}

fn projection_aggregate_operator(function: &str) -> bool {
    matches!(
        function,
        "count" | "min" | "max" | "sum" | "avg" | "distinct_count" | "list"
    )
}

fn projection_builtin_operator(function: &str) -> bool {
    matches!(
        function,
        "if" | "ifelse"
            | "count"
            | "min"
            | "max"
            | "sum"
            | "avg"
            | "distinct_count"
            | "list"
            | "exists"
            | "association"
    )
}

fn split_comparison_expression(expression: &str) -> Option<(&str, &str)> {
    let bytes = expression.as_bytes();
    let mut depth = 0u32;
    let mut quote: Option<u8> = None;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b'=' | b'!' | b'>' | b'<' if depth == 0 => {
                let op_len = if bytes.get(index + 1) == Some(&b'=') {
                    2
                } else if matches!(byte, b'>' | b'<') {
                    1
                } else {
                    index += 1;
                    continue;
                };
                let left = expression[..index].trim();
                let right = expression[index + op_len..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some((left, right));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn known_projection_path(expression: &str) -> bool {
    matches!(
        expression,
        "goid"
            | "object.goid"
            | "Object.goid"
            | "association.goid"
            | "record.id"
            | "record.record_id"
            | "record.kind"
            | "object.type_id"
            | "object_type_id"
            | "temporal.timestamp_us"
            | "timestamp_us"
            | "temporal.csn"
            | "csn"
            | "temporal.branch_key"
            | "branch_key"
            | "object_type"
            | "object.type"
            | "Object.type"
            | "association.source_goid"
            | "association.target_goid"
            | "association.association_type"
            | "association.mapping_rule_id"
            | "association.source_evidence_id"
            | "association.source_role"
            | "association.target_role"
            | "association.valid_from"
            | "association.valid_to"
            | "association.observed_at"
            | "association.cardinality_policy"
    )
}

fn validate_projection_path(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    expression: &str,
) -> Result<(), String> {
    let property_name = expression
        .rsplit('.')
        .next()
        .ok_or_else(|| format!("unsupported projection expression '{expression}'"))?;
    if property_name.is_empty() {
        return Err(format!("unsupported projection expression '{expression}'"));
    }
    let Some(anchor) = &projection.anchor else {
        return Ok(());
    };
    let matching = model
        .rows
        .iter()
        .chain(model.reconstructed_rows.iter())
        .filter(|row| {
            anchor
                .object_type
                .as_ref()
                .map(|object_type| &row.object_type == object_type)
                .unwrap_or_else(|| {
                    anchor
                        .association_type
                        .as_ref()
                        .map(|association_type| row_matches_association(row, association_type))
                        .unwrap_or(true)
                })
        })
        .collect::<Vec<_>>();
    if matching.is_empty()
        || matching.iter().any(|row| {
            row.properties
                .iter()
                .any(|property| property.property_name == property_name)
        })
    {
        Ok(())
    } else {
        Err(format!(
            "projection '{}' references undeclared path '{expression}'",
            projection.projection_id
        ))
    }
}

fn ordering_expression(ordering: &str) -> &str {
    let value = ordering.trim();
    let value = value.strip_prefix('-').unwrap_or(value).trim();
    for suffix in [" desc", " asc", ":desc", ":asc"] {
        if let Some(stripped) = value.strip_suffix(suffix) {
            return stripped.trim();
        }
    }
    value
}

fn ordering_descending(ordering: &str) -> bool {
    let value = ordering.trim();
    value.starts_with('-') || value.ends_with(" desc") || value.ends_with(":desc")
}

fn sort_projection_rows_by_ordering(
    rows: &mut [ProjectionRow],
    projection: &MapProjectionEntry,
    anchor_row: Option<&ProjectionRow>,
) -> Result<(), String> {
    for ordering in projection.ordering.iter().rev() {
        let expression = ordering_expression(ordering);
        if expression == "value" {
            continue;
        }
        let descending = ordering_descending(ordering);
        rows.sort_by(|left, right| {
            let left_value = ordering_value(left, expression, anchor_row);
            let right_value = ordering_value(right, expression, anchor_row);
            let ordering = compare_json_order(&left_value, &right_value);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    Ok(())
}

fn ordering_value(
    row: &ProjectionRow,
    expression: &str,
    _anchor_row: Option<&ProjectionRow>,
) -> Value {
    match expression {
        "temporal.timestamp_us" | "timestamp_us" => json!(row.timestamp_us),
        "temporal.csn" | "csn" => json!(row.csn),
        "temporal.branch_key" | "branch_key" => json!(row.branch_key),
        "record.id" | "record.record_id" => json!(hex_encode(&row.record_id)),
        "record.kind" => json!(record_kind_name(row.record_kind)),
        "segment_id" => json!(row.segment_id),
        "row_index" => json!(row.row_index),
        "goid" | "object.goid" | "association.goid" => json!(hex_encode(&row.goid)),
        other => {
            let property_name = other.rsplit('.').next().unwrap_or(other);
            projection_property_by_name(row, property_name)
        }
    }
}

fn project_one(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
) -> Result<Vec<Value>, String> {
    let row_grain = projection
        .row_grain
        .as_deref()
        .ok_or_else(|| "projection row_grain is required".to_string())?;
    match row_grain {
        "one_row_per_object" => project_object_rows(model, projection, false, options),
        "one_row_per_event_object" | "one_row_per_object_as_of_time" => {
            project_object_rows(model, projection, false, options)
        }
        "one_row_per_association" | "one_row_per_link_object" => {
            project_object_rows(model, projection, true, options)
        }
        "one_row_per_property_version" => project_property_versions(model, projection, options),
        "one_row_per_evidence_assertion" => project_evidence_rows(model, projection, options),
        other => Err(format!("unsupported projection row_grain '{other}'")),
    }
}

fn project_object_rows(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    associations: bool,
    options: &ProjectionBatchOptions,
) -> Result<Vec<Value>, String> {
    let anchor = projection
        .anchor
        .as_ref()
        .ok_or_else(|| "projection anchor is required".to_string())?;
    let mut rows = Vec::new();
    let projection_rows = model.rows_for_projection(projection)?;
    let mut projection_row_ordinal = 0u64;
    for row in projection_rows.iter() {
        if associations {
            let Some(association_type) = &anchor.association_type else {
                continue;
            };
            if !row_matches_association(row, association_type) {
                continue;
            }
        } else {
            let Some(object_type) = &anchor.object_type else {
                continue;
            };
            if &row.object_type != object_type {
                continue;
            }
        }
        let mut out = Map::new();
        out.insert("projection_id".into(), json!(projection.projection_id));
        if let Some(output_table) = &projection.output_table {
            out.insert("output_table".into(), json!(output_table));
        }
        for projected in project_columns_for_row(model, projection, row, out)? {
            let Value::Object(projected_row) = projected else {
                return Err("projection produced a non-object row".into());
            };
            let row_ordinal = projection_row_ordinal;
            projection_row_ordinal = projection_row_ordinal.saturating_add(1);
            if !candidate_projection_row_allowed(options, row_ordinal) {
                continue;
            }
            if !row_matches_projection_filters(&projected_row, &options.pushed_filters) {
                continue;
            }
            rows.push(Value::Object(projected_row));
            if reached_projection_limit(&mut rows, options.max_rows) {
                return Ok(rows);
            }
        }
    }
    Ok(rows)
}

fn project_columns_for_row(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    base: Map<String, Value>,
) -> Result<Vec<Value>, String> {
    let mut rows = vec![base];
    for column in &projection.columns {
        let value = projection_value(model, projection, row, &column.value)?;
        rows = apply_multi_value_policy(rows, &column.name, value, projection)?;
    }
    Ok(rows.into_iter().map(Value::Object).collect())
}

fn apply_multi_value_policy(
    rows: Vec<Map<String, Value>>,
    column_name: &str,
    value: Value,
    projection: &MapProjectionEntry,
) -> Result<Vec<Map<String, Value>>, String> {
    let policy = projection.multi_value_policy.as_deref().unwrap_or("reject");
    let Some(values) = value.as_array() else {
        return Ok(rows
            .into_iter()
            .map(|mut row| {
                row.insert(column_name.to_string(), value.clone());
                row
            })
            .collect());
    };
    let values = ordered_multi_values(values, projection);
    match policy {
        "explode" => {
            if values.is_empty() {
                return Ok(rows
                    .into_iter()
                    .map(|mut row| {
                        row.insert(column_name.to_string(), Value::Null);
                        row
                    })
                    .collect());
            }
            let mut out = Vec::with_capacity(rows.len() * values.len());
            for row in rows {
                for value in &values {
                    let mut row = row.clone();
                    row.insert(column_name.to_string(), value.clone());
                    out.push(row);
                }
            }
            Ok(out)
        }
        "list" | "aggregate" => Ok(rows
            .into_iter()
            .map(|mut row| {
                row.insert(column_name.to_string(), Value::Array(values.clone()));
                row
            })
            .collect()),
        "first" => {
            let selected = values.first().cloned().unwrap_or(Value::Null);
            Ok(rows
                .into_iter()
                .map(|mut row| {
                    row.insert(column_name.to_string(), selected.clone());
                    row
                })
                .collect())
        }
        "last" => {
            let selected = values.last().cloned().unwrap_or(Value::Null);
            Ok(rows
                .into_iter()
                .map(|mut row| {
                    row.insert(column_name.to_string(), selected.clone());
                    row
                })
                .collect())
        }
        "reject" if values.len() <= 1 => {
            let selected = values.first().cloned().unwrap_or(Value::Null);
            Ok(rows
                .into_iter()
                .map(|mut row| {
                    row.insert(column_name.to_string(), selected.clone());
                    row
                })
                .collect())
        }
        "reject" => Err(format!(
            "projection '{}' column '{column_name}' produced {} values with multi_value_policy='reject'",
            projection.projection_id,
            values.len()
        )),
        other => Err(format!(
            "projection '{}' uses unsupported multi_value_policy '{other}'",
            projection.projection_id
        )),
    }
}

fn ordered_multi_values(values: &[Value], projection: &MapProjectionEntry) -> Vec<Value> {
    let mut out = values.to_vec();
    for ordering in projection.ordering.iter().rev() {
        if ordering_expression(ordering) != "value" {
            continue;
        }
        let descending = ordering_descending(ordering);
        out.sort_by(|left, right| {
            let ordering = compare_json_order(left, right);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    out
}

fn project_property_versions(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    let projection_rows = model.rows_for_projection(projection)?;
    let mut projection_row_ordinal = 0u64;
    for row in projection_rows.iter() {
        for property in &row.properties {
            let mut out = Map::new();
            out.insert("projection_id".into(), json!(projection.projection_id));
            out.insert("object_goid".into(), json!(hex_encode(&row.goid)));
            out.insert("property_id".into(), json!(property.property_id));
            out.insert("property_name".into(), json!(property.property_name));
            out.insert("value".into(), property.value.clone());
            let row_ordinal = projection_row_ordinal;
            projection_row_ordinal = projection_row_ordinal.saturating_add(1);
            if !candidate_projection_row_allowed(options, row_ordinal) {
                continue;
            }
            if !row_matches_projection_filters(&out, &options.pushed_filters) {
                continue;
            }
            rows.push(Value::Object(out));
            if reached_projection_limit(&mut rows, options.max_rows) {
                return Ok(rows);
            }
        }
    }
    Ok(rows)
}

fn project_evidence_rows(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    options: &ProjectionBatchOptions,
) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    let mut projection_row_ordinal = 0u64;
    for evidence in &model.evidence_entries {
        let mut out = Map::new();
        out.insert("projection_id".into(), json!(projection.projection_id));
        for column in &projection.columns {
            let key = column
                .value
                .strip_prefix("evidence.")
                .ok_or_else(|| format!("unsupported evidence expression '{}'", column.value))?;
            out.insert(
                column.name.clone(),
                projection_evidence_value(evidence, key),
            );
        }
        let row_ordinal = projection_row_ordinal;
        projection_row_ordinal = projection_row_ordinal.saturating_add(1);
        if !candidate_projection_row_allowed(options, row_ordinal) {
            continue;
        }
        if !row_matches_projection_filters(&out, &options.pushed_filters) {
            continue;
        }
        rows.push(Value::Object(out));
        if reached_projection_limit(&mut rows, options.max_rows) {
            return Ok(rows);
        }
    }
    Ok(rows)
}

fn candidate_projection_row_allowed(options: &ProjectionBatchOptions, row_ordinal: u64) -> bool {
    options
        .candidate_projection_rows
        .as_ref()
        .map(|candidates| !candidates.is_empty() && candidates.contains(row_ordinal))
        .unwrap_or(true)
}

fn row_matches_projection_filters(row: &Map<String, Value>, filters: &[ProjectionFilter]) -> bool {
    filters.iter().all(|filter| filter.matches(row))
}

impl ProjectionFilter {
    fn column_name(&self) -> &str {
        match self {
            Self::Compare { column, .. }
            | Self::InList { column, .. }
            | Self::IsNull { column, .. } => column,
        }
    }

    fn matches(&self, row: &Map<String, Value>) -> bool {
        let value = row.get(self.column_name()).unwrap_or(&Value::Null);
        projection_filter_matches_value(self, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_not_equal_filter_matches_non_null_unequal_values_only() {
        let filter = ProjectionFilter::Compare {
            column: "status".into(),
            op: ProjectionFilterOp::Ne,
            literal: ProjectionFilterLiteral::Utf8("closed".into()),
        };

        let mut open = Map::new();
        open.insert("status".into(), Value::String("open".into()));
        assert!(row_matches_projection_filters(
            &open,
            std::slice::from_ref(&filter)
        ));

        let mut closed = Map::new();
        closed.insert("status".into(), Value::String("closed".into()));
        assert!(!row_matches_projection_filters(
            &closed,
            std::slice::from_ref(&filter)
        ));

        let mut null = Map::new();
        null.insert("status".into(), Value::Null);
        assert!(!row_matches_projection_filters(
            &null,
            std::slice::from_ref(&filter)
        ));
    }
}

fn projection_filter_matches_value(filter: &ProjectionFilter, value: &Value) -> bool {
    match filter {
        ProjectionFilter::Compare { op, literal, .. } => match op {
            ProjectionFilterOp::Eq => projection_filter_eq(value, literal),
            ProjectionFilterOp::Ne => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_eq())
            }
            ProjectionFilterOp::Lt => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_lt())
            }
            ProjectionFilterOp::LtEq => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_gt())
            }
            ProjectionFilterOp::Gt => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_gt())
            }
            ProjectionFilterOp::GtEq => {
                projection_filter_cmp(value, literal).is_some_and(|ordering| !ordering.is_lt())
            }
        },
        ProjectionFilter::InList { literals, .. } => {
            !value.is_null()
                && literals
                    .iter()
                    .any(|literal| projection_filter_eq(value, literal))
        }
        ProjectionFilter::IsNull { negated, .. } => {
            let is_null = value.is_null();
            if *negated {
                !is_null
            } else {
                is_null
            }
        }
    }
}

fn projection_filter_eq(value: &Value, literal: &ProjectionFilterLiteral) -> bool {
    projection_filter_cmp(value, literal).is_some_and(|ordering| ordering.is_eq())
}

fn projection_filter_cmp(
    value: &Value,
    literal: &ProjectionFilterLiteral,
) -> Option<std::cmp::Ordering> {
    if value.is_null() {
        return None;
    }
    match literal {
        ProjectionFilterLiteral::Null => None,
        ProjectionFilterLiteral::Boolean(literal) => {
            value.as_bool().map(|value| value.cmp(literal))
        }
        ProjectionFilterLiteral::Int64(literal) => value
            .as_i64()
            .map(|value| value.cmp(literal))
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .map(|value| value.cmp(literal))
            })
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.total_cmp(&(*literal as f64)))
            }),
        ProjectionFilterLiteral::UInt64(literal) => value
            .as_u64()
            .map(|value| value.cmp(literal))
            .or_else(|| {
                value
                    .as_i64()
                    .and_then(|value| u64::try_from(value).ok())
                    .map(|value| value.cmp(literal))
            })
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.total_cmp(&(*literal as f64)))
            }),
        ProjectionFilterLiteral::Float64(literal) => {
            value.as_f64().map(|value| value.total_cmp(literal))
        }
        ProjectionFilterLiteral::Utf8(literal) => {
            value.as_str().map(|value| value.cmp(literal.as_str()))
        }
    }
}

fn reached_projection_limit(rows: &mut Vec<Value>, max_rows: Option<usize>) -> bool {
    let Some(max_rows) = max_rows else {
        return false;
    };
    if rows.len() > max_rows {
        rows.truncate(max_rows);
    }
    rows.len() >= max_rows
}

fn projection_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<Value, String> {
    match expression {
        "goid" | "object.goid" | "Object.goid" | "association.goid" => {
            return Ok(json!(hex_encode(&row.goid)));
        }
        "record.id" | "record.record_id" => return Ok(json!(hex_encode(&row.record_id))),
        "record.kind" => return Ok(json!(record_kind_name(row.record_kind))),
        "object.type_id" | "object_type_id" => return Ok(json!(row.object_type_id)),
        "temporal.timestamp_us" | "timestamp_us" => return Ok(json!(row.timestamp_us)),
        "temporal.csn" | "csn" => return Ok(json!(row.csn)),
        "temporal.branch_key" | "branch_key" => return Ok(json!(row.branch_key)),
        "object_type" | "object.type" | "Object.type" => return Ok(json!(row.object_type)),
        "association.source_goid" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                "source_goid",
            ))
        }
        "association.target_goid" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                "target_goid",
            ))
        }
        "association.association_type" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_TYPE,
                "association_type",
            ))
        }
        "association.mapping_rule_id" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_MAPPING_RULE_REF,
                "mapping_rule_id",
            ))
        }
        "association.source_evidence_id" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_EVIDENCE_REF,
                "source_evidence_id",
            ))
        }
        "association.source_role" => return Ok(projection_property_by_name(row, "source_role")),
        "association.target_role" => return Ok(projection_property_by_name(row, "target_role")),
        "association.valid_from" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_VALID_FROM,
                "valid_from",
            ))
        }
        "association.valid_to" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_VALID_TO,
                "valid_to",
            ))
        }
        "association.observed_at" => {
            return Ok(projection_property_by_flag_or_name(
                row,
                PROPERTY_FLAG_ASSOCIATION_OBSERVED_AT,
                "observed_at",
            ))
        }
        "association.cardinality_policy" => {
            return Ok(projection_property_by_name(row, "cardinality_policy"))
        }
        _ => {}
    }
    if let Some(literal) = literal_value(expression) {
        return Ok(literal);
    }
    if let Some(resolution) = parse_resolution_expression(expression) {
        return resolution_expression_value(model, row, resolution, expression);
    }
    if let Some(value) = conditional_expression(model, projection, row, expression)? {
        return Ok(value);
    }
    if let Some(inner) = expression
        .strip_prefix("count(association(")
        .and_then(|rest| rest.strip_suffix("))"))
    {
        let (association_type, endpoint_role) = parse_association_call_args(inner);
        let count = associated_rows(model, projection, row, association_type, endpoint_role)?.len();
        return Ok(json!(count));
    }
    if let Some((function, args)) = parse_function_call(expression) {
        return projection_function_value(model, projection, row, function, &args);
    }
    if let Some(traversal) = parse_association_traversal(expression) {
        let values = associated_rows(
            model,
            projection,
            row,
            traversal.association_type,
            traversal.endpoint_role,
        )?
        .into_iter()
        .map(|candidate| association_projection_value(&candidate, traversal.property_name))
        .collect::<Vec<_>>();
        return Ok(Value::Array(values));
    }
    let property_name = expression
        .rsplit('.')
        .next()
        .ok_or_else(|| format!("unsupported projection expression '{expression}'"))?;
    Ok(projection_property_by_name(row, property_name))
}

fn projection_function_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    match function {
        "identity" => unary_arg(model, projection, row, function, args),
        "trim" => string_unary(model, projection, row, function, args, |value| {
            value.trim().to_string()
        }),
        "lower" | "lowercase" => string_unary(model, projection, row, function, args, |value| {
            value.to_ascii_lowercase()
        }),
        "upper" | "uppercase" => string_unary(model, projection, row, function, args, |value| {
            value.to_ascii_uppercase()
        }),
        "exists" => {
            let value = unary_arg(model, projection, row, function, args)?;
            Ok(json!(
                !value.is_null() && !matches!(&value, Value::Array(values) if values.is_empty())
            ))
        }
        "coalesce" => {
            for arg in args {
                let value = projection_value(model, projection, row, arg)?;
                if !value.is_null() {
                    return Ok(value);
                }
            }
            Ok(Value::Null)
        }
        "association" => {
            if args.len() != 1 {
                return Err("projection function 'association' expects one argument".into());
            }
            let (association_type, endpoint_role) = parse_association_call_args(&args[0]);
            Ok(Value::Array(
                associated_rows(model, projection, row, association_type, endpoint_role)?
                    .into_iter()
                    .map(|candidate| json!(hex_encode(&candidate.goid)))
                    .collect(),
            ))
        }
        "count" | "min" | "max" | "sum" | "avg" | "distinct_count" | "list" => {
            aggregate_function_value(model, projection, row, function, args)
        }
        other => Err(format!("unsupported projection function '{other}'")),
    }
}

fn unary_arg(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "projection function '{function}' expects one argument"
        ));
    }
    projection_value(model, projection, row, &args[0])
}

fn string_unary(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
    op: impl FnOnce(&str) -> String,
) -> Result<Value, String> {
    let value = unary_arg(model, projection, row, function, args)?;
    Ok(value
        .as_str()
        .map(|text| json!(op(text)))
        .unwrap_or(Value::Null))
}

fn aggregate_function_value(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    function: &str,
    args: &[String],
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "projection aggregate '{function}' expects one argument"
        ));
    }
    let values = if let Some(traversal) = parse_association_traversal(&args[0]) {
        associated_rows(
            model,
            projection,
            row,
            traversal.association_type,
            traversal.endpoint_role,
        )?
        .into_iter()
        .map(|candidate| association_projection_value(&candidate, traversal.property_name))
        .collect::<Vec<_>>()
    } else if let Some((association_function, association_args)) = parse_function_call(&args[0]) {
        if association_function == "association" && association_args.len() == 1 {
            let (association_type, endpoint_role) =
                parse_association_call_args(&association_args[0]);
            associated_rows(model, projection, row, association_type, endpoint_role)?
                .into_iter()
                .map(|candidate| json!(hex_encode(&candidate.goid)))
                .collect::<Vec<_>>()
        } else {
            vec![projection_value(model, projection, row, &args[0])?]
        }
    } else {
        vec![projection_value(model, projection, row, &args[0])?]
    };
    match function {
        "count" => Ok(json!(values
            .iter()
            .filter(|value| !value.is_null())
            .count())),
        "list" => Ok(Value::Array(values)),
        "distinct_count" => {
            let set = values
                .into_iter()
                .filter(|value| !value.is_null())
                .map(|value| value.to_string())
                .collect::<std::collections::BTreeSet<_>>();
            Ok(json!(set.len()))
        }
        "min" => Ok(min_max_json(values, true)),
        "max" => Ok(min_max_json(values, false)),
        "sum" => Ok(json!(values
            .into_iter()
            .filter_map(json_number_f64)
            .sum::<f64>())),
        "avg" => {
            let numbers = values
                .into_iter()
                .filter_map(json_number_f64)
                .collect::<Vec<_>>();
            if numbers.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(json!(numbers.iter().sum::<f64>() / numbers.len() as f64))
            }
        }
        other => Err(format!("unsupported projection aggregate '{other}'")),
    }
}

fn conditional_expression(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<Option<Value>, String> {
    let Some((function, args)) = parse_function_call(expression) else {
        return Ok(None);
    };
    if !matches!(function, "if" | "ifelse") {
        return Ok(None);
    }
    if args.len() != 3 {
        return Err(format!(
            "projection conditional '{function}' expects three arguments"
        ));
    }
    let condition = projection_condition(model, projection, row, &args[0])?;
    Ok(Some(projection_value(
        model,
        projection,
        row,
        if condition { &args[1] } else { &args[2] },
    )?))
}

fn projection_condition(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    expression: &str,
) -> Result<bool, String> {
    for op in ["==", "!=", ">=", "<=", ">", "<"] {
        if let Some((left, right)) = expression.split_once(op) {
            let left = projection_value(model, projection, row, left.trim())?;
            let right = projection_value(model, projection, row, right.trim())?;
            return Ok(compare_json_values(&left, &right, op));
        }
    }
    Ok(json_truthy(&projection_value(
        model, projection, row, expression,
    )?))
}

fn associated_rows(
    model: &ProjectionModel,
    projection: &MapProjectionEntry,
    row: &ProjectionRow,
    association_type: &str,
    endpoint_role: Option<&str>,
) -> Result<Vec<ProjectionRow>, String> {
    let mut rows = model
        .rows_for_projection_for_aggregate()?
        .into_iter()
        .filter(|candidate| row_matches_association(candidate, association_type))
        .filter(|candidate| association_endpoint_matches(candidate, row, endpoint_role))
        .collect::<Vec<_>>();
    sort_projection_rows_by_ordering(&mut rows, projection, Some(row))?;
    Ok(rows)
}

fn association_endpoint_matches(
    candidate: &ProjectionRow,
    anchor: &ProjectionRow,
    endpoint_role: Option<&str>,
) -> bool {
    let anchor_goid = json!(hex_encode(&anchor.goid));
    let source_goid = projection_property_by_flag_or_name(
        candidate,
        PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
        "source_goid",
    );
    let target_goid = projection_property_by_flag_or_name(
        candidate,
        PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        "target_goid",
    );
    let Some(role) = endpoint_role.map(str::trim).filter(|role| !role.is_empty()) else {
        return source_goid == anchor_goid;
    };
    match role {
        "source" | "from" => source_goid == anchor_goid,
        "target" | "to" => target_goid == anchor_goid,
        other => {
            (source_goid == anchor_goid
                && projection_property_by_name(candidate, "source_role").as_str() == Some(other))
                || (target_goid == anchor_goid
                    && projection_property_by_name(candidate, "target_role").as_str()
                        == Some(other))
        }
    }
}

fn association_projection_value(row: &ProjectionRow, property_name: &str) -> Value {
    match property_name {
        "goid" => json!(hex_encode(&row.goid)),
        "source_goid" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            "source_goid",
        ),
        "target_goid" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID,
            "target_goid",
        ),
        "association_type" => projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TYPE,
            "association_type",
        ),
        other => projection_property_by_name(row, other),
    }
}

#[derive(Debug, Clone, Copy)]
struct AssociationTraversal<'a> {
    association_type: &'a str,
    endpoint_role: Option<&'a str>,
    property_name: &'a str,
}

fn parse_association_traversal(expression: &str) -> Option<AssociationTraversal<'_>> {
    let expression = expression.trim();
    let rest = expression.strip_prefix("association(")?;
    let (association_type, rest) = rest.split_once(").")?;
    let (association_type, endpoint_role) = match association_type.split_once(',') {
        Some((association_type, endpoint_role)) => {
            (association_type.trim(), Some(endpoint_role.trim()))
        }
        None => (association_type.trim(), None),
    };
    (!association_type.is_empty() && !rest.trim().is_empty()).then_some(AssociationTraversal {
        association_type,
        endpoint_role: endpoint_role.filter(|role| !role.is_empty()),
        property_name: rest.trim(),
    })
}

fn parse_association_call_args(input: &str) -> (&str, Option<&str>) {
    match input.split_once(',') {
        Some((association_type, endpoint_role)) => (
            association_type.trim(),
            Some(endpoint_role.trim()).filter(|role| !role.is_empty()),
        ),
        None => (input.trim(), None),
    }
}

fn parse_function_call(expression: &str) -> Option<(&str, Vec<String>)> {
    let expression = expression.trim();
    let open = expression.find('(')?;
    if !expression.ends_with(')') {
        return None;
    }
    let function = expression[..open].trim();
    if function.is_empty()
        || !function
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    let inner = &expression[open + 1..expression.len() - 1];
    Some((function, split_args(inner)))
}

fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote = None;
    let bytes = input.as_bytes();
    for (index, ch) in input.char_indices() {
        if let Some(active) = quote {
            if ch == active && bytes.get(index.wrapping_sub(1)) != Some(&b'\\') {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(input[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() || !input.is_empty() {
        args.push(tail.to_string());
    }
    args
}

fn literal_value(expression: &str) -> Option<Value> {
    let expression = expression.trim();
    if expression == "null" {
        return Some(Value::Null);
    }
    if expression == "true" {
        return Some(Value::Bool(true));
    }
    if expression == "false" {
        return Some(Value::Bool(false));
    }
    if (expression.starts_with('"') && expression.ends_with('"'))
        || (expression.starts_with('\'') && expression.ends_with('\''))
    {
        return Some(Value::String(
            expression[1..expression.len() - 1].to_string(),
        ));
    }
    if let Ok(value) = expression.parse::<i64>() {
        return Some(json!(value));
    }
    if let Ok(value) = expression.parse::<f64>() {
        return Some(json!(value));
    }
    None
}

fn min_max_json(values: Vec<Value>, min: bool) -> Value {
    values
        .into_iter()
        .filter(|value| !value.is_null())
        .min_by(|left, right| {
            let ordering = compare_json_order(left, right);
            if min {
                ordering
            } else {
                ordering.reverse()
            }
        })
        .unwrap_or(Value::Null)
}

fn compare_json_order(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (
        json_number_f64(left.clone()),
        json_number_f64(right.clone()),
    ) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        _ => left.to_string().cmp(&right.to_string()),
    }
}

fn compare_json_values(left: &Value, right: &Value, op: &str) -> bool {
    let ordering = compare_json_order(left, right);
    match op {
        "==" => left == right,
        "!=" => left != right,
        ">" => ordering.is_gt(),
        ">=" => !ordering.is_lt(),
        "<" => ordering.is_lt(),
        "<=" => !ordering.is_gt(),
        _ => false,
    }
}

fn json_number_f64(value: Value) -> Option<f64> {
    value.as_f64()
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
    }
}

fn record_kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Delta => "delta",
        RecordKind::Snapshot => "snapshot",
        RecordKind::ReservedLegacyMaterializedDelta => "reserved_legacy_materialized_delta",
        RecordKind::Baseline => "baseline",
        RecordKind::Tombstone => "tombstone",
        _ => "unknown",
    }
}

impl ProjectionModel {
    fn rows_for_projection_for_aggregate(&self) -> Result<Vec<ProjectionRow>, String> {
        Ok(self.reconstructed_rows.clone())
    }
}

#[cfg(test)]
pub(crate) fn property_by_name(row: &ObjectRow, property_name: &str) -> Value {
    row.properties
        .values()
        .find(|property| property.entry.property_name == property_name)
        .map(|property| property.value.clone())
        .unwrap_or(Value::Null)
}

fn projection_property_by_name(row: &ProjectionRow, property_name: &str) -> Value {
    row.properties
        .iter()
        .find(|property| property.property_name == property_name)
        .map(|property| property.value.clone())
        .unwrap_or(Value::Null)
}

fn projection_property_ref_by_name<'a>(
    row: &'a ProjectionRow,
    property_name: &str,
) -> Option<&'a Value> {
    row.properties
        .iter()
        .find(|property| property.property_name == property_name)
        .map(|property| &property.value)
}

fn projection_property_by_flag_or_name(
    row: &ProjectionRow,
    flag: u32,
    property_name: &str,
) -> Value {
    row.properties
        .iter()
        .find(|property| property.flags & flag != 0)
        .map(|property| property.value.clone())
        .unwrap_or_else(|| projection_property_by_name(row, property_name))
}

fn row_matches_association(row: &ProjectionRow, association_type: &str) -> bool {
    if row.object_type_flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT)
        != 0
    {
        let flagged = projection_property_by_flag_or_name(
            row,
            PROPERTY_FLAG_ASSOCIATION_TYPE,
            "association_type",
        );
        if flagged.as_str() == Some(association_type) {
            return true;
        }
        if row.object_type.strip_prefix("Association:") == Some(association_type) {
            return true;
        }
    }
    row.object_type == format!("Association:{association_type}")
}

pub fn run_fixture_path(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let fixture: Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("fixture {} is not valid JSON: {err}", path.display()))?;
    let map = PathBuf::from(required_str(&fixture, "mapping")?);
    let sources = fixture
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| "fixture.sources must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| "fixture.sources entries must be strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let file = parse_map(&map)?;
    let rows = read_sources(&sources)?;
    if let Some(expected_rows) = fixture.get("expected_projected_rows") {
        let projected = project_rows(&file, &rows)?;
        if &projected["rows"] != expected_rows {
            return Err("fixture projected rows did not match".into());
        }
    }
    println!("{}", json!({"ok": true, "fixture": path}));
    Ok(())
}
