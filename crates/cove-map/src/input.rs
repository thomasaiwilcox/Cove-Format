use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use arrow_array::RecordBatch;
use arrow_json::writer::{LineDelimited, WriterBuilder};
use cove_convert::{
    read_arrow_batches_from_bytes, read_orc_batches_from_bytes, read_parquet_batches_from_bytes,
};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRow {
    pub(crate) source_id: String,
    pub(crate) row_index: usize,
    pub(crate) values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceInputs {
    pub(crate) rows: Vec<SourceRow>,
    pub(crate) states: Vec<ObservedSourceState>,
}

#[derive(Debug, Clone)]
pub(crate) struct ObservedSourceState {
    pub(crate) source_id: String,
    pub(crate) source_kind: String,
    pub(crate) schema_fingerprint: String,
    pub(crate) snapshot_digest: String,
}

pub(crate) fn read_sources(paths: &[PathBuf]) -> Result<Vec<SourceRow>, String> {
    read_source_inputs(paths).map(|inputs| inputs.rows)
}

pub(crate) fn read_source_inputs(paths: &[PathBuf]) -> Result<SourceInputs, String> {
    let mut rows = Vec::new();
    let mut states = Vec::new();
    for path in paths {
        let source_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("source")
            .to_string();
        let bytes =
            fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let source_kind = source_kind(path)?;
        let before_len = rows.len();
        match source_kind {
            "jsonl" => rows.extend(read_jsonl(path, &bytes, &source_id)?),
            "csv" => rows.extend(read_csv_bytes(path, &bytes, &source_id)?),
            "parquet" => rows.extend(read_parquet(path, &bytes, &source_id)?),
            "orc" => rows.extend(read_orc(path, &bytes, &source_id)?),
            "arrow-ipc" => rows.extend(read_arrow_ipc(path, &bytes, &source_id)?),
            _ => unreachable!(),
        }
        let source_rows = &rows[before_len..];
        states.push(ObservedSourceState {
            source_id,
            source_kind: source_kind.to_string(),
            schema_fingerprint: observed_schema_fingerprint(source_kind, source_rows),
            snapshot_digest: format!("sha256:{}", sha256_hex(&bytes)),
        });
    }
    Ok(SourceInputs { rows, states })
}

fn source_kind(path: &Path) -> Result<&'static str, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("jsonl") => Ok("jsonl"),
        Some("csv") => Ok("csv"),
        Some("parquet") => Ok("parquet"),
        Some("orc") => Ok("orc"),
        Some("arrow" | "ipc" | "feather") => Ok("arrow-ipc"),
        _ => Err(format!(
            "{} must be .jsonl, .csv, .parquet, .orc, .arrow, .ipc, or .feather",
            path.display()
        )),
    }
}

fn read_jsonl(path: &Path, bytes: &[u8], source_id: &str) -> Result<Vec<SourceRow>, String> {
    let text = utf8_source_text(path, bytes, "JSONL")?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value: Value = serde_json::from_str(line)
                .map_err(|err| format!("{}:{} invalid JSONL: {err}", path.display(), index + 1))?;
            let object = value.as_object().ok_or_else(|| {
                format!(
                    "{}:{} JSONL row must be an object",
                    path.display(),
                    index + 1
                )
            })?;
            Ok(SourceRow {
                source_id: source_id.to_string(),
                row_index: index,
                values: object_to_btree(object),
            })
        })
        .collect()
}

fn read_parquet(path: &Path, bytes: &[u8], source_id: &str) -> Result<Vec<SourceRow>, String> {
    let (_schema, batches) =
        read_parquet_batches_from_bytes(bytes).map_err(|err| err.to_string())?;
    source_rows_from_record_batches(path, source_id, batches)
}

fn read_orc(path: &Path, bytes: &[u8], source_id: &str) -> Result<Vec<SourceRow>, String> {
    let (_schema, batches) = read_orc_batches_from_bytes(bytes).map_err(|err| err.to_string())?;
    source_rows_from_record_batches(path, source_id, batches)
}

fn read_arrow_ipc(path: &Path, bytes: &[u8], source_id: &str) -> Result<Vec<SourceRow>, String> {
    let (_schema, batches) = read_arrow_batches_from_bytes(bytes).map_err(|err| err.to_string())?;
    source_rows_from_record_batches(path, source_id, batches)
}

#[cfg(test)]
pub(crate) fn read_csv(path: &Path, source_id: &str) -> Result<Vec<SourceRow>, String> {
    let bytes = fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    read_csv_bytes(path, &bytes, source_id)
}

fn read_csv_bytes(path: &Path, bytes: &[u8], source_id: &str) -> Result<Vec<SourceRow>, String> {
    let text = utf8_source_text(path, bytes, "CSV")?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| format!("{} is empty", path.display()))?
        .split(',')
        .map(|field| field.trim().to_string())
        .collect::<Vec<_>>();
    lines
        .enumerate()
        .map(|(index, line)| {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() != header.len() {
                return Err(format!(
                    "{}:{} field count {} did not match header count {}",
                    path.display(),
                    index + 2,
                    fields.len(),
                    header.len()
                ));
            }
            let values = header
                .iter()
                .cloned()
                .zip(
                    fields
                        .into_iter()
                        .map(|field| Value::String(field.to_string())),
                )
                .collect::<BTreeMap<_, _>>();
            Ok(SourceRow {
                source_id: source_id.to_string(),
                row_index: index,
                values,
            })
        })
        .collect()
}

