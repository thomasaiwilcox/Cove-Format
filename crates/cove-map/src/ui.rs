use std::{fs, path::PathBuf};

use cove_core::artifact::covemap::CovemapFile;
use serde_json::{json, Value};

use crate::{embedded_sections, CandidateMatch, PlannedIdentity, ResolutionMetadata};

pub(crate) fn explain(file: &CovemapFile, id: &str) -> Result<Value, String> {
    for section in embedded_sections(file)? {
        if let cove_core::profile::cove_map::EmbeddedMapSection::EvidenceIndex(index) = section {
            for entry in index.entries {
                if entry.assertion_id == id || entry.output_object_id == id {
                    let mut explanation = json!({
                        "source_id": entry.source_id,
                        "source_row_identity": entry.source_row_identity,
                        "rule_id": entry.rule_id,
                        "assertion_id": entry.assertion_id,
                        "output_object_id": entry.output_object_id,
                        "observed_schema_fingerprint": entry.observed_schema_fingerprint,
                        "observed_snapshot_digest": entry.observed_snapshot_digest,
                    });
                    if !entry.operation_metadata.is_empty() {
                        let mut resolution = serde_json::Map::new();
                        for key in [
                            "identity_rule_id",
                            "object_type",
                            "join_key_sha256",
                            "resolution_role_id",
                            "resolution_kind",
                            "resolver_id",
                            "resolver_digest",
                            "catalog_digest",
                            "pipeline_digest",
                            "normalization_pipeline_id",
                            "evidence_policy",
                            "redacted_resolution_evidence",
                            "redacted",
                            "redaction_scope",
                            "raw_observed_value",
                            "normalized_value",
                            "resolved_identity_value",
                            "canonical_key",
                            "canonical_label",
                            "alias_catalog_id",
                            "alias_entry_id",
                            "alias_hit",
                            "alias_miss",
                            "alias_ambiguous",
                            "miss_policy",
                        ] {
                            if let Some(value) = entry.operation_metadata.get(key) {
                                resolution.insert(key.to_string(), value.clone());
                            }
                        }
                        let Some(object) = explanation.as_object_mut() else {
                            return Err("internal error: explain output is not an object".into());
                        };
                        object.insert("operation_metadata".into(), json!(entry.operation_metadata));
                        if !resolution.is_empty() {
                            object.insert("resolution".into(), Value::Object(resolution));
                        }
                    }
                    return Ok(explanation);
                }
            }
        }
    }
    Err(format!(
        "id {id} was not found in COVE-MAP evidence sections"
    ))
}

pub(crate) fn evidence_entry_for_identity(identity: &PlannedIdentity) -> Value {
    let mut entry = json!({
        "source_id": identity.source_id,
        "source_row_identity": identity.source_row_identity,
        "rule_id": identity.row_rule_id,
        "assertion_id": identity_assertion_id(identity),
        "output_object_id": crate::hex_encode(&identity.goid),
        "observed_schema_fingerprint": identity.schema_fingerprint,
        "identity_rule_id": identity.identity_rule_id,
        "object_type": identity.object_type,
        "join_key_sha256": identity.join_key_sha256,
    });
    add_resolution_metadata(&mut entry, &identity.resolution_metadata);
    entry
}

pub(crate) fn evidence_entry_for_candidate(candidate: &CandidateMatch) -> Value {
    let mut entry = json!({
        "source_id": candidate.source_id,
        "source_row_identity": candidate.source_row_identity,
        "rule_id": candidate.row_rule_id,
        "assertion_id": candidate_assertion_id(candidate),
        "output_object_id": candidate_match_id(candidate),
        "observed_schema_fingerprint": candidate.schema_fingerprint,
        "candidate": true,
        "identity_rule_id": candidate.identity_rule_id,
        "object_type": candidate.object_type,
        "join_key_sha256": candidate.join_key_sha256,
    });
    add_resolution_metadata(&mut entry, &candidate.resolution_metadata);
    entry
}

fn add_resolution_metadata(entry: &mut Value, metadata: &[ResolutionMetadata]) {
    if metadata.is_empty() {
        return;
    }
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    object.insert(
        "resolution_metadata".into(),
        Value::Array(metadata.iter().map(resolution_metadata_value).collect()),
    );
    let metadata = &metadata[0];
    insert_flat_resolution_metadata(object, metadata);
}

fn resolution_metadata_value(metadata: &ResolutionMetadata) -> Value {
    let mut object = serde_json::Map::new();
    insert_flat_resolution_metadata(&mut object, metadata);
    Value::Object(object)
}

