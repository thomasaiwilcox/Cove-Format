use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    artifact::covemap::CovemapFile,
    profile::cove_map::{MapCandidateMatchRule, MapNormalizationPipeline},
};
use serde_json::{json, Value};

use crate::{
    apply_resolution_pipeline, embedded_sections, mapping_identity, row_digest, sha256_hex,
    SourceRow,
};

#[derive(Debug, Clone)]
struct CandidateValue {
    source_id: String,
    row_index: usize,
    source_row_identity: String,
    row_digest: String,
    column: String,
    raw_value: String,
    normalized_value: String,
    blocking_key: String,
}

pub(crate) fn candidate_matches(file: &CovemapFile, rows: &[SourceRow]) -> Result<Value, String> {
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    let Some(catalog) = embedded_sections(file)?.into_iter().find_map(|section| {
        if let cove_core::profile::cove_map::EmbeddedMapSection::ResolutionCatalog(catalog) =
            section
        {
            Some(catalog)
        } else {
            None
        }
    }) else {
        return Ok(json!({
            "mapping_id": mapping_id,
            "mapping_version": mapping_version,
            "candidate_matches": [],
            "diagnostics": []
        }));
    };

    let pipelines = catalog
        .normalization_pipelines
        .iter()
        .map(|pipeline| (pipeline.pipeline_id.as_str(), pipeline))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut matches = Vec::new();
    for rule in &catalog.match_rules {
        let scoring_kind = rule.scoring.get("kind").and_then(Value::as_str);
        if scoring_kind != Some("token_jaccard") {
            diagnostics.push(json!({
                "match_rule_id": rule.match_rule_id,
                "severity": "warning",
                "reason": "unsupported_scoring_kind",
                "scoring_kind": scoring_kind.unwrap_or("")
            }));
            continue;
        }
        let rounding = rule.scoring.get("rounding").and_then(Value::as_str);
        if rounding != Some("floor") {
            diagnostics.push(json!({
                "match_rule_id": rule.match_rule_id,
                "severity": "warning",
                "reason": "unsupported_rounding",
                "rounding": rounding.unwrap_or("")
            }));
            continue;
        }
        let Some(pipeline) = pipelines
            .get(rule.normalization_pipeline_id.as_str())
            .copied()
        else {
            diagnostics.push(json!({
                "match_rule_id": rule.match_rule_id,
                "severity": "warning",
                "reason": "missing_normalization_pipeline",
                "normalization_pipeline_id": rule.normalization_pipeline_id
            }));
            continue;
        };
        match candidate_matches_for_rule(rule, pipeline, rows) {
            Ok(mut rule_matches) => matches.append(&mut rule_matches),
            Err(err) if rule.limits.on_limit == "emit_diagnostic_and_truncate" => {
                diagnostics.push(json!({
                    "match_rule_id": rule.match_rule_id,
                    "severity": "warning",
                    "reason": "candidate_rule_truncated",
                    "detail": err
                }));
            }
            Err(err) => return Err(err),
        }
    }
    matches.sort_by_cached_key(|value| value.to_string());
    Ok(json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "candidate_matches": matches,
        "diagnostics": diagnostics
    }))
}

fn candidate_matches_for_rule(
    rule: &MapCandidateMatchRule,
    pipeline: &MapNormalizationPipeline,
    rows: &[SourceRow],
) -> Result<Vec<Value>, String> {
    let score_scale = rule
        .scoring
        .get("score_scale")
        .and_then(Value::as_u64)
        .unwrap_or(1_000_000);
    let threshold = rule
        .scoring
        .get("candidate_threshold")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let threshold_score = (threshold * score_scale as f64).floor() as u64;

    let values = candidate_values(rule, pipeline, rows)?;
    let mut by_block = BTreeMap::<String, Vec<CandidateValue>>::new();
    for value in values {
        by_block
            .entry(value.blocking_key.clone())
            .or_default()
            .push(value);
    }
    let mut out = Vec::new();
    let mut seen_pairs = BTreeSet::<(String, String)>::new();
    for (blocking_key, mut block) in by_block {
        block.sort_by_key(candidate_sort_key);
        let possible_pairs = block.len().saturating_mul(block.len().saturating_sub(1)) / 2;
        if possible_pairs as u64 > rule.limits.max_pairs_per_block {
            return Err(format!(
                "candidate rule '{}' exceeded max_pairs_per_block for block '{}'",
                rule.match_rule_id, blocking_key
            ));
        }
        for left_index in 0..block.len() {
            for right_index in left_index + 1..block.len() {
                let left = &block[left_index];
                let right = &block[right_index];
                if left.source_id == right.source_id && left.row_index == right.row_index {
                    continue;
                }
                let pair_key = (candidate_member_key(left), candidate_member_key(right));
                if !seen_pairs.insert(pair_key) {
                    continue;
                }
                let score = token_jaccard_score(
                    &left.normalized_value,
                    &right.normalized_value,
                    score_scale,
                );
                if score < threshold_score {
                    continue;
                }
                if out.len() as u64 >= rule.limits.max_pairs_total {
                    return Err(format!(
                        "candidate rule '{}' exceeded max_pairs_total",
                        rule.match_rule_id
                    ));
                }
                out.push(candidate_match_json(rule, left, right, score, score_scale));
            }
        }
    }
    Ok(out)
}

