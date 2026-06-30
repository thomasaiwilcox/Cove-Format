use std::collections::BTreeMap;

use cove_core::{
    array::{CoveArrayValue, EncodedArray},
    artifact::{covemap::CovemapFile, covm::CovmFile, covx::CovxFile},
    compression,
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind, ValueTag,
        MAGIC_COVE, MAGIC_COVEMAP, MAGIC_COVI, MAGIC_COVM, MAGIC_COVX,
    },
    dictionary::{DictionaryValue, FileDictionary},
    materialize_stats_only_constant_page_payload,
    mount::{mount_cove_file, MountOptions},
    page::{ColumnPageIndex, PAGE_FLAG_STATS_ONLY_CONSTANT},
    page_payload::{ColumnPagePayloadV1, PageBufferKind},
    profile::{
        cove_map::MapProjectionCatalog,
        cove_o::{
            read_object_surface_from_bytes, CoveObjectSurface, ObjectTypeEntryV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_EVIDENCE_OBJECT,
            OBJECT_TYPE_FLAG_PROJECTION_OBJECT,
        },
    },
    reader::{validate_bytes, ValidatedCoveFile, ValidationOptions},
    segment::{TableSegmentIndex, TableSegmentPayloadV1},
    table::{ColumnEntry, TableCatalog, TableEntry},
    types::{
        numcode_as_date_days, numcode_as_decimal64, numcode_as_f32, numcode_as_f64, numcode_as_i16,
        numcode_as_i32, numcode_as_i64, numcode_as_i8, numcode_as_timestamp_micros,
        numcode_as_timestamp_nanos, numcode_as_u16, numcode_as_u32, numcode_as_u64, numcode_as_u8,
    },
    utility::hex_encode,
    validity::ValidityBitmap,
    wire, CoveError,
};
use cove_index::CoviArtifactV2;
use serde::{Deserialize, Serialize};
use serde_json::{json, Number, Value};
use sha2::{Digest, Sha256};

