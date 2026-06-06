use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use coveql::{
    coveql_identifier, AstEvidenceGrain, ExecuteArtifactOptions, TableExecutionAuthority,
    TableSurfaceAuthority, TableSurfaceAuthorityKind, TableSurfaceColumnContract,
    TableSurfaceContract, TableSurfaceRow, TableTemporalAuthority, COVEQL_PROFILE_CONTRACT_VERSION,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalTableSpec {
    pub(crate) table_name: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn register_external_tables(
    execute_options: &mut ExecuteArtifactOptions,
    specs: &[ExternalTableSpec],
) -> Result<(), String> {
    for spec in specs {
        let rows = read_external_table_rows(&spec.path)?;
        let authority = external_table_authority(&spec.table_name, &spec.path, rows)?;
        execute_options
            .resolve_options
            .table_authorities
            .insert(spec.table_name.clone(), authority);
    }
    Ok(())
}

fn read_external_table_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => read_external_csv_rows(path),
        Some("jsonl") | Some("ndjson") => read_external_jsonl_rows(path),
        Some("json") => read_external_json_rows(path),
        _ => {
            let text = fs::read_to_string(path).map_err(|error| {
                format!("cannot read external table {}: {error}", path.display())
            })?;
            if text.trim_start().starts_with('[') || text.trim_start().starts_with('{') {
                rows_from_json_text(&text, path)
            } else {
                read_external_jsonl_text(&text, path)
            }
        }
    }
}

fn read_external_csv_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("cannot read CSV external table {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("cannot read CSV headers {}: {error}", path.display()))?
        .iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|error| format!("cannot read CSV row {}: {error}", path.display()))?;
        let mut row = BTreeMap::new();
        for (header, value) in headers.iter().zip(record.iter()) {
            row.insert(header.clone(), parse_external_csv_cell(value));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn parse_external_csv_cell(value: &str) -> Value {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Value::Number(value.into());
    }
    if let Ok(value) = trimmed.parse::<u64>() {
        return Value::Number(value.into());
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(value) {
            return Value::Number(number);
        }
    }
    Value::String(value.to_string())
}

fn read_external_json_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read JSON external table {}: {error}",
            path.display()
        )
    })?;
    rows_from_json_text(&text, path)
}

fn rows_from_json_text(text: &str, path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        format!(
            "cannot parse JSON external table {}: {error}",
            path.display()
        )
    })?;
    let rows = match value {
        Value::Array(rows) => rows,
        Value::Object(mut object) => match object.remove("rows") {
            Some(Value::Array(rows)) => rows,
            _ => {
                return Err(format!(
                "JSON external table {} must be an array of objects or an object with a rows array",
                path.display()
            ))
            }
        },
        _ => {
            return Err(format!(
                "JSON external table {} must be an array of objects",
                path.display()
            ))
        }
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, value)| value_to_table_row(value, path, index + 1))
        .collect()
}

fn read_external_jsonl_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read JSONL external table {}: {error}",
            path.display()
        )
    })?;
    read_external_jsonl_text(&text, path)
}

