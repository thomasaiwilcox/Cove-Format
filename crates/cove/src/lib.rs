//! Canonical Rust facade for COVE v2.
//!
//! This crate is the recommended starting point for application developers.
//! It re-exports focused reader/writer/conversion facades and provides typed
//! workflow helpers for common validation, inspection, conversion, query, and
//! explain operations.

use std::{error::Error, fmt, fs, path::Path};

pub use cove_convert as convert;
use cove_core::query_discovery::build_query_discovery_manifest_value;
pub use cove_core::{
    artifact,
    constants::{self, DigestAlgorithm},
    digest::compute_digest,
    footer, header,
    mount::{self, MountOptions, MountedCoveFile},
    postscript, profile,
    query_discovery::{
        build_query_discovery_manifest, embedded_query_discovery_manifests,
        query_discovery_section_payload, query_discovery_validation_context_for_embedded_source,
        query_discovery_validation_context_for_source, render_query_discovery_template,
        QueryAiCapability, QueryDiscoveryAliasBinding, QueryDiscoveryBudget,
        QueryDiscoveryCoveQlContract, QueryDiscoveryIdentifier, QueryDiscoveryManifest,
        QueryDiscoveryOptions, QueryDiscoveryPolicy, QueryDiscoveryPropertyGlossaryEntry,
        QueryDiscoveryRelationship, QueryDiscoverySourceBinding, QueryDiscoverySourceMember,
        QueryDiscoverySurface, QueryDiscoveryValidationReport, QueryExample,
        QueryExampleValidationStatus, QueryTemplate, QueryTemplateOperatorChain,
        QueryTemplateParameterConstraint, QueryTemplateValidationStatus,
    },
    reader::{self, ValidatedCoveFile},
    table,
    utility::hex_encode,
    writer::{self, ScanProfileCoveWriter},
    CoveError,
};
pub use cove_reader as read;
pub use cove_writer as write;
pub use coveql::{
    ExecuteArtifactOptions, ExecuteArtifactQueryError, ExecutedQuery, ExplainDisclosurePolicy,
    ExplainMode, QuerySuggestion, QuerySurfaceDiscovery, QuerySurfaceDiscoveryOptions,
};

pub mod engine {
    pub use cove_engine::*;
}

pub mod prelude {
    pub use crate::{
        conversion_report, convert_file, convert_parquet_file, discover_query_surfaces_file,
        explain_query_file, inspect_file, query_discovery_manifest_file, query_file, read_table,
        register_datafusion, suggest_queries_for_file, validate_file, write_table, CoveFacadeError,
        ExplainOptions, ExplainReport, FileInspection, PreparedQueryTextOptions, QueryOptions,
        QueryResult, QueryTextError,
    };
    pub use coveql::ExplainMode;
}

pub mod query {
    pub use coveql::{
        ArtifactExecutionEngine, ExecuteArtifactOptions, ExecuteArtifactQueryError, ExecutedQuery,
        ExplainDisclosurePolicy, ExplainMode, QueryArtifactMember, QuerySuggestion,
        QuerySurfaceDiscovery, QuerySurfaceDiscoveryOptions,
    };
}

pub use cove_engine::register_datafusion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInspection {
    pub version_major: u16,
    pub version_minor: u16,
    pub primary_profile: u8,
    pub required_features: u64,
    pub optional_features: u64,
    pub section_count: usize,
    pub table_count: usize,
    pub row_count: u64,
}

#[derive(Debug)]
pub enum CoveFacadeError {
    Io(std::io::Error),
    Cove(CoveError),
    Query(ExecuteArtifactQueryError),
    QueryText(QueryTextError),
    Result(coveql::BuildExecutionError),
}

impl fmt::Display for CoveFacadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Cove(error) => write!(f, "{error}"),
            Self::Query(error) => write!(f, "{error}"),
            Self::QueryText(error) => write!(f, "{error}"),
            Self::Result(error) => write!(f, "{error}"),
        }
    }
}

impl Error for CoveFacadeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Cove(error) => Some(error),
            Self::Query(error) => Some(error),
            Self::QueryText(error) => Some(error),
            Self::Result(error) => Some(error),
        }
    }
}