use crate::{
    build_manifest_dataset_scope_context, build_physical_plan, coveql_identifier,
    execute_manifest_physical_planned_query, execute_manifest_planned_query,
    execute_physical_planned_query, execute_planned_query, parse_resolve_and_plan_query,
    parse_resolve_plan_and_build_physical_plan, parse_resolve_plan_and_execute_query,
    AstEvidenceGrain, BuildExecutionError, BuildOperationContextError, ExecutedQuery,
    ExecutionOptions, KernelExecutionOptions, ManifestDatasetMember, ManifestDatasetScopeOptions,
    ParseOptions, PhysicalPlanOptions, PlanOptions, ResolveOptions, ResolvedRoot,
    TableExecutionAuthority, TableSurfaceAuthority, TableSurfaceAuthorityKind,
    TableSurfaceColumnContract, TableSurfaceContract, TableSurfaceRow, TableTemporalAuthority,
    COVEQL_PROFILE_CONTRACT_VERSION,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySurfaceDiscoveryOptions {
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySurfaceDiscovery {
    pub source_name: Option<String>,
    pub artifact_kind: QueryArtifactKind,
    pub artifact_label: String,
    pub primary_profile: Option<String>,
    pub queryable: bool,
    pub guidance: String,
    pub object_types: Vec<QueryObjectSurface>,
    pub tables: Vec<QueryTableSurface>,
    pub projections: Vec<QueryProjectionSurface>,
    pub evidence: Vec<QueryEvidenceSurface>,
    pub sidecars: Vec<QuerySidecarSurface>,
    pub diagnostics: Vec<QuerySurfaceDiagnostic>,
}

impl QuerySurfaceDiscovery {
    pub fn has_queryable_rows(&self) -> bool {
        self.queryable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryArtifactKind {
    Cove,
    Covemap,
    Covm,
    Covx,
    Covi,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryObjectSurface {
    pub type_name: String,
    pub object_type_id: u32,
    pub row_count: usize,
    pub kind: String,
    pub properties: Vec<QueryColumnSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryTableSurface {
    pub table_name: String,
    pub table_id: String,
    pub row_count: u64,
    pub columns: Vec<QueryColumnSurface>,
    pub authority_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryProjectionSurface {
    pub projection_id: String,
    pub output_table: Option<String>,
    pub row_grain: Option<String>,
    pub columns: Vec<QueryColumnSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryEvidenceSurface {
    pub grain: String,
    pub row_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySidecarSurface {
    pub kind: String,
    pub queryable: bool,
    pub guidance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryColumnSurface {
    pub name: String,
    pub logical_type: Option<String>,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySurfaceDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySuggestion {
    pub title: String,
    pub query: String,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExecuteArtifactOptions {
    pub parse_options: ParseOptions,
    pub resolve_options: ResolveOptions,
    pub plan_options: PlanOptions,
    pub execution_engine: ArtifactExecutionEngine,
    pub execution_options: ExecutionOptions,
    pub validation_options: ValidationOptions,
    pub manifest_members: Vec<QueryArtifactMember>,
    pub manifest_scope_options: ManifestDatasetScopeOptions,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArtifactExecutionEngine {
    #[default]
    Materialized,
    Physical {
        physical_options: PhysicalPlanOptions,
        kernel_options: KernelExecutionOptions,
    },
}

#[derive(Debug, Clone)]
pub struct QueryArtifactMember {
    pub source: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub enum ExecuteArtifactQueryError {
    NotQueryable(Box<QuerySurfaceDiscovery>),
    Manifest(BuildOperationContextError),
    Execution(BuildExecutionError),
    Planning(String),
    Discovery(QueryDiscoveryError),
}

impl std::fmt::Display for ExecuteArtifactQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotQueryable(discovery) => write!(f, "{}", discovery.guidance),
            Self::Manifest(error) => write!(f, "{error}"),
            Self::Execution(error) => write!(f, "{error}"),
            Self::Planning(error) => write!(f, "{error}"),
            Self::Discovery(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ExecuteArtifactQueryError {}

impl From<BuildExecutionError> for ExecuteArtifactQueryError {
    fn from(value: BuildExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl From<QueryDiscoveryError> for ExecuteArtifactQueryError {
    fn from(value: QueryDiscoveryError) -> Self {
        Self::Discovery(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryDiscoveryError {
    message: String,
}

impl QueryDiscoveryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for QueryDiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueryDiscoveryError {}

pub fn discover_query_surfaces(
    bytes: &[u8],
    options: QuerySurfaceDiscoveryOptions,
) -> QuerySurfaceDiscovery {
    let source_name = options.source_name;
    let mut diagnostics = Vec::new();
    let artifact_kind = detect_artifact_kind(bytes);
    match artifact_kind {
        QueryArtifactKind::Cove => discover_cove_file(bytes, source_name, &mut diagnostics),
        QueryArtifactKind::Covemap => discover_covemap(bytes, source_name, &mut diagnostics),
        QueryArtifactKind::Covm => discover_covm(bytes, source_name, &mut diagnostics),
        QueryArtifactKind::Covx => discover_covx(bytes, source_name, &mut diagnostics),
        QueryArtifactKind::Covi => discover_covi(bytes, source_name, &mut diagnostics),
        QueryArtifactKind::Unknown => QuerySurfaceDiscovery {
            source_name,
            artifact_kind,
            artifact_label: "Unknown artifact".into(),
            primary_profile: None,
            queryable: false,
            guidance: "This file is not a recognized COVE artifact.".into(),
            object_types: Vec::new(),
            tables: Vec::new(),
            projections: Vec::new(),
            evidence: Vec::new(),
            sidecars: Vec::new(),
            diagnostics,
        },
    }
}

pub fn suggest_queries(discovery: &QuerySurfaceDiscovery) -> Vec<QuerySuggestion> {
    let mut suggestions = Vec::new();
    for object in discovery.object_types.iter().filter(|object| {
        !matches!(
            object.kind.as_str(),
            "association" | "evidence" | "projection"
        )
    }) {
        suggestions.push(QuerySuggestion {
            title: format!("Show {} rows", object.type_name),
            query: format!("object({}).take(10)", coveql_identifier(&object.type_name)),
            description: "Inspect the first visible object rows.".into(),
        });
        let select_columns = object
            .properties
            .iter()
            .take(3)
            .map(|column| coveql_identifier(&column.name))
            .collect::<Vec<_>>();
        if !select_columns.is_empty() {
            suggestions.push(QuerySuggestion {
                title: format!("Select {} columns", object.type_name),
                query: format!(
                    "object({}).select({}).take(10)",
                    coveql_identifier(&object.type_name),
                    select_columns.join(", ")
                ),
                description: "Return only a few readable properties.".into(),
            });
        }
    }
    for table in &discovery.tables {
        suggestions.push(QuerySuggestion {
            title: format!("Show {} table rows", table.table_name),
            query: format!("table({}).take(10)", coveql_identifier(&table.table_name)),
            description: "Inspect the first visible table rows.".into(),
        });
        let select_columns = table
            .columns
            .iter()
            .take(3)
            .map(|column| coveql_identifier(&column.name))
            .collect::<Vec<_>>();
        if !select_columns.is_empty() {
            suggestions.push(QuerySuggestion {
                title: format!("Select {} columns", table.table_name),
                query: format!(
                    "table({}).select({}).take(10)",
                    coveql_identifier(&table.table_name),
                    select_columns.join(", ")
                ),
                description: "Return only selected table columns.".into(),
            });
        }
        if let Some(first_column) = table.columns.first() {
            suggestions.push(QuerySuggestion {
                title: format!("Order {} rows", table.table_name),
                query: format!(
                    "table({}).orderBy({}).take(10)",
                    coveql_identifier(&table.table_name),
                    coveql_identifier(&first_column.name)
                ),
                description: "Sort table rows with CoveQL ordering.".into(),
            });
            suggestions.push(QuerySuggestion {
                title: format!("Window {} rows", table.table_name),
                query: format!(
                    "table({}).window(orderBy: {}).select({}, row_number: row_number()).take(10)",
                    coveql_identifier(&table.table_name),
                    coveql_identifier(&first_column.name),
                    coveql_identifier(&first_column.name)
                ),
                description: "Add a row_number window value over a deterministic order.".into(),
            });
        }
        if let Some(numeric_column) = table.columns.iter().find(|column| {
            column
                .logical_type
                .as_deref()
                .is_some_and(is_numeric_logical_type_name)
        }) {
            suggestions.push(QuerySuggestion {
                title: format!("Aggregate {} rows", table.table_name),
                query: format!(
                    "table({}).select(rows: count(*), total: sum({}), average: avg({}))",
                    coveql_identifier(&table.table_name),
                    coveql_identifier(&numeric_column.name),
                    coveql_identifier(&numeric_column.name)
                ),
                description: "Compute count, sum, and average with CoveQL aggregates.".into(),
            });
        } else {
            suggestions.push(QuerySuggestion {
                title: format!("Count {} rows", table.table_name),
                query: format!(
                    "table({}).select(rows: count(*))",
                    coveql_identifier(&table.table_name)
                ),
                description: "Count visible table rows.".into(),
            });
        }
    }
    for projection in &discovery.projections {
        suggestions.push(QuerySuggestion {
            title: format!("Show projection {}", projection.projection_id),
            query: format!(
                "projection({}).take(10)",
                coveql_identifier(&projection.projection_id)
            ),
            description: "Read a declared semantic projection.".into(),
        });
    }
    if !discovery.evidence.is_empty() {
        suggestions.push(QuerySuggestion {
            title: "Show evidence rows".into(),
            query: "evidence().take(10)".into(),
            description: "Inspect provenance/evidence rows when COVE-MAP metadata is available."
                .into(),
        });
    }
    suggestions
}

pub fn execute_query_from_artifact(
    bytes: &[u8],
    query: &str,
    mut options: ExecuteArtifactOptions,
) -> Result<ExecutedQuery, ExecuteArtifactQueryError> {
    let discovery = discover_query_surfaces(bytes, QuerySurfaceDiscoveryOptions::default());
    match discovery.artifact_kind {
        QueryArtifactKind::Cove
            if discovery.queryable || !options.resolve_options.table_authorities.is_empty() =>
        {
            if !discovery.tables.is_empty() {
                for (name, authority) in cove_t_table_authorities_from_bytes(bytes)? {
                    options
                        .resolve_options
                        .table_authorities
                        .insert(name, authority);
                }
            }
            match options.execution_engine {
                ArtifactExecutionEngine::Materialized => parse_resolve_plan_and_execute_query(
                    bytes,
                    query,
                    options.parse_options,
                    options.resolve_options,
                    options.plan_options,
                    options.execution_options,
                    options.validation_options,
                )
                .map_err(ExecuteArtifactQueryError::Execution),
                ArtifactExecutionEngine::Physical {
                    physical_options,
                    kernel_options,
                } => parse_resolve_plan_and_build_physical_plan(
                    bytes,
                    query,
                    options.parse_options,
                    options.resolve_options,
                    options.plan_options,
                    physical_options,
                    options.validation_options,
                )
                .map_err(|error| ExecuteArtifactQueryError::Planning(error.to_string()))
                .and_then(|physical| {
                    crate::execute_physical_planned_query(
                        bytes,
                        physical,
                        options.execution_options,
                        kernel_options,
                    )
                    .map(|executed| executed.executed)
                    .map_err(ExecuteArtifactQueryError::Execution)
                }),
            }
        }
        QueryArtifactKind::Covm => execute_query_from_manifest(bytes, query, options),
        _ => Err(ExecuteArtifactQueryError::NotQueryable(Box::new(discovery))),
    }
}

pub fn cove_t_table_authorities_from_bytes(
    bytes: &[u8],
) -> Result<BTreeMap<String, TableSurfaceAuthority>, QueryDiscoveryError> {
    let validated =
        validate_bytes(bytes).map_err(|error| QueryDiscoveryError::new(error.to_string()))?;
    let mounted = mount_cove_file(bytes, MountOptions::default(), None)
        .map_err(|error| QueryDiscoveryError::new(error.to_string()))?;
    let Some(catalog) = mounted.table_catalog.as_ref() else {
        return Ok(BTreeMap::new());
    };
    let rows_by_table = decode_cove_t_rows(bytes, &validated, catalog, mounted.dictionary.as_ref())
        .map_err(|error| QueryDiscoveryError::new(error.to_string()))?;
    let mut out = BTreeMap::new();
    for table in &catalog.tables {
        let table_name = table.name.clone();
        let rows = rows_by_table
            .get(&table.table_id)
            .cloned()
            .unwrap_or_default();
        out.insert(
            table_name.clone(),
            TableSurfaceAuthority {
                contract: table_surface_contract_from_cove_t(table, &rows),
                execution_authority: TableExecutionAuthority::RawRows { rows },
            },
        );
    }
    Ok(out)
}

fn execute_query_from_manifest(
    bytes: &[u8],
    query: &str,
    mut options: ExecuteArtifactOptions,
) -> Result<ExecutedQuery, ExecuteArtifactQueryError> {
    if options.manifest_members.is_empty() {
        let discovery = discover_query_surfaces(bytes, QuerySurfaceDiscoveryOptions::default());
        return Err(ExecuteArtifactQueryError::NotQueryable(Box::new(discovery)));
    }
    let members = options
        .manifest_members
        .iter()
        .map(|member| ManifestDatasetMember {
            source: member.source.as_str(),
            bytes: member.bytes.as_slice(),
        })
        .collect::<Vec<_>>();
    register_cove_t_manifest_table_authorities(&members, &mut options.resolve_options)?;
    let scope = build_manifest_dataset_scope_context(
        bytes,
        &members,
        options.manifest_scope_options.clone(),
    )
    .map_err(ExecuteArtifactQueryError::Manifest)?;
    let planning_member = members.first().ok_or_else(|| {
        ExecuteArtifactQueryError::Discovery(QueryDiscoveryError::new(
            "COVM query execution requires at least one member file",
        ))
    })?;
    let mut planned = parse_resolve_and_plan_query(
        planning_member.bytes,
        query,
        options.parse_options,
        options.resolve_options,
        options.plan_options,
        options.validation_options.clone(),
    )
    .map_err(|error| ExecuteArtifactQueryError::Planning(error.to_string()))?;
    planned.resolved.operation_context.dataset = scope;
    let local_registered_table_authority = manifest_query_uses_registered_table_authority(&planned);
    match options.execution_engine {
        ArtifactExecutionEngine::Materialized if local_registered_table_authority => {
            execute_planned_query(planning_member.bytes, planned, options.execution_options)
                .map_err(ExecuteArtifactQueryError::Execution)
        }
        ArtifactExecutionEngine::Materialized => {
            execute_manifest_planned_query(&members, planned, options.execution_options)
                .map_err(ExecuteArtifactQueryError::Execution)
        }
        ArtifactExecutionEngine::Physical {
            physical_options,
            kernel_options,
        } => build_physical_plan(
            planning_member.bytes,
            planned,
            physical_options,
            options.validation_options,
        )
        .map_err(|error| ExecuteArtifactQueryError::Planning(error.to_string()))
        .and_then(|physical| {
            if local_registered_table_authority {
                execute_physical_planned_query(
                    planning_member.bytes,
                    physical,
                    options.execution_options,
                    kernel_options,
                )
            } else {
                execute_manifest_physical_planned_query(
                    &members,
                    physical,
                    options.execution_options,
                    kernel_options,
                )
            }
            .map(|executed| executed.executed)
            .map_err(ExecuteArtifactQueryError::Execution)
        }),
    }
}

fn manifest_query_uses_registered_table_authority(planned: &crate::PlannedQuery) -> bool {
    matches!(
        &planned.resolved.root,
        ResolvedRoot::Table(table)
            if !matches!(
                table.execution_authority,
                TableExecutionAuthority::DeterministicProjection { .. }
            )
    )
}

fn register_cove_t_manifest_table_authorities(
    members: &[ManifestDatasetMember<'_>],
    resolve_options: &mut ResolveOptions,
) -> Result<(), ExecuteArtifactQueryError> {
    for member in members {
        for (name, authority) in cove_t_table_authorities_from_bytes(member.bytes)? {
            match resolve_options.table_authorities.get_mut(&name) {
                Some(existing) => merge_table_authority_rows(existing, authority)?,
                None => {
                    resolve_options.table_authorities.insert(name, authority);
                }
            }
        }
    }
    Ok(())
}

fn merge_table_authority_rows(
    existing: &mut TableSurfaceAuthority,
    incoming: TableSurfaceAuthority,
) -> Result<(), QueryDiscoveryError> {
    if existing.contract.logical_column_map != incoming.contract.logical_column_map {
        return Err(QueryDiscoveryError::new(format!(
            "manifest COVE-T table {} appears with incompatible schemas",
            existing.contract.table_name
        )));
    }
    let row_count = {
        let existing_rows = table_authority_rows_mut(&mut existing.execution_authority)?;
        let incoming_rows = table_authority_rows(incoming.execution_authority)?;
        existing_rows.extend(incoming_rows);
        existing_rows.len()
    };
    existing.contract.authority_fingerprint =
        merged_table_authority_fingerprint(&existing.contract.table_id, row_count);
    Ok(())
}

fn table_authority_rows_mut(
    authority: &mut TableExecutionAuthority,
) -> Result<&mut Vec<TableSurfaceRow>, QueryDiscoveryError> {
    match authority {
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows }
        | TableExecutionAuthority::ExternalRows { rows, .. } => Ok(rows),
        TableExecutionAuthority::DeterministicProjection { .. } => Err(QueryDiscoveryError::new(
            "manifest table authority merge requires materialized/raw rows",
        )),
    }
}

fn table_authority_rows(
    authority: TableExecutionAuthority,
) -> Result<Vec<TableSurfaceRow>, QueryDiscoveryError> {
    match authority {
        TableExecutionAuthority::MaterializedRows { rows }
        | TableExecutionAuthority::RawRows { rows }
        | TableExecutionAuthority::ExternalRows { rows, .. } => Ok(rows),
        TableExecutionAuthority::DeterministicProjection { .. } => Err(QueryDiscoveryError::new(
            "manifest table authority merge requires materialized/raw rows",
        )),
    }
}

fn merged_table_authority_fingerprint(table_id: &str, row_count: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(table_id.as_bytes());
    hasher.update((row_count as u64).to_le_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn detect_artifact_kind(bytes: &[u8]) -> QueryArtifactKind {
    if bytes.len() < 4 {
        return QueryArtifactKind::Unknown;
    }
    match bytes[bytes.len() - 4..].try_into().unwrap_or([0; 4]) {
        MAGIC_COVE => QueryArtifactKind::Cove,
        MAGIC_COVEMAP => QueryArtifactKind::Covemap,
        MAGIC_COVM => QueryArtifactKind::Covm,
        MAGIC_COVX => QueryArtifactKind::Covx,
        MAGIC_COVI => QueryArtifactKind::Covi,
        _ => QueryArtifactKind::Unknown,
    }
}

fn discover_cove_file(
    bytes: &[u8],
    source_name: Option<String>,
    diagnostics: &mut Vec<QuerySurfaceDiagnostic>,
) -> QuerySurfaceDiscovery {
    let mut object_types = Vec::new();
    let mut tables = Vec::new();
    let mut projections = Vec::new();
    let mut evidence = Vec::new();
    let mut sidecars = Vec::new();
    let mut primary_profile = None;

    match validate_bytes(bytes) {
        Ok(validated) => {
            primary_profile = Some(profile_name(validated.header.primary_profile));
            sidecars.extend(sidecars_from_sections(&validated));
            if let Ok(surface) = read_object_surface_from_bytes(bytes) {
                for object_type in &surface.object_types {
                    object_types.push(object_surface_from_type(object_type, &surface.records));
                }
                if let Some(catalog) = &surface.projection_catalog {
                    projections.extend(projection_surfaces(catalog));
                    tables.extend(projection_table_surfaces(catalog, &surface));
                }
                if let Some(index) = &surface.evidence_index {
                    evidence.push(QueryEvidenceSurface {
                        grain: "evidence".into(),
                        row_count: index.entries.len(),
                    });
                }
            }
            match mount_cove_file(bytes, MountOptions::default(), None) {
                Ok(mounted) => {
                    for table in &mounted.tables {
                        tables.push(QueryTableSurface {
                            table_name: table.name.clone(),
                            table_id: table.table_id.to_string(),
                            row_count: table.row_count,
                            columns: table
                                .columns
                                .iter()
                                .map(|column| QueryColumnSurface {
                                    name: column.name.clone(),
                                    logical_type: Some(format!("{:?}", column.logical)),
                                    nullable: column.nullable,
                                })
                                .collect(),
                            authority_kind: "raw_table".into(),
                        });
                    }
                }
                Err(error) => diagnostics.push(QuerySurfaceDiagnostic {
                    code: "E_MOUNT".into(),
                    message: error.to_string(),
                }),
            }
        }
        Err(error) => diagnostics.push(QuerySurfaceDiagnostic {
            code: "E_VALIDATE".into(),
            message: error.to_string(),
        }),
    }

    let queryable = !object_types.is_empty() || !tables.is_empty() || !projections.is_empty();
    let guidance = if queryable {
        "This COVE file has queryable CoveQL surfaces. Run `cove inspect --queries <file>` for suggested queries.".into()
    } else {
        "This COVE file does not expose an object, table, or projection row surface. Inspect it for metadata and query the related data artifact if this is a sidecar.".into()
    };
    QuerySurfaceDiscovery {
        source_name,
        artifact_kind: QueryArtifactKind::Cove,
        artifact_label: "COVE".into(),
        primary_profile,
        queryable,
        guidance,
        object_types,
        tables,
        projections,
        evidence,
        sidecars,
        diagnostics: diagnostics.clone(),
    }
}

fn discover_covemap(
    bytes: &[u8],
    source_name: Option<String>,
    diagnostics: &mut Vec<QuerySurfaceDiagnostic>,
) -> QuerySurfaceDiscovery {
    match CovemapFile::parse_validated(bytes) {
        Ok(file) => {
            diagnostics.extend(file.compatibility_warnings().into_iter().map(|message| {
                QuerySurfaceDiagnostic {
                    code: "W_COVEMAP".into(),
                    message: message.to_string(),
                }
            }));
        }
        Err(error) => diagnostics.push(QuerySurfaceDiagnostic {
            code: "E_COVEMAP".into(),
            message: error.to_string(),
        }),
    }
    QuerySurfaceDiscovery {
        source_name,
        artifact_kind: QueryArtifactKind::Covemap,
        artifact_label: "COVEMAP".into(),
        primary_profile: Some("COVE-MAP mapping definition".into()),
        queryable: false,
        guidance: "This is a COVE-MAP mapping artifact. Use it with `cove query --mapping <file.covemap> <data.cove> '<query>'` or convert source data with `cove map convert`.".into(),
        object_types: Vec::new(),
        tables: Vec::new(),
        projections: Vec::new(),
        evidence: Vec::new(),
        sidecars: vec![QuerySidecarSurface {
            kind: "COVEMAP".into(),
            queryable: false,
            guidance: "Mapping definitions describe how source rows become semantic objects; query the generated COVE-O file.".into(),
        }],
        diagnostics: diagnostics.clone(),
    }
}

fn discover_covm(
    bytes: &[u8],
    source_name: Option<String>,
    diagnostics: &mut Vec<QuerySurfaceDiagnostic>,
) -> QuerySurfaceDiscovery {
    let mut sidecars = Vec::new();
    match CovmFile::parse(bytes) {
        Ok(file) => sidecars.push(QuerySidecarSurface {
            kind: "COVM".into(),
            queryable: false,
            guidance: format!(
                "Dataset manifest references {} member file(s); pass them with `--member id=path` to query the dataset.",
                file.files.len()
            ),
        }),
        Err(error) => diagnostics.push(QuerySurfaceDiagnostic {
            code: "E_COVM".into(),
            message: error.to_string(),
        }),
    }
    QuerySurfaceDiscovery {
        source_name,
        artifact_kind: QueryArtifactKind::Covm,
        artifact_label: "COVM".into(),
        primary_profile: Some("COVM dataset manifest".into()),
        queryable: false,
        guidance: "This is a COVM dataset manifest. Query it with member files supplied via `--member <manifest-uri=path>`.".into(),
        object_types: Vec::new(),
        tables: Vec::new(),
        projections: Vec::new(),
        evidence: Vec::new(),
        sidecars,
        diagnostics: diagnostics.clone(),
    }
}

fn discover_covx(
    bytes: &[u8],
    source_name: Option<String>,
    diagnostics: &mut Vec<QuerySurfaceDiagnostic>,
) -> QuerySurfaceDiscovery {
    if let Err(error) = CovxFile::parse(bytes) {
        diagnostics.push(QuerySurfaceDiagnostic {
            code: "E_COVX".into(),
            message: error.to_string(),
        });
    }
    QuerySurfaceDiscovery {
        source_name,
        artifact_kind: QueryArtifactKind::Covx,
        artifact_label: "COVX".into(),
        primary_profile: Some("COVX archive/index sidecar".into()),
        queryable: false,
        guidance: "This is a COVX sidecar. Query the related COVE data file and pass this artifact to tools that accept sidecars.".into(),
        object_types: Vec::new(),
        tables: Vec::new(),
        projections: Vec::new(),
        evidence: Vec::new(),
        sidecars: vec![QuerySidecarSurface {
            kind: "COVX".into(),
            queryable: false,
            guidance: "COVX accelerates or describes another COVE file; it is not a row dataset by itself.".into(),
        }],
        diagnostics: diagnostics.clone(),
    }
}

fn discover_covi(
    bytes: &[u8],
    source_name: Option<String>,
    diagnostics: &mut Vec<QuerySurfaceDiagnostic>,
) -> QuerySurfaceDiscovery {
    if let Err(error) = CoviArtifactV2::parse(bytes) {
        diagnostics.push(QuerySurfaceDiagnostic {
            code: "E_COVI".into(),
            message: error.to_string(),
        });
    }
    QuerySurfaceDiscovery {
        source_name,
        artifact_kind: QueryArtifactKind::Covi,
        artifact_label: "COVI".into(),
        primary_profile: Some("COVE-I secondary index".into()),
        queryable: false,
        guidance: "This is COVE-I index metadata. Query the related data file and pass this as a sidecar when a tool supports it.".into(),
        object_types: Vec::new(),
        tables: Vec::new(),
        projections: Vec::new(),
        evidence: Vec::new(),
        sidecars: vec![QuerySidecarSurface {
            kind: "COVE-I".into(),
            queryable: false,
            guidance: "Indexes accelerate another COVE file; they are not row datasets by themselves.".into(),
        }],
        diagnostics: diagnostics.clone(),
    }
}

fn object_surface_from_type(
    object_type: &ObjectTypeEntryV1,
    records: &[cove_core::profile::cove_o::CoveObjectRecord],
) -> QueryObjectSurface {
    QueryObjectSurface {
        type_name: object_type.type_name.clone(),
        object_type_id: object_type.object_type_id,
        row_count: records
            .iter()
            .filter(|record| record.object_type_id == object_type.object_type_id)
            .count(),
        kind: object_kind(object_type.flags).into(),
        properties: object_type
            .properties
            .iter()
            .map(|property| QueryColumnSurface {
                name: property.property_name.clone(),
                logical_type: Some(format!("{:?}", property.logical_type)),
                nullable: property.nullable,
            })
            .collect(),
    }
}

fn object_kind(flags: u32) -> &'static str {
    if flags & OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT != 0 {
        "association"
    } else if flags & OBJECT_TYPE_FLAG_EVIDENCE_OBJECT != 0 {
        "evidence"
    } else if flags & OBJECT_TYPE_FLAG_PROJECTION_OBJECT != 0 {
        "projection"
    } else {
        "object"
    }
}

fn projection_surfaces(catalog: &MapProjectionCatalog) -> Vec<QueryProjectionSurface> {
    catalog
        .projections
        .iter()
        .map(|projection| QueryProjectionSurface {
            projection_id: projection.projection_id.clone(),
            output_table: projection.output_table.clone(),
            row_grain: projection.row_grain.clone(),
            columns: projection
                .columns
                .iter()
                .map(|column| QueryColumnSurface {
                    name: column.name.clone(),
                    logical_type: column.logical_type.clone(),
                    nullable: column.missing_policy != "reject",
                })
                .collect(),
        })
        .collect()
}

fn projection_table_surfaces(
    catalog: &MapProjectionCatalog,
    surface: &CoveObjectSurface,
) -> Vec<QueryTableSurface> {
    catalog
        .projections
        .iter()
        .filter_map(|projection| {
            let table_name = projection.output_table.as_ref()?;
            let row_count = projection
                .anchor
                .as_ref()
                .and_then(|anchor| anchor.object_type.as_ref())
                .map(|object_type| {
                    surface
                        .records
                        .iter()
                        .filter(|record| &record.object_type_name == object_type)
                        .count() as u64
                })
                .unwrap_or_default();
            Some(QueryTableSurface {
                table_name: table_name.clone(),
                table_id: format!("projection:{}", projection.projection_id),
                row_count,
                columns: projection
                    .columns
                    .iter()
                    .map(|column| QueryColumnSurface {
                        name: column.name.clone(),
                        logical_type: column.logical_type.clone(),
                        nullable: column.missing_policy != "reject",
                    })
                    .collect(),
                authority_kind: "deterministic_projection".into(),
            })
        })
        .collect()
}

fn is_numeric_logical_type_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "decimal64"
            | "decimal128"
    )
}

fn sidecars_from_sections(validated: &ValidatedCoveFile) -> Vec<QuerySidecarSurface> {
    let mut sidecars = Vec::new();
    for entry in &validated.footer.sections {
        if let Some(profile) = PrimaryProfile::from_u8(entry.profile) {
            match profile {
                PrimaryProfile::SemanticMapping => push_sidecar_once(
                    &mut sidecars,
                    "COVE-MAP",
                    "Semantic mapping metadata is available; evidence/projection queries may use it.",
                    true,
                ),
                PrimaryProfile::SecondaryIndex => push_sidecar_once(
                    &mut sidecars,
                    "COVE-I",
                    "Secondary index metadata may accelerate compatible queries.",
                    false,
                ),
                PrimaryProfile::LayoutPlanning => push_sidecar_once(
                    &mut sidecars,
                    "COVE-L",
                    "Layout metadata may help scan planning but is not a row surface.",
                    false,
                ),
                PrimaryProfile::CoverageMetadata => push_sidecar_once(
                    &mut sidecars,
                    "COVE-COVERAGE",
                    "Coverage metadata may prove pruning decisions but is not a row surface.",
                    false,
                ),
                PrimaryProfile::EngineExecution => push_sidecar_once(
                    &mut sidecars,
                    "COVE-E",
                    "Engine code-domain metadata is security-sensitive acceleration metadata.",
                    false,
                ),
                PrimaryProfile::HarborExecution => push_sidecar_once(
                    &mut sidecars,
                    "COVE-H",
                    "Harbor execution metadata is not directly queryable as rows.",
                    false,
                ),
                _ => {}
            }
        }
    }
    sidecars
}

fn push_sidecar_once(
    sidecars: &mut Vec<QuerySidecarSurface>,
    kind: &str,
    guidance: &str,
    queryable: bool,
) {
    if sidecars.iter().any(|sidecar| sidecar.kind == kind) {
        return;
    }
    sidecars.push(QuerySidecarSurface {
        kind: kind.into(),
        queryable,
        guidance: guidance.into(),
    });
}

fn profile_name(profile: u8) -> String {
    match PrimaryProfile::from_u8(profile) {
        Some(PrimaryProfile::Mixed) => "Mixed/Unknown".into(),
        Some(PrimaryProfile::ObjectTemporal) => "COVE-O (Object Temporal)".into(),
        Some(PrimaryProfile::TableScan) => "COVE-T (Table Scan)".into(),
        Some(PrimaryProfile::ArchiveAcceleration) => "COVE-A (Archive Acceleration)".into(),
        Some(PrimaryProfile::EngineExecution) => "COVE-E (Engine Execution)".into(),
        Some(PrimaryProfile::HarborExecution) => "COVE-H (Harbor Execution)".into(),
        Some(PrimaryProfile::SemanticMapping) => "COVE-MAP (Semantic Mapping)".into(),
        Some(PrimaryProfile::CodecExtension) => "COVE-CX (Codec Extension)".into(),
        Some(PrimaryProfile::LayoutPlanning) => "COVE-L (Layout Planning)".into(),
        Some(PrimaryProfile::RuntimeCompatibility) => "COVE-R (Runtime Compatibility)".into(),
        Some(PrimaryProfile::CoverageMetadata) => "COVE-COVERAGE".into(),
        Some(PrimaryProfile::SecondaryIndex) => "COVE-I (Secondary Index)".into(),
        Some(_) => format!("Known future profile ({profile})"),
        None => format!("Unknown({profile})"),
    }
}

fn decode_cove_t_rows(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
    catalog: &TableCatalog,
    dictionary: Option<&FileDictionary>,
) -> Result<BTreeMap<u32, Vec<TableSurfaceRow>>, CoveError> {
    let segment_index = parse_table_segment_index(bytes, validated)?;
    let segment_payloads = parse_table_segment_payloads(bytes, validated)?;
    let table_by_id = catalog
        .tables
        .iter()
        .map(|table| (table.table_id, table))
        .collect::<BTreeMap<_, _>>();
    let mut rows = BTreeMap::<u32, Vec<TableSurfaceRow>>::new();
    for table in &catalog.tables {
        let row_count = usize::try_from(table.row_count).map_err(|_| CoveError::ArithOverflow)?;
        rows.insert(table.table_id, vec![BTreeMap::new(); row_count]);
    }
    for entry in segment_index.entries {
        let table = table_by_id
            .get(&entry.table_id)
            .ok_or(CoveError::SegmentCorrupt)?;
        let (segment, segment_bytes) = segment_payloads
            .get(&(entry.table_id, entry.segment_id))
            .ok_or(CoveError::SegmentCorrupt)?;
        for column in &table.columns {
            decode_cove_t_column_into_rows(
                table,
                column,
                segment,
                segment_bytes,
                dictionary,
                rows.get_mut(&table.table_id)
                    .ok_or(CoveError::SegmentCorrupt)?,
            )?;
        }
    }
    Ok(rows)
}

fn parse_table_segment_index(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<TableSegmentIndex, CoveError> {
    let entry = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::TableSegmentIndex as u16)
        .ok_or(CoveError::SegmentCorrupt)?;
    let payload = compression::section_payload(bytes, entry)?;
    TableSegmentIndex::parse(payload.as_ref())
}

type TableSegmentPayloadsById = BTreeMap<(u32, u32), (TableSegmentPayloadV1, Vec<u8>)>;

fn parse_table_segment_payloads(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<TableSegmentPayloadsById, CoveError> {
    let mut out = BTreeMap::new();
    for entry in validated
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::TableSegmentData as u16)
    {
        let payload = compression::section_payload(bytes, entry)?;
        let parsed = TableSegmentPayloadV1::parse_with_required_features(
            payload.as_ref(),
            validated.header.required_features,
        )?;
        out.insert(
            (parsed.header.table_id, parsed.header.segment_id),
            (parsed, payload.into_owned()),
        );
    }
    Ok(out)
}

fn decode_cove_t_column_into_rows(
    table: &TableEntry,
    column: &ColumnEntry,
    segment: &TableSegmentPayloadV1,
    segment_bytes: &[u8],
    dictionary: Option<&FileDictionary>,
    rows: &mut [TableSurfaceRow],
) -> Result<(), CoveError> {
    let column_dir = segment
        .columns
        .iter()
        .find(|candidate| candidate.column_id == column.column_id)
        .ok_or(CoveError::SegmentCorrupt)?;
    let page_index_start =
        usize::try_from(column_dir.page_index_offset).map_err(|_| CoveError::OffsetRange)?;
    let page_index_end = usize::try_from(
        column_dir
            .page_index_offset
            .checked_add(column_dir.page_index_length)
            .ok_or(CoveError::ArithOverflow)?,
    )
    .map_err(|_| CoveError::OffsetRange)?;
    let page_index = ColumnPageIndex::parse(&segment_bytes[page_index_start..page_index_end])?;
    for page in page_index.entries {
        let payload_owner = if page.flags & PAGE_FLAG_STATS_ONLY_CONSTANT != 0 {
            materialize_stats_only_constant_page_payload(
                cove_core::StatsOnlyPageMaterializationContext {
                    table_id: Some(table.table_id),
                    segment_id: Some(segment.header.segment_id),
                    column_id: column.column_id,
                    logical_type: column.logical,
                    physical_kind: column.physical,
                    dictionary_len: dictionary.map(FileDictionary::len),
                    zone_stats: &[],
                },
                &page,
            )?
        } else {
            let start = usize::try_from(page.page_offset).map_err(|_| CoveError::OffsetRange)?;
            let end = usize::try_from(
                page.page_offset
                    .checked_add(page.page_length)
                    .ok_or(CoveError::ArithOverflow)?,
            )
            .map_err(|_| CoveError::OffsetRange)?;
            let page_wire = &segment_bytes[start..end];
            compression::column_page_payload(page_wire, &page)?.into_owned()
        };
        let payload = ColumnPagePayloadV1::parse(&payload_owner)?;
        let root = payload.root_node()?;
        if root.encoding_kind == CoveEncodingKind::RegisteredEncoding {
            return Err(CoveError::UnsupportedEncoding(
                "registered COVE-T page encodings need an engine codec before beginner readback can materialize them".into(),
            ));
        }
        let null_bitmap = payload.buffer_bytes(PageBufferKind::NullBitmap)?;
        let validity =
            null_bitmap.map(|bytes| ValidityBitmap::new(bytes, u64::from(page.row_count)));
        let values = payload.buffer_bytes(PageBufferKind::Values)?.unwrap_or(&[]);
        let array = EncodedArray::new(
            column.logical,
            column.physical,
            u64::from(page.row_count),
            root.encoding_kind,
            validity,
            values,
            dictionary,
        );
        let prepared = array.prepare()?;
        let morsel = segment.morsels.morsel_by_id(page.morsel_id)?;
        for local_row in 0..page.row_count {
            let segment_row = morsel
                .first_row_in_segment
                .checked_add(local_row)
                .ok_or(CoveError::ArithOverflow)?;
            let table_row = segment
                .header
                .row_start
                .checked_add(u64::from(segment_row))
                .ok_or(CoveError::ArithOverflow)?;
            let table_row = usize::try_from(table_row).map_err(|_| CoveError::ArithOverflow)?;
            let value = prepared.decode_row(u64::from(local_row))?;
            let value = cove_array_value_to_json(column.logical, value)?;
            let row = rows.get_mut(table_row).ok_or(CoveError::OffsetRange)?;
            row.insert(column.name.clone(), value);
        }
    }
    Ok(())
}

fn table_surface_contract_from_cove_t(
    table: &TableEntry,
    rows: &[TableSurfaceRow],
) -> TableSurfaceContract {
    let mut canonical_order = table
        .columns
        .iter()
        .filter(|column| column.sort_order != 0)
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let row_identity = if canonical_order.is_empty() {
        table
            .columns
            .first()
            .map(|column| vec![column.name.clone()])
            .unwrap_or_else(|| vec!["__row_number".into()])
    } else {
        canonical_order.clone()
    };
    if canonical_order.is_empty() {
        canonical_order = row_identity.clone();
    }
    TableSurfaceContract {
        table_id: format!("cove-t:{}", table.table_id),
        table_name: table.name.clone(),
        contract_version: COVEQL_PROFILE_CONTRACT_VERSION.into(),
        authority_kind: TableSurfaceAuthorityKind::RawTable,
        authority_fingerprint: table_authority_fingerprint(table, rows),
        schema_fingerprint: table_schema_fingerprint(table),
        logical_column_map: table
            .columns
            .iter()
            .map(|column| TableSurfaceColumnContract {
                name: column.name.clone(),
                logical_type: Some(format!("{:?}", column.logical)),
                nullable: column.nullable,
                source_path: Some(column.name.clone()),
                code_domain: (column.physical == CovePhysicalKind::FileCode)
                    .then_some(format!("cove-t:{}:{}", table.table_id, column.column_id)),
                collation: (column.collation_id != 0).then_some(column.collation_id.to_string()),
            })
            .collect(),
        row_grain: "cove_t_table_row".into(),
        row_identity,
        canonical_order,
        visibility_authority: "cove_t_visible_rows".into(),
        redaction_authority: "cove_t_dictionary_redaction".into(),
        temporal_authority: TableTemporalAuthority::StaticTableSnapshot,
        evidence_capabilities: vec![AstEvidenceGrain::Row],
        null_missing_nan_policy: "cove_t_null_bitmap".into(),
        collation_policy: "declared_cove_t_collation_or_binary".into(),
        code_domain_contexts: Vec::new(),
        code_domain_bridges: Vec::new(),
        projection_dependency_contract_id: None,
        datafusion_interop_contract: Some("cove_t_materialized_beginner_rows".into()),
    }
}

fn table_schema_fingerprint(table: &TableEntry) -> String {
    let mut hasher = Sha256::new();
    hasher.update(table.table_id.to_le_bytes());
    hasher.update(table.namespace.as_bytes());
    hasher.update([0]);
    hasher.update(table.name.as_bytes());
    for column in &table.columns {
        hasher.update(column.column_id.to_le_bytes());
        hasher.update(column.name.as_bytes());
        hasher.update([
            column.logical as u8,
            column.physical as u8,
            u8::from(column.nullable),
        ]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn table_authority_fingerprint(table: &TableEntry, rows: &[TableSurfaceRow]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(table_schema_fingerprint(table).as_bytes());
    hasher.update((rows.len() as u64).to_le_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn cove_array_value_to_json(
    logical: CoveLogicalType,
    value: CoveArrayValue<'_>,
) -> Result<Value, CoveError> {
    match value {
        CoveArrayValue::Null => Ok(Value::Null),
        CoveArrayValue::Boolean(value) | CoveArrayValue::ValidityBit(value) => Ok(json!(value)),
        CoveArrayValue::NumCode(value) => numcode_to_json(logical, value),
        CoveArrayValue::Varint(value) => Ok(json!(value)),
        CoveArrayValue::Int64(value) => Ok(json!(value)),
        CoveArrayValue::Bytes(bytes) => bytes_to_json(logical, bytes),
        CoveArrayValue::OwnedBytes(bytes) => bytes_to_json(logical, &bytes),
        CoveArrayValue::FileCode(_) => Err(CoveError::BadFileCode),
        CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => Err(CoveError::BadFileCode),
        CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
            if matches!(logical, CoveLogicalType::Bool) {
                return Err(CoveError::UnsupportedEncoding(
                    "dictionary boolean values need their dictionary value tag before beginner readback can expose them safely".into(),
                ));
            }
            let tag = value_tag_for_logical(logical)?;
            canonical_payload_to_json(tag, &bytes)
        }
        _ => Err(CoveError::UnsupportedEncoding(
            "future COVE array value variants are not yet exposed by beginner readback".into(),
        )),
    }
}

fn numcode_to_json(logical: CoveLogicalType, value: u64) -> Result<Value, CoveError> {
    match logical {
        CoveLogicalType::Bool => match value {
            0 => Ok(json!(false)),
            1 => Ok(json!(true)),
            _ => Err(CoveError::PageCorrupt),
        },
        CoveLogicalType::Int8 => Ok(json!(numcode_as_i8(value))),
        CoveLogicalType::Int16 => Ok(json!(numcode_as_i16(value))),
        CoveLogicalType::Int32 => Ok(json!(numcode_as_i32(value))),
        CoveLogicalType::Int64 => Ok(json!(numcode_as_i64(value))),
        CoveLogicalType::UInt8 => Ok(json!(numcode_as_u8(value))),
        CoveLogicalType::UInt16 => Ok(json!(numcode_as_u16(value))),
        CoveLogicalType::UInt32 => Ok(json!(numcode_as_u32(value))),
        CoveLogicalType::UInt64 => Ok(json!(numcode_as_u64(value))),
        CoveLogicalType::Float32 => finite_float_json(f64::from(numcode_as_f32(value))),
        CoveLogicalType::Float64 => finite_float_json(numcode_as_f64(value)),
        CoveLogicalType::Decimal64 => Ok(Value::String(numcode_as_decimal64(value).to_string())),
        CoveLogicalType::DateDays => Ok(json!(numcode_as_date_days(value))),
        CoveLogicalType::TimestampMicros => Ok(json!(numcode_as_timestamp_micros(value))),
        CoveLogicalType::TimestampNanos => Ok(json!(numcode_as_timestamp_nanos(value))),
        _ => Err(CoveError::BadSchema(format!(
            "NumCode cannot materialize logical type {logical:?}"
        ))),
    }
}

fn bytes_to_json(logical: CoveLogicalType, bytes: &[u8]) -> Result<Value, CoveError> {
    match logical {
        CoveLogicalType::Bool => match bytes {
            [0] => Ok(json!(false)),
            [1] => Ok(json!(true)),
            _ => Err(CoveError::PageCorrupt),
        },
        CoveLogicalType::Int8 => fixed_i8(bytes).map(|value| json!(value)),
        CoveLogicalType::Int16 => fixed_i16(bytes).map(|value| json!(value)),
        CoveLogicalType::Int32 => fixed_i32(bytes).map(|value| json!(value)),
        CoveLogicalType::Int64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => fixed_i64(bytes).map(|value| json!(value)),
        CoveLogicalType::UInt8 => fixed_u8(bytes).map(|value| json!(value)),
        CoveLogicalType::UInt16 => fixed_u16(bytes).map(|value| json!(value)),
        CoveLogicalType::UInt32 => fixed_u32(bytes).map(|value| json!(value)),
        CoveLogicalType::UInt64 => fixed_u64(bytes).map(|value| json!(value)),
        CoveLogicalType::Float32 => {
            fixed_u32(bytes).and_then(|bits| finite_float_json(f64::from(f32::from_bits(bits))))
        }
        CoveLogicalType::Float64 => {
            fixed_u64(bytes).and_then(|bits| finite_float_json(f64::from_bits(bits)))
        }
        CoveLogicalType::Decimal64 => {
            fixed_i64(bytes).map(|value| Value::String(value.to_string()))
        }
        CoveLogicalType::Decimal128 => {
            fixed_i128(bytes).map(|value| Value::String(value.to_string()))
        }
        CoveLogicalType::DateDays => fixed_i32(bytes).map(|value| json!(value)),
        CoveLogicalType::Uuid => {
            if bytes.len() != 16 {
                return Err(CoveError::PageCorrupt);
            }
            Ok(Value::String(hex_encode(bytes)))
        }
        CoveLogicalType::Utf8 => std::str::from_utf8(bytes)
            .map(|value| Value::String(value.to_string()))
            .map_err(|_| CoveError::PageCorrupt),
        CoveLogicalType::Binary => Ok(Value::String(hex_encode(bytes))),
        CoveLogicalType::Json => serde_json::from_slice(bytes).map_err(|_| CoveError::PageCorrupt),
        CoveLogicalType::Null => Ok(Value::Null),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            Err(CoveError::UnsupportedEncoding(
                "native nested COVE-T values are not yet exposed by beginner readback".into(),
            ))
        }
        _ => Err(CoveError::UnsupportedEncoding(
            "future COVE logical types are not yet exposed by beginner readback".into(),
        )),
    }
}

fn canonical_payload_to_json(value_tag: ValueTag, bytes: &[u8]) -> Result<Value, CoveError> {
    match value_tag {
        ValueTag::Null => Ok(Value::Null),
        ValueTag::BoolFalse => Ok(Value::Bool(false)),
        ValueTag::BoolTrue => Ok(Value::Bool(true)),
        ValueTag::Int64 | ValueTag::TimestampMicros | ValueTag::TimestampNanos => {
            fixed_i64(bytes).map(|value| json!(value))
        }
        ValueTag::UInt64 => fixed_u64(bytes).map(|value| json!(value)),
        ValueTag::Float32Bits => {
            fixed_u32(bytes).and_then(|bits| finite_float_json(f64::from(f32::from_bits(bits))))
        }
        ValueTag::Float64Bits => {
            fixed_u64(bytes).and_then(|bits| finite_float_json(f64::from_bits(bits)))
        }
        ValueTag::Decimal64 => fixed_i64(bytes).map(|value| Value::String(value.to_string())),
        ValueTag::Decimal128 => fixed_i128(bytes).map(|value| Value::String(value.to_string())),
        ValueTag::DateDays => fixed_i32(bytes).map(|value| json!(value)),
        ValueTag::Uuid => {
            if bytes.len() != 16 {
                return Err(CoveError::BadFileCode);
            }
            Ok(Value::String(hex_encode(bytes)))
        }
        ValueTag::Utf8 => {
            let payload = decode_canonical_length_prefixed(bytes)?;
            std::str::from_utf8(payload)
                .map(|value| Value::String(value.to_string()))
                .map_err(|_| CoveError::BadFileCode)
        }
        ValueTag::Binary => {
            let payload = decode_canonical_length_prefixed(bytes)?;
            Ok(Value::String(hex_encode(payload)))
        }
        ValueTag::Json => {
            let payload = decode_canonical_length_prefixed(bytes)?;
            serde_json::from_slice(payload).map_err(|_| CoveError::BadFileCode)
        }
        ValueTag::List | ValueTag::Struct | ValueTag::Map => Err(CoveError::UnsupportedEncoding(
            "nested dictionary values are not yet exposed by beginner readback".into(),
        )),
        _ => Err(CoveError::UnsupportedEncoding(
            "future COVE value tags are not yet exposed by beginner readback".into(),
        )),
    }
}

fn value_tag_for_logical(logical: CoveLogicalType) -> Result<ValueTag, CoveError> {
    Ok(match logical {
        CoveLogicalType::Null => ValueTag::Null,
        CoveLogicalType::Bool => ValueTag::BoolTrue,
        CoveLogicalType::Int8
        | CoveLogicalType::Int16
        | CoveLogicalType::Int32
        | CoveLogicalType::Int64 => ValueTag::Int64,
        CoveLogicalType::UInt8
        | CoveLogicalType::UInt16
        | CoveLogicalType::UInt32
        | CoveLogicalType::UInt64 => ValueTag::UInt64,
        CoveLogicalType::Float32 => ValueTag::Float32Bits,
        CoveLogicalType::Float64 => ValueTag::Float64Bits,
        CoveLogicalType::Decimal64 => ValueTag::Decimal64,
        CoveLogicalType::Decimal128 => ValueTag::Decimal128,
        CoveLogicalType::DateDays => ValueTag::DateDays,
        CoveLogicalType::TimestampMicros => ValueTag::TimestampMicros,
        CoveLogicalType::TimestampNanos => ValueTag::TimestampNanos,
        CoveLogicalType::Utf8 => ValueTag::Utf8,
        CoveLogicalType::Binary => ValueTag::Binary,
        CoveLogicalType::Uuid => ValueTag::Uuid,
        CoveLogicalType::Json => ValueTag::Json,
        CoveLogicalType::List => ValueTag::List,
        CoveLogicalType::Struct => ValueTag::Struct,
        CoveLogicalType::Map => ValueTag::Map,
        _ => {
            return Err(CoveError::UnsupportedEncoding(
                "future COVE logical types are not yet exposed by beginner readback".into(),
            ))
        }
    })
}

fn decode_canonical_length_prefixed(bytes: &[u8]) -> Result<&[u8], CoveError> {
    let (len, consumed) = wire::decode_u64_leb128(bytes)?;
    let len = usize::try_from(len).map_err(|_| CoveError::ArithOverflow)?;
    let start = consumed;
    let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end != bytes.len() {
        return Err(CoveError::BadFileCode);
    }
    Ok(&bytes[start..end])
}

fn finite_float_json(value: f64) -> Result<Value, CoveError> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or(CoveError::PageCorrupt)
}

fn fixed_i8(bytes: &[u8]) -> Result<i8, CoveError> {
    if bytes.len() != 1 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(i8::from_le_bytes([bytes[0]]))
}

fn fixed_u8(bytes: &[u8]) -> Result<u8, CoveError> {
    if bytes.len() != 1 {
        return Err(CoveError::PageCorrupt);
    }
    Ok(bytes[0])
}

fn fixed_i16(bytes: &[u8]) -> Result<i16, CoveError> {
    Ok(i16::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_u16(bytes: &[u8]) -> Result<u16, CoveError> {
    Ok(u16::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_i32(bytes: &[u8]) -> Result<i32, CoveError> {
    Ok(i32::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_u32(bytes: &[u8]) -> Result<u32, CoveError> {
    Ok(u32::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_i64(bytes: &[u8]) -> Result<i64, CoveError> {
    Ok(i64::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_u64(bytes: &[u8]) -> Result<u64, CoveError> {
    Ok(u64::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_i128(bytes: &[u8]) -> Result<i128, CoveError> {
    Ok(i128::from_le_bytes(fixed_bytes(bytes)?))
}

fn fixed_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CoveError> {
    if bytes.len() != N {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_core::writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment};

    fn cove_t_events_bytes() -> Vec<u8> {
        let catalog = TableCatalog {
            flags: 0,
            tables: vec![TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 3,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![
                    ColumnEntry {
                        column_id: 1,
                        name: "id".into(),
                        logical: CoveLogicalType::Int64,
                        physical: CovePhysicalKind::NumCode,
                        nullable: false,
                        sort_order: 0,
                        collation_id: 0,
                        precision: 0,
                        scale: 0,
                        flags: 0,
                    },
                    ColumnEntry {
                        column_id: 2,
                        name: "score".into(),
                        logical: CoveLogicalType::Int64,
                        physical: CovePhysicalKind::NumCode,
                        nullable: false,
                        sort_order: 0,
                        collation_id: 0,
                        precision: 0,
                        scale: 0,
                        flags: 0,
                    },
                ],
            }],
        };
        let mut ids = Vec::new();
        let mut scores = Vec::new();
        for (id, score) in [(1u64, 10u64), (2, 20), (3, 30)] {
            ids.extend_from_slice(&id.to_le_bytes());
            scores.extend_from_slice(&score.to_le_bytes());
        }
        let mut segment = ScanSegment::new(1, 0, 0, 3, 2);
        segment.set_column_pages(
            1,
            vec![ScanPageSpec::new(3, ids).with_encoding_root(CoveEncodingKind::NumCode as u32)],
        );
        segment.set_column_pages(
            2,
            vec![ScanPageSpec::new(3, scores).with_encoding_root(CoveEncodingKind::NumCode as u32)],
        );
        let mut writer = ScanProfileCoveWriter::new(catalog);
        writer.push_segment(segment);
        writer.write().unwrap()
    }

    #[test]
    fn cove_t_discovery_suggests_table_queries() {
        let bytes = cove_t_events_bytes();
        let discovery = discover_query_surfaces(
            &bytes,
            QuerySurfaceDiscoveryOptions {
                source_name: Some("events.cove".into()),
            },
        );

        assert!(discovery.queryable);
        assert_eq!(discovery.tables.len(), 1);
        assert_eq!(discovery.tables[0].table_name, "events");
        let suggestions = suggest_queries(&discovery);
        assert!(suggestions
            .iter()
            .any(|suggestion| suggestion.query == "table(events).select(id, score).take(10)"));
    }

    #[test]
    fn cove_t_artifact_query_executes_materialized_table_rows() {
        let bytes = cove_t_events_bytes();
        let executed = execute_query_from_artifact(
            &bytes,
            "table(events).where(score >= 20).select(id, score)",
            ExecuteArtifactOptions::default(),
        )
        .unwrap();

        assert_eq!(
            executed.result_json().unwrap(),
            json!([
                {"id": 2, "score": 20},
                {"id": 3, "score": 30}
            ])
        );
    }
}
