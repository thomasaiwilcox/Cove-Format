use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use crate::input::{read_source_inputs, SourceRow};

pub(crate) fn suggest_from_paths(sources: &[PathBuf]) -> Result<Value, String> {
    if sources.is_empty() {
        return Err("suggest requires at least one source path".into());
    }
    let inputs = read_source_inputs(sources)?;
    let mut source_summaries = Vec::new();
    let mut stats_by_source = BTreeMap::new();
    for path in sources {
        let source_id = source_id(path);
        let rows = inputs
            .rows
            .iter()
            .filter(|row| row.source_id == source_id)
            .cloned()
            .collect::<Vec<_>>();
        let stats = source_stats(&source_id, &rows);
        source_summaries.push(source_summary(path, &source_id, &stats, &inputs.states));
        stats_by_source.insert(source_id, stats);
    }
    let identity_candidates = stats_by_source
        .iter()
        .map(|(source_id, stats)| {
            json!({
                "source_id": source_id,
                "candidates": identity_candidates(stats),
            })
        })
        .collect::<Vec<_>>();
    let join_key_candidates = join_key_candidates(&stats_by_source);
    let starter_projections = stats_by_source
        .iter()
        .map(|(source_id, stats)| starter_projection(source_id, stats))
        .collect::<Vec<_>>();
    Ok(json!({
        "format": "cove-map-suggestions-v1",
        "non_authoritative": true,
        "warning": "Suggestions are deterministic authoring hints. They do not define canonical identity or semantic truth.",
        "sources": source_summaries,
        "identity_candidates": identity_candidates,
        "join_key_candidates": join_key_candidates,
        "starter_projections": starter_projections,
    }))
}

#[derive(Debug, Clone)]
struct SourceStats {
    row_count: usize,
    columns: BTreeMap<String, ColumnStats>,
}

#[derive(Debug, Clone)]
struct ColumnStats {
    name: String,
    kinds: BTreeSet<String>,
    non_null_count: usize,
    distinct_values: BTreeSet<String>,
    sample_values: Vec<String>,
}

fn source_stats(_source_id: &str, rows: &[SourceRow]) -> SourceStats {
    let mut columns = BTreeMap::<String, ColumnStats>::new();
    for row in rows {
        for (name, value) in &row.values {
            let stats = columns.entry(name.clone()).or_insert_with(|| ColumnStats {
                name: name.clone(),
                kinds: BTreeSet::new(),
                non_null_count: 0,
                distinct_values: BTreeSet::new(),
                sample_values: Vec::new(),
            });
            stats.kinds.insert(value_kind(value).to_string());
            if !value.is_null() {
                stats.non_null_count += 1;
                let canonical = canonical_json(value);
                stats.distinct_values.insert(canonical.clone());
                if stats.sample_values.len() < 5 && !stats.sample_values.contains(&canonical) {
                    stats.sample_values.push(canonical);
                }
            }
        }
    }
    SourceStats {
        row_count: rows.len(),
        columns,
    }
}

fn source_summary(
    path: &Path,
    source_id: &str,
    stats: &SourceStats,
    states: &[crate::ObservedSourceState],
) -> Value {
    let state = states.iter().find(|state| state.source_id == source_id);
    json!({
        "source_id": source_id,
        "path": path.display().to_string(),
        "source_kind": state.map(|state| state.source_kind.as_str()),
        "schema_fingerprint": state.map(|state| state.schema_fingerprint.as_str()),
        "snapshot_digest": state.map(|state| state.snapshot_digest.as_str()),
        "row_count": stats.row_count,
        "columns": stats.columns.values().map(|column| column_summary(column, stats.row_count)).collect::<Vec<_>>(),
        "suggested_source_declaration": {
            "source_id": source_id,
            "source_kind": state.map(|state| state.source_kind.as_str()),
            "schema_fingerprint": state.map(|state| state.schema_fingerprint.as_str()),
            "snapshot_digest": state.map(|state| state.snapshot_digest.as_str()),
            "row_identity_rules": [],
            "non_authoritative": true,
        },
    })
}

fn column_summary(column: &ColumnStats, row_count: usize) -> Value {
    json!({
        "name": column.name,
        "kinds": column.kinds.iter().cloned().collect::<Vec<_>>(),
        "non_null_count": column.non_null_count,
        "null_rate": ratio(row_count.saturating_sub(column.non_null_count), row_count),
        "distinct_count": column.distinct_values.len(),
        "uniqueness": ratio(column.distinct_values.len(), row_count),
        "sample_values": column.sample_values,
    })
}

