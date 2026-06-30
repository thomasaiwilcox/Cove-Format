use std::collections::{BTreeMap, BTreeSet};

use cove_core::artifact::covemap::CovemapFile;
use serde_json::{json, Value};

use crate::{mapping_context, mapping_identity, reviewed_decision_replay_binding};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayError {
    kind: ReplayErrorKind,
    message: String,
}

impl ReplayError {
    fn new(message: String) -> Self {
        let kind = ReplayErrorKind::from_message(&message);
        Self { kind, message }
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ReplayErrorKind::InvalidReport
            | ReplayErrorKind::MappingMismatch
            | ReplayErrorKind::SourceBinding
            | ReplayErrorKind::ResolverDigest
            | ReplayErrorKind::ReviewDigest => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ReplayError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayErrorKind {
    InvalidReport,
    MappingMismatch,
    SourceBinding,
    ResolverDigest,
    ReviewDigest,
}

impl ReplayErrorKind {
    fn from_message(message: &str) -> Self {
        if message.starts_with("MAP_REPLAY_STALE_MAPPING") {
            Self::MappingMismatch
        } else if message.starts_with("MAP_REPLAY_SOURCE_") {
            Self::SourceBinding
        } else if message.starts_with("MAP_REPLAY_RESOLVER_DIGEST_")
            || message.starts_with("MAP_REPLAY_STALE_RESOLVER")
        {
            Self::ResolverDigest
        } else if message.starts_with("MAP_REPLAY_REVIEW_DIGEST_")
            || message.starts_with("MAP_REPLAY_STALE_REVIEW")
        {
            Self::ReviewDigest
        } else {
            Self::InvalidReport
        }
    }
}

pub(crate) fn verify_replay_report(
    file: &CovemapFile,
    report: &Value,
) -> Result<Value, ReplayError> {
    verify_replay_report_inner(file, report).map_err(ReplayError::new)
}

fn verify_replay_report_inner(file: &CovemapFile, report: &Value) -> Result<Value, String> {
    let report = report
        .as_object()
        .ok_or_else(|| "replay report must be a JSON object".to_string())?;
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    verify_optional_string(report.get("mapping_id"), "mapping_id", &mapping_id)?;
    verify_optional_string(
        report.get("mapping_version"),
        "mapping_version",
        &mapping_version,
    )?;

    let context = mapping_context(file)?;
    let source_count = verify_report_sources(report.get("sources"), &context.sources)?;
    let resolver_digest_count =
        verify_report_resolver_digests(report.get("resolver_catalog_digests"), file)?;
    verify_report_reviewed_decision_digest(report, file)?;

    Ok(json!({
        "ok": true,
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "source_binding_count": source_count,
        "resolver_catalog_digest_count": resolver_digest_count,
        "reviewed_decision_count": reviewed_decision_replay_binding(file)?.count
    }))
}

fn verify_optional_string(
    value: Option<&Value>,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let actual = value
        .as_str()
        .ok_or_else(|| format!("replay report field '{field}' must be a string"))?;
    if actual != expected {
        return Err(format!(
            "MAP_REPLAY_STALE_MAPPING: report {field} '{actual}' does not match mapping '{expected}'"
        ));
    }
    Ok(())
}

fn verify_report_sources(
    sources: Option<&Value>,
    expected_sources: &BTreeMap<String, cove_core::profile::cove_map::MapSourceEntry>,
) -> Result<usize, String> {
    let Some(sources) = sources else {
        if expected_sources
            .values()
            .any(|source| source.replay_claimed)
        {
            return Err("MAP_REPLAY_SOURCE_MISSING: replay report missing sources".into());
        }
        return Ok(0);
    };
    let sources = sources
        .as_array()
        .ok_or_else(|| "replay report sources must be an array".to_string())?;
    let mut seen = BTreeMap::<String, (Option<String>, Option<String>)>::new();
    for source in sources {
        let source_id = required_str(source, "source_id")?;
        let Some(expected) = expected_sources.get(source_id) else {
            return Err(format!(
                "MAP_REPLAY_SOURCE_UNKNOWN: report references source '{source_id}' not declared by mapping"
            ));
        };
        let schema_fingerprint = optional_str(source, "schema_fingerprint")?.map(str::to_string);
        let snapshot_digest = optional_str(source, "snapshot_digest")?.map(str::to_string);
        match seen.get(source_id) {
            Some((seen_schema, seen_snapshot))
                if seen_schema.as_deref() != schema_fingerprint.as_deref()
                    || seen_snapshot.as_deref() != snapshot_digest.as_deref() =>
            {
                return Err(format!(
                    "MAP_REPLAY_SOURCE_MISMATCH: report has inconsistent bindings for source '{source_id}'"
                ));
            }
            Some(_) => {}
            None => {
                seen.insert(
                    source_id.to_string(),
                    (schema_fingerprint.clone(), snapshot_digest.clone()),
                );
            }
        }
        if let Some(expected_schema) = expected.schema_fingerprint.as_deref() {
            let Some(actual_schema) = schema_fingerprint.as_deref() else {
                return Err(format!(
                    "MAP_REPLAY_SOURCE_MISSING: source '{source_id}' missing schema_fingerprint"
                ));
            };
            if actual_schema != expected_schema {
                return Err(format!(
                    "MAP_REPLAY_SOURCE_STALE: source '{source_id}' schema_fingerprint changed"
                ));
            }
        }
        if let Some(expected_snapshot) = expected.snapshot_digest.as_deref() {
            let Some(actual_snapshot) = snapshot_digest.as_deref() else {
                return Err(format!(
                    "MAP_REPLAY_SOURCE_MISSING: source '{source_id}' missing snapshot_digest"
                ));
            };
            if actual_snapshot != expected_snapshot {
                return Err(format!(
                    "MAP_REPLAY_SOURCE_STALE: source '{source_id}' snapshot_digest changed"
                ));
            }
        }
    }
    for (source_id, expected) in expected_sources {
        if expected.replay_claimed && !seen.contains_key(source_id) {
            return Err(format!(
                "MAP_REPLAY_SOURCE_MISSING: replay report missing source '{source_id}'"
            ));
        }
    }
    Ok(seen.len())
}

fn verify_report_resolver_digests(
    digests: Option<&Value>,
    file: &CovemapFile,
) -> Result<usize, String> {
    let digests = digests
        .ok_or_else(|| {
            "MAP_REPLAY_RESOLVER_DIGEST_MISSING: report missing resolver_catalog_digests"
                .to_string()
        })?
        .as_array()
        .ok_or_else(|| "replay report resolver_catalog_digests must be an array".to_string())?;
    if digests.is_empty() {
        return Ok(0);
    }
    let context = mapping_context(file)?;
    let catalog = context.resolution_catalog.as_ref().ok_or_else(|| {
        "MAP_REPLAY_RESOLVER_DIGEST_MISSING: mapping has no MAP_RESOLUTION_CATALOG".to_string()
    })?;
    let mut seen = BTreeSet::<String>::new();
    for digest in digests {
        let resolver_id = required_str(digest, "resolver_id")?;
        let resolver = catalog
            .resolvers
            .iter()
            .find(|resolver| resolver.resolver_id == resolver_id)
            .ok_or_else(|| {
                format!("MAP_REPLAY_STALE_RESOLVER: resolver '{resolver_id}' is not in mapping")
            })?;
        verify_required_string(
            digest,
            "normalization_pipeline_id",
            &resolver.normalization_pipeline_id,
            resolver_id,
        )?;
        verify_required_string(
            digest,
            "resolver_digest",
            &resolver.resolver_digest,
            resolver_id,
        )?;
        verify_required_string(
            digest,
            "catalog_digest",
            &resolver.catalog_digest,
            resolver_id,
        )?;
        verify_required_string(
            digest,
            "pipeline_digest",
            &resolver.pipeline_digest,
            resolver_id,
        )?;
        seen.insert(resolver_id.to_string());
    }
    Ok(seen.len())
}

fn verify_required_string(
    value: &Value,
    field: &str,
    expected: &str,
    resolver_id: &str,
) -> Result<(), String> {
    let actual = required_str(value, field)?;
    if actual != expected {
        return Err(format!(
            "MAP_REPLAY_STALE_RESOLVER: resolver '{resolver_id}' {field} changed"
        ));
    }
    Ok(())
}

fn verify_report_reviewed_decision_digest(
    report: &serde_json::Map<String, Value>,
    file: &CovemapFile,
) -> Result<(), String> {
    let binding = reviewed_decision_replay_binding(file)?;
    let digest = report
        .get("reviewed_decision_catalog_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "MAP_REPLAY_REVIEW_DIGEST_MISSING: report missing reviewed_decision_catalog_digest"
                .to_string()
        })?;
    if digest != binding.digest {
        return Err(
            "MAP_REPLAY_STALE_REVIEW: reviewed decision catalog digest changed".to_string(),
        );
    }
    let count = report
        .get("reviewed_decision_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "MAP_REPLAY_REVIEW_DIGEST_MISSING: report missing reviewed_decision_count".to_string()
        })?;
    if count != binding.count as u64 {
        return Err("MAP_REPLAY_STALE_REVIEW: reviewed decision catalog count changed".to_string());
    }
    Ok(())
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("replay report missing non-empty string field '{key}'"))
}

fn optional_str<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match value.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        Some(Value::Null) | None => Ok(None),
        _ => Err(format!(
            "replay report field '{key}' must be a non-empty string when present"
        )),
    }
}
