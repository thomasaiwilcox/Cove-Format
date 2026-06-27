use std::path::Path;

use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapPayloadEncodingV2, CovemapSection, CovemapSectionEntryV1,
    },
    checksum,
    constants::SectionKind,
    durable,
};
use serde_json::{json, Map, Value};

use crate::{mapping_identity, parse_map};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewImportOptions {
    pub(crate) replace: bool,
}

pub(crate) fn review_worklist_from_candidate_matches(candidates: &Value) -> Result<Value, String> {
    let matches = candidates
        .get("candidate_matches")
        .and_then(Value::as_array)
        .ok_or_else(|| "review input missing candidate_matches array".to_string())?;
    let mapping_id = candidates
        .get("mapping_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mapping_version = candidates
        .get("mapping_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    let review_items = matches
        .iter()
        .map(review_item_from_candidate)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "schema_id": "org.coveformat.covemap.review-worklist.v1",
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "candidate_match_count": matches.len(),
        "review_items": review_items,
        "reviewed_decisions": []
    }))
}

pub(crate) fn import_reviewed_decisions_from_paths(
    map_path: &Path,
    review_path: &Path,
    output_path: &Path,
    options: &ReviewImportOptions,
) -> Result<Value, String> {
    let file = parse_map(map_path)?;
    let review_bytes = std::fs::read(review_path)
        .map_err(|err| format!("cannot read {}: {err}", review_path.display()))?;
    let (updated, mut report) =
        import_reviewed_decisions_from_bytes(&file, &review_bytes, options)?;
    let bytes = updated
        .serialize()
        .map_err(|err| format!("cannot serialize reviewed COVE-MAP: {err}"))?;
    CovemapFile::parse_validated(&bytes)
        .map_err(|err| format!("review import produced invalid COVE-MAP: {err}"))?;
    durable::durable_replace(output_path, &bytes)
        .map_err(|err| format!("cannot durably write {}: {err}", output_path.display()))?;
    if let Value::Object(object) = &mut report {
        object.insert(
            "output".to_string(),
            Value::String(output_path.display().to_string()),
        );
    }
    Ok(report)
}

pub(crate) fn import_reviewed_decisions_from_bytes(
    file: &CovemapFile,
    review_bytes: &[u8],
    options: &ReviewImportOptions,
) -> Result<(CovemapFile, Value), String> {
    let review: Value = serde_json::from_slice(review_bytes)
        .map_err(|err| format!("invalid review JSON: {err}"))?;
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    validate_review_identity(&review, &mapping_id, &mapping_version)?;
    let imported = reviewed_decisions_from_review(&review)?;
    let mut updated = file.clone();

    let section_index = updated
        .sections
        .iter()
        .position(|section| section.entry.section_id == SectionKind::MapResolutionCatalog as u32);
    let mut payload = if let Some(index) = section_index {
        serde_json::from_slice(&updated.sections[index].payload)
            .map_err(|err| format!("invalid MAP_RESOLUTION_CATALOG JSON: {err}"))?
    } else {
        resolution_catalog_payload(&mapping_id, &mapping_version)
    };

    let existing_count = reviewed_decisions_array(&payload)?.len();
    let reviewed_decisions = merged_reviewed_decisions(
        reviewed_decisions_array(&payload)?,
        &imported,
        options.replace,
    )?;
    let imported_count = imported.len();
    let final_count = reviewed_decisions.len();
    reviewed_decisions_array_mut(&mut payload)?.clear();
    reviewed_decisions_array_mut(&mut payload)?.extend(reviewed_decisions);

    let payload_bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|err| format!("cannot encode MAP_RESOLUTION_CATALOG JSON: {err}"))?;
    match section_index {
        Some(index) => {
            updated.sections[index] =
                resolution_section_with_existing_entry(&updated.sections[index], payload_bytes);
        }
        None => updated.sections.push(new_resolution_section(payload_bytes)),
    }

    let report = json!({
        "ok": true,
        "replace": options.replace,
        "imported_reviewed_decision_count": imported_count,
        "existing_reviewed_decision_count": existing_count,
        "reviewed_decision_count": final_count
    });
    Ok((updated, report))
}

