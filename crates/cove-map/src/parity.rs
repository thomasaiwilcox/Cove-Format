use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};

use crate::{
    input::read_source_inputs,
    project::{
        project_cove_o_path_output, project_rows_with_source_states_output, ProjectionFormat,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParityOptions {
    pub(crate) projection_id: String,
    pub(crate) expected: PathBuf,
    pub(crate) expected_query: Option<String>,
    pub(crate) key: Vec<String>,
}

pub(crate) fn parity_from_paths(
    map: &Path,
    sources: &[PathBuf],
    options: &ParityOptions,
) -> Result<Value, String> {
    if sources.is_empty() {
        return Err("parity requires at least one source path".into());
    }
    let file = crate::parse_map(map)?;
    let inputs = read_source_inputs(sources)?;
    crate::validate_source_inputs(&file, &inputs.states)?;
    let projected = project_rows_with_source_states_output(
        &file,
        &inputs.rows,
        &inputs.states,
        ProjectionFormat::Json,
        Some(&options.projection_id),
    )?;
    parity_from_projected_json(&projected, options)
}

pub(crate) fn parity_from_cove_o_path(
    object: &Path,
    options: &ParityOptions,
) -> Result<Value, String> {
    let projected = project_cove_o_path_output(
        object,
        None,
        ProjectionFormat::Json,
        Some(&options.projection_id),
    )?;
    parity_from_projected_json(&projected, options)
}

pub(crate) fn parity_has_failures(report: &Value) -> bool {
    report.get("status").and_then(Value::as_str) != Some("ok")
}

fn parity_from_projected_json(projected: &[u8], options: &ParityOptions) -> Result<Value, String> {
    let projected_value: Value = serde_json::from_slice(projected)
        .map_err(|err| format!("projection output was not JSON: {err}"))?;
    let projected_rows = projected_value
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| "projection JSON did not contain rows".to_string())?
        .iter()
        .filter_map(Value::as_object)
        .map(strip_projection_metadata)
        .collect::<Vec<_>>();
    let expected_rows = expected_rows(options)?;
    let warnings = if options.key.is_empty() {
        vec![json!({
            "code": "ordered_comparison_without_key",
            "message": "No --key was supplied; rows were compared in canonical row order.",
        })]
    } else {
        Vec::new()
    };
    let diff = if options.key.is_empty() {
        ordered_diff(&projected_rows, &expected_rows)
    } else {
        keyed_diff(&projected_rows, &expected_rows, &options.key)
    };
    let status = if diff
        .get("changed_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 0
        && diff
            .get("missing_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
        && diff.get("extra_count").and_then(Value::as_u64).unwrap_or(0) == 0
        && diff
            .get("duplicate_key_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
    {
        "ok"
    } else {
        "mismatch"
    };
    Ok(json!({
        "format": "cove-map-parity-report-v1",
        "status": status,
        "projection_id": options.projection_id,
        "key": options.key,
        "projected_row_count": projected_rows.len(),
        "expected_row_count": expected_rows.len(),
        "warnings": warnings,
        "diff": diff,
    }))
}

fn expected_rows(options: &ParityOptions) -> Result<Vec<Map<String, Value>>, String> {
    let inputs = read_source_inputs(std::slice::from_ref(&options.expected))?;
    let mut rows = inputs
        .rows
        .into_iter()
        .map(|row| row.values.into_iter().collect::<Map<_, _>>())
        .collect::<Vec<_>>();
    if let Some(query) = &options.expected_query {
        rows = apply_expected_query(rows, query)?;
    }
    Ok(rows)
}

fn strip_projection_metadata(row: &Map<String, Value>) -> Map<String, Value> {
    row.iter()
        .filter(|(key, _)| key.as_str() != "projection_id" && key.as_str() != "output_table")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn ordered_diff(projected: &[Map<String, Value>], expected: &[Map<String, Value>]) -> Value {
    let mut changed = Vec::new();
    let min_len = projected.len().min(expected.len());
    for index in 0..min_len {
        if canonical_row(&projected[index]) != canonical_row(&expected[index]) {
            changed.push(json!({
                "row_index": index,
                "projected": projected[index],
                "expected": expected[index],
            }));
        }
    }
    let missing = expected
        .iter()
        .enumerate()
        .skip(projected.len())
        .map(|(index, row)| json!({"row_index": index, "expected": row}))
        .collect::<Vec<_>>();
    let extra = projected
        .iter()
        .enumerate()
        .skip(expected.len())
        .map(|(index, row)| json!({"row_index": index, "projected": row}))
        .collect::<Vec<_>>();
    json!({
        "mode": "ordered",
        "missing_count": missing.len(),
        "extra_count": extra.len(),
        "changed_count": changed.len(),
        "duplicate_key_count": 0,
        "missing_rows": missing,
        "extra_rows": extra,
        "changed_rows": changed,
        "duplicate_keys": [],
    })
}

fn keyed_diff(
    projected: &[Map<String, Value>],
    expected: &[Map<String, Value>],
    key_columns: &[String],
) -> Value {
    let projected_index = row_index(projected, key_columns);
    let expected_index = row_index(expected, key_columns);
    let projected_keys = projected_index
        .rows
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_keys = expected_index.rows.keys().cloned().collect::<BTreeSet<_>>();
    let missing = expected_keys
        .difference(&projected_keys)
        .filter_map(|key| {
            expected_index
                .rows
                .get(key)
                .map(|row| json!({"key": key, "expected": row}))
        })
        .collect::<Vec<_>>();
    let extra = projected_keys
        .difference(&expected_keys)
        .filter_map(|key| {
            projected_index
                .rows
                .get(key)
                .map(|row| json!({"key": key, "projected": row}))
        })
        .collect::<Vec<_>>();
    let mut changed = Vec::new();
    for key in projected_keys.intersection(&expected_keys) {
        let projected_row = &projected_index.rows[key];
        let expected_row = &expected_index.rows[key];
        if canonical_row(projected_row) != canonical_row(expected_row) {
            changed.push(json!({
                "key": key,
                "projected": projected_row,
                "expected": expected_row,
                "changed_columns": changed_columns(projected_row, expected_row),
            }));
        }
    }
    let duplicate_keys = projected_index
        .duplicates
        .into_iter()
        .chain(expected_index.duplicates)
        .collect::<Vec<_>>();
    json!({
        "mode": "keyed",
        "missing_count": missing.len(),
        "extra_count": extra.len(),
        "changed_count": changed.len(),
        "duplicate_key_count": duplicate_keys.len(),
        "missing_rows": missing,
        "extra_rows": extra,
        "changed_rows": changed,
        "duplicate_keys": duplicate_keys,
    })
}

struct RowIndex {
    rows: BTreeMap<String, Map<String, Value>>,
    duplicates: Vec<Value>,
}

fn row_index(rows: &[Map<String, Value>], key_columns: &[String]) -> RowIndex {
    let mut out = BTreeMap::new();
    let mut duplicates = Vec::new();
    for row in rows {
        let key = key_value(row, key_columns);
        if out.insert(key.clone(), row.clone()).is_some() {
            duplicates.push(json!({"key": key}));
        }
    }
    RowIndex {
        rows: out,
        duplicates,
    }
}

fn key_value(row: &Map<String, Value>, key_columns: &[String]) -> String {
    key_columns
        .iter()
        .map(|column| canonical_value(row.get(column).unwrap_or(&Value::Null)))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn changed_columns(left: &Map<String, Value>, right: &Map<String, Value>) -> Vec<String> {
    let keys = left
        .keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter(|key| {
            canonical_value(left.get(key).unwrap_or(&Value::Null))
                != canonical_value(right.get(key).unwrap_or(&Value::Null))
        })
        .collect()
}

fn apply_expected_query(
    mut rows: Vec<Map<String, Value>>,
    query: &str,
) -> Result<Vec<Map<String, Value>>, String> {
    if let Some((column, literal)) = parse_eq_filter(query)? {
        rows.retain(|row| row.get(&column).is_some_and(|value| value == &literal));
    }
    if let Some(columns) = parse_select(query)? {
        rows = rows
            .into_iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|column| {
                        (
                            column.clone(),
                            row.get(column).cloned().unwrap_or(Value::Null),
                        )
                    })
                    .collect::<Map<_, _>>()
            })
            .collect();
    }
    Ok(rows)
}

fn parse_select(query: &str) -> Result<Option<Vec<String>>, String> {
    let Some(start) = query.find("select(") else {
        return Ok(None);
    };
    let start = start + "select(".len();
    let end = query[start..]
        .find(')')
        .map(|offset| start + offset)
        .ok_or_else(|| "--expected-query select(...) is missing ')'".to_string())?;
    let columns = query[start..end]
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .map(|column| {
            column
                .split(':')
                .next_back()
                .unwrap_or(column)
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>();
    Ok(Some(columns))
}

fn parse_eq_filter(query: &str) -> Result<Option<(String, Value)>, String> {
    let Some(where_start) = query.find("where(") else {
        return Ok(None);
    };
    let start = where_start + "where(".len();
    let end = query[start..]
        .find(')')
        .map(|offset| start + offset)
        .ok_or_else(|| "--expected-query where(...) is missing ')'".to_string())?;
    let expr = query[start..end].trim();
    let Some((column, literal)) = expr.split_once("==") else {
        return Err("--expected-query currently supports where(column == literal)".into());
    };
    Ok(Some((
        column.trim().to_string(),
        parse_literal(literal.trim()),
    )))
}

fn parse_literal(raw: &str) -> Value {
    if raw == "null" {
        Value::Null
    } else if raw == "true" {
        json!(true)
    } else if raw == "false" {
        json!(false)
    } else if let Ok(value) = raw.parse::<i64>() {
        json!(value)
    } else if let Ok(value) = raw.parse::<f64>() {
        json!(value)
    } else {
        Value::String(raw.trim_matches('"').trim_matches('\'').to_string())
    }
}

fn canonical_row(row: &Map<String, Value>) -> String {
    let mut entries = row.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(key, _)| *key);
    entries
        .into_iter()
        .map(|(key, value)| format!("{key}:{}", canonical_value(value)))
        .collect::<Vec<_>>()
        .join("|")
}

fn canonical_value(value: &Value) -> String {
    match value {
        Value::Object(object) => canonical_row(object),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}