impl From<std::io::Error> for CoveFacadeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CoveError> for CoveFacadeError {
    fn from(error: CoveError) -> Self {
        Self::Cove(error)
    }
}

impl From<ExecuteArtifactQueryError> for CoveFacadeError {
    fn from(error: ExecuteArtifactQueryError) -> Self {
        Self::Query(error)
    }
}

impl From<QueryTextError> for CoveFacadeError {
    fn from(error: QueryTextError) -> Self {
        Self::QueryText(error)
    }
}

impl From<coveql::BuildExecutionError> for CoveFacadeError {
    fn from(error: coveql::BuildExecutionError) -> Self {
        Self::Result(error)
    }
}

#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub execute: ExecuteArtifactOptions,
    pub take: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub executed: ExecutedQuery,
}

impl QueryResult {
    /// Return the query result payload as JSON.
    ///
    /// # Errors
    ///
    /// Returns a [`CoveFacadeError`] if the executed query result cannot be
    /// represented as the facade JSON result shape.
    pub fn result_json(&self) -> Result<serde_json::Value, CoveFacadeError> {
        self.executed.result_json().map_err(CoveFacadeError::from)
    }

    pub fn explain_json(&self) -> serde_json::Value {
        self.executed.explain_json()
    }

    pub fn explain_text(&self) -> String {
        self.executed.explain_text()
    }
}

#[derive(Debug, Clone)]
pub struct ExplainOptions {
    pub query: QueryOptions,
    pub mode: ExplainMode,
}

impl Default for ExplainOptions {
    fn default() -> Self {
        Self {
            query: QueryOptions::default(),
            mode: ExplainMode::Public,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExplainReport {
    pub executed: ExecutedQuery,
    pub json: serde_json::Value,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PreparedQueryTextOptions {
    pub take: Option<usize>,
    pub explain: Option<ExplainMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryTextError {
    NonPositiveTake,
}

impl fmt::Display for QueryTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonPositiveTake => f.write_str("take requires a positive integer"),
        }
    }
}

impl Error for QueryTextError {}

/// Read and validate a COVE file for ordinary facade use.
///
/// # Errors
///
/// Returns a [`CoveError`] if the file cannot be read or validation rejects its
/// wire layout, required features, table catalog, or semantic metadata.
pub fn validate_file(path: impl AsRef<Path>) -> Result<ValidatedCoveFile, CoveError> {
    reader::read_file(path.as_ref())
}

/// Inspect mounted COVE file metadata without returning the mounted sections.
///
/// # Errors
///
/// Returns a [`CoveError`] if the file cannot be read or mounting rejects the
/// header, footer, section directory, tables, or required feature set.
pub fn inspect_file(path: impl AsRef<Path>) -> Result<FileInspection, CoveError> {
    let bytes = fs::read(path)?;
    let mounted = mount::mount_cove_file(&bytes, MountOptions::default(), None)?;
    Ok(FileInspection {
        version_major: mounted.header.version_major,
        version_minor: mounted.header.version_minor,
        primary_profile: mounted.header.primary_profile,
        required_features: mounted.header.required_features,
        optional_features: mounted.header.optional_features,
        section_count: mounted.footer.sections.len(),
        table_count: mounted.tables.len(),
        row_count: mounted.tables.iter().map(|table| table.row_count).sum(),
    })
}

/// Mount a COVE file for table-oriented reads.
///
/// # Errors
///
/// Returns a [`CoveError`] if the file cannot be read or the mount process
/// rejects the COVE header, footer, sections, or table catalog.
pub fn read_table(path: impl AsRef<Path>) -> Result<MountedCoveFile, CoveError> {
    let bytes = fs::read(path)?;
    mount::mount_cove_file(&bytes, MountOptions::default(), None)
}

/// Durably publish a scan-profile COVE table using the supplied writer.
///
/// # Errors
///
/// Returns a [`CoveError`] if the writer cannot serialize a valid COVE table or
/// durable publication to the target path fails.
pub fn write_table(
    path: impl AsRef<Path>,
    writer: &ScanProfileCoveWriter,
) -> Result<(), CoveError> {
    writer.publish_durable(path.as_ref()).map(|_| ())
}

/// Convert a supported source file into a COVE conversion result.
///
/// # Errors
///
/// Returns a [`CoveError`] if the source cannot be read, its schema cannot be
/// converted into the COVE model, or the conversion result cannot be produced.
pub fn convert_file(
    input: impl AsRef<Path>,
    options: convert::ConversionOptions,
) -> Result<convert::ConversionResult, CoveError> {
    convert::convert_file_to_cove(input, options)
        .map_err(|err| CoveError::BadSchema(err.to_string()))
}

/// Convert a Parquet source file into a COVE conversion result.
///
/// # Errors
///
/// Returns a [`CoveError`] if the Parquet file cannot be read, digested, parsed,
/// or converted into the requested COVE layout.
pub fn convert_parquet_file(
    input: impl AsRef<Path>,
    mut options: convert::convert::ParquetConversionOptions,
) -> Result<convert::ConversionResult, CoveError> {
    let input = input.as_ref();
    let bytes = fs::read(input)?;
    options.source_identifier = Some(input.display().to_string());
    options.source_digest = Some(format!(
        "sha256:{}",
        hex_encode(&compute_digest(DigestAlgorithm::Sha256, &bytes)?)
    ));
    convert::convert::convert_parquet_bytes(&bytes, &options)
}

pub fn conversion_report(result: &convert::convert::ParquetConversionResult) -> serde_json::Value {
    result.report.to_json_value()
}

/// Discover CoveQL query surfaces from a COVE artifact file.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the file cannot be read before surface
/// discovery is performed.
pub fn discover_query_surfaces_file(
    path: impl AsRef<Path>,
) -> Result<QuerySurfaceDiscovery, CoveFacadeError> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    Ok(coveql::discover_query_surfaces(
        &bytes,
        QuerySurfaceDiscoveryOptions {
            source_name: Some(path.display().to_string()),
        },
    ))
}

pub fn discover_query_surfaces_bytes(bytes: &[u8]) -> QuerySurfaceDiscovery {
    coveql::discover_query_surfaces(bytes, QuerySurfaceDiscoveryOptions::default())
}

/// Build a canonical COVE-QD query-discovery manifest for a COVE artifact file.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the file cannot be read or the manifest
/// cannot be generated from canonical metadata.
pub fn query_discovery_manifest_file(
    path: impl AsRef<Path>,
    mut options: QueryDiscoveryOptions,
) -> Result<QueryDiscoveryManifest, CoveFacadeError> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    if options.source_name.is_none() {
        options.source_name = Some(path.display().to_string());
    }
    build_facade_query_discovery_manifest(&bytes, options)
}