pub(crate) fn export_reviewed_decisions(file: &CovemapFile) -> Result<Value, String> {
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    let decisions = match file
        .sections
        .iter()
        .find(|section| section.entry.section_id == SectionKind::MapResolutionCatalog as u32)
    {
        Some(section) => {
            let payload: Value = serde_json::from_slice(&section.payload)
                .map_err(|err| format!("invalid MAP_RESOLUTION_CATALOG JSON: {err}"))?;
            reviewed_decisions_array(&payload)?.clone()
        }
        None => Vec::new(),
    };

    Ok(json!({
        "schema_id": "org.coveformat.covemap.review-worklist.v1",
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "candidate_match_count": 0,
        "review_items": [],
        "reviewed_decision_count": decisions.len(),
        "reviewed_decisions": decisions
    }))
}

fn validate_review_identity(
    review: &Value,
    mapping_id: &str,
    mapping_version: &str,
) -> Result<(), String> {
    if let Some(value) = review.get("mapping_id").and_then(Value::as_str) {
        if !value.is_empty() && value != mapping_id {
            return Err(format!(
                "review mapping_id '{value}' does not match mapping '{mapping_id}'"
            ));
        }
    }
    if let Some(value) = review.get("mapping_version").and_then(Value::as_str) {
        if !value.is_empty() && value != mapping_version {
            return Err(format!(
                "review mapping_version '{value}' does not match mapping '{mapping_version}'"
            ));
        }
    }
    Ok(())
}

fn reviewed_decisions_from_review(review: &Value) -> Result<Vec<Value>, String> {
    let decisions = review
        .get("reviewed_decisions")
        .and_then(Value::as_array)
        .ok_or_else(|| "review JSON missing reviewed_decisions array".to_string())?;
    let mut out = Vec::with_capacity(decisions.len());
    for decision in decisions {
        if !decision.is_object() {
            return Err("reviewed_decisions entries must be objects".into());
        }
        out.push(decision.clone());
    }
    Ok(out)
}

fn merged_reviewed_decisions(
    existing: &[Value],
    imported: &[Value],
    replace: bool,
) -> Result<Vec<Value>, String> {
    if replace {
        return Ok(imported.to_vec());
    }

    let mut by_id = Map::<String, Value>::new();
    for decision in existing {
        let decision_id = decision_id(decision)?;
        by_id.insert(decision_id.to_string(), decision.clone());
    }
    for decision in imported {
        let decision_id = decision_id(decision)?;
        match by_id.get(decision_id) {
            Some(existing) if existing != decision => {
                return Err(format!(
                    "reviewed_decisions contains conflicting decision_id '{decision_id}'"
                ));
            }
            Some(_) => {}
            None => {
                by_id.insert(decision_id.to_string(), decision.clone());
            }
        }
    }

    let mut out = by_id
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    out.sort_by(|left, right| {
        decision_id(left)
            .unwrap_or("")
            .cmp(decision_id(right).unwrap_or(""))
    });
    Ok(out)
}

fn decision_id(decision: &Value) -> Result<&str, String> {
    decision
        .get("decision_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "reviewed_decision missing non-empty decision_id".to_string())
}

fn reviewed_decisions_array(payload: &Value) -> Result<&Vec<Value>, String> {
    payload
        .get("reviewed_decisions")
        .and_then(Value::as_array)
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG missing reviewed_decisions array".to_string())
}

fn reviewed_decisions_array_mut(payload: &mut Value) -> Result<&mut Vec<Value>, String> {
    payload
        .get_mut("reviewed_decisions")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG missing reviewed_decisions array".to_string())
}

fn resolution_catalog_payload(mapping_id: &str, mapping_version: &str) -> Value {
    json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapResolutionCatalog as u16,
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "normalization_pipelines": [],
        "resolvers": [],
        "match_rules": [],
        "reviewed_decisions": []
    })
}