fn candidate_values(
    rule: &MapCandidateMatchRule,
    pipeline: &MapNormalizationPipeline,
    rows: &[SourceRow],
) -> Result<Vec<CandidateValue>, String> {
    let mut values = Vec::new();
    for row in rows {
        for input in &rule.inputs {
            if input.source_id != "*" && input.source_id != row.source_id {
                continue;
            }
            let Some(raw_value) = row
                .values
                .get(&input.column)
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let normalized_value = apply_resolution_pipeline(&raw_value, pipeline)?;
            if normalized_value.trim().is_empty() {
                continue;
            }
            values.push(CandidateValue {
                source_id: row.source_id.clone(),
                row_index: row.row_index,
                source_row_identity: format!("{}:{}", row.source_id, row.row_index),
                row_digest: row_digest(row),
                column: input.column.clone(),
                blocking_key: blocking_key(rule, &normalized_value)?,
                raw_value,
                normalized_value,
            });
        }
    }
    values.sort_by_key(candidate_sort_key);
    Ok(values)
}

fn blocking_key(rule: &MapCandidateMatchRule, normalized_value: &str) -> Result<String, String> {
    match rule.blocking.get("kind").and_then(Value::as_str) {
        Some("normalized_prefix") => {
            let length = rule
                .blocking
                .get("length")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    format!(
                        "candidate rule '{}' normalized_prefix blocking requires length",
                        rule.match_rule_id
                    )
                })? as usize;
            Ok(normalized_value.chars().take(length).collect())
        }
        Some("none") | None => Ok(String::new()),
        Some(other) => Err(format!(
            "candidate rule '{}' uses unsupported blocking kind '{}'",
            rule.match_rule_id, other
        )),
    }
}

fn token_jaccard_score(left: &str, right: &str, score_scale: u64) -> u64 {
    let left = left.split_whitespace().collect::<BTreeSet<_>>();
    let right = right.split_whitespace().collect::<BTreeSet<_>>();
    let union = left.union(&right).count();
    if union == 0 {
        return 0;
    }
    let intersection = left.intersection(&right).count();
    ((intersection as u128 * score_scale as u128) / union as u128) as u64
}

fn candidate_match_json(
    rule: &MapCandidateMatchRule,
    left: &CandidateValue,
    right: &CandidateValue,
    score: u64,
    score_scale: u64,
) -> Value {
    let candidate_match_id = candidate_rule_match_id(rule, left, right, score);
    json!({
        "candidate_match_id": candidate_match_id,
        "match_rule_id": rule.match_rule_id,
        "object_type": rule.object_type,
        "scoring_kind": "token_jaccard",
        "candidate_score": score,
        "score": score,
        "score_scale": score_scale,
        "blocking_key": left.blocking_key,
        "left": candidate_member_json(left),
        "right": candidate_member_json(right),
        "merge_behavior": "never"
    })
}

fn candidate_member_json(value: &CandidateValue) -> Value {
    json!({
        "source_id": value.source_id,
        "row_index": value.row_index,
        "source_row_identity": value.source_row_identity,
        "row_digest": value.row_digest,
        "column": value.column,
        "raw_value": value.raw_value,
        "normalized_value": value.normalized_value
    })
}

fn candidate_rule_match_id(
    rule: &MapCandidateMatchRule,
    left: &CandidateValue,
    right: &CandidateValue,
    score: u64,
) -> String {
    let material = serde_json::to_vec(&json!([
        rule.match_rule_id,
        candidate_member_key(left),
        candidate_member_key(right),
        left.blocking_key,
        score
    ]))
    .expect("candidate match id material is JSON-serializable");
    format!(
        "candidate-match:{}:{}",
        rule.match_rule_id,
        sha256_hex(&material)
    )
}

fn candidate_sort_key(value: &CandidateValue) -> (String, usize, String, String, String) {
    (
        value.source_id.clone(),
        value.row_index,
        value.normalized_value.clone(),
        value.row_digest.clone(),
        value.column.clone(),
    )
}

fn candidate_member_key(value: &CandidateValue) -> String {
    format!(
        "{}:{}:{}:{}",
        value.source_id, value.row_index, value.column, value.row_digest
    )
}