fn insert_flat_resolution_metadata(
    object: &mut serde_json::Map<String, Value>,
    metadata: &ResolutionMetadata,
) {
    object.insert("resolution_role_id".into(), json!(metadata.role_id));
    object.insert("resolution_kind".into(), json!(metadata.resolution_kind));
    object.insert("resolver_id".into(), json!(metadata.resolver_id));
    object.insert("resolver_digest".into(), json!(metadata.resolver_digest));
    object.insert("catalog_digest".into(), json!(metadata.catalog_digest));
    object.insert("pipeline_digest".into(), json!(metadata.pipeline_digest));
    object.insert(
        "normalization_pipeline_id".into(),
        json!(metadata.normalization_pipeline_id),
    );
    object.insert("evidence_policy".into(), json!(metadata.evidence_policy));
    if metadata.redacted_resolution_evidence {
        object.insert("redacted_resolution_evidence".into(), json!(true));
        object.insert("redacted".into(), json!(true));
        object.insert("redaction_scope".into(), json!("resolver_evidence"));
    } else {
        object.insert(
            "raw_observed_value".into(),
            json!(metadata.raw_observed_value),
        );
        object.insert("normalized_value".into(), json!(metadata.normalized_value));
    }
    if let Some(value) = &metadata.resolved_identity_value {
        object.insert("resolved_identity_value".into(), json!(value));
    }
    if let Some(value) = &metadata.canonical_key {
        object.insert("canonical_key".into(), json!(value));
    }
    if let Some(value) = &metadata.canonical_label {
        object.insert("canonical_label".into(), json!(value));
    }
    if let Some(value) = &metadata.alias_catalog_id {
        object.insert("alias_catalog_id".into(), json!(value));
    }
    if let Some(value) = &metadata.alias_entry_id {
        object.insert("alias_entry_id".into(), json!(value));
    }
    if metadata.alias_hit {
        object.insert("alias_hit".into(), json!(true));
    }
    if metadata.alias_miss {
        object.insert("alias_miss".into(), json!(true));
    }
    if metadata.alias_ambiguous {
        object.insert("alias_ambiguous".into(), json!(true));
    }
    if let Some(value) = &metadata.miss_policy {
        object.insert("miss_policy".into(), json!(value));
    }
}

pub(crate) fn identity_assertion_id(identity: &PlannedIdentity) -> String {
    format!(
        "assertion:{}:{}:{}",
        identity.identity_rule_id, identity.source_row_identity, identity.row_digest
    )
}

pub(crate) fn candidate_assertion_id(candidate: &CandidateMatch) -> String {
    format!(
        "candidate:{}:{}:{}",
        candidate.identity_rule_id, candidate.source_row_identity, candidate.row_digest
    )
}

pub(crate) fn candidate_match_id(candidate: &CandidateMatch) -> String {
    format!(
        "candidate-match:{}:{}",
        candidate.identity_rule_id, candidate.join_key_sha256
    )
}

pub(crate) fn print_json(value: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value)
            .map_err(|err| format!("cannot serialize JSON output: {err}"))?
    );
    Ok(())
}

pub(crate) fn write_or_print(output: Option<PathBuf>, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("cannot serialize JSON output: {err}"))?;
    if let Some(output) = output {
        fs::write(&output, text).map_err(|err| format!("cannot write {}: {err}", output.display()))
    } else {
        println!("{text}");
        Ok(())
    }
}

pub(crate) fn print_usage() {
    println!(
        "Usage: cove map <subcommand> [options]\n\n\
Subcommands:\n  \
validate <mapping.covemap>\n  \
preview <mapping.covemap>\n  \
	  plan-keys <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
	  candidates [--out candidates.json] <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
	  review [--out reviewed.json] <candidate-matches.json>\n  \
	  review export <mapping.covemap> [--out reviewed.json]\n  \
	  review import <mapping.covemap> <reviewed.json> --out <mapping.covemap> [--replace]\n  \
	  aliases import <mapping.covemap> <aliases.csv> --catalog-id <id> --resolver-id <id> --out <mapping.covemap>\n  \
	  replay verify <mapping.covemap> <conversion-report.json>\n  \
	  convert [--format json|cove-o] [-o output] <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
build --out-dir <dir> [--verify] [--publish-covm] [--force] [--json] [--object-name name.cove] [--projection-output cove-t|none] [--evidence-encoding compact|expanded|both] [--section-compression zstd|none] <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
delta build <manifest.covm> --dataset <dir> --out-dir <dir> | --base <manifest.covm> --dataset <dir> --mapping <mapping.covemap> --out <delta.covedelta> <source...>\n  \
publish --bundle-dir <dir> --out <dataset.covm> [--force] [--json]\n  \
doctor [--json] [--strict] --bundle-dir <dir>\n  \
doctor [--json] [--strict] <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
suggest [--json] [--out suggestions.json] <source.csv|source.jsonl|source.parquet>...\n  \
parity [--json] --projection-id <id> --expected <table.csv|jsonl|parquet> [--expected-query <coveql>] [--key col[,col...]] <mapping.covemap> <source...>\n  \
parity-cove-o [--json] --projection-id <id> --expected <table.csv|jsonl|parquet> [--expected-query <coveql>] [--key col[,col...]] <object.cove>\n  \
explain <mapping.covemap> <goid|assertion-id>\n  \
diff <left.covemap> <right.covemap>\n  \
  project [-o output.json] <mapping.covemap> <source.csv|source.jsonl|source.parquet|source.orc|source.arrow>...\n  \
project-cove-o [--mapping mapping.covemap] [-o output.json] <object.cove>\n  \
test <fixture.json>"
    );
}