fn resolution_section_with_existing_entry(
    existing: &CovemapSection,
    payload: Vec<u8>,
) -> CovemapSection {
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: SectionKind::MapResolutionCatalog as u32,
            offset: existing.entry.offset,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: existing.entry.compression,
            payload_encoding: if existing.entry.payload_encoding == 0 {
                CovemapPayloadEncodingV2::CoveMapJsonV2 as u8
            } else {
                existing.entry.payload_encoding
            },
            required: existing.entry.required,
            reserved: 0,
            checksum: checksum::crc32c(&payload),
        },
        payload,
    }
}

fn new_resolution_section(payload: Vec<u8>) -> CovemapSection {
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: SectionKind::MapResolutionCatalog as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: 0,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: checksum::crc32c(&payload),
        },
        payload,
    }
}

fn review_item_from_candidate(candidate: &Value) -> Result<Value, String> {
    let candidate_match_id = required_str(candidate, "candidate_match_id")?;
    let object_type = required_str(candidate, "object_type")?;
    let left = candidate
        .get("left")
        .ok_or_else(|| "candidate match missing left member".to_string())?;
    let right = candidate
        .get("right")
        .ok_or_else(|| "candidate match missing right member".to_string())?;
    let left_ref = row_digest_reference(object_type, left)?;
    let right_ref = row_digest_reference(object_type, right)?;
    Ok(json!({
        "candidate_match_id": candidate_match_id,
        "match_rule_id": candidate.get("match_rule_id").cloned().unwrap_or(Value::Null),
        "object_type": object_type,
        "candidate_score": candidate.get("candidate_score").cloned().unwrap_or(Value::Null),
        "score_scale": candidate.get("score_scale").cloned().unwrap_or(Value::Null),
        "blocking_key": candidate.get("blocking_key").cloned().unwrap_or(Value::Null),
        "status": "pending",
        "left": review_member(left),
        "right": review_member(right),
        "same_object_decision_template": decision_template(
            candidate_match_id,
            "same_object",
            left_ref.clone(),
            right_ref.clone()
        ),
        "do_not_merge_decision_template": decision_template(
            candidate_match_id,
            "do_not_merge",
            left_ref,
            right_ref
        )
    }))
}

fn row_digest_reference(object_type: &str, member: &Value) -> Result<Value, String> {
    Ok(json!({
        "kind": "row_digest",
        "object_type": object_type,
        "source_id": required_str(member, "source_id")?,
        "source_row_identity": required_str(member, "source_row_identity")?,
        "row_digest": required_str(member, "row_digest")?
    }))
}

fn review_member(member: &Value) -> Value {
    json!({
        "source_id": member.get("source_id").cloned().unwrap_or(Value::Null),
        "row_index": member.get("row_index").cloned().unwrap_or(Value::Null),
        "source_row_identity": member.get("source_row_identity").cloned().unwrap_or(Value::Null),
        "row_digest": member.get("row_digest").cloned().unwrap_or(Value::Null),
        "column": member.get("column").cloned().unwrap_or(Value::Null),
        "raw_value": member.get("raw_value").cloned().unwrap_or(Value::Null),
        "normalized_value": member.get("normalized_value").cloned().unwrap_or(Value::Null)
    })
}

fn decision_template(candidate_match_id: &str, decision: &str, left: Value, right: Value) -> Value {
    json!({
        "decision_id": format!("review:{candidate_match_id}:{decision}"),
        "decision": decision,
        "confidence_class": "reviewed_authoritative",
        "reviewed_by": "<reviewer>",
        "reviewed_at": "<iso-8601-utc>",
        "reason": "",
        "left": left,
        "right": right
    })
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("candidate review input missing string field '{key}'"))
}