fn identity_candidates(stats: &SourceStats) -> Vec<Value> {
    let mut candidates = stats
        .columns
        .values()
        .filter_map(|column| {
            let uniqueness = ratio(column.distinct_values.len(), stats.row_count);
            let null_rate = ratio(
                stats.row_count.saturating_sub(column.non_null_count),
                stats.row_count,
            );
            let name_score = identity_name_score(&column.name);
            let score = (uniqueness * 0.65) + ((1.0 - null_rate) * 0.2) + name_score;
            (score >= 0.75).then(|| {
                json!({
                    "column": column.name,
                    "score": round3(score.min(1.0)),
                    "reason": identity_reason(&column.name, uniqueness, null_rate),
                    "uniqueness": round3(uniqueness),
                    "null_rate": round3(null_rate),
                    "candidate_only": true,
                    "non_authoritative": true,
                })
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right["score"]
            .as_f64()
            .partial_cmp(&left["score"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
}

fn join_key_candidates(stats_by_source: &BTreeMap<String, SourceStats>) -> Vec<Value> {
    let mut out = Vec::new();
    let sources = stats_by_source.keys().cloned().collect::<Vec<_>>();
    for left_index in 0..sources.len() {
        for right_index in (left_index + 1)..sources.len() {
            let left_id = &sources[left_index];
            let right_id = &sources[right_index];
            let left = &stats_by_source[left_id];
            let right = &stats_by_source[right_id];
            for left_column in left.columns.values() {
                for right_column in right.columns.values() {
                    let name_match = normalized_column_name(&left_column.name)
                        == normalized_column_name(&right_column.name);
                    let overlap = value_overlap(left_column, right_column);
                    if name_match || overlap >= 0.2 {
                        out.push(json!({
                            "left_source": left_id,
                            "left_column": left_column.name,
                            "right_source": right_id,
                            "right_column": right_column.name,
                            "score": round3((if name_match { 0.45 } else { 0.0 }) + (overlap * 0.55)),
                            "overlap": round3(overlap),
                            "candidate_only": true,
                            "non_authoritative": true,
                        }));
                    }
                }
            }
        }
    }
    out.sort_by(|left, right| {
        right["score"]
            .as_f64()
            .partial_cmp(&left["score"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn starter_projection(source_id: &str, stats: &SourceStats) -> Value {
    json!({
        "projection_id": format!("{source_id}_starter.v1"),
        "output_table": format!("{source_id}_starter"),
        "row_grain": "one_row_per_object",
        "temporal_mode": {"as_of": "latest_committed"},
        "columns": stats.columns.values().take(12).map(|column| {
            json!({
                "name": column.name,
                "value": format!("property.{}", column.name),
                "logical_type": suggested_logical_type(column),
            })
        }).collect::<Vec<_>>(),
        "output_modes": ["json", "cove-t"],
        "candidate_only": true,
        "non_authoritative": true,
    })
}

fn source_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("source")
        .to_string()
}

fn value_kind(value: &Value) -> &'static str {
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

fn suggested_logical_type(column: &ColumnStats) -> &'static str {
    if column.kinds.contains("bool") {
        "bool"
    } else if column.kinds.contains("int") || column.kinds.contains("uint") {
        "int64"
    } else if column.kinds.contains("float") {
        "float64"
    } else if column.kinds.contains("array") || column.kinds.contains("object") {
        "json"
    } else {
        "utf8"
    }
}

fn identity_name_score(name: &str) -> f64 {
    let normalized = normalized_column_name(name);
    if normalized == "id" || normalized.ends_with("_id") || normalized.ends_with("id") {
        0.18
    } else if normalized.contains("uuid")
        || normalized.contains("key")
        || normalized.contains("email")
        || normalized.contains("code")
    {
        0.12
    } else {
        0.0
    }
}

fn identity_reason(name: &str, uniqueness: f64, null_rate: f64) -> String {
    let mut reasons = Vec::new();
    if identity_name_score(name) > 0.0 {
        reasons.push("identifier-like column name");
    }
    if uniqueness >= 0.9 {
        reasons.push("high uniqueness");
    }
    if null_rate <= 0.05 {
        reasons.push("low null rate");
    }
    reasons.join(", ")
}

fn normalized_column_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn value_overlap(left: &ColumnStats, right: &ColumnStats) -> f64 {
    if left.distinct_values.is_empty() || right.distinct_values.is_empty() {
        return 0.0;
    }
    let intersection = left
        .distinct_values
        .intersection(&right.distinct_values)
        .count();
    let denominator = left.distinct_values.len().min(right.distinct_values.len());
    ratio(intersection, denominator)
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!("{key}:{}", canonical_json(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