fn utf8_source_text<'a>(path: &Path, bytes: &'a [u8], format: &str) -> Result<&'a str, String> {
    std::str::from_utf8(bytes)
        .map_err(|err| format!("cannot decode {} as {format} UTF-8: {err}", path.display()))
}

fn source_rows_from_record_batches(
    path: &Path,
    source_id: &str,
    batches: Vec<RecordBatch>,
) -> Result<Vec<SourceRow>, String> {
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, LineDelimited>(Vec::new());
    for batch in &batches {
        writer
            .write(batch)
            .map_err(|err| format!("cannot encode {} rows as JSON: {err}", path.display()))?;
    }
    writer.finish().map_err(|err| {
        format!(
            "cannot finish JSON conversion for {}: {err}",
            path.display()
        )
    })?;
    let text = String::from_utf8(writer.into_inner()).map_err(|err| {
        format!(
            "JSON conversion for {} was not UTF-8: {err}",
            path.display()
        )
    })?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value: Value = serde_json::from_str(line).map_err(|err| {
                format!(
                    "{}:{} invalid converted JSON row: {err}",
                    path.display(),
                    index + 1
                )
            })?;
            let object = value.as_object().ok_or_else(|| {
                format!(
                    "{}:{} converted row must be an object",
                    path.display(),
                    index + 1
                )
            })?;
            Ok(SourceRow {
                source_id: source_id.to_string(),
                row_index: index,
                values: object_to_btree(object),
            })
        })
        .collect()
}

pub(crate) fn validate_source_inputs(
    file: &CovemapFile,
    states: &[ObservedSourceState],
) -> Result<(), String> {
    let context = mapping_context(file)?;
    let mut observed = BTreeMap::<String, &ObservedSourceState>::new();
    for state in states {
        if observed.insert(state.source_id.clone(), state).is_some() {
            return Err(format!(
                "source '{}' was supplied more than once",
                state.source_id
            ));
        }
        let expected = context.sources.get(&state.source_id).ok_or_else(|| {
            format!(
                "source '{}' is not declared by the mapping",
                state.source_id
            )
        })?;
        if expected.replay_claimed {
            let expected_schema = expected.schema_fingerprint.as_deref().ok_or_else(|| {
                format!(
                    "source '{}' claims replayability but has no schema_fingerprint",
                    state.source_id
                )
            })?;
            let expected_digest = expected.snapshot_digest.as_deref().ok_or_else(|| {
                format!(
                    "source '{}' claims replayability but has no snapshot_digest",
                    state.source_id
                )
            })?;
            if !is_reference_schema_fingerprint(expected_schema) {
                return Err(format!(
                    "source '{}' replay schema_fingerprint must use cove-map-schema-v1:<sha256>",
                    state.source_id
                ));
            }
            if !is_sha256_digest(expected_digest) {
                return Err(format!(
                    "source '{}' replay snapshot_digest must use sha256:<64 hex>",
                    state.source_id
                ));
            }
            if expected_schema != state.schema_fingerprint
                || expected_digest != state.snapshot_digest
            {
                return Err(format!(
                    "source '{}' does not match replay fingerprint",
                    state.source_id
                ));
            }
            continue;
        }
        if expected
            .schema_fingerprint
            .as_deref()
            .is_some_and(is_reference_schema_fingerprint)
            && expected.schema_fingerprint.as_deref() != Some(state.schema_fingerprint.as_str())
        {
            return Err(format!(
                "source '{}' schema_fingerprint mismatch",
                state.source_id
            ));
        }
        if expected
            .snapshot_digest
            .as_deref()
            .is_some_and(is_sha256_digest)
            && expected.snapshot_digest.as_deref() != Some(state.snapshot_digest.as_str())
        {
            return Err(format!(
                "source '{}' snapshot_digest mismatch",
                state.source_id
            ));
        }
    }
    let row_sources = context
        .row_rules
        .iter()
        .map(|rule| rule.source_id.as_str())
        .collect::<BTreeSet<_>>();
    for (source_id, source) in &context.sources {
        if (source.replay_claimed || row_sources.contains(source_id.as_str()))
            && !observed.contains_key(source_id)
        {
            return Err(format!(
                "source '{}' is required by the mapping but was not supplied",
                source_id
            ));
        }
    }
    Ok(())
}

fn observed_schema_fingerprint(source_kind: &str, rows: &[SourceRow]) -> String {
    let mut fields = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows {
        for (key, value) in &row.values {
            fields
                .entry(key.clone())
                .or_default()
                .insert(json_primitive_kind(value).to_string());
        }
    }
    let schema = fields
        .into_iter()
        .map(|(key, kinds)| format!("{key}:{}", kinds.into_iter().collect::<Vec<_>>().join(",")))
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "cove-map-schema-v1:{}",
        sha256_hex(format!("{source_kind}\n{schema}").as_bytes())
    )
}

fn json_primitive_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() => "int",
        Value::Number(number) if number.is_u64() => "uint",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_reference_schema_fingerprint(value: &str) -> bool {
    value
        .strip_prefix("cove-map-schema-v1:")
        .is_some_and(is_lower_hex_sha256)
}

fn is_sha256_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(is_lower_hex_sha256)
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