/// Build a canonical COVE-QD query-discovery manifest from artifact bytes.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the manifest cannot be generated from
/// canonical metadata or validated after example planning.
pub fn query_discovery_manifest_bytes(
    bytes: &[u8],
    options: QueryDiscoveryOptions,
) -> Result<QueryDiscoveryManifest, CoveFacadeError> {
    build_facade_query_discovery_manifest(bytes, options)
}

fn build_facade_query_discovery_manifest(
    bytes: &[u8],
    options: QueryDiscoveryOptions,
) -> Result<QueryDiscoveryManifest, CoveFacadeError> {
    if !options.validate_examples {
        return Ok(build_query_discovery_manifest(bytes, options)?);
    }
    let mut build_options = options;
    build_options.validate_examples = false;
    let mut value = build_query_discovery_manifest_value(bytes, &build_options)?;
    validate_query_discovery_examples(bytes, &mut value);
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CoveError::QueryDiscoveryInvalid(error.to_string()))?;
    Ok(QueryDiscoveryManifest::parse(&canonical)?)
}

fn validate_query_discovery_examples(bytes: &[u8], manifest: &mut serde_json::Value) {
    let Some(examples) = manifest
        .get_mut("examples")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    let mut diagnostics = Vec::new();
    for (index, example) in examples.iter_mut().enumerate() {
        let Some(query) = example.get("query").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let validation = coveql::parse_resolve_and_plan_query(
            bytes,
            query,
            coveql::ParseOptions::default(),
            coveql::ResolveOptions::default(),
            coveql::PlanOptions::default(),
            reader::ValidationOptions::default(),
        );
        if validation.is_ok() {
            example["query_validation"] = serde_json::json!("planned_dry_run");
        } else {
            example["query_validation"] = serde_json::json!("not_validated");
            diagnostics.push(serde_json::json!({
                "code": "QD_EXAMPLE_VALIDATION_FAILED",
                "severity": "warning",
                "message": "Generated example could not be validated by a no-payload planning dry-run.",
                "target_kind": "json_pointer",
                "target": format!("/examples/{index}"),
                "withheld": false
            }));
        }
    }
    if diagnostics.is_empty() {
        return;
    }
    let manifest_diagnostics = manifest
        .as_object_mut()
        .expect("query-discovery manifest value must be an object")
        .entry("diagnostics")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let Some(existing) = manifest_diagnostics.as_array_mut() {
        existing.extend(diagnostics);
    }
}