fn read_external_jsonl_text(text: &str, path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value = serde_json::from_str::<Value>(line).map_err(|error| {
                format!(
                    "cannot parse JSONL external table {} line {}: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            value_to_table_row(value, path, index + 1)
        })
        .collect()
}

fn value_to_table_row(
    value: Value,
    path: &Path,
    row_number: usize,
) -> Result<TableSurfaceRow, String> {
    let Value::Object(object) = value else {
        return Err(format!(
            "external table {} row {} must be a JSON object",
            path.display(),
            row_number
        ));
    };
    Ok(object.into_iter().collect::<BTreeMap<_, _>>())
}

fn external_table_authority(
    table_name: &str,
    path: &Path,
    mut rows: Vec<TableSurfaceRow>,
) -> Result<TableSurfaceAuthority, String> {
    let columns = external_table_columns(&rows);
    if columns.is_empty() {
        return Err(format!(
            "external table {table_name} from {} has no columns",
            path.display()
        ));
    }
    let row_identity = if columns.iter().any(|column| column.name == "id") {
        vec!["id".into()]
    } else {
        vec![columns[0].name.clone()]
    };
    let canonical_order = row_identity.clone();
    let provider_id = format!("external-file:{}", path.display());
    let table_id = format!("external:{}", coveql_identifier(table_name));
    for row in &mut rows {
        for column in &columns {
            row.entry(column.name.clone()).or_insert(Value::Null);
        }
    }
    Ok(TableSurfaceAuthority {
        contract: TableSurfaceContract {
            table_id: table_id.clone(),
            table_name: table_name.to_string(),
            contract_version: COVEQL_PROFILE_CONTRACT_VERSION.into(),
            authority_kind: TableSurfaceAuthorityKind::ExternalRegisteredTable,
            authority_fingerprint: external_table_fingerprint(
                table_name,
                path,
                rows.len(),
                &columns,
            ),
            schema_fingerprint: external_table_schema_fingerprint(table_name, &columns),
            logical_column_map: columns,
            row_grain: "external_file_row".into(),
            row_identity,
            canonical_order,
            visibility_authority: "external_file_visible_rows".into(),
            redaction_authority: "external_file_no_redaction_metadata".into(),
            temporal_authority: TableTemporalAuthority::StaticTableSnapshot,
            evidence_capabilities: vec![AstEvidenceGrain::Row],
            null_missing_nan_policy: "json_null_and_missing_are_null".into(),
            collation_policy: "binary_utf8".into(),
            code_domain_contexts: Vec::new(),
            code_domain_bridges: Vec::new(),
            projection_dependency_contract_id: None,
            datafusion_interop_contract: Some("cli_external_file_rows".into()),
        },
        execution_authority: TableExecutionAuthority::ExternalRows { provider_id, rows },
    })
}

fn external_table_columns(rows: &[TableSurfaceRow]) -> Vec<TableSurfaceColumnContract> {
    let mut names = BTreeSet::new();
    for row in rows {
        names.extend(row.keys().cloned());
    }
    names
        .into_iter()
        .map(|name| {
            let values = rows
                .iter()
                .map(|row| row.get(&name).unwrap_or(&Value::Null))
                .collect::<Vec<_>>();
            TableSurfaceColumnContract {
                name,
                logical_type: Some(infer_external_logical_type(&values).into()),
                nullable: values.iter().any(|value| value.is_null()),
                source_path: None,
                code_domain: None,
                collation: None,
            }
        })
        .collect()
}

fn infer_external_logical_type(values: &[&Value]) -> &'static str {
    let non_null = values
        .iter()
        .copied()
        .filter(|value| !value.is_null())
        .collect::<Vec<_>>();
    if non_null.is_empty() {
        return "null";
    }
    if non_null.iter().all(|value| value.is_boolean()) {
        return "bool";
    }
    if non_null.iter().all(|value| {
        value.as_i64().is_some()
            || value
                .as_u64()
                .is_some_and(|value| i64::try_from(value).is_ok())
    }) {
        return "int64";
    }
    if non_null.iter().all(|value| value.is_number()) {
        return "float64";
    }
    if non_null.iter().all(|value| value.is_string()) {
        return "utf8";
    }
    "json"
}

fn external_table_fingerprint(
    table_name: &str,
    path: &Path,
    row_count: usize,
    columns: &[TableSurfaceColumnContract],
) -> String {
    format!(
        "external-file:{}:{}:{}:{}",
        table_name,
        path.display(),
        row_count,
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn external_table_schema_fingerprint(
    table_name: &str,
    columns: &[TableSurfaceColumnContract],
) -> String {
    format!(
        "external-schema:{}:{}",
        table_name,
        columns
            .iter()
            .map(|column| format!(
                "{}:{}:{}",
                column.name,
                column.logical_type.as_deref().unwrap_or("unknown"),
                column.nullable
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}