/// Suggest CoveQL queries for the surfaces discovered in a COVE artifact file.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the file cannot be read before query
/// surface discovery and suggestion generation.
pub fn suggest_queries_for_file(
    path: impl AsRef<Path>,
) -> Result<Vec<QuerySuggestion>, CoveFacadeError> {
    let discovery = discover_query_surfaces_file(path)?;
    Ok(coveql::suggest_queries(&discovery))
}

pub fn suggest_queries_for_bytes(bytes: &[u8]) -> Vec<QuerySuggestion> {
    let discovery = discover_query_surfaces_bytes(bytes);
    coveql::suggest_queries(&discovery)
}

/// Execute a CoveQL query against a COVE artifact file.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the file cannot be read, query text
/// preparation fails, or CoveQL parsing, planning, policy resolution, or
/// execution fails.
pub fn query_file(
    path: impl AsRef<Path>,
    query: impl AsRef<str>,
    options: QueryOptions,
) -> Result<QueryResult, CoveFacadeError> {
    let bytes = fs::read(path)?;
    query_bytes(&bytes, query, options)
}

/// Execute a CoveQL query against in-memory COVE artifact bytes.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if query text preparation fails or CoveQL
/// parsing, planning, policy resolution, or execution fails.
pub fn query_bytes(
    bytes: &[u8],
    query: impl AsRef<str>,
    options: QueryOptions,
) -> Result<QueryResult, CoveFacadeError> {
    let query = prepare_query_text(
        query.as_ref(),
        PreparedQueryTextOptions {
            take: options.take,
            explain: None,
        },
    )?;
    let executed = coveql::execute_query_from_artifact(bytes, &query, options.execute)?;
    Ok(QueryResult { executed })
}

/// Execute a CoveQL explain query against a COVE artifact file.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if the file cannot be read, query text
/// preparation fails, or CoveQL explain execution fails.
pub fn explain_query_file(
    path: impl AsRef<Path>,
    query: impl AsRef<str>,
    options: ExplainOptions,
) -> Result<ExplainReport, CoveFacadeError> {
    let bytes = fs::read(path)?;
    explain_query_bytes(&bytes, query, options)
}

/// Execute a CoveQL explain query against in-memory COVE artifact bytes.
///
/// # Errors
///
/// Returns a [`CoveFacadeError`] if query text preparation fails or CoveQL
/// parsing, planning, policy resolution, or explain execution fails.
pub fn explain_query_bytes(
    bytes: &[u8],
    query: impl AsRef<str>,
    mut options: ExplainOptions,
) -> Result<ExplainReport, CoveFacadeError> {
    let query = prepare_query_text(
        query.as_ref(),
        PreparedQueryTextOptions {
            take: options.query.take,
            explain: Some(options.mode),
        },
    )?;
    options
        .query
        .execute
        .resolve_options
        .security
        .explain_policy = explain_policy_for_mode(options.mode);
    let executed = coveql::execute_query_from_artifact(bytes, &query, options.query.execute)?;
    let json = executed.explain_json();
    let text = coveql::render_explain_text(&json);
    Ok(ExplainReport {
        executed,
        json,
        text,
    })
}

pub fn explain_policy_for_mode(mode: ExplainMode) -> ExplainDisclosurePolicy {
    match mode {
        ExplainMode::Developer => ExplainDisclosurePolicy::Developer,
        ExplainMode::Proof | ExplainMode::Coded | ExplainMode::Ai => ExplainDisclosurePolicy::Proof,
        ExplainMode::Forensic => ExplainDisclosurePolicy::Forensic,
        ExplainMode::Public => ExplainDisclosurePolicy::PublicOnly,
    }
}

/// Prepare user query text by applying facade-level `take` and explain options.
///
/// # Errors
///
/// Returns a [`QueryTextError`] if the requested query text options are invalid,
/// such as a non-positive `take` value.
pub fn prepare_query_text(
    query: &str,
    options: PreparedQueryTextOptions,
) -> Result<String, QueryTextError> {
    let mut text = query.trim().to_string();
    if let Some(take) = options.take {
        if take == 0 {
            return Err(QueryTextError::NonPositiveTake);
        }
        if !text.contains(".take(") {
            text.push_str(&format!(".take({take})"));
        }
    }
    if let Some(mode) = options.explain {
        if !text.contains(".explain(") {
            text.push_str(&format!(".explain(\"{}\")", mode.as_str()));
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str, extension: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}_{}_{}.{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        );
        std::env::temp_dir().join(unique)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn facade_exposes_core_header_constants() {
        assert_eq!(constants::VERSION_MAJOR_V1, 2);
    }

    #[test]
    fn facade_validates_and_inspects_written_file() {
        let path = temp_path("cove_facade_empty", "cove");
        std::fs::write(
            &path,
            writer::MinimalCoveWriter::write_empty_file().unwrap(),
        )
        .unwrap();
        let validated = validate_file(&path).unwrap();
        assert_eq!(validated.header.version_major, 2);
        let inspection = inspect_file(&path).unwrap();
        assert_eq!(inspection.section_count, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn write_table_publishes_readable_file() {
        let path = temp_path("cove_write_table", "cove");
        let catalog = table::TableCatalog {
            flags: 0,
            tables: vec![table::TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 1,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![table::ColumnEntry {
                    column_id: 1,
                    name: "active".into(),
                    logical: constants::CoveLogicalType::Bool,
                    physical: constants::CovePhysicalKind::Boolean,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                }],
            }],
        };
        let mut writer = ScanProfileCoveWriter::new(catalog);
        writer.push_segment(writer::ScanSegment::new(1, 0, 0, 1, 1));

        write_table(&path, &writer).unwrap();

        let validated = validate_file(&path).unwrap();
        assert_eq!(validated.header.version_major, 2);
        let mounted = read_table(&path).unwrap();
        assert_eq!(mounted.tables.len(), 1);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn facade_converts_csv_through_generic_convert_file() {
        let path = temp_path("cove_facade_people", "csv");
        std::fs::write(&path, "id,name\n1,Ada\n2,Linus\n").unwrap();
        let result = convert_file(
            &path,
            convert::ConversionOptions {
                source_format: Some(convert::SourceFormat::Csv),
                ..convert::ConversionOptions::default()
            },
        )
        .unwrap();
        assert!(result.report.validation_result);
        assert_eq!(result.report.source_identifier, path.display().to_string());
        assert!(result.report.source_digest.starts_with("sha256:"));
        assert_eq!(result.report.row_count, 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn facade_queries_and_explains_converted_csv() {
        let dir = temp_dir("cove_facade_query");
        std::fs::create_dir_all(&dir).unwrap();
        let csv_path = dir.join("cove_facade_people.csv");
        let cove_path = dir.join("cove_facade_people.cove");
        std::fs::write(&csv_path, "id,name\n1,Ada\n2,Linus\n").unwrap();
        let result = convert_file(
            &csv_path,
            convert::ConversionOptions {
                source_format: Some(convert::SourceFormat::Csv),
                ..convert::ConversionOptions::default()
            },
        )
        .unwrap();
        std::fs::write(&cove_path, &result.cove_bytes).unwrap();

        let queried = query_file(
            &cove_path,
            "table(cove_facade_people).select(id, name)",
            QueryOptions {
                take: Some(1),
                ..QueryOptions::default()
            },
        )
        .unwrap();
        let rows = queried.result_json().unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);

        let explained = explain_query_file(
            &cove_path,
            "table(cove_facade_people).select(id, name)",
            ExplainOptions {
                mode: ExplainMode::Developer,
                ..ExplainOptions::default()
            },
        )
        .unwrap();
        assert!(explained.json.is_object());
        assert!(!explained.text.trim().is_empty());

        let discovery = discover_query_surfaces_file(&cove_path).unwrap();
        assert!(discovery.queryable);
        let suggestions = suggest_queries_for_file(&cove_path).unwrap();
        assert!(!suggestions.is_empty());

        let _ = std::fs::remove_file(csv_path);
        let _ = std::fs::remove_file(cove_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn query_discovery_does_not_create_roots_absent_from_canonical_metadata() {
        let dir = temp_dir("cove_facade_qd_lie");
        std::fs::create_dir_all(&dir).unwrap();
        let csv_path = dir.join("events.csv");
        let cove_path = dir.join("events.cove");
        std::fs::write(&csv_path, "id,name\n1,Ada\n").unwrap();
        let result = convert_file(
            &csv_path,
            convert::ConversionOptions {
                source_format: Some(convert::SourceFormat::Csv),
                ..convert::ConversionOptions::default()
            },
        )
        .unwrap();
        std::fs::write(&cove_path, &result.cove_bytes).unwrap();

        let manifest =
            query_discovery_manifest_file(&cove_path, QueryDiscoveryOptions::default()).unwrap();
        let mut value = manifest.value().clone();
        value["surfaces"]["tables"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "fake",
                "query_name": "fake",
                "query_identifier": "fake",
                "display_name": "fake",
                "root": "table(fake)",
                "columns": [{
                    "name": "id",
                    "query_name": "id",
                    "query_identifier": "id",
                    "display_name": "id",
                    "logical_type": "int64",
                    "nullable": false,
                    "operations": ["select", "filter"]
                }]
            }));
        value["templates"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id": "lying_table",
                "profiles": ["table"],
                "binding_mode": "typed_ast_fragments",
                "template_display": "table({table}).where({predicate}).select({columns}).take({limit})",
                "operator_chain": [
                    {"method": "root", "kind": "table", "arg": "{table}"},
                    {"method": "where", "arg": "{predicate}"},
                    {"method": "select", "arg": "{columns}"},
                    {"method": "take", "arg": "{limit}"}
                ],
                "template_validation": "operator_chain_validated",
                "parameters": [
                    {
                        "name": "table",
                        "kind": "root_name",
                        "allowed_query_identifiers": ["fake"]
                    },
                    {
                        "name": "predicate",
                        "kind": "predicate",
                        "root": "table({table})",
                        "allowed_query_identifiers_by_root": {"table(fake)": ["id"]},
                        "allowed_operators": ["eq"],
                        "allowed_literal_types": ["int64"],
                        "max_depth": 1
                    },
                    {
                        "name": "columns",
                        "kind": "select_list",
                        "root": "table({table})",
                        "allowed_query_identifiers_by_root": {"table(fake)": ["id"]},
                        "max_items": 1
                    },
                    {
                        "name": "limit",
                        "kind": "uint",
                        "default": 1,
                        "max": 1
                    }
                ]
            }));
        let canonical = serde_json::to_vec(&value).unwrap();
        let lying_manifest = QueryDiscoveryManifest::parse(&canonical).unwrap();
        let query = render_query_discovery_template(
            &lying_manifest,
            "lying_table",
            &[
                ("table".to_string(), "fake".to_string()),
                ("id".to_string(), "1".to_string()),
                ("columns".to_string(), "id".to_string()),
            ],
        )
        .unwrap();

        assert!(query_file(&cove_path, &query, QueryOptions::default()).is_err());

        let _ = std::fs::remove_file(csv_path);
        let _ = std::fs::remove_file(cove_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn prepare_query_text_appends_take_and_explain_once() {
        let prepared = prepare_query_text(
            "table(people).select(id)",
            PreparedQueryTextOptions {
                take: Some(3),
                explain: Some(ExplainMode::Coded),
            },
        )
        .unwrap();
        assert_eq!(
            prepared,
            r#"table(people).select(id).take(3).explain("coded")"#
        );

        let existing = prepare_query_text(
            r#"table(people).take(5).explain("public")"#,
            PreparedQueryTextOptions {
                take: Some(3),
                explain: Some(ExplainMode::Coded),
            },
        )
        .unwrap();
        assert_eq!(existing, r#"table(people).take(5).explain("public")"#);
    }
}
