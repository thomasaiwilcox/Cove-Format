//! Stable facade for building, validating, replaying, and projecting COVE-MAP artifacts.
//!
//! Public helpers return typed map errors at API and CLI boundaries while keeping
//! the COVE wire sections, projection JSON, and command-line diagnostics stable.

use std::collections::{BTreeMap, BTreeSet};

use cove_core::{
    artifact::covemap::CovemapFile,
    canonical::{CanonicalField, CanonicalValue},
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile,
        SectionKind, FEATURE_FILE_DICTIONARY, FEATURE_OBJECT_PROFILE, FEATURE_SEMANTIC_MAP,
        FEATURE_TRUST_CHAIN,
    },
    dictionary::{FileDictionary, FileDictionaryEncoding, FileDictionaryKey},
    durable,
    nested_schema::NestedSchemaNodeV1,
    page::{ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN},
    page_payload::ColumnPagePayloadV1,
    profile::{
        cove_map::{
            MapAliasEntry, MapIdentityRule, MapJoinKeyComponent, MapNormalizationPipeline,
            MapPropertyBinding, MapResolver, MapRowSemanticRule, SourceOperationKind,
        },
        cove_o::{
            temporal_row_trust_payload, CoveObjectState, CoveRecordRefV1, ObjectTypeCatalog,
            ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1,
            TemporalSegmentData, TemporalSegmentHeaderV1, TemporalSegmentIndex,
            TemporalSegmentIndexEntryV1, TrustManifest, TrustManifestEntryV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            OBJECT_TYPE_FLAG_LINK_OBJECT, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
            PROPERTY_FLAG_EVIDENCE_REF, PROPERTY_FLAG_MAPPING_RULE_REF, TEMPORAL_ROW_ENTRY_LEN,
            TEMPORAL_SEGMENT_HEADER_LEN,
        },
    },
    reader::{validate_bytes_with_options, ValidationOptions},
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    trust_chain,
    writer::{MinimalCoveWriter, SectionPayload},
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

mod alias_import;
mod api;
mod build;
mod candidates;
mod cli;
mod context;
mod emit;
mod identity;
mod input;
mod materialization;
mod parity;
mod project;
mod replay;
mod review;
mod sections;
mod suggest;
mod support;
mod ui;
mod verify;

#[cfg(test)]
use crate::cli::{parse_args, Command, OutputFormat};
pub use api::{
    candidate_matches_from_paths, conversion_report_from_paths, conversion_summary_from_paths,
    cove_o_from_paths, projected_output_from_cove_o_bytes, projected_output_from_cove_o_path,
    projected_output_from_paths, projected_record_batch_from_cove_o_bytes,
    projected_record_batches_from_cove_o_bytes,
    projected_record_batches_from_cove_o_bytes_with_catalog,
    projected_record_batches_from_cove_o_surface_with_catalog, projected_rows_from_cove_o_path,
    projected_rows_from_paths, projection_arrow_schema, projection_catalog_from_cove_o_bytes,
    projection_covi_filter_plan, projection_descriptors_from_cove_o_path,
    projection_read_requirements_for_catalog, verify_replay_report_from_paths, MapApiError,
    MapApiResult, ProjectionColumnDescriptor, ProjectionColumnLineageDescriptor,
    ProjectionCoviFilterDiagnostic, ProjectionCoviFilterLookup, ProjectionCoviFilterPlan,
    ProjectionDescriptor,
};
pub(crate) use api::{parse_map, plan_keys, preview};
use build::verify_from_paths;
pub use build::{
    build_from_cove_o_bytes, build_from_paths, build_semantic_delta_from_paths,
    publish_covm_from_bundle, MapBuildOptions, MapBuildProjectionOutput, MapBuildResult,
    MapBuildSectionCompression, MapEvidenceEncoding, MapSemanticDeltaBuildOptions,
    MapSemanticDeltaBuildResult, MapSemanticDeltaParent,
};
pub(crate) use candidates::candidate_matches;
pub(crate) use context::{mapping_context, MappingContext};
#[cfg(test)]
use emit::build_cove_o;
use emit::build_cove_o_with_source_states;
pub(crate) use identity::{plan_identities, CandidateMatch, PlannedIdentity};
#[cfg(test)]
use input::read_csv;
use input::{
    read_source_inputs, read_sources, validate_source_inputs, ObservedSourceState, SourceRow,
};
pub use materialization::CoveObjectCheckpointTemporalSection;
pub(crate) use materialization::{
    MaterializedModel, MaterializedProperty, NestedShapeByProperty, ObjectRow,
    ReconstructedTemporalSegmentBuild, TemporalSegmentBuild,
};
use parity::{parity_from_cove_o_path, parity_from_paths, parity_has_failures, ParityOptions};
use project::{
    diff_maps, project_cove_o_path_output, project_rows_with_source_states_output, run_fixture_path,
};
#[cfg(test)]
use project::{project_cove_o_path, project_rows, property_by_name};
pub use project::{
    ProjectionBatchOptions, ProjectionCandidateRows, ProjectionFilter, ProjectionFilterLiteral,
    ProjectionFilterOp, ProjectionFormat, ProjectionReadRequirements,
};
pub(crate) use replay::verify_replay_report;
pub(crate) use review::review_worklist_from_candidate_matches;
pub(crate) use sections::{embedded_sections, mapping_identity, section_kind};
#[cfg(test)]
use std::path::PathBuf;
use suggest::suggest_from_paths;
pub(crate) use support::*;
pub(crate) use ui::{
    candidate_assertion_id, candidate_match_id, evidence_entry_for_candidate,
    evidence_entry_for_identity, explain, identity_assertion_id, print_json, print_usage,
    write_or_print,
};
use verify::{report_has_failures, verify_bundle_dir};

pub use cli::{run_cli, MapCliError};

#[derive(Debug, Clone)]
struct ReviewedDecisionReplayBinding {
    count: usize,
    digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct JoinKeyEvaluation {
    pub(crate) tuple: Vec<u8>,
    pub(crate) materializes_identity: bool,
    pub(crate) effective_confidence_class: Option<String>,
    pub(crate) resolution_metadata: Vec<ResolutionMetadata>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolutionMetadata {
    pub(crate) role_id: String,
    pub(crate) resolution_kind: String,
    pub(crate) resolver_id: String,
    pub(crate) resolver_digest: String,
    pub(crate) catalog_digest: String,
    pub(crate) pipeline_digest: String,
    pub(crate) normalization_pipeline_id: String,
    pub(crate) evidence_policy: String,
    pub(crate) redacted_resolution_evidence: bool,
    pub(crate) raw_observed_value: String,
    pub(crate) normalized_value: String,
    pub(crate) resolved_identity_value: Option<String>,
    pub(crate) canonical_key: Option<String>,
    pub(crate) canonical_label: Option<String>,
    pub(crate) alias_catalog_id: Option<String>,
    pub(crate) alias_entry_id: Option<String>,
    pub(crate) alias_hit: bool,
    pub(crate) alias_miss: bool,
    pub(crate) alias_ambiguous: bool,
    pub(crate) miss_policy: Option<String>,
}

fn materialize_with_source_states(
    file: &CovemapFile,
    rows: &[SourceRow],
    source_states: &[ObservedSourceState],
) -> Result<MaterializedModel, String> {
    let context = mapping_context(file)?;
    let identity_plan = plan_identities(file, rows)?;
    let planned = &identity_plan.canonical;
    let object_types = object_types_from_mapping(&context)?;
    let type_ids = object_types
        .iter()
        .map(|ty| (ty.type_name.clone(), ty.object_type_id))
        .collect::<BTreeMap<_, _>>();
    let properties_by_type = object_types
        .iter()
        .map(|ty| {
            (
                ty.object_type_id,
                ty.properties
                    .iter()
                    .map(|property| (property.property_id, property.clone()))
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let source_rows = rows
        .iter()
        .map(|row| ((row.source_id.clone(), row.row_index), row))
        .collect::<BTreeMap<_, _>>();
    let planned_by_key = planned
        .iter()
        .map(|identity| {
            (
                (
                    identity.source_id.clone(),
                    identity.row_index,
                    identity.identity_rule_id.clone(),
                ),
                identity,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut planned_by_join = BTreeMap::<(String, String), Vec<&PlannedIdentity>>::new();
    for identity in planned {
        planned_by_join
            .entry((
                identity.identity_rule_id.clone(),
                identity.join_key_sha256.clone(),
            ))
            .or_default()
            .push(identity);
    }
    let row_rules = context
        .row_rules
        .iter()
        .map(|rule| (rule.rule_id.clone(), rule))
        .collect::<BTreeMap<_, _>>();
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    let candidate_rule_output = candidate_matches(file, rows)?;
    let candidate_rule_matches = candidate_rule_output["candidate_matches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut object_rows = Vec::new();
    let mut assertions = Vec::new();
    let mut evidence_entries = Vec::new();
    for row_rule in &context.row_rules {
        for binding in &row_rule.property_bindings {
            push_unique_assertion(
                &mut assertions,
                &binding.assertion_id,
                &format!("property:{}", binding.assertion_id),
            );
        }
        for binding in &row_rule.association_bindings {
            push_unique_assertion(
                &mut assertions,
                &binding.assertion_id,
                &format!("association:{}", binding.assertion_id),
            );
        }
    }

    for candidate in &identity_plan.candidates {
        let assertion_id = candidate_assertion_id(candidate);
        let candidate_id = candidate_match_id(candidate);
        push_unique_assertion(&mut assertions, &assertion_id, &candidate_id);
        let mut evidence = evidence_entry_for_candidate(candidate);
        if let Some(row_rule) = row_rules.get(&candidate.row_rule_id) {
            add_operation_metadata(&mut evidence, row_rule, None);
        }
        evidence_entries.push(evidence);
    }
    for candidate in &candidate_rule_matches {
        let candidate_id = candidate
            .get("candidate_match_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "candidate rule output missing candidate_match_id".to_string())?;
        push_unique_assertion(&mut assertions, candidate_id, candidate_id);
        evidence_entries.push(candidate_rule_evidence_entry(candidate)?);
    }

    for identity in planned {
        let row_rule = row_rules.get(&identity.row_rule_id).ok_or_else(|| {
            format!(
                "planned row references missing row rule '{}'",
                identity.row_rule_id
            )
        })?;
        let source_row = source_rows
            .get(&(identity.source_id.clone(), identity.row_index))
            .ok_or_else(|| "planned identity references missing source row".to_string())?;
        let assertion_id = identity_assertion_id(identity);
        if !row_rule_materializes_object(row_rule)? {
            if row_rule_emits_non_object_evidence(row_rule) {
                push_unique_assertion(&mut assertions, &assertion_id, &hex_encode(&identity.goid));
                let mut evidence = evidence_entry_for_identity(identity);
                add_operation_metadata(&mut evidence, row_rule, Some(source_row));
                evidence_entries.push(evidence);
            }
            continue;
        }
        let object_type_id = *type_ids
            .get(&identity.object_type)
            .ok_or_else(|| format!("unknown object type '{}'", identity.object_type))?;
        let properties = materialize_properties(
            &context,
            row_rule,
            source_row,
            object_type_id,
            &properties_by_type,
        )?;
        let record_id = record_id_for(
            &identity.source_id,
            identity.row_index,
            &identity.row_rule_id,
            &identity.goid,
        );
        object_rows.push(ObjectRow {
            goid: identity.goid,
            record_id,
            object_type_id,
            object_type: identity.object_type.clone(),
            source_id: identity.source_id.clone(),
            source_row_index: identity.row_index,
            record_kind: record_kind_for_row_rule(row_rule)?,
            properties,
        });
        push_unique_assertion(&mut assertions, &assertion_id, &hex_encode(&identity.goid));
        let mut evidence = evidence_entry_for_identity(identity);
        add_operation_metadata(&mut evidence, row_rule, Some(source_row));
        evidence_entries.push(evidence);
    }

    materialize_associations(
        file,
        &context,
        planned,
        &planned_by_key,
        &planned_by_join,
        &source_rows,
        &type_ids,
        &properties_by_type,
        &mut object_rows,
        &mut assertions,
        &mut evidence_entries,
    )?;

    resolve_property_conflicts(&mut object_rows, &mut evidence_entries)?;
    prune_empty_shadow_rows(&mut object_rows);

    object_rows.sort_by_key(|row| {
        (
            row.object_type_id,
            row.source_id.clone(),
            row.source_row_index,
            row.goid,
            row.record_id,
        )
    });
    let reviewed_decision_replay = reviewed_decision_replay_binding(file)?;
    let conversion_report = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "sources": conversion_report_sources(rows, source_states),
        "source_count": rows.iter().map(|row| row.source_id.clone()).collect::<BTreeSet<_>>().len(),
        "row_count": rows.len(),
        "object_count": object_rows.iter().filter(|row| !row.object_type.starts_with("Association:")).count(),
        "association_count": object_rows.iter().filter(|row| row.object_type.starts_with("Association:")).count(),
        "property_value_count": object_rows.iter().map(|row| row.properties.len()).sum::<usize>(),
        "candidate_match_count": identity_plan.candidates.len() + candidate_rule_matches.len(),
        "resolver_hit_count": evidence_bool_count(&evidence_entries, "alias_hit"),
        "resolver_miss_count": evidence_bool_count(&evidence_entries, "alias_miss"),
        "ambiguous_alias_count": evidence_bool_count(&evidence_entries, "alias_ambiguous"),
        "resolver_catalog_digests": resolver_catalog_digests(&evidence_entries),
        "reviewed_decision_count": reviewed_decision_replay.count,
        "reviewed_decision_catalog_digest": reviewed_decision_replay.digest,
        "resolver_goid_impact": resolver_goid_impact(&evidence_entries),
        "candidate_matches": identity_plan.candidates.iter().map(|candidate| {
            json!({
                "candidate_match_id": candidate_match_id(candidate),
                "source_id": candidate.source_id,
                "source_row_identity": candidate.source_row_identity,
                "row_rule_id": candidate.row_rule_id,
                "identity_rule_id": candidate.identity_rule_id,
                "object_type": candidate.object_type,
                "join_key_sha256": candidate.join_key_sha256,
            })
        }).chain(candidate_rule_matches.iter().map(candidate_rule_report_entry)).collect::<Vec<_>>(),
        "generated_artifacts": ["cove-o", "map-assertion-log", "map-identity-equivalence-index", "map-evidence-index"],
        "unsupported": [],
        "operation_counts": operation_counts(&evidence_entries),
        "governance": governance_report(&context, rows)?,
    });
    let assertion_log = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "assertions": assertions,
    });
    let identity_equivalence_index =
        identity_equivalence_index(&mapping_id, &mapping_version, planned);
    let evidence_index = json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "entries": evidence_entries,
    });
    Ok(MaterializedModel {
        object_types,
        rows: object_rows,
        assertions,
        assertion_log,
        identity_equivalence_index,
        evidence_entries,
        evidence_index,
        conversion_report,
    })
}

fn push_unique_assertion(assertions: &mut Vec<Value>, assertion_id: &str, output_object_id: &str) {
    if assertions.iter().any(|entry| {
        entry.get("assertion_id").and_then(Value::as_str) == Some(assertion_id)
            || entry.get("output_object_id").and_then(Value::as_str) == Some(output_object_id)
    }) {
        return;
    }
    assertions.push(json!({
        "assertion_id": assertion_id,
        "output_object_id": output_object_id,
    }));
}

fn candidate_rule_report_entry(candidate: &Value) -> Value {
    let left = candidate.get("left").and_then(Value::as_object);
    let right = candidate.get("right").and_then(Value::as_object);
    json!({
        "candidate_match_id": candidate.get("candidate_match_id").cloned().unwrap_or(Value::Null),
        "match_rule_id": candidate.get("match_rule_id").cloned().unwrap_or(Value::Null),
        "object_type": candidate.get("object_type").cloned().unwrap_or(Value::Null),
        "candidate_score": candidate.get("candidate_score").cloned().unwrap_or(Value::Null),
        "score_scale": candidate.get("score_scale").cloned().unwrap_or(Value::Null),
        "blocking_key": candidate.get("blocking_key").cloned().unwrap_or(Value::Null),
        "left_source_id": nested_candidate_value(left, "source_id"),
        "left_source_row_identity": nested_candidate_value(left, "source_row_identity"),
        "left_raw_observed_value": nested_candidate_value(left, "raw_value"),
        "left_normalized_value": nested_candidate_value(left, "normalized_value"),
        "left_row_digest": nested_candidate_value(left, "row_digest"),
        "right_source_id": nested_candidate_value(right, "source_id"),
        "right_source_row_identity": nested_candidate_value(right, "source_row_identity"),
        "right_raw_observed_value": nested_candidate_value(right, "raw_value"),
        "right_normalized_value": nested_candidate_value(right, "normalized_value"),
        "right_row_digest": nested_candidate_value(right, "row_digest"),
    })
}

fn candidate_rule_evidence_entry(candidate: &Value) -> Result<Value, String> {
    let left = candidate
        .get("left")
        .and_then(Value::as_object)
        .ok_or_else(|| "candidate rule output missing left member".to_string())?;
    let report = candidate_rule_report_entry(candidate);
    Ok(json!({
        "source_id": nested_candidate_value(Some(left), "source_id"),
        "source_row_identity": nested_candidate_value(Some(left), "source_row_identity"),
        "rule_id": candidate.get("match_rule_id").cloned().unwrap_or(Value::Null),
        "assertion_id": candidate.get("candidate_match_id").cloned().unwrap_or(Value::Null),
        "output_object_id": candidate.get("candidate_match_id").cloned().unwrap_or(Value::Null),
        "candidate": true,
        "candidate_match_id": report["candidate_match_id"].clone(),
        "candidate_score": report["candidate_score"].clone(),
        "match_rule_id": report["match_rule_id"].clone(),
        "object_type": report["object_type"].clone(),
        "blocking_key": report["blocking_key"].clone(),
        "left_source_id": report["left_source_id"].clone(),
        "left_source_row_identity": report["left_source_row_identity"].clone(),
        "left_raw_observed_value": report["left_raw_observed_value"].clone(),
        "left_normalized_value": report["left_normalized_value"].clone(),
        "left_row_digest": report["left_row_digest"].clone(),
        "right_source_id": report["right_source_id"].clone(),
        "right_source_row_identity": report["right_source_row_identity"].clone(),
        "right_raw_observed_value": report["right_raw_observed_value"].clone(),
        "right_normalized_value": report["right_normalized_value"].clone(),
        "right_row_digest": report["right_row_digest"].clone(),
    }))
}

fn nested_candidate_value(object: Option<&Map<String, Value>>, key: &str) -> Value {
    object
        .and_then(|object| object.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn conversion_report_sources(rows: &[SourceRow], source_states: &[ObservedSourceState]) -> Value {
    if !source_states.is_empty() {
        return Value::Array(
            source_states
                .iter()
                .map(|state| {
                    json!({
                        "source_id": state.source_id,
                        "source_kind": state.source_kind,
                        "schema_fingerprint": state.schema_fingerprint,
                        "snapshot_digest": state.snapshot_digest,
                    })
                })
                .collect(),
        );
    }
    Value::Array(
        rows.iter()
            .map(|row| {
                json!({
                    "source_id": row.source_id,
                    "schema_fingerprint": schema_fingerprint(row),
                })
            })
            .collect(),
    )
}

fn add_operation_metadata(
    evidence: &mut Value,
    row_rule: &MapRowSemanticRule,
    source_row: Option<&SourceRow>,
) {
    let Some(object) = evidence.as_object_mut() else {
        return;
    };
    object.insert(
        "source_operation_kind".into(),
        json!(row_rule.source_operation_kind.as_str()),
    );
    object.insert(
        "operation_effect".into(),
        json!(operation_effect(row_rule.source_operation_kind)),
    );
    object.insert("operation_target".into(), json!(operation_target(row_rule)));
    if let Some(source_row) = source_row {
        copy_operation_policy_value(object, source_row, "correction_of");
        copy_operation_policy_value(object, source_row, "replacement_of");
        copy_operation_policy_value(object, source_row, "redaction_scope");
        copy_operation_policy_value(object, source_row, "expires_previous");
        copy_operation_policy_value(object, source_row, "closes_association");
    }
}

fn copy_operation_policy_value(object: &mut Map<String, Value>, source_row: &SourceRow, key: &str) {
    if let Some(value) = source_row.values.get(key).filter(|value| !value.is_null()) {
        object.insert(key.to_string(), value.clone());
    }
}

fn operation_counts(evidence_entries: &[Value]) -> Value {
    let mut counts = BTreeMap::<String, u64>::new();
    for entry in evidence_entries {
        if let Some(kind) = entry.get("source_operation_kind").and_then(Value::as_str) {
            *counts.entry(kind.to_string()).or_default() += 1;
        }
    }
    json!(counts)
}

fn evidence_bool_count(evidence_entries: &[Value], key: &str) -> usize {
    evidence_entries
        .iter()
        .filter(|entry| entry.get(key).and_then(Value::as_bool) == Some(true))
        .count()
}

fn reviewed_decision_replay_binding(
    file: &CovemapFile,
) -> Result<ReviewedDecisionReplayBinding, String> {
    let decisions = match file
        .sections
        .iter()
        .find(|section| section.entry.section_id == SectionKind::MapResolutionCatalog as u32)
    {
        Some(section) => {
            let payload: Value = serde_json::from_slice(&section.payload)
                .map_err(|err| format!("invalid MAP_RESOLUTION_CATALOG JSON: {err}"))?;
            payload
                .get("reviewed_decisions")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    "MAP_RESOLUTION_CATALOG missing reviewed_decisions array".to_string()
                })?
                .clone()
        }
        None => Vec::new(),
    };
    let count = decisions.len();
    let digest = alias_import::digest_json(&json!({
        "reviewed_decisions": decisions
    }))?;
    Ok(ReviewedDecisionReplayBinding { count, digest })
}

fn resolver_catalog_digests(evidence_entries: &[Value]) -> Value {
    let digests = evidence_entries
        .iter()
        .filter_map(|entry| {
            Some((
                entry.get("resolver_id")?.as_str()?.to_string(),
                entry
                    .get("normalization_pipeline_id")?
                    .as_str()?
                    .to_string(),
                entry.get("resolver_digest")?.as_str()?.to_string(),
                entry.get("catalog_digest")?.as_str()?.to_string(),
                entry.get("pipeline_digest")?.as_str()?.to_string(),
            ))
        })
        .collect::<BTreeSet<_>>();
    Value::Array(
        digests
            .into_iter()
            .map(
                |(
                    resolver_id,
                    normalization_pipeline_id,
                    resolver_digest,
                    catalog_digest,
                    pipeline_digest,
                )| {
                    json!({
                        "resolver_id": resolver_id,
                        "normalization_pipeline_id": normalization_pipeline_id,
                        "resolver_digest": resolver_digest,
                        "catalog_digest": catalog_digest,
                        "pipeline_digest": pipeline_digest,
                    })
                },
            )
            .collect(),
    )
}

fn resolver_goid_impact(evidence_entries: &[Value]) -> Value {
    let mut impacted =
        BTreeMap::<(String, String, String, String, String), BTreeSet<String>>::new();
    for entry in evidence_entries {
        let Some(resolver_id) = entry.get("resolver_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(normalization_pipeline_id) = entry
            .get("normalization_pipeline_id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(resolver_digest) = entry.get("resolver_digest").and_then(Value::as_str) else {
            continue;
        };
        let Some(catalog_digest) = entry.get("catalog_digest").and_then(Value::as_str) else {
            continue;
        };
        let Some(pipeline_digest) = entry.get("pipeline_digest").and_then(Value::as_str) else {
            continue;
        };
        let Some(output_object_id) = entry.get("output_object_id").and_then(Value::as_str) else {
            continue;
        };
        impacted
            .entry((
                resolver_id.to_string(),
                normalization_pipeline_id.to_string(),
                resolver_digest.to_string(),
                catalog_digest.to_string(),
                pipeline_digest.to_string(),
            ))
            .or_default()
            .insert(output_object_id.to_string());
    }
    Value::Array(
        impacted
            .into_iter()
            .map(
                |(
                    (
                        resolver_id,
                        normalization_pipeline_id,
                        resolver_digest,
                        catalog_digest,
                        pipeline_digest,
                    ),
                    affected_goids,
                )| {
                    let affected_goids = affected_goids.into_iter().collect::<Vec<_>>();
                    json!({
                        "resolver_id": resolver_id,
                        "normalization_pipeline_id": normalization_pipeline_id,
                        "resolver_digest": resolver_digest,
                        "catalog_digest": catalog_digest,
                        "pipeline_digest": pipeline_digest,
                        "affected_goid_count": affected_goids.len(),
                        "affected_goids": affected_goids,
                    })
                },
            )
            .collect(),
    )
}

fn operation_effect(kind: SourceOperationKind) -> &'static str {
    match kind {
        SourceOperationKind::Fact => "fact",
        SourceOperationKind::Insert => "insert_object_state",
        SourceOperationKind::Upsert => "upsert_object_state",
        SourceOperationKind::PatchProperty => "patch_property",
        SourceOperationKind::ReplaceObjectState => "replace_object_state",
        SourceOperationKind::CloseAssociation => "close_association",
        SourceOperationKind::ExpireAndCreate => "expire_and_create",
        SourceOperationKind::TombstoneObject => "tombstone_object",
        SourceOperationKind::TombstoneProperty => "tombstone_property",
        SourceOperationKind::TombstoneAssociation => "tombstone_association",
        SourceOperationKind::RedactEvidence => "redact_evidence",
        SourceOperationKind::EvidenceOnly => "evidence_only",
        SourceOperationKind::Correction => "correction",
    }
}

fn operation_target(row_rule: &MapRowSemanticRule) -> &'static str {
    if let Some(target) = row_rule.tombstone_target.as_deref() {
        return match target {
            "property" => "property",
            "association" => "association",
            "source_record" => "source_record",
            "evidence" => "evidence",
            _ => "object",
        };
    }
    match row_rule.source_operation_kind {
        SourceOperationKind::PatchProperty | SourceOperationKind::TombstoneProperty => "property",
        SourceOperationKind::CloseAssociation | SourceOperationKind::TombstoneAssociation => {
            "association"
        }
        SourceOperationKind::RedactEvidence | SourceOperationKind::EvidenceOnly => "evidence",
        _ => "object",
    }
}

fn row_rule_emits_non_object_evidence(row_rule: &MapRowSemanticRule) -> bool {
    row_rule.assertion_kinds.iter().any(|kind| {
        matches!(
            kind.as_str(),
            "evidence" | "candidate_match" | "conflict" | "projection"
        )
    }) || matches!(
        row_rule.source_operation_kind,
        SourceOperationKind::EvidenceOnly | SourceOperationKind::RedactEvidence
    )
}

fn governance_report(context: &MappingContext, rows: &[SourceRow]) -> Result<Value, String> {
    let used_source_ids = rows
        .iter()
        .map(|row| row.source_id.clone())
        .collect::<BTreeSet<_>>();
    let mut sources = Vec::new();
    let mut access_policy_ids = BTreeSet::<String>::new();
    let mut sensitivity_identities = BTreeSet::<(Option<String>, Option<i64>)>::new();
    let mut max_sensitivity_rank = 0i64;
    let mut labels_by_rank = BTreeMap::<i64, BTreeSet<String>>::new();

    for source_id in used_source_ids {
        let Some(source) = context.sources.get(&source_id) else {
            sources.push(json!({ "source_id": source_id }));
            continue;
        };
        for policy_id in &source.access_policy_ids {
            access_policy_ids.insert(policy_id.clone());
        }
        if source.sensitivity_label.is_some() || source.sensitivity_rank.is_some() {
            sensitivity_identities
                .insert((source.sensitivity_label.clone(), source.sensitivity_rank));
        }
        let rank = source.sensitivity_rank.unwrap_or(0);
        max_sensitivity_rank = max_sensitivity_rank.max(rank);
        if let Some(label) = &source.sensitivity_label {
            labels_by_rank
                .entry(rank)
                .or_default()
                .insert(label.clone());
        }
        sources.push(json!({
            "source_id": source.source_id,
            "source_priority": source.source_priority,
            "sensitivity_label": source.sensitivity_label.clone(),
            "sensitivity_rank": source.sensitivity_rank,
            "access_policy_ids": source.access_policy_ids.clone(),
        }));
    }

    if context.governance_reconciliation_policy == "reject_on_mixed_sensitivity"
        && sensitivity_identities.len() > 1
    {
        return Err("mixed source sensitivity labels require governance reconciliation".into());
    }

    Ok(json!({
        "reconciliation_policy": context.governance_reconciliation_policy,
        "sources": sources,
        "effective_sensitivity_rank": max_sensitivity_rank,
        "effective_sensitivity_labels": labels_by_rank
            .remove(&max_sensitivity_rank)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>(),
        "access_policy_ids": access_policy_ids.into_iter().collect::<Vec<_>>(),
    }))
}

fn materialize_properties(
    context: &MappingContext,
    row_rule: &MapRowSemanticRule,
    source_row: &SourceRow,
    object_type_id: u32,
    properties_by_type: &BTreeMap<u32, BTreeMap<u32, PropertyEntryV1>>,
) -> Result<BTreeMap<u32, MaterializedProperty>, String> {
    let declared = properties_by_type
        .get(&object_type_id)
        .ok_or_else(|| format!("object_type_id {object_type_id} has no property catalog"))?;
    let mut properties = BTreeMap::new();
    for (index, binding) in row_rule.property_bindings.iter().enumerate() {
        let property_id = property_id_from_binding(binding, index as u32 + 1);
        let entry = declared.get(&property_id).ok_or_else(|| {
            format!(
                "row rule '{}' references undeclared property '{}'",
                row_rule.rule_id, binding.property_id
            )
        })?;
        let value = source_value_for_binding(source_row, binding)?;
        validate_property_conflict_policy(&binding.conflict_policy)?;
        if value.is_null() && !entry.nullable {
            return Err(format!(
                "non-nullable property '{}' was null/missing for {}:{}",
                binding.property_name, source_row.source_id, source_row.row_index
            ));
        }
        let source_order = context
            .source_order
            .get(&source_row.source_id)
            .copied()
            .unwrap_or(usize::MAX);
        let source_priority = binding
            .source_priority
            .or_else(|| {
                context
                    .sources
                    .get(&source_row.source_id)
                    .and_then(|source| source.source_priority)
            })
            .unwrap_or(source_order as i64);
        if properties
            .insert(
                property_id,
                MaterializedProperty {
                    entry: entry.clone(),
                    value,
                    assertion_id: binding.assertion_id.clone(),
                    source_id: source_row.source_id.clone(),
                    source_row_index: source_row.row_index,
                    source_priority,
                    source_order,
                    conflict_policy: binding.conflict_policy.clone(),
                },
            )
            .is_some()
            && binding.conflict_policy == "reject_conflict"
        {
            return Err(format!(
                "duplicate materialized value for property '{}'",
                binding.property_name
            ));
        }
    }
    Ok(properties)
}

fn validate_property_conflict_policy(policy: &str) -> Result<(), String> {
    match policy {
        "reject_conflict" | "source_priority_wins" => Ok(()),
        other => Err(format!("unsupported property conflict_policy '{other}'")),
    }
}

fn prune_empty_shadow_rows(rows: &mut Vec<ObjectRow>) {
    let populated = rows
        .iter()
        .filter(|row| !row.properties.is_empty())
        .map(|row| (row.object_type_id, row.goid))
        .collect::<BTreeSet<_>>();
    rows.retain(|row| {
        !(row.properties.is_empty() && populated.contains(&(row.object_type_id, row.goid)))
    });
}

fn resolve_property_conflicts(
    rows: &mut [ObjectRow],
    evidence_entries: &mut Vec<Value>,
) -> Result<(), String> {
    let mut groups = BTreeMap::<([u8; 16], u32), Vec<(usize, MaterializedProperty)>>::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (property_id, property) in &row.properties {
            groups
                .entry((row.goid, *property_id))
                .or_default()
                .push((row_index, property.clone()));
        }
    }

    let mut removals = Vec::<(usize, u32, String)>::new();
    for ((goid, property_id), candidates) in groups {
        if candidates.len() <= 1 {
            continue;
        }
        let policies = candidates
            .iter()
            .map(|(_, property)| property.conflict_policy.as_str())
            .collect::<BTreeSet<_>>();
        if policies.len() != 1 {
            return Err(format!(
                "conflicting policies declared for property_id {property_id} on {}",
                hex_encode(&goid)
            ));
        }
        let policy = policies.iter().next().copied().unwrap_or("reject_conflict");
        validate_property_conflict_policy(policy)?;

        let non_null = candidates
            .iter()
            .filter(|(_, property)| !property.value.is_null())
            .cloned()
            .collect::<Vec<_>>();
        if non_null.is_empty() {
            continue;
        }

        match policy {
            "reject_conflict" => {
                let first = &non_null[0].1.value;
                if non_null
                    .iter()
                    .any(|(_, property)| property.value != *first)
                {
                    return Err(format!(
                        "unresolved property conflict for property_id {property_id} on {}",
                        hex_encode(&goid)
                    ));
                }
                for (row_index, property) in candidates {
                    if property.value.is_null() {
                        removals.push((
                            row_index,
                            property_id,
                            "null_does_not_overwrite_non_null".into(),
                        ));
                    }
                }
            }
            "source_priority_wins" => {
                let (winner_row, winner) = non_null
                    .iter()
                    .min_by_key(|(row_index, property)| {
                        (
                            property.source_priority,
                            property.source_order,
                            property.source_row_index,
                            property.assertion_id.clone(),
                            *row_index,
                        )
                    })
                    .map(|(row_index, property)| (*row_index, property.clone()))
                    .ok_or_else(|| "empty source-priority conflict group".to_string())?;
                for (row_index, property) in candidates {
                    if row_index != winner_row || property.assertion_id != winner.assertion_id {
                        removals.push((row_index, property_id, "source_priority_wins".into()));
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    for (row_index, property_id, reason) in removals {
        if let Some(property) = rows
            .get_mut(row_index)
            .and_then(|row| row.properties.remove(&property_id))
        {
            let source_id = property.source_id.clone();
            evidence_entries.push(json!({
                "source_id": source_id,
                "source_row_identity": format!("{}:{}", property.source_id, property.source_row_index),
                "rule_id": "property_conflict_resolution",
                "assertion_id": property.assertion_id,
                "output_object_id": hex_encode(&rows[row_index].goid),
                "property_id": property_id,
                "property_name": property.entry.property_name,
                "suppressed": true,
                "suppressed_reason": reason,
                "suppressed_value": property.value,
            }));
        }
    }

    Ok(())
}

fn source_value_for_binding(
    source_row: &SourceRow,
    binding: &MapPropertyBinding,
) -> Result<Value, String> {
    source_value_for_expression(
        source_row,
        &binding.value_expression,
        Some(&binding.source_column),
        &binding.missing_policy,
        &binding.property_name,
    )
}

fn source_value_for_expression(
    source_row: &SourceRow,
    expression: &str,
    fallback_column: Option<&str>,
    missing_policy: &str,
    label: &str,
) -> Result<Value, String> {
    let expression = expression.trim();
    let column = expression.strip_prefix("source.").unwrap_or_else(|| {
        if expression.is_empty() {
            fallback_column.unwrap_or("")
        } else {
            expression
        }
    });
    match source_row.values.get(column) {
        Some(value) if !value.is_null() => Ok(value.clone()),
        _ if missing_policy == "reject" => Err(format!(
            "source column '{}' required by '{}' is missing/null",
            column, label
        )),
        _ => Ok(Value::Null),
    }
}

fn association_validity_value(
    source_row: &SourceRow,
    expression: Option<&str>,
    missing_policy: &str,
    label: &str,
) -> Result<Option<Value>, String> {
    let Some(expression) = expression else {
        return Ok(Some(Value::Null));
    };
    let value = source_value_for_expression(source_row, expression, None, "null", label)?;
    if !value.is_null() {
        return Ok(Some(value));
    }
    match missing_policy {
        "reject" => Err(format!(
            "association {label} expression '{expression}' is missing/null"
        )),
        "skip" => Ok(None),
        _ => Ok(Some(Value::Null)),
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_associations(
    file: &CovemapFile,
    context: &MappingContext,
    planned: &[PlannedIdentity],
    planned_by_key: &BTreeMap<(String, usize, String), &PlannedIdentity>,
    planned_by_join: &BTreeMap<(String, String), Vec<&PlannedIdentity>>,
    source_rows: &BTreeMap<(String, usize), &SourceRow>,
    type_ids: &BTreeMap<String, u32>,
    properties_by_type: &BTreeMap<u32, BTreeMap<u32, PropertyEntryV1>>,
    object_rows: &mut Vec<ObjectRow>,
    assertions: &mut Vec<Value>,
    evidence_entries: &mut Vec<Value>,
) -> Result<(), String> {
    let (mapping_id, mapping_version) = mapping_identity(file)?;
    let row_rules = context
        .row_rules
        .iter()
        .map(|rule| (rule.rule_id.clone(), rule))
        .collect::<BTreeMap<_, _>>();
    for identity in planned {
        let row_rule = row_rules.get(&identity.row_rule_id).ok_or_else(|| {
            format!(
                "planned identity references missing row rule '{}'",
                identity.row_rule_id
            )
        })?;
        if !row_rule_materializes_associations(row_rule)? {
            continue;
        }
        for binding in &row_rule.association_bindings {
            let source_rule = if binding.source_identity_rule_id.is_empty() {
                &row_rule.identity_rule_id
            } else {
                &binding.source_identity_rule_id
            };
            if &identity.identity_rule_id != source_rule {
                continue;
            }
            let source_row = source_rows
                .get(&(identity.source_id.clone(), identity.row_index))
                .ok_or_else(|| "association references missing source row".to_string())?;
            let Some(source_endpoint) = resolve_association_endpoint(
                &binding.source_endpoint_expression,
                source_rule,
                identity,
                source_row,
                context,
                type_ids,
                planned_by_key,
                planned_by_join,
            )?
            else {
                if binding.missing_policy == "skip" {
                    continue;
                }
                return Err(format!(
                    "association '{}' could not resolve source endpoint '{}'",
                    binding.association_type, binding.source_endpoint_expression
                ));
            };
            let Some(target) = resolve_association_endpoint(
                &binding.target_endpoint_expression,
                &binding.target_identity_rule_id,
                identity,
                source_row,
                context,
                type_ids,
                planned_by_key,
                planned_by_join,
            )?
            else {
                if binding.missing_policy == "skip" {
                    continue;
                }
                return Err(format!(
                    "association '{}' could not resolve target identity rule '{}'",
                    binding.association_type, binding.target_identity_rule_id
                ));
            };
            let object_type = format!("Association:{}", binding.association_type);
            let object_type_id = *type_ids
                .get(&object_type)
                .ok_or_else(|| format!("missing association object type '{object_type}'"))?;
            let declared = properties_by_type
                .get(&object_type_id)
                .ok_or_else(|| format!("association type '{object_type}' has no properties"))?;
            let association_goid = association_goid(
                mapping_id.as_bytes(),
                mapping_version.as_bytes(),
                binding,
                &source_endpoint.goid,
                &target.goid,
            );
            let assertion_id = format!(
                "{}:{}:{}",
                binding.assertion_id, identity.source_row_identity, identity.row_digest
            );
            let source_evidence_id = format!("{}:{}", identity.source_id, identity.row_index);
            let Some(valid_from) = association_validity_value(
                source_row,
                binding.valid_from_expression.as_deref(),
                &binding.missing_policy,
                "valid_from",
            )?
            else {
                continue;
            };
            let Some(valid_to) = association_validity_value(
                source_row,
                binding.valid_to_expression.as_deref(),
                &binding.missing_policy,
                "valid_to",
            )?
            else {
                continue;
            };
            let property_values = BTreeMap::from([
                (1u32, json!(hex_encode(&source_endpoint.goid))),
                (2u32, json!(hex_encode(&target.goid))),
                (3u32, json!(binding.association_type)),
                (4u32, json!(row_rule.rule_id)),
                (5u32, json!(source_evidence_id)),
                (6u32, json!(binding.source_role)),
                (7u32, json!(binding.target_role)),
                (8u32, valid_from),
                (9u32, valid_to),
                (10u32, json!(binding.cardinality_policy)),
            ]);
            let mut properties = BTreeMap::new();
            for (property_id, value) in property_values {
                let entry = declared.get(&property_id).ok_or_else(|| {
                    format!("association property_id {property_id} is not declared")
                })?;
                properties.insert(
                    property_id,
                    MaterializedProperty {
                        entry: entry.clone(),
                        value,
                        assertion_id: binding.assertion_id.clone(),
                        source_id: identity.source_id.clone(),
                        source_row_index: identity.row_index,
                        source_priority: context
                            .sources
                            .get(&identity.source_id)
                            .and_then(|source| source.source_priority)
                            .unwrap_or_else(|| {
                                context
                                    .source_order
                                    .get(&identity.source_id)
                                    .copied()
                                    .unwrap_or(usize::MAX) as i64
                            }),
                        source_order: context
                            .source_order
                            .get(&identity.source_id)
                            .copied()
                            .unwrap_or(usize::MAX),
                        conflict_policy: "reject_conflict".into(),
                    },
                );
            }
            let record_id = record_id_for(
                &identity.source_id,
                identity.row_index,
                &binding.assertion_id,
                &association_goid,
            );
            object_rows.push(ObjectRow {
                goid: association_goid,
                record_id,
                object_type_id,
                object_type: object_type.clone(),
                source_id: identity.source_id.clone(),
                source_row_index: identity.row_index,
                record_kind: association_record_kind_for_row_rule(row_rule),
                properties,
            });
            push_unique_assertion(
                &mut *assertions,
                &assertion_id,
                &hex_encode(&association_goid),
            );
            let mut evidence = json!({
                "source_id": identity.source_id,
                "source_row_identity": identity.source_row_identity,
                "rule_id": row_rule.rule_id,
                "assertion_id": assertion_id,
                "output_object_id": hex_encode(&association_goid),
                "observed_schema_fingerprint": identity.schema_fingerprint,
            });
            add_operation_metadata(&mut evidence, row_rule, Some(source_row));
            evidence_entries.push(evidence);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resolve_association_endpoint<'a>(
    expression: &str,
    default_identity_rule_id: &str,
    current_identity: &'a PlannedIdentity,
    source_row: &SourceRow,
    context: &MappingContext,
    type_ids: &BTreeMap<String, u32>,
    planned_by_key: &BTreeMap<(String, usize, String), &'a PlannedIdentity>,
    planned_by_join: &BTreeMap<(String, String), Vec<&'a PlannedIdentity>>,
) -> Result<Option<&'a PlannedIdentity>, String> {
    let expression = expression.trim();
    if expression == "source.goid" {
        return Ok(Some(current_identity));
    }
    let rule_id = if expression == "target.goid" || expression.is_empty() {
        default_identity_rule_id
    } else if let Some(rule_id) = expression
        .strip_prefix("identity(")
        .and_then(|value| value.strip_suffix(')'))
    {
        rule_id.trim()
    } else {
        return Err(format!(
            "unsupported association endpoint expression '{expression}'"
        ));
    };

    if rule_id == current_identity.identity_rule_id {
        return Ok(Some(current_identity));
    }
    if let Some(identity) = planned_by_key.get(&(
        source_row.source_id.clone(),
        source_row.row_index,
        rule_id.to_string(),
    )) {
        return Ok(Some(*identity));
    }
    let rule = context.identity_rules.get(rule_id).ok_or_else(|| {
        format!("association endpoint references missing identity rule '{rule_id}'")
    })?;
    let object_type_id = *type_ids
        .get(&rule.object_type)
        .ok_or_else(|| format!("unknown object type '{}'", rule.object_type))?;
    let evaluation =
        join_key_tuple_from_rule_with_context(rule, source_row, object_type_id, Some(context))?;
    if !evaluation.materializes_identity {
        return Ok(None);
    }
    let digest = sha256_hex(&evaluation.tuple);
    let Some(matches) = planned_by_join.get(&(rule_id.to_string(), digest.clone())) else {
        return Ok(None);
    };
    let distinct_goids = matches
        .iter()
        .map(|identity| identity.goid)
        .collect::<BTreeSet<_>>();
    if distinct_goids.len() == 1 {
        return Ok(matches.first().copied());
    }
    Err(format!(
        "association endpoint identity rule '{rule_id}' join key '{digest}' is ambiguous across {} GOIDs",
        distinct_goids.len()
    ))
}

fn row_rule_materializes_object(row_rule: &MapRowSemanticRule) -> Result<bool, String> {
    match row_rule.row_semantics_kind.as_str() {
        "Object" | "EventObject" | "LinkObject" | "Composite" | "Dispatched"
        | "KeyValueFragment" | "Tombstone" => Ok(true),
        "AssociationOnly" | "EvidenceOnly" | "ProjectionOnly" => Ok(false),
        other => Err(format!("unsupported row_semantics_kind '{other}'")),
    }
}

fn row_rule_materializes_associations(row_rule: &MapRowSemanticRule) -> Result<bool, String> {
    match row_rule.row_semantics_kind.as_str() {
        "Object" | "EventObject" | "LinkObject" | "AssociationOnly" | "Composite"
        | "Dispatched" | "KeyValueFragment" => Ok(true),
        "EvidenceOnly" | "ProjectionOnly" | "Tombstone" => Ok(false),
        other => Err(format!("unsupported row_semantics_kind '{other}'")),
    }
}

fn record_kind_for_row_rule(row_rule: &MapRowSemanticRule) -> Result<RecordKind, String> {
    if row_rule.row_semantics_kind == "Tombstone" {
        return Ok(RecordKind::Tombstone);
    }
    match row_rule.source_operation_kind {
        SourceOperationKind::PatchProperty
        | SourceOperationKind::CloseAssociation
        | SourceOperationKind::ExpireAndCreate
        | SourceOperationKind::RedactEvidence
        | SourceOperationKind::Correction => return Ok(RecordKind::Delta),
        SourceOperationKind::ReplaceObjectState => return Ok(RecordKind::Snapshot),
        SourceOperationKind::TombstoneObject
        | SourceOperationKind::TombstoneProperty
        | SourceOperationKind::TombstoneAssociation => return Ok(RecordKind::Tombstone),
        SourceOperationKind::Fact
        | SourceOperationKind::Insert
        | SourceOperationKind::Upsert
        | SourceOperationKind::EvidenceOnly => {}
    }
    record_kind_from_name(&row_rule.record_kind)
}

fn association_record_kind_for_row_rule(row_rule: &MapRowSemanticRule) -> RecordKind {
    match row_rule.source_operation_kind {
        SourceOperationKind::CloseAssociation
        | SourceOperationKind::ExpireAndCreate
        | SourceOperationKind::Correction => RecordKind::Delta,
        SourceOperationKind::TombstoneAssociation => RecordKind::Tombstone,
        SourceOperationKind::ReplaceObjectState => RecordKind::Snapshot,
        _ => RecordKind::Baseline,
    }
}

fn identity_equivalence_index(
    mapping_id: &str,
    mapping_version: &str,
    planned: &[PlannedIdentity],
) -> Value {
    let mut groups = BTreeMap::<String, Vec<&PlannedIdentity>>::new();
    for identity in planned {
        groups
            .entry(identity.equivalence_id.clone())
            .or_default()
            .push(identity);
    }
    let mut equivalences = Vec::new();
    let mut components = Vec::new();
    for (equivalence_id, mut members) in groups {
        members.sort_by_key(|member| {
            (
                member.canonical_anchor.clone(),
                member.identity_rule_id.clone(),
                member.source_id.clone(),
                member.row_index,
            )
        });
        let Some(anchor) = members.first().copied() else {
            continue;
        };
        for member in members.iter().skip(1) {
            if member.identity_alias == anchor.identity_alias {
                continue;
            }
            equivalences.push(json!({
                "left_identity": anchor.identity_alias,
                "right_identity": member.identity_alias,
            }));
        }
        components.push(json!({
            "equivalence_id": equivalence_id,
            "goid": hex_encode(&anchor.goid),
            "canonical_anchor": anchor.canonical_anchor,
            "members": members.iter().map(|member| json!({
                "source_id": member.source_id,
                "row_index": member.row_index,
                "source_row_identity": member.source_row_identity,
                "row_rule_id": member.row_rule_id,
                "identity_rule_id": member.identity_rule_id,
                "identity_alias": member.identity_alias,
                "object_type": member.object_type,
                "join_key_sha256": member.join_key_sha256,
                "row_digest": member.row_digest,
            })).collect::<Vec<_>>(),
        }));
    }
    json!({
        "mapping_id": mapping_id,
        "mapping_version": mapping_version,
        "equivalences": equivalences,
        "components": components,
    })
}

fn record_id_for(source_id: &str, row_index: usize, rule_id: &str, goid: &[u8; 16]) -> [u8; 16] {
    let record_material = format!("{source_id}:{row_index}:{rule_id}:{}", hex_encode(goid));
    first_16(&sha256_array(record_material.as_bytes()))
}

fn association_goid(
    mapping_id: &[u8],
    mapping_version: &[u8],
    binding: &cove_core::profile::cove_map::MapAssociationBinding,
    source_goid: &[u8; 16],
    target_goid: &[u8; 16],
) -> [u8; 16] {
    let mut tuple = Vec::new();
    tuple.extend_from_slice(source_goid);
    tuple.extend_from_slice(target_goid);
    goid16_parts(&[
        mapping_id,
        mapping_version,
        format!("Association:{}", binding.association_type).as_bytes(),
        binding.assertion_id.as_bytes(),
        &tuple,
    ])
}

fn object_types_from_mapping(context: &MappingContext) -> Result<Vec<ObjectTypeEntryV1>, String> {
    let mut object_type_names = context
        .identity_rules
        .values()
        .map(|rule| rule.object_type.clone())
        .collect::<BTreeSet<_>>();
    for row_rule in &context.row_rules {
        for binding in &row_rule.association_bindings {
            object_type_names.insert(format!("Association:{}", binding.association_type));
        }
    }
    let mut out = Vec::new();
    for (index, type_name) in object_type_names.into_iter().enumerate() {
        let mut properties = Vec::new();
        let mut seen_properties = BTreeSet::new();
        for row_rule in &context.row_rules {
            let Some(identity_rule) = context.identity_rules.get(&row_rule.identity_rule_id) else {
                continue;
            };
            if identity_rule.object_type != type_name {
                continue;
            }
            for (property_index, binding) in row_rule.property_bindings.iter().enumerate() {
                let logical = logical_type_from_name(&binding.logical_type)?;
                let property_id = property_id_from_binding(binding, property_index as u32 + 1);
                if !seen_properties.insert(property_id) {
                    continue;
                }
                properties.push(PropertyEntryV1 {
                    property_id,
                    property_name: binding.property_name.clone(),
                    logical_type: logical,
                    physical_kind: physical_kind_from_binding(binding, logical)?,
                    nullable: binding.nullable,
                    collation_id: 0,
                    flags: 0,
                });
            }
        }
        if type_name.starts_with("Association:") {
            properties.extend(association_properties());
        }
        out.push(ObjectTypeEntryV1 {
            object_type_id: (index + 1) as u32,
            flags: if type_name.starts_with("Association:") {
                OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT
            } else {
                OBJECT_TYPE_FLAG_ENTITY_OBJECT
            },
            type_name,
            properties,
        });
    }
    Ok(out)
}

fn property_id_from_binding(binding: &MapPropertyBinding, fallback: u32) -> u32 {
    stable_u32(&binding.property_id, fallback)
}

fn physical_kind_from_binding(
    binding: &MapPropertyBinding,
    logical: CoveLogicalType,
) -> Result<CovePhysicalKind, String> {
    match binding.physical_kind.as_str() {
        "auto" | "" => Ok(physical_for_logical(logical)),
        "boolean" | "bool" => Ok(CovePhysicalKind::Boolean),
        "filecode" | "file_code" => Ok(CovePhysicalKind::FileCode),
        "numcode" | "num_code" => Ok(CovePhysicalKind::NumCode),
        "fixedbytes" | "fixed_bytes" => Ok(CovePhysicalKind::FixedBytes),
        "varbytes" | "var_bytes" => Ok(CovePhysicalKind::VarBytes),
        other => Err(format!("unsupported MAP physical kind '{other}'")),
    }
}

fn association_properties() -> Vec<PropertyEntryV1> {
    vec![
        PropertyEntryV1 {
            property_id: 1,
            property_name: "source_goid".into(),
            logical_type: CoveLogicalType::Uuid,
            physical_kind: CovePhysicalKind::FixedBytes,
            nullable: false,
            collation_id: 0,
            flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
        },
        PropertyEntryV1 {
            property_id: 2,
            property_name: "target_goid".into(),
            logical_type: CoveLogicalType::Uuid,
            physical_kind: CovePhysicalKind::FixedBytes,
            nullable: false,
            collation_id: 0,
            flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        },
        PropertyEntryV1 {
            property_id: 3,
            property_name: "association_type".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: PROPERTY_FLAG_ASSOCIATION_TYPE,
        },
        PropertyEntryV1 {
            property_id: 4,
            property_name: "mapping_rule_id".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: PROPERTY_FLAG_MAPPING_RULE_REF,
        },
        PropertyEntryV1 {
            property_id: 5,
            property_name: "source_evidence_id".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: PROPERTY_FLAG_EVIDENCE_REF,
        },
        PropertyEntryV1 {
            property_id: 6,
            property_name: "source_role".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: 0,
        },
        PropertyEntryV1 {
            property_id: 7,
            property_name: "target_role".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: 0,
        },
        PropertyEntryV1 {
            property_id: 8,
            property_name: "valid_from".into(),
            logical_type: CoveLogicalType::Json,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: true,
            collation_id: 0,
            flags: 0,
        },
        PropertyEntryV1 {
            property_id: 9,
            property_name: "valid_to".into(),
            logical_type: CoveLogicalType::Json,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: true,
            collation_id: 0,
            flags: 0,
        },
        PropertyEntryV1 {
            property_id: 10,
            property_name: "cardinality_policy".into(),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::VarBytes,
            nullable: false,
            collation_id: 0,
            flags: 0,
        },
    ]
}

fn build_temporal_segments(
    materialized: &MaterializedModel,
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<TemporalSegmentBuild>, String> {
    let mut grouped = BTreeMap::<u32, Vec<ObjectRow>>::new();
    for row in &materialized.rows {
        grouped
            .entry(row.object_type_id)
            .or_default()
            .push(row.clone());
    }
    let object_types = materialized
        .object_types
        .iter()
        .map(|ty| (ty.object_type_id, ty))
        .collect::<BTreeMap<_, _>>();
    let mut out = Vec::new();
    for (segment_index, (object_type_id, mut rows)) in grouped.into_iter().enumerate() {
        rows.sort_by_key(|row| (row.source_row_index, row.goid, row.record_id));
        let object_type = object_types
            .get(&object_type_id)
            .ok_or_else(|| format!("missing object_type_id {object_type_id}"))?;
        let segment_id = u32::try_from(segment_index)
            .map_err(|_| "too many COVE-O temporal segments".to_string())?;
        let payload =
            temporal_segment_payload(segment_id, object_type, &rows, nested_shapes, dictionary)?;
        out.push(TemporalSegmentBuild {
            segment_id,
            object_type_id,
            rows,
            payload,
        });
    }
    Ok(out)
}

pub fn compact_cove_o_from_object_states(
    object_types: Vec<ObjectTypeEntryV1>,
    states: &[CoveObjectState],
) -> Result<Vec<u8>, String> {
    let segments = reconstructed_temporal_segments(&object_types, states)?;
    let segment_index = reconstructed_temporal_segment_index(&segments)?;
    let trust_manifest = reconstructed_trust_manifest(&segments)?;
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: object_types,
    };

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_TRUST_CHAIN;
    writer.sections.push(object_section(
        SectionKind::ObjectTypeCatalog,
        catalog.types.len() as u64,
        0,
        catalog.serialize().map_err(|err| err.to_string())?,
    ));
    writer.sections.push(object_section(
        SectionKind::TemporalSegmentIndex,
        segments.len() as u64,
        states.len() as u64,
        segment_index.serialize().map_err(|err| err.to_string())?,
    ));
    for segment in &segments {
        writer.sections.push(object_section(
            SectionKind::TemporalSegmentData,
            1,
            segment.rows.len() as u64,
            segment.payload.clone(),
        ));
    }
    writer.sections.push(object_section(
        SectionKind::TrustManifest,
        trust_manifest.entries.len() as u64,
        0,
        trust_manifest.serialize().map_err(|err| err.to_string())?,
    ));

    let bytes = writer.write().map_err(|err| err.to_string())?;
    validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..ValidationOptions::default()
        },
    )
    .map_err(|err| err.to_string())?;
    Ok(bytes)
}

pub fn checkpoint_temporal_sections_from_object_states(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
) -> Result<Vec<CoveObjectCheckpointTemporalSection>, String> {
    reconstructed_temporal_segments_with_record_kind(
        object_types,
        states,
        Some(RecordKind::Snapshot),
    )
    .map(|segments| {
        segments
            .into_iter()
            .map(|segment| CoveObjectCheckpointTemporalSection {
                object_type_id: segment.object_type_id,
                row_count: segment.rows.len() as u64,
                payload: segment.payload,
            })
            .collect()
    })
}

fn reconstructed_temporal_segments(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
) -> Result<Vec<ReconstructedTemporalSegmentBuild>, String> {
    reconstructed_temporal_segments_with_record_kind(object_types, states, None)
}

fn reconstructed_temporal_segments_with_record_kind(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
    record_kind_override: Option<RecordKind>,
) -> Result<Vec<ReconstructedTemporalSegmentBuild>, String> {
    let object_types_by_id = object_types
        .iter()
        .map(|object_type| (object_type.object_type_id, object_type))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<u32, Vec<CoveObjectState>>::new();
    for state in states {
        if state.record_kind == RecordKind::ReservedLegacyMaterializedDelta {
            return Err("cannot compact reserved legacy materialized-delta records".into());
        }
        grouped
            .entry(state.object_type_id)
            .or_default()
            .push(state.clone());
    }

    let mut out = Vec::new();
    for (segment_index, (object_type_id, mut rows)) in grouped.into_iter().enumerate() {
        rows.sort_by_key(|state| {
            (
                state.timestamp_us,
                state.csn,
                state.branch_key,
                state.goid,
                state.latest_record_id,
            )
        });
        let object_type = object_types_by_id
            .get(&object_type_id)
            .ok_or_else(|| format!("missing object_type_id {object_type_id}"))?;
        let segment_id = u32::try_from(segment_index)
            .map_err(|_| "too many reconstructed COVE-O temporal segments".to_string())?;
        let payload = reconstructed_temporal_segment_payload(
            segment_id,
            object_type,
            &rows,
            record_kind_override,
        )?;
        out.push(ReconstructedTemporalSegmentBuild {
            segment_id,
            object_type_id,
            rows,
            payload,
        });
    }
    Ok(out)
}

fn reconstructed_temporal_segment_payload(
    segment_id: u32,
    object_type: &ObjectTypeEntryV1,
    rows: &[CoveObjectState],
    record_kind_override: Option<RecordKind>,
) -> Result<Vec<u8>, String> {
    let row_count =
        u32::try_from(rows.len()).map_err(|_| "too many reconstructed COVE-O rows".to_string())?;
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes_len = rows
        .len()
        .checked_mul(TEMPORAL_ROW_ENTRY_LEN)
        .ok_or_else(|| "temporal row directory length overflow".to_string())?;
    let column_directory_offset = row_directory_offset
        .checked_add(row_bytes_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let column_count = u32::try_from(object_type.properties.len())
        .map_err(|_| "too many reconstructed COVE-O property columns".to_string())?;
    let column_dir_len = object_type
        .properties
        .len()
        .checked_mul(TABLE_COLUMN_DIRECTORY_ENTRY_LEN)
        .ok_or_else(|| "temporal column directory length overflow".to_string())?;
    let page_index_offset = column_directory_offset
        .checked_add(column_dir_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let total_page_index_len = object_type
        .properties
        .len()
        .checked_mul(COLUMN_PAGE_INDEX_ENTRY_LEN)
        .ok_or_else(|| "temporal page index length overflow".to_string())?;
    let data_offset = page_index_offset
        .checked_add(total_page_index_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: object_type.object_type_id,
        time_range_start_us: rows.first().map_or(0, |row| row.timestamp_us),
        time_range_end_us: rows.last().map_or(0, |row| row.timestamp_us),
        csn_min: rows.first().map_or(0, |row| row.csn),
        csn_max: rows.last().map_or(0, |row| row.csn),
        row_count,
        morsel_count: if row_count == 0 { 0 } else { 1 },
        morsel_row_count: if row_count == 0 { 0 } else { row_count },
        column_count,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };

    let mut out = header.serialize().to_vec();
    for row in rows {
        out.extend_from_slice(
            &TemporalRowEntryV1 {
                timestamp_us: row.timestamp_us,
                csn: row.csn,
                branch_key: row.branch_key,
                goid: row.goid,
                record_id: row.latest_record_id,
                record_kind: record_kind_override.unwrap_or(row.record_kind),
                prev_ref: None,
            }
            .serialize(),
        );
    }

    let mut column_directory = Vec::new();
    let mut page_index_bytes = Vec::new();
    let mut page_payload_bytes = Vec::new();
    let mut next_page_index_offset = page_index_offset;
    let mut next_data_offset = data_offset;
    for property in &object_type.properties {
        let column_page_index_offset = next_page_index_offset;
        let column_data_offset = next_data_offset;
        let page_payload = reconstructed_property_page_payload(property, rows)?;
        let page_length = page_payload.len() as u64;
        let page_checksum = checksum::crc32c(&page_payload);
        let null_count = rows
            .iter()
            .filter(|row| {
                reconstructed_property_value(row, property.property_id).is_none_or(Value::is_null)
            })
            .count() as u32;
        let page = ColumnPageIndexEntryV1 {
            column_id: property.property_id,
            morsel_id: 0,
            row_count,
            non_null_count: row_count.saturating_sub(null_count),
            null_count,
            encoding_root: encoding_for_physical(property.physical_kind) as u32,
            page_offset: next_data_offset,
            page_length,
            uncompressed_length: page_length,
            stats_ref: 0,
            flags: CompressionCodec::None as u32,
            checksum: page_checksum,
        };
        page_index_bytes.extend_from_slice(&page.serialize());
        page_payload_bytes.extend_from_slice(&page_payload);
        next_page_index_offset = next_page_index_offset
            .checked_add(COLUMN_PAGE_INDEX_ENTRY_LEN as u64)
            .ok_or_else(|| "temporal page index offset overflow".to_string())?;
        next_data_offset = next_data_offset
            .checked_add(page_length)
            .ok_or_else(|| "temporal data offset overflow".to_string())?;
        column_directory.push(TableColumnDirectoryEntryV1 {
            column_id: property.property_id,
            logical_type: property.logical_type,
            physical_kind: property.physical_kind,
            flags: 0,
            page_index_offset: column_page_index_offset,
            page_index_length: COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
            data_offset: column_data_offset,
            data_length: next_data_offset - column_data_offset,
            stats_ref: 0,
            domain_ref: 0,
            checksum: 0,
        });
    }
    for entry in &column_directory {
        out.extend_from_slice(&entry.serialize());
    }
    out.extend_from_slice(&page_index_bytes);
    out.extend_from_slice(&page_payload_bytes);
    Ok(out)
}

fn reconstructed_property_page_payload(
    property: &PropertyEntryV1,
    rows: &[CoveObjectState],
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many rows".to_string())?;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut values = Vec::new();
    let mut null_count = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        let value = reconstructed_property_value(row, property.property_id).unwrap_or(&Value::Null);
        if row
            .properties
            .iter()
            .any(|candidate| candidate.property_id == property.property_id && candidate.redacted)
        {
            return Err("cannot compact redacted COVE-O property values".into());
        }
        if value.is_null() {
            null_count += 1;
            null_bitmap[row_index / 8] |= 1u8 << (row_index % 8);
        }
        append_property_value_bytes(property, value, None, None, &mut values)?;
    }
    ColumnPagePayloadV1::build_single_node(
        row_count,
        encoding_for_physical(property.physical_kind),
        property.logical_type,
        property.physical_kind,
        (null_count != 0).then_some(null_bitmap),
        values,
    )
    .map_err(|err| err.to_string())
}

fn reconstructed_property_value(state: &CoveObjectState, property_id: u32) -> Option<&Value> {
    state
        .properties
        .iter()
        .find(|property| property.property_id == property_id)
        .map(|property| &property.value)
}

fn reconstructed_temporal_segment_index(
    segments: &[ReconstructedTemporalSegmentBuild],
) -> Result<TemporalSegmentIndex, String> {
    let mut entries = Vec::with_capacity(segments.len());
    for segment in segments {
        let min_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .min()
            .unwrap_or([0; 16]);
        let max_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .max()
            .unwrap_or([0; 16]);
        let (delta_count, snapshot_count, baseline_count, tombstone_count) =
            reconstructed_row_kind_counts(&segment.rows);
        entries.push(TemporalSegmentIndexEntryV1 {
            segment_id: segment.segment_id,
            object_type_id: segment.object_type_id,
            time_range_start_us: segment.rows.first().map_or(0, |row| row.timestamp_us),
            time_range_end_us: segment.rows.last().map_or(0, |row| row.timestamp_us),
            csn_min: segment.rows.first().map_or(0, |row| row.csn),
            csn_max: segment.rows.last().map_or(0, |row| row.csn),
            row_count: u32::try_from(segment.rows.len())
                .map_err(|_| "too many COVE-O rows".to_string())?,
            delta_count,
            snapshot_count,
            baseline_count,
            tombstone_count,
            min_goid,
            max_goid,
            offset: 0,
            length: segment.payload.len() as u64,
            checksum: 0,
        });
    }
    Ok(TemporalSegmentIndex { flags: 0, entries })
}

fn reconstructed_row_kind_counts(rows: &[CoveObjectState]) -> (u32, u32, u32, u32) {
    let mut delta = 0;
    let mut snapshot = 0;
    let mut baseline = 0;
    let mut tombstone = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta += 1,
            RecordKind::Snapshot => snapshot += 1,
            RecordKind::Baseline => baseline += 1,
            RecordKind::Tombstone => tombstone += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta, snapshot, baseline, tombstone)
}

fn reconstructed_trust_manifest(
    segments: &[ReconstructedTemporalSegmentBuild],
) -> Result<TrustManifest, String> {
    let mut previous = [0u8; 32];
    let mut entries = Vec::new();
    for segment in segments {
        let parsed_segment =
            TemporalSegmentData::parse(&segment.payload).map_err(|err| err.to_string())?;
        for index in 0..parsed_segment.rows.len() {
            let payload = temporal_row_trust_payload(
                &parsed_segment,
                index as u32,
                Option::<&FileDictionary>::None,
                &[],
            )
            .map_err(|err| err.to_string())?;
            let expected_hash =
                trust_chain::chain(&previous, &payload).map_err(|err| err.to_string())?;
            entries.push(TrustManifestEntryV1 {
                segment_id: segment.segment_id,
                row_index: index as u32,
                expected_hash,
            });
            previous = expected_hash;
        }
    }
    Ok(TrustManifest { entries })
}

fn temporal_segment_payload(
    segment_id: u32,
    object_type: &ObjectTypeEntryV1,
    rows: &[ObjectRow],
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many COVE-O rows".to_string())?;
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes_len = rows
        .len()
        .checked_mul(TEMPORAL_ROW_ENTRY_LEN)
        .ok_or_else(|| "temporal row directory length overflow".to_string())?;
    let column_directory_offset = row_directory_offset
        .checked_add(row_bytes_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let column_count = u32::try_from(object_type.properties.len())
        .map_err(|_| "too many COVE-O property columns".to_string())?;
    let column_dir_len = object_type
        .properties
        .len()
        .checked_mul(TABLE_COLUMN_DIRECTORY_ENTRY_LEN)
        .ok_or_else(|| "temporal column directory length overflow".to_string())?;
    let page_index_offset = column_directory_offset
        .checked_add(column_dir_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let total_page_index_len = object_type
        .properties
        .len()
        .checked_mul(COLUMN_PAGE_INDEX_ENTRY_LEN)
        .ok_or_else(|| "temporal page index length overflow".to_string())?;
    let data_offset = page_index_offset
        .checked_add(total_page_index_len as u64)
        .ok_or_else(|| "temporal offset overflow".to_string())?;
    let header = TemporalSegmentHeaderV1 {
        segment_id,
        object_type_id: object_type.object_type_id,
        time_range_start_us: 0,
        time_range_end_us: 0,
        csn_min: 0,
        csn_max: rows.len().saturating_sub(1) as u64,
        row_count,
        morsel_count: if row_count == 0 { 0 } else { 1 },
        morsel_row_count: if row_count == 0 { 0 } else { row_count },
        column_count,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let mut out = header.serialize().to_vec();
    let prev_refs = temporal_prev_refs(segment_id, rows);
    for (index, row) in rows.iter().enumerate() {
        out.extend_from_slice(
            &TemporalRowEntryV1 {
                timestamp_us: 0,
                csn: index as u64,
                branch_key: 0,
                goid: row.goid,
                record_id: row.record_id,
                record_kind: row.record_kind,
                prev_ref: prev_refs[index],
            }
            .serialize(),
        );
    }
    let mut column_directory = Vec::new();
    let mut page_index_bytes = Vec::new();
    let mut page_payload_bytes = Vec::new();
    let mut next_page_index_offset = page_index_offset;
    let mut next_data_offset = data_offset;
    for property in &object_type.properties {
        let column_page_index_offset = next_page_index_offset;
        let column_data_offset = next_data_offset;
        let page_payload = build_property_page_payload(
            object_type.object_type_id,
            property,
            rows,
            nested_shapes,
            dictionary,
        )?;
        let page_length = page_payload.len() as u64;
        let page_checksum = checksum::crc32c(&page_payload);
        let null_count = rows
            .iter()
            .filter(|row| {
                row.properties
                    .get(&property.property_id)
                    .is_none_or(|value| value.value.is_null())
            })
            .count() as u32;
        let page = ColumnPageIndexEntryV1 {
            column_id: property.property_id,
            morsel_id: 0,
            row_count,
            non_null_count: row_count.saturating_sub(null_count),
            null_count,
            encoding_root: encoding_for_physical(property.physical_kind) as u32,
            page_offset: next_data_offset,
            page_length,
            uncompressed_length: page_length,
            stats_ref: 0,
            flags: CompressionCodec::None as u32,
            checksum: page_checksum,
        };
        page_index_bytes.extend_from_slice(&page.serialize());
        page_payload_bytes.extend_from_slice(&page_payload);
        next_page_index_offset = next_page_index_offset
            .checked_add(COLUMN_PAGE_INDEX_ENTRY_LEN as u64)
            .ok_or_else(|| "temporal page index offset overflow".to_string())?;
        next_data_offset = next_data_offset
            .checked_add(page_length)
            .ok_or_else(|| "temporal data offset overflow".to_string())?;
        column_directory.push(TableColumnDirectoryEntryV1 {
            column_id: property.property_id,
            logical_type: property.logical_type,
            physical_kind: property.physical_kind,
            flags: 0,
            page_index_offset: column_page_index_offset,
            page_index_length: COLUMN_PAGE_INDEX_ENTRY_LEN as u64,
            data_offset: column_data_offset,
            data_length: next_data_offset - column_data_offset,
            stats_ref: 0,
            domain_ref: 0,
            checksum: 0,
        });
    }
    for entry in &column_directory {
        out.extend_from_slice(&entry.serialize());
    }
    out.extend_from_slice(&page_index_bytes);
    out.extend_from_slice(&page_payload_bytes);
    Ok(out)
}

fn build_property_page_payload(
    object_type_id: u32,
    property: &PropertyEntryV1,
    rows: &[ObjectRow],
    nested_shapes: &NestedShapeByProperty,
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<Vec<u8>, String> {
    let row_count = u32::try_from(rows.len()).map_err(|_| "too many rows".to_string())?;
    let mut null_bitmap = vec![0u8; rows.len().div_ceil(8)];
    let mut values = Vec::new();
    let mut null_count = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        let value = row
            .properties
            .get(&property.property_id)
            .map(|property| &property.value)
            .unwrap_or(&Value::Null);
        if value.is_null() {
            null_count += 1;
            null_bitmap[row_index / 8] |= 1u8 << (row_index % 8);
        }
        append_property_value_bytes(
            property,
            value,
            nested_shapes.get(&(object_type_id, property.property_id)),
            dictionary,
            &mut values,
        )?;
    }
    ColumnPagePayloadV1::build_single_node(
        row_count,
        encoding_for_physical(property.physical_kind),
        property.logical_type,
        property.physical_kind,
        (null_count != 0).then_some(null_bitmap),
        values,
    )
    .map_err(|err| err.to_string())
}

fn nested_shapes_for_model(
    file: &CovemapFile,
    materialized: &MaterializedModel,
) -> Result<NestedShapeByProperty, String> {
    let mut out = NestedShapeByProperty::new();
    let object_types_by_name = materialized
        .object_types
        .iter()
        .map(|object_type| (object_type.type_name.as_str(), object_type))
        .collect::<BTreeMap<_, _>>();
    for section in embedded_sections(file)? {
        let cove_core::profile::cove_map::EmbeddedMapSection::ProjectionCatalog(catalog) = section
        else {
            continue;
        };
        for projection in catalog.projections {
            let output_table = projection
                .output_table
                .as_deref()
                .unwrap_or(&projection.projection_id);
            let Some(object_type) = object_types_by_name.get(output_table) else {
                continue;
            };
            let properties_by_name = object_type
                .properties
                .iter()
                .map(|property| (property.property_name.as_str(), property))
                .collect::<BTreeMap<_, _>>();
            for column in projection.columns {
                let Some(shape) = column.nested_shape.as_deref() else {
                    continue;
                };
                let Some(property) = properties_by_name.get(column.name.as_str()) else {
                    continue;
                };
                let shape_value: Value = serde_json::from_str(shape).map_err(|err| {
                    format!(
                        "projection column '{}' has invalid nested_shape JSON: {err}",
                        column.name
                    )
                })?;
                let mut node =
                    project::nested_schema_node_from_shape(&column.name, &shape_value, true)?;
                node.name = column.name.clone();
                node.logical = property.logical_type;
                node.physical = physical_for_logical(property.logical_type);
                out.insert((object_type.object_type_id, property.property_id), node);
            }
        }
    }
    Ok(out)
}

fn file_dictionary_for_model(
    materialized: &MaterializedModel,
    nested_shapes: &NestedShapeByProperty,
) -> Result<Option<FileDictionaryEncoding>, String> {
    let mut keys = BTreeSet::<FileDictionaryKey>::new();
    let properties_by_type = materialized
        .object_types
        .iter()
        .flat_map(|object_type| {
            object_type
                .properties
                .iter()
                .map(move |property| ((object_type.object_type_id, property.property_id), property))
        })
        .collect::<BTreeMap<_, _>>();
    for row in &materialized.rows {
        for (property_id, property_value) in &row.properties {
            let Some(property) = properties_by_type.get(&(row.object_type_id, *property_id)) else {
                continue;
            };
            if property.physical_kind != CovePhysicalKind::FileCode
                || property_value.value.is_null()
            {
                continue;
            }
            keys.insert(file_dictionary_key_for_property(
                property.logical_type,
                &property_value.value,
                nested_shapes.get(&(row.object_type_id, *property_id)),
            )?);
        }
    }
    if keys.is_empty() {
        return Ok(None);
    }
    FileDictionaryEncoding::from_keys(keys)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn file_dictionary_index_bytes(dictionary: &FileDictionary) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        cove_core::dictionary::DICT_HEADER_SIZE
            + dictionary.entries.len() * cove_core::dictionary::DICT_INDEX_ENTRY_SIZE,
    );
    out.extend_from_slice(&dictionary.header.serialize());
    for entry in &dictionary.entries {
        out.extend_from_slice(&entry.serialize());
    }
    out
}

fn file_dictionary_key_for_property(
    logical: CoveLogicalType,
    value: &Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
) -> Result<FileDictionaryKey, String> {
    if logical == CoveLogicalType::Json {
        let text = serde_json::to_string(value).map_err(|err| err.to_string())?;
        let canonical = CanonicalValue::Json(&text);
        return Ok(FileDictionaryKey {
            value_tag: canonical.value_tag() as u16,
            canonical: canonical.encode().map_err(|err| err.to_string())?,
        });
    }
    let canonical = canonical_value_for_logical(logical, value, nested_shape)?;
    let value_tag = canonical.value_tag() as u16;
    let canonical = canonical.encode().map_err(|err| err.to_string())?;
    Ok(FileDictionaryKey {
        value_tag,
        canonical,
    })
}

fn canonical_value_for_logical<'a>(
    logical: CoveLogicalType,
    value: &'a Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
) -> Result<CanonicalValue<'a>, String> {
    if value.is_null() {
        return Ok(CanonicalValue::Null);
    }
    match logical {
        CoveLogicalType::Null => Ok(CanonicalValue::Null),
        CoveLogicalType::Bool => Ok(CanonicalValue::Bool(json_bool(value)?)),
        CoveLogicalType::Int8 => Ok(CanonicalValue::Int {
            width: 1,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int16 => Ok(CanonicalValue::Int {
            width: 2,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int32 => Ok(CanonicalValue::Int {
            width: 4,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::Int64 => Ok(CanonicalValue::Int {
            width: 8,
            value: i128::from(json_i64(value)?),
        }),
        CoveLogicalType::UInt8 => Ok(CanonicalValue::Uint {
            width: 1,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt16 => Ok(CanonicalValue::Uint {
            width: 2,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt32 => Ok(CanonicalValue::Uint {
            width: 4,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::UInt64 => Ok(CanonicalValue::Uint {
            width: 8,
            value: u128::from(json_u64(value)?),
        }),
        CoveLogicalType::Float32 => Ok(CanonicalValue::Float32(json_f64(value)? as f32)),
        CoveLogicalType::Float64 => Ok(CanonicalValue::Float64(json_f64(value)?)),
        CoveLogicalType::Decimal64 => Ok(CanonicalValue::Decimal64(json_i64(value)?)),
        CoveLogicalType::Decimal128 => Ok(CanonicalValue::Decimal128(json_i128(value)?)),
        CoveLogicalType::DateDays => Ok(CanonicalValue::DateDays(
            json_i64(value)?
                .try_into()
                .map_err(|_| "date_days out of i32 range".to_string())?,
        )),
        CoveLogicalType::TimestampMicros => Ok(CanonicalValue::TimestampMicros(json_i64(value)?)),
        CoveLogicalType::TimestampNanos => Ok(CanonicalValue::TimestampNanos(json_i64(value)?)),
        CoveLogicalType::Utf8 => Ok(CanonicalValue::Utf8(json_string(value)?)),
        CoveLogicalType::Binary => Ok(CanonicalValue::Bytes(json_string(value)?.as_bytes())),
        CoveLogicalType::Uuid => Ok(CanonicalValue::Uuid(json_uuid(value)?)),
        CoveLogicalType::Json => unreachable!("JSON is handled before borrowing conversion"),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            if let Some(shape) = nested_shape {
                canonical_value_for_nested_shape(shape, value)
            } else {
                match logical {
                    CoveLogicalType::List => canonical_list_value(value),
                    CoveLogicalType::Struct => canonical_struct_value(value),
                    CoveLogicalType::Map => canonical_map_value(value),
                    _ => unreachable!(),
                }
            }
        }
        _ => Err("unsupported future logical type for FileCode dictionary".into()),
    }
}

fn canonical_value_for_nested_shape<'a>(
    shape: &NestedSchemaNodeV1,
    value: &'a Value,
) -> Result<CanonicalValue<'a>, String> {
    if value.is_null() {
        return Ok(CanonicalValue::Null);
    }
    match shape.logical {
        CoveLogicalType::List => {
            let item_shape = shape
                .children
                .first()
                .ok_or_else(|| "list nested_shape requires one child".to_string())?;
            let items = value
                .as_array()
                .ok_or_else(|| "list property value must be an array".to_string())?
                .iter()
                .map(|item| canonical_value_for_nested_shape(item_shape, item))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CanonicalValue::List(items))
        }
        CoveLogicalType::Struct => {
            let object = value
                .as_object()
                .ok_or_else(|| "struct property value must be an object".to_string())?;
            let mut fields = Vec::with_capacity(shape.children.len());
            for (index, child) in shape.children.iter().enumerate() {
                let child_value = object.get(&child.name).unwrap_or(&Value::Null);
                fields.push(CanonicalField {
                    field_id: stable_u32(&child.name, index as u32 + 1) as u64,
                    value: canonical_value_for_nested_shape(child, child_value)?,
                });
            }
            Ok(CanonicalValue::Struct(fields))
        }
        CoveLogicalType::Map => {
            if shape.children.len() != 2 {
                return Err("map nested_shape requires key and value children".into());
            }
            let key_shape = &shape.children[0];
            let value_shape = &shape.children[1];
            let mut entries = Vec::new();
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        entries.push((
                            canonical_map_object_key_for_shape(key_shape, key)?,
                            canonical_value_for_nested_shape(value_shape, value)?,
                        ));
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        let pair = item.as_array().ok_or_else(|| {
                            "map array entries must be [key, value] pairs".to_string()
                        })?;
                        if pair.len() != 2 {
                            return Err("map array entries must be [key, value] pairs".into());
                        }
                        entries.push((
                            canonical_value_for_nested_shape(key_shape, &pair[0])?,
                            canonical_value_for_nested_shape(value_shape, &pair[1])?,
                        ));
                    }
                }
                _ => return Err("map property value must be an object or pair array".into()),
            }
            Ok(CanonicalValue::Map(entries))
        }
        _ => canonical_value_for_logical(shape.logical, value, None),
    }
}

fn canonical_map_object_key_for_shape<'a>(
    shape: &NestedSchemaNodeV1,
    key: &'a str,
) -> Result<CanonicalValue<'a>, String> {
    match shape.logical {
        CoveLogicalType::Bool => match key {
            "true" => Ok(CanonicalValue::Bool(true)),
            "false" => Ok(CanonicalValue::Bool(false)),
            _ => Err("map object key is not a boolean".into()),
        },
        CoveLogicalType::Int8 => Ok(CanonicalValue::Int {
            width: 1,
            value: key
                .parse::<i8>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int8".to_string())?,
        }),
        CoveLogicalType::Int16 => Ok(CanonicalValue::Int {
            width: 2,
            value: key
                .parse::<i16>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int16".to_string())?,
        }),
        CoveLogicalType::Int32 => Ok(CanonicalValue::Int {
            width: 4,
            value: key
                .parse::<i32>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int32".to_string())?,
        }),
        CoveLogicalType::Int64 => Ok(CanonicalValue::Int {
            width: 8,
            value: key
                .parse::<i64>()
                .map(i128::from)
                .map_err(|_| "map object key is not an int64".to_string())?,
        }),
        CoveLogicalType::UInt8 => Ok(CanonicalValue::Uint {
            width: 1,
            value: key
                .parse::<u8>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint8".to_string())?,
        }),
        CoveLogicalType::UInt16 => Ok(CanonicalValue::Uint {
            width: 2,
            value: key
                .parse::<u16>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint16".to_string())?,
        }),
        CoveLogicalType::UInt32 => Ok(CanonicalValue::Uint {
            width: 4,
            value: key
                .parse::<u32>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint32".to_string())?,
        }),
        CoveLogicalType::UInt64 => Ok(CanonicalValue::Uint {
            width: 8,
            value: key
                .parse::<u64>()
                .map(u128::from)
                .map_err(|_| "map object key is not a uint64".to_string())?,
        }),
        CoveLogicalType::Float32 => Ok(CanonicalValue::Float32(
            key.parse::<f32>()
                .map_err(|_| "map object key is not a float32".to_string())?,
        )),
        CoveLogicalType::Float64 => Ok(CanonicalValue::Float64(
            key.parse::<f64>()
                .map_err(|_| "map object key is not a float64".to_string())?,
        )),
        CoveLogicalType::Decimal64 => Ok(CanonicalValue::Decimal64(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a decimal64".to_string())?,
        )),
        CoveLogicalType::Decimal128 => Ok(CanonicalValue::Decimal128(
            key.parse::<i128>()
                .map_err(|_| "map object key is not a decimal128".to_string())?,
        )),
        CoveLogicalType::DateDays => Ok(CanonicalValue::DateDays(
            key.parse::<i32>()
                .map_err(|_| "map object key is not a date_days".to_string())?,
        )),
        CoveLogicalType::TimestampMicros => Ok(CanonicalValue::TimestampMicros(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a timestamp_micros".to_string())?,
        )),
        CoveLogicalType::TimestampNanos => Ok(CanonicalValue::TimestampNanos(
            key.parse::<i64>()
                .map_err(|_| "map object key is not a timestamp_nanos".to_string())?,
        )),
        CoveLogicalType::Utf8 => Ok(CanonicalValue::Utf8(key)),
        CoveLogicalType::Binary => Ok(CanonicalValue::Bytes(key.as_bytes())),
        CoveLogicalType::Json => Ok(CanonicalValue::Json(key)),
        CoveLogicalType::Uuid => Ok(CanonicalValue::Uuid(hex_decode_16(key)?)),
        CoveLogicalType::List | CoveLogicalType::Struct | CoveLogicalType::Map => {
            Err("map object keys cannot use nested logical types".into())
        }
        _ => Err("unsupported future map key logical type".into()),
    }
}

fn canonical_value_from_json<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    match value {
        Value::Null => Ok(CanonicalValue::Null),
        Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(CanonicalValue::Int {
                    width: 8,
                    value: i128::from(value),
                })
            } else if let Some(value) = number.as_u64() {
                Ok(CanonicalValue::Uint {
                    width: 8,
                    value: u128::from(value),
                })
            } else {
                Ok(CanonicalValue::Float64(
                    number
                        .as_f64()
                        .ok_or_else(|| "non-finite JSON number".to_string())?,
                ))
            }
        }
        Value::String(value) => Ok(CanonicalValue::Utf8(value)),
        Value::Array(_) => canonical_list_value(value),
        Value::Object(_) => canonical_struct_value(value),
    }
}

fn canonical_list_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "list property value must be an array".to_string())?
        .iter()
        .map(canonical_value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CanonicalValue::List(items))
}

fn canonical_struct_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "struct property value must be an object".to_string())?;
    let mut fields = Vec::with_capacity(object.len());
    for (index, (name, value)) in object.iter().enumerate() {
        fields.push(CanonicalField {
            field_id: stable_u32(name, index as u32 + 1) as u64,
            value: canonical_value_from_json(value)?,
        });
    }
    fields.sort_by_key(|field| field.field_id);
    Ok(CanonicalValue::Struct(fields))
}

fn canonical_map_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let mut entries = Vec::new();
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                entries.push((CanonicalValue::Utf8(key), canonical_value_from_json(value)?));
            }
        }
        Value::Array(items) => {
            for item in items {
                let pair = item
                    .as_array()
                    .ok_or_else(|| "map array entries must be [key, value] pairs".to_string())?;
                if pair.len() != 2 {
                    return Err("map array entries must be [key, value] pairs".into());
                }
                entries.push((
                    canonical_value_from_json(&pair[0])?,
                    canonical_value_from_json(&pair[1])?,
                ));
            }
        }
        _ => return Err("map property value must be an object or pair array".into()),
    }
    Ok(CanonicalValue::Map(entries))
}

fn append_property_value_bytes(
    property: &PropertyEntryV1,
    value: &Value,
    nested_shape: Option<&NestedSchemaNodeV1>,
    dictionary: Option<&FileDictionaryEncoding>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    if value.is_null() {
        append_null_placeholder(property, out)?;
        return Ok(());
    }
    match property.physical_kind {
        CovePhysicalKind::Boolean => out.push(if json_bool(value)? { 1 } else { 0 }),
        CovePhysicalKind::NumCode => out.extend_from_slice(&json_numcode(value)?.to_le_bytes()),
        CovePhysicalKind::FixedBytes => {
            let bytes = fixed_bytes_for_property(property, value)?;
            out.extend_from_slice(&bytes);
        }
        CovePhysicalKind::VarBytes => {
            let bytes = var_bytes_for_property(property, value)?;
            let len = u32::try_from(bytes.len())
                .map_err(|_| "property value is too large".to_string())?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        CovePhysicalKind::FileCode => {
            let dictionary = dictionary.ok_or_else(|| {
                "COVE-MAP writer needs a file dictionary for FileCode properties".to_string()
            })?;
            let key = file_dictionary_key_for_property(property.logical_type, value, nested_shape)?;
            let code = dictionary
                .file_code_for_key(&key)
                .map_err(|err| err.to_string())?;
            out.extend_from_slice(&code.to_le_bytes());
        }
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            return Err("COVE-MAP writer does not materialize nested properties yet".into())
        }
        _ => return Err("unsupported future COVE physical kind".into()),
    }
    Ok(())
}

fn append_null_placeholder(property: &PropertyEntryV1, out: &mut Vec<u8>) -> Result<(), String> {
    match property.physical_kind {
        CovePhysicalKind::Boolean => out.push(0),
        CovePhysicalKind::NumCode => out.extend_from_slice(&0u64.to_le_bytes()),
        CovePhysicalKind::FixedBytes => {
            let width = match property.logical_type {
                CoveLogicalType::Uuid | CoveLogicalType::Decimal128 => 16,
                CoveLogicalType::Decimal64 => 8,
                _ => return Err("unsupported fixed-width null placeholder".into()),
            };
            out.resize(out.len() + width, 0);
        }
        CovePhysicalKind::VarBytes => out.extend_from_slice(&0u32.to_le_bytes()),
        CovePhysicalKind::FileCode => out.extend_from_slice(&0u32.to_le_bytes()),
        CovePhysicalKind::List | CovePhysicalKind::Struct | CovePhysicalKind::Map => {
            return Err("nested null placeholders are not supported".into())
        }
        _ => return Err("unsupported future COVE physical kind".into()),
    }
    Ok(())
}

fn temporal_segment_index(
    segments: &[TemporalSegmentBuild],
) -> Result<TemporalSegmentIndex, String> {
    let mut entries = Vec::with_capacity(segments.len());
    for segment in segments {
        let min_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .min()
            .unwrap_or([0; 16]);
        let max_goid = segment
            .rows
            .iter()
            .map(|row| row.goid)
            .max()
            .unwrap_or([0; 16]);
        let (delta_count, snapshot_count, baseline_count, tombstone_count) =
            row_kind_counts(&segment.rows);
        entries.push(TemporalSegmentIndexEntryV1 {
            segment_id: segment.segment_id,
            object_type_id: segment.object_type_id,
            time_range_start_us: 0,
            time_range_end_us: 0,
            csn_min: 0,
            csn_max: segment.rows.len().saturating_sub(1) as u64,
            row_count: u32::try_from(segment.rows.len())
                .map_err(|_| "too many COVE-O rows".to_string())?,
            delta_count,
            snapshot_count,
            baseline_count,
            tombstone_count,
            min_goid,
            max_goid,
            offset: 0,
            length: segment.payload.len() as u64,
            checksum: 0,
        });
    }
    Ok(TemporalSegmentIndex { flags: 0, entries })
}

fn row_kind_counts(rows: &[ObjectRow]) -> (u32, u32, u32, u32) {
    let mut delta = 0;
    let mut snapshot = 0;
    let mut baseline = 0;
    let mut tombstone = 0;
    for row in rows {
        match row.record_kind {
            RecordKind::Delta => delta += 1,
            RecordKind::Snapshot => snapshot += 1,
            RecordKind::Baseline => baseline += 1,
            RecordKind::Tombstone => tombstone += 1,
            RecordKind::ReservedLegacyMaterializedDelta => {}
            _ => {}
        }
    }
    (delta, snapshot, baseline, tombstone)
}

fn temporal_prev_refs(segment_id: u32, rows: &[ObjectRow]) -> Vec<Option<CoveRecordRefV1>> {
    let mut latest_by_goid = BTreeMap::<[u8; 16], u32>::new();
    let mut refs = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let prev_ref = if matches!(
            row.record_kind,
            RecordKind::Delta | RecordKind::Snapshot | RecordKind::Tombstone
        ) {
            latest_by_goid
                .get(&row.goid)
                .copied()
                .map(|row_index| CoveRecordRefV1 {
                    segment_id,
                    row_index,
                    target_kind: 0,
                })
        } else {
            None
        };
        refs.push(prev_ref);
        latest_by_goid.insert(row.goid, index as u32);
    }
    refs
}

fn trust_manifest(
    segments: &[TemporalSegmentBuild],
    dictionary: Option<&FileDictionaryEncoding>,
) -> Result<TrustManifest, String> {
    let mut previous = [0u8; 32];
    let mut entries = Vec::new();
    for segment in segments {
        let parsed_segment =
            TemporalSegmentData::parse(&segment.payload).map_err(|err| err.to_string())?;
        let dictionary = dictionary.map(|encoding| &encoding.dictionary);
        for index in 0..parsed_segment.rows.len() {
            let payload =
                temporal_row_trust_payload(&parsed_segment, index as u32, dictionary, &[])
                    .map_err(|err| err.to_string())?;
            let expected_hash =
                trust_chain::chain(&previous, &payload).map_err(|err| err.to_string())?;
            entries.push(TrustManifestEntryV1 {
                segment_id: segment.segment_id,
                row_index: index as u32,
                expected_hash,
            });
            previous = expected_hash;
        }
    }
    Ok(TrustManifest { entries })
}

fn object_section(
    kind: SectionKind,
    item_count: u64,
    row_count: u64,
    data: Vec<u8>,
) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count,
        row_count,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data,
    }
}

fn dictionary_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_FILE_DICTIONARY,
        optional_features: 0,
        data,
    }
}

fn map_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_SEMANTIC_MAP,
        data: ensure_covemap_payload_envelope(kind, data),
    }
}

fn ensure_covemap_payload_envelope(kind: SectionKind, data: Vec<u8>) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(&data) else {
        return data;
    };
    let Value::Object(object) = &mut value else {
        return data;
    };
    object.insert(
        "schema_id".to_string(),
        Value::String("org.coveformat.covemap.v2".to_string()),
    );
    object.insert(
        "section_id".to_string(),
        Value::Number(serde_json::Number::from(kind as u16)),
    );
    serde_json::to_vec_pretty(&value).unwrap_or(data)
}

fn map_passthrough_sections(
    file: &CovemapFile,
    materialized: &MaterializedModel,
) -> Result<Vec<SectionPayload>, String> {
    file.sections
        .iter()
        .filter_map(|section| {
            let kind = u16::try_from(section.entry.section_id)
                .ok()
                .and_then(SectionKind::from_u16)?;
            matches!(
                kind,
                SectionKind::MapSourceCatalog
                    | SectionKind::MapFunctionRegistry
                    | SectionKind::MapIdentityRuleCatalog
                    | SectionKind::MapRowSemanticsCatalog
                    | SectionKind::MapProjectionCatalog
                    | SectionKind::MapResolutionCatalog
            )
            .then(|| {
                let data = if kind == SectionKind::MapProjectionCatalog {
                    enriched_projection_catalog_payload(section.payload.as_slice(), materialized)
                } else {
                    Ok(section.payload.clone())
                };
                data.map(|data| map_section(kind, 1, data))
            })
        })
        .collect()
}

fn enriched_projection_catalog_payload(
    payload: &[u8],
    materialized: &MaterializedModel,
) -> Result<Vec<u8>, String> {
    let section = cove_core::profile::cove_map::parse_embedded_section(
        SectionKind::MapProjectionCatalog,
        payload,
    )
    .map_err(|err| format!("cannot parse MAP_PROJECTION_CATALOG for lineage enrichment: {err}"))?;
    let cove_core::profile::cove_map::EmbeddedMapSection::ProjectionCatalog(catalog) = section
    else {
        return Err("MAP_PROJECTION_CATALOG parser returned a non-projection section".into());
    };
    let catalog = project::enrich_projection_catalog_lineage(catalog, &materialized.object_types);
    serde_json::to_vec_pretty(&project::projection_catalog_json_value(&catalog))
        .map_err(|err| format!("cannot encode enriched MAP_PROJECTION_CATALOG: {err}"))
}

#[derive(Debug, Clone, Copy)]
struct JoinKeyComponent<'a> {
    role_id: &'a str,
    logical_type_id: &'a str,
    value: Option<&'a [u8]>,
}

fn join_key_tuple(
    object_type_id: u32,
    identity_rule_id: &str,
    components: &[JoinKeyComponent<'_>],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"COVE-MAP-JOIN-KEY-V1");
    out.extend_from_slice(&object_type_id.to_le_bytes());
    append_len_bytes(&mut out, identity_rule_id.as_bytes());
    out.extend_from_slice(&(components.len() as u32).to_le_bytes());
    for component in components {
        append_len_bytes(&mut out, component.role_id.as_bytes());
        append_len_bytes(&mut out, component.logical_type_id.as_bytes());
        match component.value {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                append_len_bytes(&mut out, value);
            }
        }
    }
    out
}

pub(crate) fn join_key_tuple_from_rule_with_context(
    rule: &MapIdentityRule,
    row: &SourceRow,
    object_type_id: u32,
    context: Option<&MappingContext>,
) -> Result<JoinKeyEvaluation, String> {
    let mut encoded_values = Vec::<Option<Vec<u8>>>::with_capacity(rule.join_keys.len());
    let mut resolution_metadata = Vec::new();
    let mut materializes_identity = true;
    let mut effective_confidence_class = None::<String>;
    for component in &rule.join_keys {
        let raw_value = row.values.get(&component.source_column);
        if raw_value.is_none() || matches!(raw_value, Some(Value::Null)) {
            if matches!(
                component.null_policy.as_str(),
                "reject" | "reject-null" | "all_components_required"
            ) {
                return Err(format!(
                    "identity rule '{}' rejected null/missing source column '{}'",
                    rule.rule_id, component.source_column
                ));
            }
            encoded_values.push(None);
            continue;
        }
        let value = if component.resolution.is_some() {
            let Some(context) = context else {
                return Err(format!(
                    "identity rule '{}' uses resolver-backed join key '{}' without resolution context",
                    rule.rule_id, component.role_id
                ));
            };
            let resolved = resolve_join_key_component(
                component,
                raw_value.unwrap(),
                &rule.object_type,
                context,
            )?;
            materializes_identity &= resolved.materializes_identity;
            if let Some(class) = &resolved.effective_confidence_class {
                effective_confidence_class = Some(match effective_confidence_class.take() {
                    Some(existing) => most_restrictive_confidence(&existing, class).to_string(),
                    None => class.clone(),
                });
            }
            resolution_metadata.push(resolved.metadata);
            Value::String(resolved.identity_value)
        } else {
            apply_canonicalization(
                raw_value.unwrap(),
                &component.canonicalization,
                &rule.function_ids,
            )?
        };
        encoded_values.push(Some(canonical_component_bytes(
            &component.logical_type,
            &value,
        )?));
    }
    let components = rule
        .join_keys
        .iter()
        .zip(encoded_values.iter())
        .map(|(component, bytes)| JoinKeyComponent {
            role_id: component.role_id.as_str(),
            logical_type_id: component.logical_type.as_str(),
            value: bytes.as_deref(),
        })
        .collect::<Vec<_>>();
    Ok(JoinKeyEvaluation {
        tuple: join_key_tuple(object_type_id, &rule.rule_id, &components),
        materializes_identity,
        effective_confidence_class,
        resolution_metadata,
    })
}

struct ResolvedJoinKeyComponent {
    identity_value: String,
    materializes_identity: bool,
    effective_confidence_class: Option<String>,
    metadata: ResolutionMetadata,
}

fn resolve_join_key_component(
    component: &MapJoinKeyComponent,
    raw_value: &Value,
    object_type: &str,
    context: &MappingContext,
) -> Result<ResolvedJoinKeyComponent, String> {
    let binding = component
        .resolution
        .as_ref()
        .ok_or_else(|| "missing resolution binding".to_string())?;
    let catalog = context
        .resolution_catalog
        .as_ref()
        .ok_or_else(|| "MAP_RESOLUTION_CATALOG_MISSING: resolver-backed identity rule has no resolution catalog".to_string())?;
    let resolver = catalog
        .resolvers
        .iter()
        .find(|resolver| resolver.resolver_id == binding.resolver_id)
        .ok_or_else(|| {
            format!(
                "MAP_RESOLUTION_CATALOG_MISSING: identity rule references missing resolver '{}'",
                binding.resolver_id
            )
        })?;
    if resolver.kind != "alias_catalog" {
        return Err(format!(
            "MAP_RESOLVER_UNSUPPORTED: unsupported resolver kind '{}'",
            resolver.kind
        ));
    }
    if resolver.object_type != object_type {
        return Err(format!(
            "MAP_RESOLUTION_CATALOG_MISMATCH: resolver '{}' targets object type '{}' but identity rule targets '{}'",
            resolver.resolver_id, resolver.object_type, object_type
        ));
    }
    let pipeline = catalog
        .normalization_pipelines
        .iter()
        .find(|pipeline| pipeline.pipeline_id == resolver.normalization_pipeline_id)
        .ok_or_else(|| {
            format!(
                "MAP_PIPELINE_DIGEST_MISMATCH: resolver '{}' references missing pipeline '{}'",
                resolver.resolver_id, resolver.normalization_pipeline_id
            )
        })?;
    let raw_observed_value = string_arg(raw_value, "resolver alias lookup")?.to_string();
    let normalized_value = apply_resolution_pipeline(&raw_observed_value, pipeline)?;
    let alias_catalog = resolver.alias_catalog.as_ref().ok_or_else(|| {
        format!(
            "MAP_RESOLVER_UNSUPPORTED: resolver '{}' has no alias catalog",
            resolver.resolver_id
        )
    })?;
    let mut hits = Vec::<&MapAliasEntry>::new();
    for entry in &alias_catalog.entries {
        for alias in &entry.aliases {
            if apply_resolution_pipeline(alias, pipeline)? == normalized_value {
                hits.push(entry);
                break;
            }
        }
    }
    hits.sort_by_key(|entry| entry.alias_entry_id.clone());
    hits.dedup_by_key(|entry| entry.alias_entry_id.clone());

    if hits.is_empty() {
        return resolve_alias_miss(component, resolver, &raw_observed_value, &normalized_value);
    }

    let canonical_keys = hits
        .iter()
        .map(|entry| entry.canonical_key.as_str())
        .collect::<BTreeSet<_>>();
    let ambiguous = canonical_keys.len() > 1 || hits.iter().any(|entry| entry.ambiguous);
    if ambiguous {
        if resolver.ambiguous_policy == "candidate_only" {
            return Ok(ResolvedJoinKeyComponent {
                identity_value: normalized_value.clone(),
                materializes_identity: false,
                effective_confidence_class: Some("candidate_only".to_string()),
                metadata: resolution_metadata_base(
                    component,
                    resolver,
                    &raw_observed_value,
                    &normalized_value,
                    Some(normalized_value.clone()),
                )
                .with_alias_ambiguous(alias_catalog.alias_catalog_id.clone()),
            });
        }
        let display_value = resolver_error_observed_value(resolver, &normalized_value);
        return Err(format!(
            "MAP_ALIAS_AMBIGUOUS: normalized alias '{}' matched multiple canonical keys for resolver '{}'",
            display_value, resolver.resolver_id
        ));
    }

    let entry = hits[0];
    Ok(ResolvedJoinKeyComponent {
        identity_value: entry.canonical_key.clone(),
        materializes_identity: true,
        effective_confidence_class: Some(resolver.confidence_class.clone()),
        metadata: resolution_metadata_base(
            component,
            resolver,
            &raw_observed_value,
            &normalized_value,
            Some(entry.canonical_key.clone()),
        )
        .with_alias_hit(
            entry.canonical_key.clone(),
            entry.canonical_label.clone(),
            alias_catalog.alias_catalog_id.clone(),
            entry.alias_entry_id.clone(),
        ),
    })
}

fn resolve_alias_miss(
    component: &MapJoinKeyComponent,
    resolver: &MapResolver,
    raw_observed_value: &str,
    normalized_value: &str,
) -> Result<ResolvedJoinKeyComponent, String> {
    match resolver.on_miss.as_str() {
        "reject" => {
            let display_value = resolver_error_observed_value(resolver, raw_observed_value);
            Err(format!(
                "MAP_ALIAS_MISS: resolver '{}' did not match '{}'",
                resolver.resolver_id, display_value
            ))
        }
        "candidate_only" => Ok(ResolvedJoinKeyComponent {
            identity_value: normalized_value.to_string(),
            materializes_identity: false,
            effective_confidence_class: Some("candidate_only".to_string()),
            metadata: resolution_metadata_base(
                component,
                resolver,
                raw_observed_value,
                normalized_value,
                Some(normalized_value.to_string()),
            )
            .with_alias_miss(),
        }),
        "source_scoped" => Ok(ResolvedJoinKeyComponent {
            identity_value: normalized_value.to_string(),
            materializes_identity: true,
            effective_confidence_class: Some("source_scoped".to_string()),
            metadata: resolution_metadata_base(
                component,
                resolver,
                raw_observed_value,
                normalized_value,
                Some(normalized_value.to_string()),
            )
            .with_alias_miss(),
        }),
        "normalized_value" => {
            let class = resolver.miss_confidence_class.clone().ok_or_else(|| {
                format!(
                    "MAP_RESOLUTION_NOT_REPLAYABLE: resolver '{}' missing miss_confidence_class",
                    resolver.resolver_id
                )
            })?;
            Ok(ResolvedJoinKeyComponent {
                identity_value: normalized_value.to_string(),
                materializes_identity: true,
                effective_confidence_class: Some(class),
                metadata: resolution_metadata_base(
                    component,
                    resolver,
                    raw_observed_value,
                    normalized_value,
                    Some(normalized_value.to_string()),
                )
                .with_alias_miss(),
            })
        }
        other => Err(format!(
            "MAP_RESOLVER_UNSUPPORTED: unsupported resolver miss policy '{other}'"
        )),
    }
}

fn resolver_error_observed_value(resolver: &MapResolver, observed_value: &str) -> String {
    if resolver.evidence_policy == "redact_raw" {
        "<redacted>".to_string()
    } else {
        observed_value.to_string()
    }
}

fn resolution_metadata_base(
    component: &MapJoinKeyComponent,
    resolver: &MapResolver,
    raw_observed_value: &str,
    normalized_value: &str,
    resolved_identity_value: Option<String>,
) -> ResolutionMetadata {
    ResolutionMetadata {
        role_id: component.role_id.clone(),
        resolution_kind: resolver.kind.clone(),
        resolver_id: resolver.resolver_id.clone(),
        resolver_digest: resolver.resolver_digest.clone(),
        catalog_digest: resolver.catalog_digest.clone(),
        pipeline_digest: resolver.pipeline_digest.clone(),
        normalization_pipeline_id: resolver.normalization_pipeline_id.clone(),
        evidence_policy: resolver.evidence_policy.clone(),
        redacted_resolution_evidence: resolver.evidence_policy == "redact_raw",
        raw_observed_value: raw_observed_value.to_string(),
        normalized_value: normalized_value.to_string(),
        resolved_identity_value,
        canonical_key: None,
        canonical_label: None,
        alias_catalog_id: None,
        alias_entry_id: None,
        alias_hit: false,
        alias_miss: false,
        alias_ambiguous: false,
        miss_policy: Some(resolver.on_miss.clone()),
    }
}

impl ResolutionMetadata {
    fn with_alias_hit(
        mut self,
        canonical_key: String,
        canonical_label: String,
        alias_catalog_id: String,
        alias_entry_id: String,
    ) -> Self {
        self.canonical_key = Some(canonical_key);
        self.canonical_label = Some(canonical_label);
        self.alias_catalog_id = Some(alias_catalog_id);
        self.alias_entry_id = Some(alias_entry_id);
        self.alias_hit = true;
        self.miss_policy = None;
        self
    }

    fn with_alias_miss(mut self) -> Self {
        self.alias_miss = true;
        self
    }

    fn with_alias_ambiguous(mut self, alias_catalog_id: String) -> Self {
        self.alias_catalog_id = Some(alias_catalog_id);
        self.alias_ambiguous = true;
        self
    }
}

fn apply_resolution_pipeline(
    raw: &str,
    pipeline: &MapNormalizationPipeline,
) -> Result<String, String> {
    let mut value = raw.to_string();
    for function in &pipeline.functions {
        value = match function.function_id.as_str() {
            "identity" => value,
            "trim" => value.trim().to_string(),
            "unicode_nfkc" => {
                let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
                normalizer.normalize(&value).into_owned()
            }
            "unicode_casefold" => {
                let case_mapper = icu_casemap::CaseMapper::new();
                case_mapper.fold_string(&value).into_owned()
            }
            "strip_punctuation" => value
                .chars()
                .filter(|ch| !ch.is_ascii_punctuation())
                .collect::<String>(),
            "collapse_whitespace" => value.split_whitespace().collect::<Vec<_>>().join(" "),
            "strip_legal_suffix" => strip_legal_suffix(&value, pipeline, function.table_id.as_deref())?,
            "sort_tokens" => {
                let mut tokens = value.split_whitespace().collect::<Vec<_>>();
                tokens.sort_unstable();
                tokens.join(" ")
            }
            other => {
                return Err(format!(
                    "MAP_RESOLVER_UNSUPPORTED: normalization function '{other}' is not implemented by resolver execution"
                ))
            }
        };
    }
    Ok(value)
}

fn strip_legal_suffix(
    value: &str,
    pipeline: &MapNormalizationPipeline,
    table_id: Option<&str>,
) -> Result<String, String> {
    let table_id = table_id.ok_or_else(|| {
        format!(
            "MAP_RESOLUTION_NOT_REPLAYABLE: strip_legal_suffix in pipeline '{}' has no table_id",
            pipeline.pipeline_id
        )
    })?;
    let table = pipeline
        .tables
        .iter()
        .find(|table| table.table_id == table_id)
        .ok_or_else(|| {
            format!(
                "MAP_RESOLUTION_NOT_REPLAYABLE: strip_legal_suffix references missing table '{table_id}'"
            )
        })?;
    let mut text = value.trim().to_string();
    loop {
        let mut changed = false;
        for suffix in &table.values {
            let suffix = suffix.trim();
            if suffix.is_empty() {
                continue;
            }
            if text == suffix {
                return Ok(text);
            }
            let Some(prefix) = text.strip_suffix(suffix) else {
                continue;
            };
            if prefix.is_empty() || !prefix.chars().next_back().is_some_and(char::is_whitespace) {
                continue;
            }
            text = prefix.trim_end().to_string();
            changed = true;
            break;
        }
        if !changed {
            return Ok(text);
        }
    }
}

fn most_restrictive_confidence(left: &str, right: &str) -> String {
    if identity_confidence_rank(left) >= identity_confidence_rank(right) {
        left.to_string()
    } else {
        right.to_string()
    }
}

fn identity_confidence_rank(class: &str) -> u8 {
    match class {
        "authoritative" | "reviewed_authoritative" => 0,
        "strong_deterministic" => 1,
        "source_scoped" => 2,
        "weak_deterministic" => 3,
        "candidate_only" | "candidate" => 4,
        _ => 5,
    }
}

fn apply_canonicalization(
    value: &Value,
    canonicalization: &str,
    declared_functions: &[String],
) -> Result<Value, String> {
    let function_id = if canonicalization == "none" {
        "identity"
    } else {
        canonicalization
    };
    if !declared_functions
        .iter()
        .any(|function| function == function_id || function == canonicalization)
    {
        return Err(format!(
            "canonicalization function '{canonicalization}' was not declared on the identity rule"
        ));
    }
    if !deterministic_builtin_function_ids().contains(&function_id) {
        return Err(format!(
            "canonicalization function '{canonicalization}' is not implemented by the deterministic reference runner"
        ));
    }
    match function_id {
        "identity" => Ok(value.clone()),
        "trim" => Ok(Value::String(string_arg(value, "trim")?.trim().to_string())),
        "ascii_lower" => Ok(Value::String(
            string_arg(value, "ascii_lower")?.to_ascii_lowercase(),
        )),
        "unicode_nfc" => {
            let text = string_arg(value, function_id)?;
            let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfc();
            Ok(Value::String(normalizer.normalize(text).into_owned()))
        }
        "unicode_nfkc" => {
            let text = string_arg(value, function_id)?;
            let normalizer = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc();
            Ok(Value::String(normalizer.normalize(text).into_owned()))
        }
        "unicode_casefold" => {
            let case_mapper = icu_casemap::CaseMapper::new();
            Ok(Value::String(
                case_mapper
                    .fold_string(string_arg(value, "unicode_casefold")?)
                    .into_owned(),
            ))
        }
        "trim_lower" => Ok(Value::String(
            string_arg(value, "trim_lower")?.trim().to_ascii_lowercase(),
        )),
        "concat_delimited" => {
            let items = value
                .as_array()
                .ok_or_else(|| "concat_delimited requires a JSON array".to_string())?;
            let mut out = Vec::new();
            for item in items {
                out.push(string_arg(item, "concat_delimited")?);
            }
            Ok(Value::String(out.join("|")))
        }
        "parse_int64" => {
            let text = string_arg(value, "parse_int64")?.trim();
            let parsed = text
                .parse::<i64>()
                .map_err(|_| "parse_int64 requires a base-10 int64 string".to_string())?;
            Ok(Value::Number(parsed.into()))
        }
        "parse_decimal" => {
            let text = string_arg(value, "parse_decimal")?.trim();
            validate_decimal_text(text)?;
            Ok(Value::String(text.to_string()))
        }
        "parse_timestamp_utc" => {
            let text = string_arg(value, "parse_timestamp_utc")?.trim();
            validate_utc_timestamp_text(text)?;
            Ok(Value::String(text.to_string()))
        }
        "sha256_hex" => Ok(Value::String(sha256_hex(
            string_arg(value, "sha256_hex")?.as_bytes(),
        ))),
        _ => unreachable!("registry membership checked above"),
    }
}

fn deterministic_builtin_function_ids() -> &'static [&'static str] {
    &[
        "identity",
        "trim",
        "ascii_lower",
        "unicode_nfc",
        "unicode_nfkc",
        "unicode_casefold",
        "trim_lower",
        "concat_delimited",
        "parse_int64",
        "parse_decimal",
        "parse_timestamp_utc",
        "sha256_hex",
    ]
}

fn string_arg<'a>(value: &'a Value, function_id: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{function_id} requires a string value"))
}

fn validate_decimal_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("parse_decimal requires a non-empty decimal string".into());
    }
    let mut chars = text.chars();
    if matches!(chars.clone().next(), Some('+') | Some('-')) {
        chars.next();
    }
    let mut digits = 0usize;
    let mut dots = 0usize;
    for ch in chars {
        match ch {
            '0'..='9' => digits += 1,
            '.' => dots += 1,
            _ => return Err("parse_decimal only accepts base-10 decimal text".into()),
        }
    }
    if digits == 0 || dots > 1 {
        return Err("parse_decimal only accepts base-10 decimal text".into());
    }
    Ok(())
}

fn validate_utc_timestamp_text(text: &str) -> Result<(), String> {
    let has_utc_suffix = text.ends_with('Z') || text.ends_with("+00:00");
    if has_utc_suffix && text.contains('T') {
        Ok(())
    } else {
        Err("parse_timestamp_utc requires an ISO-8601 UTC timestamp".into())
    }
}

fn canonical_component_bytes(logical_type: &str, value: &Value) -> Result<Vec<u8>, String> {
    let canonical = match logical_type {
        "bool" | "boolean" => CanonicalValue::Bool(
            value
                .as_bool()
                .ok_or_else(|| "bool join key value must be JSON bool".to_string())?,
        ),
        "int64" | "int" => CanonicalValue::Int {
            width: 8,
            value: json_i64(value)? as i128,
        },
        "uint64" | "uint" => CanonicalValue::Uint {
            width: 8,
            value: json_u64(value)? as u128,
        },
        "float64" => CanonicalValue::Float64(json_f64(value)?),
        "utf8" | "string" => CanonicalValue::Utf8(
            value
                .as_str()
                .ok_or_else(|| "utf8 join key value must be JSON string".to_string())?,
        ),
        "binary" => CanonicalValue::Bytes(
            value
                .as_str()
                .ok_or_else(|| "binary join key value must be encoded as a string".to_string())?
                .as_bytes(),
        ),
        other => {
            return Err(format!(
                "logical type '{other}' is not supported in COVE-MAP join keys"
            ))
        }
    };
    canonical.encode().map_err(|err| err.to_string())
}

fn mapped_goid(
    mapping_id: &[u8],
    mapping_version: &[u8],
    object_type_id: u32,
    anchor_kind: &[u8],
    anchor_bytes: &[u8],
    source_scope: Option<&str>,
) -> [u8; 16] {
    let object_type_id = object_type_id.to_le_bytes();
    let source_scope = source_scope.unwrap_or("").as_bytes();
    goid16_parts(&[
        mapping_id,
        mapping_version,
        &object_type_id,
        anchor_kind,
        anchor_bytes,
        source_scope,
    ])
}

fn goid16_parts(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, sync::Arc};

    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use arrow_ipc::writer::FileWriter as IpcFileWriter;
    use cove_core::{
        artifact::covedelta::{
            CoveDeltaFile, DELTA_FEATURE_INLINE_DICTIONARY, DELTA_FEATURE_SPARSE_PATCH_ROWS,
            DELTA_PROPERTY_OP_SET_VALUE,
        },
        artifact::covemap::{
            CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1, CovemapSection,
            CovemapSectionEntryV1,
        },
        artifact::covm::CovmDeltaArtifactRefV1,
        compression,
        constants::{DigestAlgorithm, FEATURE_CODEC_ZSTD, FEATURE_SEMANTIC_MAP},
        digest::compute_digest,
        profile::cove_map::{is_compact_evidence_index_bytes, MapEvidenceIndex},
        profile::cove_o::{
            read_object_surface_from_base_and_delta_files, read_object_surface_from_bytes,
            reconstruct_object_states, reconstruct_object_states_from_base_and_delta_files,
            CoveObjectState, TemporalSegmentData, PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
            PROPERTY_FLAG_ASSOCIATION_TO_GOID, PROPERTY_FLAG_ASSOCIATION_TYPE,
            PROPERTY_FLAG_EVIDENCE_REF,
        },
        reader::validate_bytes,
    };
    use orc_rust::ArrowWriterBuilder as OrcWriterBuilder;
    use parquet::arrow::ArrowWriter;

    fn test_section(kind: SectionKind, value: Value) -> CovemapSection {
        let payload = serde_json::to_vec_pretty(&covemap_payload_value(kind, value))
            .expect("serializing serde_json::Value cannot fail");
        CovemapSection {
            entry: CovemapSectionEntryV1 {
                section_id: kind as u32,
                offset: 0,
                length: payload.len() as u64,
                uncompressed_length: payload.len() as u64,
                compression: 0,
                payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
                required: true,
                reserved: 0,
                checksum: 0,
            },
            payload,
        }
    }

    fn mutate_section_payload(file: &mut CovemapFile, index: usize, edit: impl FnOnce(&mut Value)) {
        let mut payload: Value = serde_json::from_slice(&file.sections[index].payload).unwrap();
        edit(&mut payload);
        let bytes =
            serde_json::to_vec_pretty(&payload).expect("serializing serde_json::Value cannot fail");
        file.sections[index].entry.length = bytes.len() as u64;
        file.sections[index].entry.uncompressed_length = bytes.len() as u64;
        file.sections[index].payload = bytes;
    }

    fn covemap_payload_value(kind: SectionKind, mut value: Value) -> Value {
        if let Value::Object(object) = &mut value {
            object.insert(
                "schema_id".to_string(),
                Value::String("org.coveformat.covemap.v2".to_string()),
            );
            object.insert(
                "section_id".to_string(),
                Value::Number((kind as u16).into()),
            );
        }
        value
    }

    fn test_covemap(sections: Vec<CovemapSection>) -> CovemapFile {
        CovemapFile {
            header: CovemapHeaderV1::new([0x42; 16], 0),
            mapping_version: "test/v1".into(),
            sections,
            postscript: CovemapPostscriptV1 {
                required_features: FEATURE_SEMANTIC_MAP,
                optional_features: 0,
                file_len: 0,
                header_offset: 0,
                header_length: 0,
                checksum: 0,
            },
        }
    }

    fn resolution_catalog_section(mapping_id: &str, mapping_version: &str) -> CovemapSection {
        company_resolution_catalog_section_with_policy(
            mapping_id,
            mapping_version,
            "candidate_only",
            None,
            "retain_raw",
        )
    }

    fn redacted_resolution_catalog_section(
        mapping_id: &str,
        mapping_version: &str,
    ) -> CovemapSection {
        company_resolution_catalog_section_with_policy(
            mapping_id,
            mapping_version,
            "candidate_only",
            None,
            "redact_raw",
        )
    }

    fn company_resolution_catalog_section_with_miss_policy(
        mapping_id: &str,
        mapping_version: &str,
        on_miss: &str,
        miss_confidence_class: Option<&str>,
    ) -> CovemapSection {
        company_resolution_catalog_section_with_policy(
            mapping_id,
            mapping_version,
            on_miss,
            miss_confidence_class,
            "retain_raw",
        )
    }

    fn normalized_miss_resolution_catalog_section(
        mapping_id: &str,
        mapping_version: &str,
    ) -> CovemapSection {
        company_resolution_catalog_section_with_miss_policy(
            mapping_id,
            mapping_version,
            "normalized_value",
            Some("weak_deterministic"),
        )
    }

    fn company_resolution_catalog_section_with_policy(
        mapping_id: &str,
        mapping_version: &str,
        on_miss: &str,
        miss_confidence_class: Option<&str>,
        evidence_policy: &str,
    ) -> CovemapSection {
        let pipeline_digest_input = json!({
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        });
        let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
        let alias_catalog_digest_input = json!({
            "alias_catalog_id": "company_aliases",
            "entries": [{
                "alias_entry_id": "company:tesco",
                "canonical_key": "uk-company:tesco",
                "canonical_label": "Tesco",
                "aliases": ["Tesco", "Tesco PLC", "tesco supermarket"]
            }]
        });
        let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
        let resolver_digest_input = json!({
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "pipeline_digest": pipeline_digest,
            "on_hit": "canonical_key",
            "on_miss": on_miss,
            "miss_confidence_class": miss_confidence_class,
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "evidence_policy": evidence_policy,
        });
        let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
        let mut payload = json!({
            "mapping_id": mapping_id,
            "mapping_version": mapping_version,
            "normalization_pipelines": [{
                "pipeline_id": "company_name.v1",
                "functions": [
                    {"function_id": "unicode_nfkc", "version": "1"},
                    {"function_id": "unicode_casefold", "version": "1"},
                    {"function_id": "trim", "version": "1"},
                    {"function_id": "collapse_whitespace", "version": "1"}
                ],
                "tables": []
            }],
            "resolvers": [{
                "resolver_id": "uk_company_name_resolver",
                "kind": "alias_catalog",
                "object_type": "Company",
                "authority": "curated",
                "confidence_class": "authoritative",
                "normalization_pipeline_id": "company_name.v1",
                "on_hit": "canonical_key",
                "on_miss": on_miss,
                "miss_confidence_class": miss_confidence_class,
                "ambiguous_policy": "reject_auto_merge",
                "catalog_digest": catalog_digest,
                "pipeline_digest": pipeline_digest,
                "resolver_digest": resolver_digest,
                "evidence_policy": evidence_policy,
                "alias_catalog": {
                    "alias_catalog_id": "company_aliases",
                    "entries": [{
                        "alias_entry_id": "company:tesco",
                        "canonical_key": "uk-company:tesco",
                        "canonical_label": "Tesco",
                        "aliases": ["tesco supermarket", "Tesco PLC", "Tesco"]
                    }]
                }
            }],
            "match_rules": [],
            "reviewed_decisions": []
        });
        if miss_confidence_class.is_none() {
            payload["resolvers"][0]
                .as_object_mut()
                .unwrap()
                .remove("miss_confidence_class");
        }
        test_section(SectionKind::MapResolutionCatalog, payload)
    }

    fn team_resolution_catalog_section(mapping_id: &str, mapping_version: &str) -> CovemapSection {
        let pipeline_digest_input = json!({
            "pipeline_id": "team_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        });
        let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
        let alias_catalog_digest_input = json!({
            "alias_catalog_id": "team_aliases",
            "entries": [{
                "alias_entry_id": "team:alpha",
                "canonical_key": "team:alpha",
                "canonical_label": "Alpha Team",
                "aliases": ["Alpha Team Ltd", "Team Alpha", "alpha team"]
            }]
        });
        let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
        let resolver_digest_input = json!({
            "resolver_id": "team_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Team",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "team_name.v1",
            "pipeline_digest": pipeline_digest,
            "on_hit": "canonical_key",
            "on_miss": "reject",
            "miss_confidence_class": null,
            "ambiguous_policy": "reject_auto_merge",
            "catalog_digest": catalog_digest,
            "evidence_policy": "retain_raw",
        });
        let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
        test_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": mapping_id,
                "mapping_version": mapping_version,
                "normalization_pipelines": [{
                    "pipeline_id": "team_name.v1",
                    "functions": [
                        {"function_id": "unicode_nfkc", "version": "1"},
                        {"function_id": "unicode_casefold", "version": "1"},
                        {"function_id": "trim", "version": "1"},
                        {"function_id": "collapse_whitespace", "version": "1"}
                    ],
                    "tables": []
                }],
                "resolvers": [{
                    "resolver_id": "team_name_resolver",
                    "kind": "alias_catalog",
                    "object_type": "Team",
                    "authority": "curated",
                    "confidence_class": "authoritative",
                    "normalization_pipeline_id": "team_name.v1",
                    "on_hit": "canonical_key",
                    "on_miss": "reject",
                    "ambiguous_policy": "reject_auto_merge",
                    "catalog_digest": catalog_digest,
                    "pipeline_digest": pipeline_digest,
                    "resolver_digest": resolver_digest,
                    "alias_catalog": {
                        "alias_catalog_id": "team_aliases",
                        "entries": [{
                            "alias_entry_id": "team:alpha",
                            "canonical_key": "team:alpha",
                            "canonical_label": "Alpha Team",
                            "aliases": ["Team Alpha", "Alpha Team Ltd", "alpha team"]
                        }]
                    }
                }],
                "match_rules": [],
                "reviewed_decisions": []
            }),
        )
    }

    fn ambiguous_company_resolution_catalog_section(
        mapping_id: &str,
        mapping_version: &str,
        ambiguous_policy: &str,
    ) -> CovemapSection {
        ambiguous_company_resolution_catalog_section_with_policy(
            mapping_id,
            mapping_version,
            ambiguous_policy,
            "retain_raw",
        )
    }

    fn ambiguous_company_resolution_catalog_section_with_policy(
        mapping_id: &str,
        mapping_version: &str,
        ambiguous_policy: &str,
        evidence_policy: &str,
    ) -> CovemapSection {
        let pipeline_digest_input = json!({
            "pipeline_id": "company_name.v1",
            "functions": [
                {"function_id": "unicode_nfkc", "version": "1"},
                {"function_id": "unicode_casefold", "version": "1"},
                {"function_id": "trim", "version": "1"},
                {"function_id": "collapse_whitespace", "version": "1"}
            ],
            "tables": []
        });
        let pipeline_digest = test_sha256_digest(&test_canonical_json(&pipeline_digest_input));
        let alias_catalog_digest_input = json!({
            "alias_catalog_id": "company_aliases",
            "entries": [{
                "alias_entry_id": "company:tesco",
                "canonical_key": "uk-company:tesco",
                "canonical_label": "Tesco",
                "aliases": ["Tesco", "Tesco PLC", "tesco supermarket"],
                "ambiguous": true
            }]
        });
        let catalog_digest = test_sha256_digest(&test_canonical_json(&alias_catalog_digest_input));
        let resolver_digest_input = json!({
            "resolver_id": "uk_company_name_resolver",
            "kind": "alias_catalog",
            "object_type": "Company",
            "authority": "curated",
            "confidence_class": "authoritative",
            "normalization_pipeline_id": "company_name.v1",
            "pipeline_digest": pipeline_digest,
            "on_hit": "canonical_key",
            "on_miss": "candidate_only",
            "miss_confidence_class": null,
            "ambiguous_policy": ambiguous_policy,
            "catalog_digest": catalog_digest,
            "evidence_policy": evidence_policy,
        });
        let resolver_digest = test_sha256_digest(&test_canonical_json(&resolver_digest_input));
        test_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": mapping_id,
                "mapping_version": mapping_version,
                "normalization_pipelines": [{
                    "pipeline_id": "company_name.v1",
                    "functions": [
                        {"function_id": "unicode_nfkc", "version": "1"},
                        {"function_id": "unicode_casefold", "version": "1"},
                        {"function_id": "trim", "version": "1"},
                        {"function_id": "collapse_whitespace", "version": "1"}
                    ],
                    "tables": []
                }],
                "resolvers": [{
                    "resolver_id": "uk_company_name_resolver",
                    "kind": "alias_catalog",
                    "object_type": "Company",
                    "authority": "curated",
                    "confidence_class": "authoritative",
                    "normalization_pipeline_id": "company_name.v1",
                    "on_hit": "canonical_key",
                    "on_miss": "candidate_only",
                    "ambiguous_policy": ambiguous_policy,
                    "catalog_digest": catalog_digest,
                    "pipeline_digest": pipeline_digest,
                    "resolver_digest": resolver_digest,
                    "evidence_policy": evidence_policy,
                    "alias_catalog": {
                        "alias_catalog_id": "company_aliases",
                        "entries": [{
                            "alias_entry_id": "company:tesco",
                            "canonical_key": "uk-company:tesco",
                            "canonical_label": "Tesco",
                            "aliases": ["tesco supermarket", "Tesco PLC", "Tesco"],
                            "ambiguous": true
                        }]
                    }
                }],
                "match_rules": [],
                "reviewed_decisions": []
            }),
        )
    }

    fn company_resolution_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "test/v1",
                    "sources": [{
                        "source_id": "suppliers",
                        "row_identity_rules": ["company_by_resolved_name"]
                    }]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "test/v1",
                    "functions": [
                        {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                    ]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [{
                        "rule_id": "company_by_resolved_name",
                        "object_type": "Company",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "auto_merge": true,
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "company",
                            "source_column": "company_name",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared",
                            "resolution": {
                                "resolver_id": "uk_company_name_resolver"
                            }
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "company-map",
                    "mapping_version": "test/v1",
                    "rules": [{
                        "rule_id": "supplier_company",
                        "source_id": "suppliers",
                        "identity_rule_id": "company_by_resolved_name",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }]
                }),
            ),
            resolution_catalog_section("company-map", "test/v1"),
        ])
    }

    fn test_sha256_digest(bytes: &[u8]) -> String {
        format!("sha256:{}", sha256_hex(bytes))
    }

    fn test_canonical_json(value: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        write_test_canonical_json(value, &mut out);
        out
    }

    fn write_test_canonical_json(value: &Value, out: &mut Vec<u8>) {
        match value {
            Value::Null => out.extend_from_slice(b"null"),
            Value::Bool(true) => out.extend_from_slice(b"true"),
            Value::Bool(false) => out.extend_from_slice(b"false"),
            Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
            Value::String(value) => {
                out.extend_from_slice(Value::String(value.clone()).to_string().as_bytes());
            }
            Value::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    write_test_canonical_json(value, out);
                }
                out.push(b']');
            }
            Value::Object(object) => {
                out.push(b'{');
                let mut keys = object
                    .keys()
                    .filter(|key| key.as_str() != "non_semantic_metadata")
                    .collect::<Vec<_>>();
                keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(Value::String((*key).clone()).to_string().as_bytes());
                    out.push(b':');
                    write_test_canonical_json(object.get(*key).unwrap(), out);
                }
                out.push(b'}');
            }
        }
    }

    fn people_batch() -> RecordBatch {
        RecordBatch::try_from_iter(vec![
            (
                "person_id",
                Arc::new(StringArray::from(vec!["p1"])) as arrow_array::ArrayRef,
            ),
            (
                "team_id",
                Arc::new(StringArray::from(vec!["t1"])) as arrow_array::ArrayRef,
            ),
            (
                "valid_from",
                Arc::new(StringArray::from(vec!["2026-01-01"])) as arrow_array::ArrayRef,
            ),
            (
                "valid_to",
                Arc::new(StringArray::from(vec!["2026-12-31"])) as arrow_array::ArrayRef,
            ),
        ])
        .unwrap()
    }

    fn write_arrow_ipc(batch: &RecordBatch) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = IpcFileWriter::try_new(&mut bytes, &batch.schema()).unwrap();
            writer.write(batch).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    fn write_parquet(batch: &RecordBatch) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
            writer.write(batch).unwrap();
            writer.close().unwrap();
        }
        bytes
    }

    fn write_orc(batch: &RecordBatch) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = OrcWriterBuilder::new(&mut bytes, batch.schema())
                .try_build()
                .unwrap();
            writer.write(batch).unwrap();
            writer.close().unwrap();
        }
        bytes
    }

    fn two_source_identity_map(do_not_merge: Vec<Value>) -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [
                        {"source_id": "crm", "row_identity_rules": ["person_by_id"]},
                        {"source_id": "support", "row_identity_rules": ["person_by_id"]}
                    ]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [{
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": do_not_merge
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "crm_person",
                            "source_id": "crm",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        },
                        {
                            "rule_id": "support_person",
                            "source_id": "support",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
        ])
    }

    fn reviewed_decisions_section(
        mapping_id: &str,
        mapping_version: &str,
        decisions: Vec<Value>,
    ) -> CovemapSection {
        test_section(
            SectionKind::MapResolutionCatalog,
            json!({
                "mapping_id": mapping_id,
                "mapping_version": mapping_version,
                "normalization_pipelines": [],
                "resolvers": [],
                "match_rules": [],
                "reviewed_decisions": decisions
            }),
        )
    }

    fn add_reviewed_decisions(file: &mut CovemapFile, decisions: Vec<Value>) {
        file.sections.push(reviewed_decisions_section(
            "people-map",
            "test/v1",
            decisions,
        ));
    }

    fn set_person_reviewed_equivalence(file: &mut CovemapFile, allowed: bool) {
        mutate_section_payload(file, 2, |payload| {
            payload["identity_rules"][0]["allow_reviewed_equivalence"] = json!(allowed);
        });
    }

    fn identity_alias_ref(object_type: &str, identity_alias: &str) -> Value {
        json!({
            "kind": "identity_alias",
            "object_type": object_type,
            "identity_alias": identity_alias
        })
    }

    fn reviewed_same_object_decision(left: Value, right: Value, anchor: Option<Value>) -> Value {
        let mut decision = json!({
            "decision_id": "review:same-object",
            "decision": "same_object",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": left,
            "right": right
        });
        if let Some(anchor) = anchor {
            decision["canonical_anchor"] = anchor;
        }
        decision
    }

    fn reviewed_do_not_merge_decision(left: Value, right: Value) -> Value {
        json!({
            "decision_id": "review:do-not-merge",
            "decision": "do_not_merge",
            "confidence_class": "reviewed_authoritative",
            "reviewed_by": "mapping-author",
            "reviewed_at": "2026-06-25T00:00:00Z",
            "left": left,
            "right": right
        })
    }

    fn reviewed_rows(ids: &[(&str, &str)]) -> Vec<SourceRow> {
        ids.iter()
            .map(|(source_id, id)| SourceRow {
                source_id: (*source_id).into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!(*id))]),
            })
            .collect()
    }

    fn three_source_identity_map() -> CovemapFile {
        let mut file = two_source_identity_map(Vec::new());
        mutate_section_payload(&mut file, 0, |payload| {
            payload["sources"].as_array_mut().unwrap().push(json!({
                "source_id": "ops",
                "row_identity_rules": ["person_by_id"]
            }));
        });
        mutate_section_payload(&mut file, 3, |payload| {
            payload["rules"].as_array_mut().unwrap().push(json!({
                "rule_id": "ops_person",
                "source_id": "ops",
                "identity_rule_id": "person_by_id",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": []
            }));
        });
        file
    }

    fn cross_rule_reviewed_identity_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [{
                        "source_id": "people",
                        "row_identity_rules": ["person_by_id", "person_by_email"]
                    }]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "allow_reviewed_equivalence": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        },
                        {
                            "rule_id": "person_by_email",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "allow_reviewed_equivalence": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "email",
                                "source_column": "email",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "person_id_row",
                            "source_id": "people",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        },
                        {
                            "rule_id": "person_email_row",
                            "source_id": "people",
                            "identity_rule_id": "person_by_email",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
        ])
    }

    fn add_optional_i64(object: &mut Value, key: &str, value: Option<i64>) {
        if let Some(value) = value {
            object
                .as_object_mut()
                .unwrap()
                .insert(key.into(), json!(value));
        }
    }

    fn two_source_property_map(
        conflict_policy: &str,
        crm_priority: Option<i64>,
        support_priority: Option<i64>,
    ) -> CovemapFile {
        let mut crm = json!({
            "source_id": "crm",
            "row_identity_rules": ["person_by_id"]
        });
        add_optional_i64(&mut crm, "source_priority", crm_priority);
        let mut support = json!({
            "source_id": "support",
            "row_identity_rules": ["person_by_id"]
        });
        add_optional_i64(&mut support, "source_priority", support_priority);

        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [crm, support]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [{
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "crm_person",
                            "source_id": "crm",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "crm_name",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "conflict_policy": conflict_policy
                            }]
                        },
                        {
                            "rule_id": "support_person",
                            "source_id": "support",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "support_name",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "conflict_policy": conflict_policy
                            }]
                        }
                    ]
                }),
            ),
        ])
    }

    fn conflict_rows(crm_name: Value, support_name: Value) -> Vec<SourceRow> {
        vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), crm_name)]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), support_name)]),
            },
        ]
    }

    fn build_projection_map() -> CovemapFile {
        let mut file = two_source_property_map("source_priority_wins", Some(10), Some(1));
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_projection",
                    "output_table": "people_projection",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "name", "value": "name", "logical_type": "utf8"}
                    ],
                    "output_modes": ["json", "cove-t"]
                }]
            }),
        ));
        file
    }

    fn temp_build_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cove-map-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_build_fixture(label: &str, file: &CovemapFile) -> (PathBuf, Vec<PathBuf>, PathBuf) {
        let dir = temp_build_dir(label);
        let map = dir.join("people.covemap");
        fs::write(&map, file.serialize().unwrap()).unwrap();
        let crm = dir.join("crm.csv");
        let support = dir.join("support.csv");
        fs::write(&crm, "id,name\n1,CRM Name\n").unwrap();
        fs::write(&support, "id,name\n1,Support Name\n").unwrap();
        (map, vec![crm, support], dir)
    }

    fn write_map_and_sources(
        label: &str,
        map_file: &CovemapFile,
        sources: &[(&str, &str)],
    ) -> (PathBuf, Vec<PathBuf>, PathBuf) {
        let dir = temp_build_dir(label);
        let map = dir.join("mapping.covemap");
        fs::write(&map, map_file.serialize().unwrap()).unwrap();
        let source_paths = sources
            .iter()
            .map(|(name, contents)| {
                let path = dir.join(name);
                fs::write(&path, contents).unwrap();
                path
            })
            .collect::<Vec<_>>();
        (map, source_paths, dir)
    }

    fn empty_cove_o_parent_bytes() -> Vec<u8> {
        compact_cove_o_from_object_states(Vec::new(), &[]).unwrap()
    }

    fn delta_parent_from_base_bytes(base: &[u8]) -> MapSemanticDeltaParent {
        let validation = validate_bytes(base).unwrap();
        let digest = compute_digest(DigestAlgorithm::Sha256, base).unwrap();
        let mut digest_array = [0u8; 32];
        digest_array.copy_from_slice(&digest);
        let file_id = validation.header.file_id;
        MapSemanticDeltaParent {
            dataset_id: [0xDD; 16],
            parent_snapshot_id: file_id,
            chain_ordinal: 1,
            chain_depth: 1,
            parent_ref: CovmDeltaArtifactRefV1 {
                chain_ordinal: 0,
                flags: 0,
                artifact_id: file_id,
                snapshot_id: file_id,
                parent_snapshot_id: [0; 16],
                file_len: base.len() as u64,
                footer_crc32c: validation.postscript.footer.crc32c,
                digest_algorithm: DigestAlgorithm::Sha256 as u16,
                digest_len: 32,
                digest: digest_array,
                uri_ref: 0,
                checksum: 0,
            },
        }
    }

    fn semantic_delta_options(
        out: PathBuf,
        parent: MapSemanticDeltaParent,
    ) -> MapSemanticDeltaBuildOptions {
        MapSemanticDeltaBuildOptions {
            out,
            force: true,
            parent,
            parent_object_types: Vec::new(),
            parent_object_states: Vec::new(),
            parent_evidence_entries: Vec::new(),
            parent_projection_catalog: None,
            csn_start: 1,
            commit_time_start_us: 1_800_000_000_000_000,
            source_publish_range_us: None,
        }
    }

    fn semantic_delta_options_from_parent_bytes(
        out: PathBuf,
        parent_bytes: &[u8],
    ) -> MapSemanticDeltaBuildOptions {
        let surface = read_object_surface_from_bytes(parent_bytes).unwrap();
        let parent_object_states =
            reconstruct_object_states(&surface, &Default::default()).unwrap();
        let mut options = semantic_delta_options(out, delta_parent_from_base_bytes(parent_bytes));
        options.parent_object_types = surface.object_types;
        options.parent_object_states = parent_object_states;
        options.parent_evidence_entries = surface
            .evidence_index
            .map(|index| index.entries)
            .unwrap_or_default();
        options.parent_projection_catalog = surface.projection_catalog;
        options
    }

    fn build_semantic_delta_fixture(
        label: &str,
        map_file: &CovemapFile,
        sources: &[(&str, &str)],
        parent: MapSemanticDeltaParent,
    ) -> (Value, Vec<u8>, PathBuf) {
        let (map, source_paths, dir) = write_map_and_sources(label, map_file, sources);
        let out = dir.join("semantic.covedelta");
        let result = build_semantic_delta_from_paths(
            &map,
            &source_paths,
            semantic_delta_options(out.clone(), parent),
        )
        .unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(
            result.report["format"],
            json!("cove-map-semantic-delta-build-report-v1")
        );
        (result.report, bytes, dir)
    }

    fn full_rebuild_state_keys(
        label: &str,
        map_file: &CovemapFile,
        sources: &[(&str, &str)],
    ) -> Vec<StateKey> {
        let (_map, source_paths, _dir) = write_map_and_sources(label, map_file, sources);
        let inputs = read_source_inputs(&source_paths).unwrap();
        validate_source_inputs(map_file, &inputs.states).unwrap();
        let object_bytes =
            build_cove_o_with_source_states(map_file, &inputs.rows, &inputs.states).unwrap();
        let surface = read_object_surface_from_bytes(&object_bytes).unwrap();
        let states = reconstruct_object_states(&surface, &Default::default()).unwrap();
        state_keys(&states)
    }

    type StateKey = (u32, [u8; 16], Vec<(u32, Value)>);
    type EvidenceKey = (String, String, String, String, String);

    fn state_keys(states: &[CoveObjectState]) -> Vec<StateKey> {
        let mut keys = states
            .iter()
            .map(|state| {
                let mut properties = state
                    .properties
                    .iter()
                    .map(|property| (property.property_id, property.value.clone()))
                    .collect::<Vec<_>>();
                properties.sort_by_key(|(property_id, _)| *property_id);
                (state.object_type_id, state.goid, properties)
            })
            .collect::<Vec<_>>();
        keys.sort_by_key(|(object_type_id, goid, _)| (*object_type_id, *goid));
        keys
    }

    fn composed_delta_state_keys(base: &[u8], delta_bytes: &[u8]) -> Vec<StateKey> {
        let delta = CoveDeltaFile::parse(delta_bytes).unwrap();
        delta.validate_object_delta().unwrap();
        let states = reconstruct_object_states_from_base_and_delta_files(
            base,
            &[delta],
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        state_keys(&states)
    }

    fn evidence_keys_from_cove_o_bytes(bytes: &[u8]) -> BTreeSet<EvidenceKey> {
        let surface = read_object_surface_from_bytes(bytes).unwrap();
        evidence_keys_from_surface(&surface)
    }

    fn composed_delta_evidence_keys(base: &[u8], delta_bytes: &[u8]) -> BTreeSet<EvidenceKey> {
        let delta = CoveDeltaFile::parse(delta_bytes).unwrap();
        delta.validate_object_delta().unwrap();
        let surface = read_object_surface_from_base_and_delta_files(base, &[delta]).unwrap();
        evidence_keys_from_surface(&surface)
    }

    fn evidence_keys_from_surface(
        surface: &cove_core::profile::cove_o::CoveObjectSurface,
    ) -> BTreeSet<EvidenceKey> {
        surface
            .evidence_index
            .as_ref()
            .map(|index| {
                index
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            entry.source_id.clone(),
                            entry.source_row_identity.clone(),
                            entry.rule_id.clone(),
                            entry.assertion_id.clone(),
                            entry.output_object_id.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn primitive_projection_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [{
                        "source_id": "people",
                        "row_identity_rules": ["person_by_id"]
                    }]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [{
                        "rule_id": "person_by_id",
                        "object_type": "Person",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "person_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [{
                        "rule_id": "person_row",
                        "source_id": "people",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "Object",
                        "assertion_kinds": ["object", "property", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": [],
                        "property_bindings": [
                            {
                                "assertion_id": "active",
                                "property_id": "active",
                                "property_name": "active",
                                "source_column": "active",
                                "logical_type": "bool",
                                "nullable": true,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "score",
                                "property_id": "score",
                                "property_name": "score",
                                "source_column": "score",
                                "logical_type": "int64",
                                "nullable": true,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "rating",
                                "property_id": "rating",
                                "property_name": "rating",
                                "source_column": "rating",
                                "logical_type": "int64",
                                "nullable": true,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "status",
                                "property_id": "status",
                                "property_name": "status",
                                "source_column": "status",
                                "logical_type": "utf8",
                                "nullable": true,
                                "conflict_policy": "reject_conflict"
                            },
                            {
                                "assertion_id": "nickname",
                                "property_id": "nickname",
                                "property_name": "nickname",
                                "source_column": "nickname",
                                "logical_type": "utf8",
                                "nullable": true,
                                "conflict_policy": "reject_conflict"
                            }
                        ]
                    }]
                }),
            ),
            test_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "projections": [
                        {
                            "projection_id": "people_primitives.v1",
                            "output_table": "people_primitives",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "columns": [
                                {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                                {"name": "active", "value": "property.active", "logical_type": "bool"},
                                {"name": "score", "value": "score", "logical_type": "int64"},
                                {"name": "rating", "value": "rating", "logical_type": "int64"},
                                {"name": "status", "value": "status", "logical_type": "utf8"},
                                {"name": "nickname", "value": "nickname", "logical_type": "utf8"}
                            ],
                            "output_modes": ["arrow"]
                        },
                        {
                            "projection_id": "people_primitives_ordered.v1",
                            "output_table": "people_primitives",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "columns": [
                                {"name": "active", "value": "property.active", "logical_type": "bool"},
                                {"name": "score", "value": "score", "logical_type": "int64"},
                                {"name": "status", "value": "status", "logical_type": "utf8"}
                            ],
                            "ordering": ["score asc"],
                            "output_modes": ["arrow"]
                        }
                    ]
                }),
            ),
        ])
    }

    fn primitive_projection_rows() -> Vec<SourceRow> {
        vec![
            SourceRow {
                source_id: "people".into(),
                row_index: 0,
                values: BTreeMap::from([
                    ("id".into(), json!("p1")),
                    ("active".into(), json!(true)),
                    ("score".into(), json!(10)),
                    ("rating".into(), json!(15)),
                    ("status".into(), json!("open")),
                    ("nickname".into(), json!("alpha")),
                ]),
            },
            SourceRow {
                source_id: "people".into(),
                row_index: 1,
                values: BTreeMap::from([
                    ("id".into(), json!("p2")),
                    ("active".into(), json!(false)),
                    ("score".into(), json!(20)),
                    ("rating".into(), json!(25)),
                    ("status".into(), json!("closed")),
                    ("nickname".into(), Value::Null),
                ]),
            },
            SourceRow {
                source_id: "people".into(),
                row_index: 2,
                values: BTreeMap::from([
                    ("id".into(), json!("p3")),
                    ("active".into(), json!(true)),
                    ("score".into(), json!(30)),
                    ("rating".into(), json!(35)),
                    ("status".into(), json!("open")),
                ]),
            },
            SourceRow {
                source_id: "people".into(),
                row_index: 3,
                values: BTreeMap::from([
                    ("id".into(), json!("p4")),
                    ("active".into(), json!(true)),
                    ("score".into(), json!(40)),
                    ("rating".into(), json!(45)),
                    ("status".into(), Value::Null),
                    ("nickname".into(), json!("delta")),
                ]),
            },
        ]
    }

    fn int64_column_values(batches: &[RecordBatch], column_name: &str) -> Vec<i64> {
        let mut out = Vec::new();
        for batch in batches {
            let index = batch.schema().index_of(column_name).unwrap();
            let array = batch
                .column(index)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            for row in 0..array.len() {
                if array.is_valid(row) {
                    out.push(array.value(row));
                }
            }
        }
        out
    }

    fn association_readback_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [{
                        "source_id": "people",
                        "row_identity_rules": ["person_by_id", "team_by_id"]
                    }]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "person_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        },
                        {
                            "rule_id": "team_by_id",
                            "object_type": "Team",
                            "semantic_role": "organization",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "team_id",
                                "source_column": "team_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "person_row",
                            "source_id": "people",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "association", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "association_bindings": [{
                                "assertion_id": "member_of_assertion",
                                "association_type": "member_of",
                                "source_identity_rule_id": "person_by_id",
                                "source_endpoint_expression": "source.goid",
                                "target_identity_rule_id": "team_by_id",
                                "target_endpoint_expression": "identity(team_by_id)",
                                "source_role": "member",
                                "target_role": "team",
                                "valid_from_expression": "source.valid_from",
                                "valid_to_expression": "source.valid_to",
                                "cardinality_policy": "many_to_one",
                                "missing_policy": "reject"
                            }]
                        },
                        {
                            "rule_id": "team_row",
                            "source_id": "people",
                            "identity_rule_id": "team_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
        ])
    }

    fn alias_backed_association_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [
                        {
                            "source_id": "memberships",
                            "row_identity_rules": ["person_by_id"]
                        },
                        {
                            "source_id": "teams",
                            "row_identity_rules": ["team_by_name"]
                        }
                    ]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [
                        {"function_id": "identity", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_nfkc", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "unicode_casefold", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "trim", "version": "1", "deterministic": true, "dependency": "pure"},
                        {"function_id": "collapse_whitespace", "version": "1", "deterministic": true, "dependency": "pure"}
                    ]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "person_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        },
                        {
                            "rule_id": "team_by_name",
                            "object_type": "Team",
                            "semantic_role": "organization",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "team",
                                "source_column": "team_name",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared",
                                "resolution": {
                                    "resolver_id": "team_name_resolver"
                                }
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "membership_row",
                            "source_id": "memberships",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "association", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "association_bindings": [{
                                "assertion_id": "member_of_assertion",
                                "association_type": "member_of",
                                "source_identity_rule_id": "person_by_id",
                                "source_endpoint_expression": "source.goid",
                                "target_identity_rule_id": "team_by_name",
                                "target_endpoint_expression": "identity(team_by_name)",
                                "source_role": "member",
                                "target_role": "team",
                                "valid_from_expression": "source.valid_from",
                                "valid_to_expression": "source.valid_to",
                                "cardinality_policy": "many_to_one",
                                "missing_policy": "reject"
                            }]
                        },
                        {
                            "rule_id": "team_row",
                            "source_id": "teams",
                            "identity_rule_id": "team_by_name",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
            team_resolution_catalog_section("people-map", "test/v1"),
        ])
    }

    fn source_scoped_ambiguous_association_map() -> CovemapFile {
        test_covemap(vec![
            test_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "sources": [
                        {
                            "source_id": "memberships",
                            "row_identity_rules": ["person_by_id"]
                        },
                        {
                            "source_id": "teams_a",
                            "row_identity_rules": ["team_by_id"]
                        },
                        {
                            "source_id": "teams_b",
                            "row_identity_rules": ["team_by_id"]
                        }
                    ]
                }),
            ),
            test_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            test_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "person_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        },
                        {
                            "rule_id": "team_by_id",
                            "object_type": "Team",
                            "semantic_role": "organization",
                            "confidence_class": "source_scoped",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "team_id",
                                "source_column": "team_id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            test_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "people-map",
                    "mapping_version": "test/v1",
                    "rules": [
                        {
                            "rule_id": "membership_row",
                            "source_id": "memberships",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "association", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "association_bindings": [{
                                "assertion_id": "member_of_assertion",
                                "association_type": "member_of",
                                "source_identity_rule_id": "person_by_id",
                                "source_endpoint_expression": "source.goid",
                                "target_identity_rule_id": "team_by_id",
                                "target_endpoint_expression": "identity(team_by_id)",
                                "source_role": "member",
                                "target_role": "team",
                                "valid_from_expression": "source.valid_from",
                                "valid_to_expression": "source.valid_to",
                                "cardinality_policy": "many_to_one",
                                "missing_policy": "reject"
                            }]
                        },
                        {
                            "rule_id": "team_a_row",
                            "source_id": "teams_a",
                            "identity_rule_id": "team_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        },
                        {
                            "rule_id": "team_b_row",
                            "source_id": "teams_b",
                            "identity_rule_id": "team_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": []
                        }
                    ]
                }),
            ),
        ])
    }

    fn governance_map(policy: &str) -> CovemapFile {
        let mut file = two_source_identity_map(Vec::new());
        file.sections[0] = test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "governance_reconciliation_policy": policy,
                "sources": [
                    {
                        "source_id": "crm",
                        "row_identity_rules": ["person_by_id"],
                        "sensitivity_label": "public",
                        "sensitivity_rank": 1,
                        "access_policy_ids": ["internal"]
                    },
                    {
                        "source_id": "support",
                        "row_identity_rules": ["person_by_id"],
                        "sensitivity_label": "restricted",
                        "sensitivity_rank": 5,
                        "access_policy_ids": ["hipaa"]
                    }
                ]
            }),
        );
        file
    }

    #[test]
    fn parses_validate_command() {
        assert_eq!(
            parse_args(["validate".to_string(), "mapping.covemap".to_string()])
                .unwrap()
                .unwrap(),
            Command::Validate {
                map: PathBuf::from("mapping.covemap")
            }
        );
    }

    #[test]
    fn parses_convert_cove_o_format() {
        let command = parse_args([
            "convert".to_string(),
            "--format".to_string(),
            "cove-o".to_string(),
            "-o".to_string(),
            "out.cove".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            command,
            Command::Convert {
                map: PathBuf::from("mapping.covemap"),
                sources: vec![PathBuf::from("source.jsonl")],
                output: Some(PathBuf::from("out.cove")),
                format: OutputFormat::CoveO,
            }
        );
    }

    #[test]
    fn parses_project_cove_o_command() {
        let command = parse_args([
            "project-cove-o".to_string(),
            "--mapping".to_string(),
            "mapping.covemap".to_string(),
            "-o".to_string(),
            "projection.json".to_string(),
            "object.cove".to_string(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            command,
            Command::ProjectCoveO {
                object: PathBuf::from("object.cove"),
                mapping: Some(PathBuf::from("mapping.covemap")),
                output: Some(PathBuf::from("projection.json")),
                format: ProjectionFormat::Json,
                projection_id: None,
            }
        );
    }

    #[test]
    fn parses_build_command_defaults() {
        let command = parse_args([
            "build".to_string(),
            "--out-dir".to_string(),
            "bundle".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            command,
            Command::Build {
                map: PathBuf::from("mapping.covemap"),
                sources: vec![PathBuf::from("source.jsonl")],
                out_dir: PathBuf::from("bundle"),
                force: false,
                json: false,
                object_name: None,
                projection_output: MapBuildProjectionOutput::CoveT,
                evidence_encoding: MapEvidenceEncoding::Compact,
                section_compression: MapBuildSectionCompression::Zstd,
                verify: false,
                publish_covm: false,
            }
        );
    }

    #[test]
    fn parses_build_command_options() {
        let command = parse_args([
            "build".to_string(),
            "--out-dir".to_string(),
            "bundle".to_string(),
            "--force".to_string(),
            "--json".to_string(),
            "--verify".to_string(),
            "--publish-covm".to_string(),
            "--object-name".to_string(),
            "people.cove".to_string(),
            "--projection-output".to_string(),
            "none".to_string(),
            "--evidence-encoding".to_string(),
            "expanded".to_string(),
            "--section-compression".to_string(),
            "none".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            command,
            Command::Build {
                map: PathBuf::from("mapping.covemap"),
                sources: vec![PathBuf::from("source.jsonl")],
                out_dir: PathBuf::from("bundle"),
                force: true,
                json: true,
                object_name: Some("people.cove".into()),
                projection_output: MapBuildProjectionOutput::None,
                evidence_encoding: MapEvidenceEncoding::Expanded,
                section_compression: MapBuildSectionCompression::None,
                verify: true,
                publish_covm: true,
            }
        );
        let err = parse_args([
            "build".to_string(),
            "--out-dir".to_string(),
            "bundle".to_string(),
            "--evidence-encoding".to_string(),
            "json".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap_err();
        assert!(err.contains("--evidence-encoding"));
        let err = parse_args([
            "build".to_string(),
            "--out-dir".to_string(),
            "bundle".to_string(),
            "--section-compression".to_string(),
            "brotli".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap_err();
        assert!(err.contains("--section-compression"));
        assert_eq!(
            parse_args([
                "publish".to_string(),
                "--bundle-dir".to_string(),
                "bundle".to_string(),
                "--out".to_string(),
                "dataset.covm".to_string(),
                "--force".to_string(),
                "--json".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Publish {
                bundle_dir: PathBuf::from("bundle"),
                output: PathBuf::from("dataset.covm"),
                force: true,
                json: true,
            }
        );
        assert_eq!(
            parse_args([
                "candidates".to_string(),
                "--out".to_string(),
                "candidates.json".to_string(),
                "mapping.covemap".to_string(),
                "suppliers.csv".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Candidates {
                map: PathBuf::from("mapping.covemap"),
                sources: vec![PathBuf::from("suppliers.csv")],
                output: Some(PathBuf::from("candidates.json")),
            }
        );
        assert_eq!(
            parse_args([
                "review".to_string(),
                "--out".to_string(),
                "reviewed.json".to_string(),
                "candidates.json".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Review {
                candidates: PathBuf::from("candidates.json"),
                output: Some(PathBuf::from("reviewed.json")),
            }
        );
        assert_eq!(
            parse_args([
                "review".to_string(),
                "import".to_string(),
                "mapping.covemap".to_string(),
                "reviewed.json".to_string(),
                "--out".to_string(),
                "mapping-reviewed.covemap".to_string(),
                "--replace".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::ReviewImport {
                map: PathBuf::from("mapping.covemap"),
                review: PathBuf::from("reviewed.json"),
                output: PathBuf::from("mapping-reviewed.covemap"),
                replace: true,
            }
        );
        assert_eq!(
            parse_args([
                "review".to_string(),
                "export".to_string(),
                "mapping-reviewed.covemap".to_string(),
                "--out".to_string(),
                "reviewed-export.json".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::ReviewExport {
                map: PathBuf::from("mapping-reviewed.covemap"),
                output: Some(PathBuf::from("reviewed-export.json")),
            }
        );
        assert_eq!(
            parse_args([
                "aliases".to_string(),
                "import".to_string(),
                "mapping.covemap".to_string(),
                "aliases.csv".to_string(),
                "--catalog-id".to_string(),
                "company_aliases".to_string(),
                "--resolver-id".to_string(),
                "uk_company_name_resolver".to_string(),
                "--out".to_string(),
                "mapping-with-aliases.covemap".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::AliasesImport {
                map: PathBuf::from("mapping.covemap"),
                aliases: PathBuf::from("aliases.csv"),
                catalog_id: "company_aliases".into(),
                resolver_id: "uk_company_name_resolver".into(),
                output: PathBuf::from("mapping-with-aliases.covemap"),
            }
        );
        assert_eq!(
            parse_args([
                "replay".to_string(),
                "verify".to_string(),
                "mapping.covemap".to_string(),
                "conversion-report.json".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::ReplayVerify {
                map: PathBuf::from("mapping.covemap"),
                report: PathBuf::from("conversion-report.json"),
            }
        );
    }

    #[test]
    fn run_cli_reports_typed_usage_error_with_stable_text() {
        let error = run_cli(["unknown".to_string()]).unwrap_err();
        assert!(matches!(error, MapCliError::Usage(_)));
        assert_eq!(error.to_string(), "unknown subcommand unknown");
    }

    #[test]
    fn parses_doctor_suggest_and_parity_commands() {
        assert_eq!(
            parse_args([
                "doctor".to_string(),
                "--json".to_string(),
                "--strict".to_string(),
                "--bundle-dir".to_string(),
                "bundle".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Doctor {
                bundle_dir: Some(PathBuf::from("bundle")),
                map: None,
                sources: Vec::new(),
                json: true,
                strict: true,
            }
        );
        assert_eq!(
            parse_args([
                "suggest".to_string(),
                "--json".to_string(),
                "--out".to_string(),
                "suggestions.json".to_string(),
                "people.csv".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Suggest {
                sources: vec![PathBuf::from("people.csv")],
                output: Some(PathBuf::from("suggestions.json")),
                json: true,
            }
        );
        assert_eq!(
            parse_args([
                "parity".to_string(),
                "--json".to_string(),
                "--projection-id".to_string(),
                "people.v1".to_string(),
                "--expected".to_string(),
                "expected.csv".to_string(),
                "--key".to_string(),
                "id,name".to_string(),
                "mapping.covemap".to_string(),
                "people.csv".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::Parity {
                map: PathBuf::from("mapping.covemap"),
                sources: vec![PathBuf::from("people.csv")],
                options: ParityOptions {
                    projection_id: "people.v1".into(),
                    expected: PathBuf::from("expected.csv"),
                    expected_query: None,
                    key: vec!["id".into(), "name".into()],
                },
                json: true,
            }
        );
        assert_eq!(
            parse_args([
                "parity-cove-o".to_string(),
                "--projection-id".to_string(),
                "people.v1".to_string(),
                "--expected".to_string(),
                "expected.csv".to_string(),
                "--expected-query".to_string(),
                r#"where(status == "open")"#.to_string(),
                "object.cove".to_string(),
            ])
            .unwrap()
            .unwrap(),
            Command::ParityCoveO {
                object: PathBuf::from("object.cove"),
                options: ParityOptions {
                    projection_id: "people.v1".into(),
                    expected: PathBuf::from("expected.csv"),
                    expected_query: Some(r#"where(status == "open")"#.into()),
                    key: Vec::new(),
                },
                json: false,
            }
        );
        assert!(parse_args([
            "doctor".to_string(),
            "--bundle-dir".to_string(),
            "bundle".to_string(),
            "mapping.covemap".to_string(),
        ])
        .unwrap_err()
        .contains("either --bundle-dir"));
        assert!(parse_args([
            "parity-cove-o".to_string(),
            "--expected".to_string(),
            "expected.csv".to_string(),
            "object.cove".to_string(),
        ])
        .unwrap_err()
        .contains("--projection-id"));
    }

    #[test]
    fn rejects_build_without_out_dir_or_bad_projection_output() {
        assert!(parse_args([
            "build".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap_err()
        .contains("--out-dir"));
        assert!(parse_args([
            "build".to_string(),
            "--out-dir".to_string(),
            "bundle".to_string(),
            "--projection-output".to_string(),
            "parquet".to_string(),
            "mapping.covemap".to_string(),
            "source.jsonl".to_string(),
        ])
        .unwrap_err()
        .contains("cove-t or none"));
    }

    #[test]
    fn join_key_is_deterministic() {
        let components = [
            JoinKeyComponent {
                role_id: "email",
                logical_type_id: "utf8",
                value: Some(b"a@example.com"),
            },
            JoinKeyComponent {
                role_id: "tenant",
                logical_type_id: "utf8",
                value: Some(b"t1"),
            },
        ];
        assert_eq!(
            join_key_tuple(1, "person_by_email", &components),
            join_key_tuple(1, "person_by_email", &components)
        );
    }

    #[test]
    fn join_key_distinguishes_null_from_empty_value() {
        let null_component = [JoinKeyComponent {
            role_id: "email",
            logical_type_id: "utf8",
            value: None,
        }];
        let empty_component = [JoinKeyComponent {
            role_id: "email",
            logical_type_id: "utf8",
            value: Some(b""),
        }];
        assert_ne!(
            join_key_tuple(1, "person_by_email", &null_component),
            join_key_tuple(1, "person_by_email", &empty_component)
        );
    }

    #[test]
    fn unicode_casefold_uses_full_unicode_mapping() {
        let folded = apply_canonicalization(
            &json!("Straße"),
            "unicode_casefold",
            &["unicode_casefold".to_string()],
        )
        .unwrap();
        assert_eq!(folded, json!("strasse"));
    }

    #[test]
    fn goid_is_sha256_truncated_to_16_bytes() {
        let goid = goid16_parts(&[b"map", b"v1", b"person", b"rule", b"key"]);
        assert_eq!(goid.len(), 16);
        assert_eq!(
            goid,
            goid16_parts(&[b"map", b"v1", b"person", b"rule", b"key"])
        );
    }

    #[test]
    fn csv_reader_is_deterministic_for_simple_rows() {
        let dir = std::env::temp_dir().join(format!("cove-map-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("people.csv");
        fs::write(&path, "id,name\n1,Ada\n2,Linus\n").unwrap();
        let rows = read_csv(&path, "people").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values["id"], json!("1"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cross_source_authoritative_identity_merges_to_one_goid() {
        let file = two_source_identity_map(Vec::new());
        let rows = vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
        ];
        let planned = plan_identities(&file, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);
        let index = identity_equivalence_index("people-map", "test/v1", &planned.canonical);
        assert_eq!(index["equivalences"].as_array().unwrap().len(), 0);
        assert_eq!(index["components"].as_array().unwrap().len(), 1);
        assert_eq!(
            index["components"][0]["members"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn alias_catalog_resolver_merges_alias_hits_and_emits_evidence() {
        let file = company_resolution_map();
        file.validate_map_sections().unwrap();
        let rows = ["Tesco", "Tesco PLC", "tesco supermarket"]
            .into_iter()
            .enumerate()
            .map(|(row_index, company_name)| SourceRow {
                source_id: "suppliers".into(),
                row_index,
                values: BTreeMap::from([("company_name".into(), json!(company_name))]),
            })
            .collect::<Vec<_>>();

        let planned = plan_identities(&file, &rows).unwrap();
        assert_eq!(planned.candidates.len(), 0);
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);
        assert!(planned.canonical.iter().all(|identity| {
            identity.resolution_metadata[0].canonical_key.as_deref() == Some("uk-company:tesco")
                && identity.resolution_metadata[0].alias_hit
        }));

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(materialized.rows.len(), 3);
        assert_eq!(
            materialized.conversion_report["candidate_match_count"],
            json!(0)
        );
        assert_eq!(
            materialized.conversion_report["resolver_hit_count"],
            json!(3)
        );
        assert_eq!(
            materialized.conversion_report["resolver_miss_count"],
            json!(0)
        );
        let impact = materialized.conversion_report["resolver_goid_impact"]
            .as_array()
            .unwrap();
        assert_eq!(impact.len(), 1);
        assert_eq!(
            impact[0]["normalization_pipeline_id"],
            json!("company_name.v1")
        );
        assert_eq!(impact[0]["affected_goid_count"], json!(1));
        assert_eq!(impact[0]["affected_goids"].as_array().unwrap().len(), 1);
        assert!(materialized.evidence_entries.iter().all(|entry| {
            entry["resolver_id"] == json!("uk_company_name_resolver")
                && entry["canonical_key"] == json!("uk-company:tesco")
                && entry["canonical_label"] == json!("Tesco")
                && entry["alias_hit"] == json!(true)
        }));
    }

    #[test]
    fn resolver_backed_identity_rule_rejects_object_type_mismatch_at_runtime() {
        let mut file = company_resolution_map();
        mutate_section_payload(&mut file, 2, |payload| {
            payload["identity_rules"][0]["object_type"] = json!("Person");
        });
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        }];

        let err = plan_identities(&file, &rows).unwrap_err();
        assert!(err.contains("resolver 'uk_company_name_resolver' targets object type 'Company'"));
    }

    #[test]
    fn replay_verify_accepts_current_report_and_rejects_stale_resolver() {
        let file = company_resolution_map();
        let rows = ["Tesco", "Tesco PLC"]
            .into_iter()
            .enumerate()
            .map(|(row_index, company_name)| SourceRow {
                source_id: "suppliers".into(),
                row_index,
                values: BTreeMap::from([("company_name".into(), json!(company_name))]),
            })
            .collect::<Vec<_>>();
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();

        let report = verify_replay_report(&file, &materialized.conversion_report).unwrap();
        assert_eq!(report["ok"], json!(true));
        assert_eq!(report["resolver_catalog_digest_count"], json!(1));

        let mut stale = materialized.conversion_report.clone();
        stale["resolver_catalog_digests"][0]["resolver_digest"] =
            json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let err = verify_replay_report(&file, &stale).unwrap_err();
        assert!(err.contains("MAP_REPLAY_STALE_RESOLVER"));
    }

    #[test]
    fn replay_verify_rejects_stale_source_binding() {
        let state = ObservedSourceState {
            source_id: "crm".into(),
            source_kind: "csv".into(),
            schema_fingerprint: "cove-map-schema-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            snapshot_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        };
        let mut file = two_source_identity_map(Vec::new());
        file.sections[0] = test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [
                    {
                        "source_id": "crm",
                        "row_identity_rules": ["person_by_id"],
                        "schema_fingerprint": state.schema_fingerprint.clone(),
                        "snapshot_digest": state.snapshot_digest.clone(),
                        "replay_claimed": true
                    },
                    {"source_id": "support", "row_identity_rules": ["person_by_id"]}
                ]
            }),
        );
        let rows = vec![SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([("id".into(), json!("1"))]),
        }];
        let materialized =
            materialize_with_source_states(&file, &rows, std::slice::from_ref(&state)).unwrap();
        verify_replay_report(&file, &materialized.conversion_report).unwrap();

        let mut stale = materialized.conversion_report.clone();
        stale["sources"][0]["snapshot_digest"] =
            json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let err = verify_replay_report(&file, &stale).unwrap_err();
        assert!(err.contains("MAP_REPLAY_SOURCE_STALE"));
    }

    #[test]
    fn aliases_import_updates_catalog_digests_and_runtime_lookup() {
        let file = company_resolution_map();
        let csv = br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:acme,Acme,Acme Ltd,curated,authoritative,{"source":"manual"}
uk-company:acme,Acme,ACME LIMITED,curated,authoritative,
"#;
        let options = alias_import::AliasImportOptions {
            catalog_id: "company_aliases".into(),
            resolver_id: "uk_company_name_resolver".into(),
        };
        let (updated, report) =
            alias_import::import_aliases_from_csv_bytes(&file, csv, &options).unwrap();
        assert_eq!(report["alias_entry_count"], json!(1));
        assert_eq!(report["alias_count"], json!(2));
        assert!(report["catalog_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(report["resolver_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        updated.validate_map_sections().unwrap();
        let serialized = updated.serialize().unwrap();
        CovemapFile::parse_validated(&serialized).unwrap();

        let rows = ["Acme Ltd", "ACME LIMITED"]
            .into_iter()
            .enumerate()
            .map(|(row_index, company_name)| SourceRow {
                source_id: "suppliers".into(),
                row_index,
                values: BTreeMap::from([("company_name".into(), json!(company_name))]),
            })
            .collect::<Vec<_>>();
        let planned = plan_identities(&updated, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);
        assert!(planned.canonical.iter().all(|identity| {
            identity.resolution_metadata[0].canonical_key.as_deref() == Some("uk-company:acme")
                && identity.resolution_metadata[0].alias_hit
        }));
    }

    #[test]
    fn redacted_resolver_evidence_omits_raw_alias_but_preserves_hit_proof() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() =
            redacted_resolution_catalog_section("company-map", "test/v1");
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        }];

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let entry = materialized
            .evidence_entries
            .iter()
            .find(|entry| entry["alias_hit"] == json!(true))
            .unwrap();
        assert_eq!(entry["evidence_policy"], json!("redact_raw"));
        assert_eq!(entry["redacted_resolution_evidence"], json!(true));
        assert_eq!(entry["redacted"], json!(true));
        assert_eq!(entry["redaction_scope"], json!("resolver_evidence"));
        assert!(entry.get("raw_observed_value").is_none());
        assert!(entry.get("normalized_value").is_none());
        assert_eq!(entry["canonical_key"], json!("uk-company:tesco"));
        assert!(entry["resolver_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(entry["catalog_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert!(entry["pipeline_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn alias_catalog_candidate_only_miss_emits_candidate_without_goid() {
        let file = company_resolution_map();
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Unknown Stores"))]),
        }];

        let planned = plan_identities(&file, &rows).unwrap();
        assert!(planned.canonical.is_empty());
        assert_eq!(planned.candidates.len(), 1);
        assert_eq!(
            planned.candidates[0].resolution_metadata[0].normalized_value,
            "unknown stores"
        );
        assert!(planned.candidates[0].resolution_metadata[0].alias_miss);

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert!(materialized.rows.is_empty());
        assert_eq!(
            materialized.conversion_report["resolver_hit_count"],
            json!(0)
        );
        assert_eq!(
            materialized.conversion_report["resolver_miss_count"],
            json!(1)
        );
        assert_eq!(
            materialized.conversion_report["candidate_match_count"],
            json!(1)
        );
        assert_eq!(materialized.evidence_entries.len(), 1);
        assert_eq!(materialized.evidence_entries[0]["candidate"], json!(true));
        assert_eq!(materialized.evidence_entries[0]["alias_miss"], json!(true));
        assert_eq!(
            materialized.evidence_entries[0]["miss_policy"],
            json!("candidate_only")
        );
    }

    #[test]
    fn redacted_alias_miss_error_omits_raw_alias_value() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() = company_resolution_catalog_section_with_policy(
            "company-map",
            "test/v1",
            "reject",
            None,
            "redact_raw",
        );
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Protected Unknown Stores"))]),
        }];

        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("MAP_ALIAS_MISS"));
        assert!(err.contains("<redacted>"));
        assert!(!err.contains("Protected Unknown Stores"));
        assert!(!err.contains("protected unknown stores"));
    }

    #[test]
    fn alias_catalog_ambiguous_hit_rejects_auto_merge_by_default() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() = ambiguous_company_resolution_catalog_section(
            "company-map",
            "test/v1",
            "reject_auto_merge",
        );
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
        }];

        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("MAP_ALIAS_AMBIGUOUS"));
    }

    #[test]
    fn redacted_ambiguous_alias_error_omits_normalized_alias_value() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() =
            ambiguous_company_resolution_catalog_section_with_policy(
                "company-map",
                "test/v1",
                "reject_auto_merge",
                "redact_raw",
            );
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
        }];

        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("MAP_ALIAS_AMBIGUOUS"));
        assert!(err.contains("<redacted>"));
        assert!(!err.contains("Tesco"));
        assert!(!err.contains("tesco"));
    }

    #[test]
    fn alias_catalog_ambiguous_hit_can_route_to_candidate_only() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() = ambiguous_company_resolution_catalog_section(
            "company-map",
            "test/v1",
            "candidate_only",
        );
        file.validate_map_sections().unwrap();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco"))]),
        }];

        let planned = plan_identities(&file, &rows).unwrap();
        assert!(planned.canonical.is_empty());
        assert_eq!(planned.candidates.len(), 1);
        assert!(planned.candidates[0].resolution_metadata[0].alias_ambiguous);

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert!(materialized.rows.is_empty());
        assert_eq!(
            materialized.conversion_report["ambiguous_alias_count"],
            json!(1)
        );
        assert_eq!(materialized.evidence_entries[0]["candidate"], json!(true));
        assert_eq!(
            materialized.evidence_entries[0]["alias_ambiguous"],
            json!(true)
        );
    }

    #[test]
    fn resolution_property_expressions_project_from_identity_evidence() {
        let mut file = company_resolution_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"},
                        {"name": "canonical_label", "value": "identity(company_by_resolved_name).resolution(company).canonical_label"},
                        {"name": "normalized_value", "value": "identity(company_by_resolved_name).resolution(company).normalized_value"},
                        {"name": "raw_observed_value", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        }];

        let projected = project_rows(&file, &rows).unwrap();
        let projected_row = &projected["rows"][0];
        assert_eq!(projected_row["canonical_key"], json!("uk-company:tesco"));
        assert_eq!(projected_row["canonical_label"], json!("Tesco"));
        assert_eq!(projected_row["normalized_value"], json!("tesco plc"));
        assert_eq!(projected_row["raw_observed_value"], json!("Tesco PLC"));

        let bytes = build_cove_o(&file, &rows).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "cove-map-resolution-projection-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let object_path = dir.join("company.cove");
        fs::write(&object_path, bytes).unwrap();
        let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
        assert_eq!(persisted_projected["rows"], projected["rows"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolution_property_expressions_select_declared_role() {
        let mut file = company_resolution_map();
        mutate_section_payload(&mut file, 2, |payload| {
            payload["identity_rules"][0]["join_keys"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "role_id": "parent_company",
                    "source_column": "parent_company_name",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "declared",
                    "resolution": {
                        "resolver_id": "uk_company_name_resolver"
                    }
                }));
        });
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "company_raw", "value": "identity(company_by_resolved_name).resolution(company).raw_observed_value"},
                        {"name": "parent_raw", "value": "identity(company_by_resolved_name).resolution(parent_company).raw_observed_value"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("company_name".into(), json!("Tesco")),
                ("parent_company_name".into(), json!("Tesco PLC")),
            ]),
        }];

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(
            materialized.evidence_entries[0]["resolution_metadata"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let projected = project_rows(&file, &rows).unwrap();
        let projected_row = &projected["rows"][0];
        assert_eq!(projected_row["company_raw"], json!("Tesco"));
        assert_eq!(projected_row["parent_raw"], json!("Tesco PLC"));
    }

    #[test]
    fn resolution_property_expressions_fail_closed_without_resolver_hit() {
        let mut file = company_resolution_map();
        *file.sections.last_mut().unwrap() =
            normalized_miss_resolution_catalog_section("company-map", "test/v1");
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "company-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "company_resolution_projection",
                    "output_table": "company_resolution",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Company"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "canonical_key", "value": "identity(company_by_resolved_name).resolution(company).canonical_key"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Unknown Stores"))]),
        }];

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(materialized.rows.len(), 1);
        assert_eq!(materialized.evidence_entries[0]["alias_miss"], json!(true));
        assert!(materialized.evidence_entries[0]["alias_hit"].is_null());

        let err = project_rows(&file, &rows).unwrap_err();
        assert!(err.contains("found no resolver hit"));
    }

    fn add_company_candidate_match_rule(file: &mut CovemapFile, max_pairs_per_block: u64) {
        mutate_section_payload(file, 4, |payload| {
            payload["match_rules"].as_array_mut().unwrap().push(json!({
                "match_rule_id": "company_name_similarity",
                "object_type": "Company",
                "inputs": [{
                    "source_id": "suppliers",
                    "column": "company_name"
                }],
                "blocking": {
                    "kind": "normalized_prefix",
                    "length": 4
                },
                "normalization_pipeline_id": "company_name.v1",
                "scoring": {
                    "kind": "token_jaccard",
                    "candidate_threshold": 0.3,
                    "merge_behavior": "never",
                    "score_scale": 1000000,
                    "rounding": "floor"
                },
                "limits": {
                    "max_pairs_per_block": max_pairs_per_block,
                    "max_pairs_total": 100,
                    "on_limit": "fail_closed"
                },
                "output": {
                    "assertion_kinds": ["candidate_match", "evidence"]
                }
            }));
        });
    }

    #[test]
    fn candidate_match_rule_emits_stable_token_jaccard_json() {
        let mut file = company_resolution_map();
        add_company_candidate_match_rule(&mut file, 10);
        file.validate_map_sections().unwrap();
        let rows = vec![
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 0,
                values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
            },
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 1,
                values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
            },
        ];

        let candidates = candidate_matches(&file, &rows).unwrap();
        let matches = candidates["candidate_matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0]["match_rule_id"],
            json!("company_name_similarity")
        );
        assert_eq!(matches[0]["candidate_score"], json!(333333));
        assert_eq!(matches[0]["score_scale"], json!(1000000));
        assert_eq!(matches[0]["blocking_key"], json!("tesc"));
        assert_eq!(matches[0]["merge_behavior"], json!("never"));
        assert_eq!(matches[0]["left"]["normalized_value"], json!("tesco plc"));
        assert_eq!(
            matches[0]["right"]["normalized_value"],
            json!("tesco supermarket")
        );

        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(materialized.rows.len(), 2);
        assert_eq!(
            materialized.conversion_report["candidate_match_count"],
            json!(1)
        );
        assert_eq!(
            materialized.conversion_report["candidate_matches"][0]["match_rule_id"],
            json!("company_name_similarity")
        );
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry["candidate_match_id"] == matches[0]["candidate_match_id"]
                && entry["match_rule_id"] == json!("company_name_similarity")
                && entry["candidate_score"] == json!(333333)
                && entry["left_normalized_value"] == json!("tesco plc")
                && entry["right_normalized_value"] == json!("tesco supermarket")
        }));
    }

    #[test]
    fn review_worklist_from_candidate_matches_emits_decision_templates() {
        let mut file = company_resolution_map();
        add_company_candidate_match_rule(&mut file, 10);
        let rows = vec![
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 0,
                values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
            },
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 1,
                values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
            },
        ];

        let candidates = candidate_matches(&file, &rows).unwrap();
        let review = review_worklist_from_candidate_matches(&candidates).unwrap();
        assert_eq!(
            review["schema_id"],
            json!("org.coveformat.covemap.review-worklist.v1")
        );
        assert_eq!(review["candidate_match_count"], json!(1));
        assert_eq!(
            review["review_items"][0]["same_object_decision_template"]["left"]["kind"],
            json!("row_digest")
        );
        assert_eq!(
            review["review_items"][0]["same_object_decision_template"]["left"]["source_id"],
            json!("suppliers")
        );
        assert_eq!(
            review["review_items"][0]["same_object_decision_template"]["left"]
                ["source_row_identity"],
            json!("suppliers:0")
        );
        assert_eq!(
            review["review_items"][0]["do_not_merge_decision_template"]["decision"],
            json!("do_not_merge")
        );
        assert_eq!(
            review["review_items"][0]["left"]["normalized_value"],
            json!("tesco plc")
        );
    }

    #[test]
    fn candidate_match_rule_limits_fail_closed() {
        let mut file = company_resolution_map();
        add_company_candidate_match_rule(&mut file, 0);
        let rows = vec![
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 0,
                values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
            },
            SourceRow {
                source_id: "suppliers".into(),
                row_index: 1,
                values: BTreeMap::from([("company_name".into(), json!("Tesco supermarket"))]),
            },
        ];

        let err = candidate_matches(&file, &rows).unwrap_err();
        assert!(err.contains("max_pairs_per_block"));
    }

    #[test]
    fn explain_includes_resolution_metadata_from_evidence_index() {
        let mut file = company_resolution_map();
        let rows = vec![SourceRow {
            source_id: "suppliers".into(),
            row_index: 0,
            values: BTreeMap::from([("company_name".into(), json!("Tesco PLC"))]),
        }];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        file.sections.push(test_section(
            SectionKind::MapEvidenceIndex,
            materialized.evidence_index.clone(),
        ));

        let goid = hex_encode(&materialized.rows[0].goid);
        let explained = explain(&file, &goid).unwrap();
        assert_eq!(
            explained["operation_metadata"]["identity_rule_id"],
            json!("company_by_resolved_name")
        );
        assert_eq!(
            explained["resolution"]["resolver_id"],
            json!("uk_company_name_resolver")
        );
        assert_eq!(
            explained["resolution"]["resolution_role_id"],
            json!("company")
        );
        assert_eq!(
            explained["resolution"]["raw_observed_value"],
            json!("Tesco PLC")
        );
        assert_eq!(
            explained["resolution"]["canonical_key"],
            json!("uk-company:tesco")
        );
    }

    #[test]
    fn candidate_identity_rules_emit_evidence_without_goids() {
        let mut file = two_source_identity_map(Vec::new());
        file.sections[2] = test_section(
            SectionKind::MapIdentityRuleCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "identity_rules": [{
                    "rule_id": "person_by_id",
                    "object_type": "Person",
                    "semantic_role": "subject",
                    "confidence_class": "candidate",
                    "candidate_only": true,
                    "property_conflicts_declared": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "person_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }],
                "do_not_merge": []
            }),
        );
        file.sections[3] = test_section(
            SectionKind::MapRowSemanticsCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "rules": [
                    {
                        "rule_id": "crm_candidate_person",
                        "source_id": "crm",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "EvidenceOnly",
                        "assertion_kinds": ["candidate_match", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    },
                    {
                        "rule_id": "support_candidate_person",
                        "source_id": "support",
                        "identity_rule_id": "person_by_id",
                        "row_semantics_kind": "EvidenceOnly",
                        "assertion_kinds": ["candidate_match", "evidence"],
                        "function_ids": ["identity"],
                        "output_assertion_ids": [],
                        "association_endpoints": []
                    }
                ]
            }),
        );
        let rows = vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
        ];
        let plan = plan_identities(&file, &rows).unwrap();
        assert!(plan.canonical.is_empty());
        assert_eq!(plan.candidates.len(), 2);
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert!(materialized.rows.is_empty());
        assert_eq!(
            materialized.conversion_report["candidate_match_count"],
            json!(2)
        );
        assert_eq!(
            materialized.identity_equivalence_index["equivalences"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert!(materialized
            .evidence_entries
            .iter()
            .all(|entry| entry["candidate"] == json!(true)));
    }

    #[test]
    fn do_not_merge_conflict_rejects_identity_resolution() {
        let file = two_source_identity_map(vec![json!({
            "left_identity": "crm:0",
            "right_identity": "support:0"
        })]);
        let rows = vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
        ];
        assert!(plan_identities(&file, &rows).is_err());
    }

    #[test]
    fn reviewed_same_object_merges_only_when_identity_rule_allows_it() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        add_reviewed_decisions(
            &mut file,
            vec![reviewed_same_object_decision(
                identity_alias_ref("Person", "crm:0"),
                identity_alias_ref("Person", "support:0"),
                None,
            )],
        );

        let planned = plan_identities(&file, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);

        let mut disallowed = two_source_identity_map(Vec::new());
        add_reviewed_decisions(
            &mut disallowed,
            vec![reviewed_same_object_decision(
                identity_alias_ref("Person", "crm:0"),
                identity_alias_ref("Person", "support:0"),
                None,
            )],
        );
        let err = plan_identities(&disallowed, &rows).unwrap_err();
        assert!(err.contains("does not allow reviewed equivalence"));
    }

    #[test]
    fn reviewed_row_digest_reference_rejects_ambiguous_matches() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "1")]);
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        let reference = json!({
            "kind": "row_digest",
            "object_type": "Person",
            "row_digest": row_digest(&rows[0])
        });
        add_reviewed_decisions(
            &mut file,
            vec![reviewed_same_object_decision(
                reference.clone(),
                reference,
                None,
            )],
        );

        let err = plan_identities(&file, &rows).unwrap_err();
        assert!(err.contains("row_digest reference matched"));
    }

    #[test]
    fn review_import_creates_resolution_catalog_and_merges_decisions() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        let decision = reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        );
        let review = serde_json::to_vec(&json!({
            "schema_id": "org.coveformat.covemap.review-worklist.v1",
            "mapping_id": "people-map",
            "mapping_version": "test/v1",
            "reviewed_decisions": [decision.clone()]
        }))
        .unwrap();

        let (updated, report) = review::import_reviewed_decisions_from_bytes(
            &file,
            &review,
            &review::ReviewImportOptions { replace: false },
        )
        .unwrap();
        assert_eq!(report["existing_reviewed_decision_count"], json!(0));
        assert_eq!(report["imported_reviewed_decision_count"], json!(1));
        assert_eq!(report["reviewed_decision_count"], json!(1));
        updated.validate_map_sections().unwrap();
        let serialized = updated.serialize().unwrap();
        CovemapFile::parse_validated(&serialized).unwrap();
        let exported = review::export_reviewed_decisions(&updated).unwrap();
        assert_eq!(
            exported["schema_id"],
            json!("org.coveformat.covemap.review-worklist.v1")
        );
        assert_eq!(exported["mapping_id"], json!("people-map"));
        assert_eq!(exported["mapping_version"], json!("test/v1"));
        assert_eq!(exported["reviewed_decision_count"], json!(1));
        assert_eq!(exported["reviewed_decisions"], json!([decision]));

        let planned = plan_identities(&updated, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);
    }

    #[test]
    fn reviewed_decision_catalog_digest_binds_conversion_report() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let decision = reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        );

        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        add_reviewed_decisions(&mut file, vec![decision.clone()]);
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(
            materialized.conversion_report["reviewed_decision_count"],
            json!(1)
        );
        let original_digest = materialized.conversion_report["reviewed_decision_catalog_digest"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(original_digest.starts_with("sha256:"));

        let mut changed_decision = decision;
        changed_decision["reason"] = json!("manual adjudication update");
        let mut changed = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut changed, true);
        add_reviewed_decisions(&mut changed, vec![changed_decision]);
        let changed_materialized = materialize_with_source_states(&changed, &rows, &[]).unwrap();
        assert_eq!(
            changed_materialized.conversion_report["reviewed_decision_count"],
            json!(1)
        );
        assert_ne!(
            original_digest,
            changed_materialized.conversion_report["reviewed_decision_catalog_digest"]
                .as_str()
                .unwrap()
        );
    }

    #[test]
    fn semantic_delta_reviewed_decision_changes_fingerprint_and_matches_rebuild() {
        let base = empty_cove_o_parent_bytes();
        let parent = delta_parent_from_base_bytes(&base);
        let sources = [("crm.csv", "id\n1\n"), ("support.csv", "id\n2\n")];
        let plain = two_source_identity_map(Vec::new());
        let mut reviewed = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut reviewed, true);
        add_reviewed_decisions(
            &mut reviewed,
            vec![reviewed_same_object_decision(
                identity_alias_ref("Person", "crm:0"),
                identity_alias_ref("Person", "support:0"),
                None,
            )],
        );

        let (plain_report, plain_delta, plain_dir) = build_semantic_delta_fixture(
            "semantic-delta-reviewed-plain",
            &plain,
            &sources,
            parent.clone(),
        );
        let (reviewed_report, reviewed_delta, reviewed_dir) = build_semantic_delta_fixture(
            "semantic-delta-reviewed-merged",
            &reviewed,
            &sources,
            parent,
        );

        assert_ne!(
            plain_report["fingerprints"]["semantic_map_sha256"],
            reviewed_report["fingerprints"]["semantic_map_sha256"]
        );
        assert_eq!(
            reviewed_report["object_delta_validation"]["evidence_patches"],
            json!(1)
        );
        assert!(
            reviewed_report["object_delta_validation"]["touched_object_ranges"]
                .as_u64()
                .unwrap_or_default()
                > 0,
            "report={reviewed_report}"
        );
        assert_ne!(
            composed_delta_state_keys(&base, &plain_delta),
            composed_delta_state_keys(&base, &reviewed_delta)
        );
        assert_eq!(
            composed_delta_state_keys(&base, &reviewed_delta),
            full_rebuild_state_keys("semantic-delta-reviewed-full", &reviewed, &sources)
        );

        fs::remove_dir_all(plain_dir).unwrap();
        fs::remove_dir_all(reviewed_dir).unwrap();
    }

    #[test]
    fn semantic_delta_existing_parent_reviewed_identity_remap_matches_rebuild() {
        let sources = [("crm.csv", "id\n1\n"), ("support.csv", "id\n2\n")];
        let plain = two_source_identity_map(Vec::new());
        let (_plain_map, plain_source_paths, plain_dir) =
            write_map_and_sources("semantic-delta-reviewed-parent", &plain, &sources);
        let inputs = read_source_inputs(&plain_source_paths).unwrap();
        validate_source_inputs(&plain, &inputs.states).unwrap();
        let parent_bytes =
            build_cove_o_with_source_states(&plain, &inputs.rows, &inputs.states).unwrap();
        let parent_surface = read_object_surface_from_bytes(&parent_bytes).unwrap();
        let parent_states =
            reconstruct_object_states(&parent_surface, &Default::default()).unwrap();
        assert_eq!(state_keys(&parent_states).len(), 2);

        let mut reviewed = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut reviewed, true);
        add_reviewed_decisions(
            &mut reviewed,
            vec![reviewed_same_object_decision(
                identity_alias_ref("Person", "crm:0"),
                identity_alias_ref("Person", "support:0"),
                None,
            )],
        );
        validate_source_inputs(&reviewed, &inputs.states).unwrap();
        let rebuilt_bytes =
            build_cove_o_with_source_states(&reviewed, &inputs.rows, &inputs.states).unwrap();
        let rebuilt_surface = read_object_surface_from_bytes(&rebuilt_bytes).unwrap();
        let rebuilt_states =
            reconstruct_object_states(&rebuilt_surface, &Default::default()).unwrap();
        assert_eq!(state_keys(&rebuilt_states).len(), 1);

        let (reviewed_map, reviewed_source_paths, reviewed_dir) =
            write_map_and_sources("semantic-delta-reviewed-remap", &reviewed, &sources);
        let out = reviewed_dir.join("semantic.covedelta");
        let result = build_semantic_delta_from_paths(
            &reviewed_map,
            &reviewed_source_paths,
            semantic_delta_options_from_parent_bytes(out, &parent_bytes),
        )
        .unwrap();
        assert!(
            result.report["object_delta_validation"]["tombstone_object_ranges"]
                .as_u64()
                .unwrap_or_default()
                > 0,
            "report={}",
            result.report
        );
        let delta_bytes = fs::read(reviewed_dir.join("semantic.covedelta")).unwrap();
        assert_eq!(
            composed_delta_state_keys(&parent_bytes, &delta_bytes),
            state_keys(&rebuilt_states)
        );
        assert_eq!(
            composed_delta_evidence_keys(&parent_bytes, &delta_bytes),
            evidence_keys_from_cove_o_bytes(&rebuilt_bytes)
        );

        fs::remove_dir_all(plain_dir).unwrap();
        fs::remove_dir_all(reviewed_dir).unwrap();
    }

    #[test]
    fn semantic_delta_alias_catalog_change_updates_fingerprint_and_matches_rebuild() {
        let base = empty_cove_o_parent_bytes();
        let parent = delta_parent_from_base_bytes(&base);
        let base_map = company_resolution_map();
        let alias_csv =
            br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:tesco,Tesco,Tesco PLC,curated,authoritative,{}
uk-company:tesco,Tesco,Tesco Holdings,curated,authoritative,{}
"#;
        let (updated_map, alias_report) = alias_import::import_aliases_from_csv_bytes(
            &base_map,
            alias_csv,
            &alias_import::AliasImportOptions {
                catalog_id: "company_aliases".into(),
                resolver_id: "uk_company_name_resolver".into(),
            },
        )
        .unwrap();
        assert_eq!(alias_report["alias_count"], json!(2));
        let sources = [("suppliers.csv", "company_name\nTesco PLC\n")];

        let (base_report, base_delta, base_dir) = build_semantic_delta_fixture(
            "semantic-delta-alias-base",
            &base_map,
            &sources,
            parent.clone(),
        );
        let (updated_report, updated_delta, updated_dir) = build_semantic_delta_fixture(
            "semantic-delta-alias-updated",
            &updated_map,
            &sources,
            parent,
        );

        assert_eq!(updated_report["counts"]["evidence_entries"], json!(1));
        assert!(
            updated_report["object_delta_validation"]["touched_object_ranges"]
                .as_u64()
                .unwrap_or_default()
                > 0,
            "report={updated_report}"
        );
        assert_ne!(
            base_report["fingerprints"]["semantic_map_sha256"],
            updated_report["fingerprints"]["semantic_map_sha256"]
        );
        assert_eq!(
            composed_delta_state_keys(&base, &base_delta),
            composed_delta_state_keys(&base, &updated_delta)
        );
        assert_eq!(
            composed_delta_state_keys(&base, &updated_delta),
            full_rebuild_state_keys("semantic-delta-alias-full", &updated_map, &sources)
        );

        fs::remove_dir_all(base_dir).unwrap();
        fs::remove_dir_all(updated_dir).unwrap();
    }

    #[test]
    fn semantic_delta_emits_inline_dictionary_overlay_for_filecode_properties() {
        let base = empty_cove_o_parent_bytes();
        let parent = delta_parent_from_base_bytes(&base);
        let mut map_file = two_source_property_map("reject_conflict", None, None);
        mutate_section_payload(&mut map_file, 3, |payload| {
            for rule in payload["rules"].as_array_mut().unwrap() {
                rule["property_bindings"][0]["physical_kind"] = json!("filecode");
            }
        });
        let sources = [
            ("crm.csv", "id,name\n1,Ada\n"),
            ("support.csv", "id,name\n2,Grace\n"),
        ];
        let (map, source_paths, dir) =
            write_map_and_sources("semantic-delta-filecode-overlay", &map_file, &sources);
        let out = dir.join("semantic.covedelta");
        let result = build_semantic_delta_from_paths(
            &map,
            &source_paths,
            semantic_delta_options(out, parent),
        )
        .unwrap();
        let delta_bytes = fs::read(dir.join("semantic.covedelta")).unwrap();
        let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        let validation = delta.validate_object_delta().unwrap();
        assert_eq!(validation.dictionary_overlay_entries.len(), 2);
        assert_eq!(validation.inline_values.len(), 2);
        assert!(
            delta.header.required_delta_features & DELTA_FEATURE_INLINE_DICTIONARY != 0,
            "report={}",
            result.report
        );
        let states = reconstruct_object_states_from_base_and_delta_files(
            &base,
            &[delta],
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        let names = states
            .iter()
            .flat_map(|state| state.properties.iter())
            .filter(|property| property.property_name == "name")
            .map(|property| property.value.clone())
            .collect::<Vec<_>>();
        assert!(names.contains(&json!("Ada")));
        assert!(names.contains(&json!("Grace")));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn semantic_delta_emits_sparse_property_ops_for_null_only_delta_rows() {
        let map_file = two_source_property_map("reject_conflict", None, None);
        let parent_sources = [
            ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada\"}\n"),
            ("support.jsonl", "{\"id\":\"2\",\"name\":\"Grace\"}\n"),
        ];
        let (_parent_map, parent_source_paths, parent_dir) =
            write_map_and_sources("semantic-delta-sparse-parent", &map_file, &parent_sources);
        let parent_inputs = read_source_inputs(&parent_source_paths).unwrap();
        validate_source_inputs(&map_file, &parent_inputs.states).unwrap();
        let parent_bytes =
            build_cove_o_with_source_states(&map_file, &parent_inputs.rows, &parent_inputs.states)
                .unwrap();

        let delta_sources = [
            ("crm.jsonl", "{\"id\":\"1\",\"name\":null}\n"),
            ("support.jsonl", "{\"id\":\"3\",\"name\":\"Hedy\"}\n"),
        ];
        let (map, source_paths, delta_dir) =
            write_map_and_sources("semantic-delta-sparse-null", &map_file, &delta_sources);
        let out = delta_dir.join("semantic.covedelta");
        let result = build_semantic_delta_from_paths(
            &map,
            &source_paths,
            semantic_delta_options_from_parent_bytes(out.clone(), &parent_bytes),
        )
        .unwrap();
        assert_eq!(
            result.report["object_delta_validation"]["sparse_patch_rows"],
            json!(1),
            "report={}",
            result.report
        );

        let delta_bytes = fs::read(out).unwrap();
        let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        let validation = delta.validate_object_delta().unwrap();
        assert_eq!(validation.sparse_patch_records.len(), 1);
        assert_ne!(
            delta.header.required_delta_features & DELTA_FEATURE_SPARSE_PATCH_ROWS,
            0
        );
        let states = reconstruct_object_states_from_base_and_delta_files(
            &parent_bytes,
            &[delta],
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert!(states.iter().any(|state| {
            state
                .properties
                .iter()
                .any(|property| property.property_name == "name" && property.value == Value::Null)
        }));

        fs::remove_dir_all(parent_dir).unwrap();
        fs::remove_dir_all(delta_dir).unwrap();
    }

    #[test]
    fn semantic_delta_emits_sparse_set_value_property_ops() {
        let map_file = two_source_property_map("reject_conflict", None, None);
        let parent_sources = [
            ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada\"}\n"),
            ("support.jsonl", "{\"id\":\"2\",\"name\":\"Grace\"}\n"),
        ];
        let (_parent_map, parent_source_paths, parent_dir) = write_map_and_sources(
            "semantic-delta-sparse-set-parent",
            &map_file,
            &parent_sources,
        );
        let parent_inputs = read_source_inputs(&parent_source_paths).unwrap();
        validate_source_inputs(&map_file, &parent_inputs.states).unwrap();
        let parent_bytes =
            build_cove_o_with_source_states(&map_file, &parent_inputs.rows, &parent_inputs.states)
                .unwrap();

        let delta_sources = [
            ("crm.jsonl", "{\"id\":\"1\",\"name\":\"Ada Lovelace\"}\n"),
            ("support.jsonl", "{\"id\":\"3\",\"name\":\"Hedy\"}\n"),
        ];
        let (map, source_paths, delta_dir) =
            write_map_and_sources("semantic-delta-sparse-set", &map_file, &delta_sources);
        let out = delta_dir.join("semantic.covedelta");
        let _ = build_semantic_delta_from_paths(
            &map,
            &source_paths,
            semantic_delta_options_from_parent_bytes(out.clone(), &parent_bytes),
        )
        .unwrap();

        let delta_bytes = fs::read(out).unwrap();
        let delta = CoveDeltaFile::parse(&delta_bytes).unwrap();
        let validation = delta.validate_object_delta().unwrap();
        assert_eq!(validation.sparse_patch_records.len(), 1);
        assert_eq!(
            validation.sparse_patch_records[0].changed_properties[0].property_op,
            DELTA_PROPERTY_OP_SET_VALUE
        );
        assert!(!validation.inline_values.is_empty());
        let states = reconstruct_object_states_from_base_and_delta_files(
            &parent_bytes,
            &[delta],
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert!(states.iter().any(|state| {
            state.properties.iter().any(|property| {
                property.property_name == "name" && property.value == json!("Ada Lovelace")
            })
        }));

        fs::remove_dir_all(parent_dir).unwrap();
        fs::remove_dir_all(delta_dir).unwrap();
    }

    #[test]
    fn semantic_delta_existing_parent_alias_identity_remap_matches_rebuild() {
        let sources = [(
            "suppliers.csv",
            "company_name\nAcme Trading\nAcme Holdings\n",
        )];
        let mut base_map = company_resolution_map();
        base_map.sections[4] = normalized_miss_resolution_catalog_section("company-map", "test/v1");
        let (_base_map_path, base_source_paths, base_dir) =
            write_map_and_sources("semantic-delta-alias-parent", &base_map, &sources);
        let inputs = read_source_inputs(&base_source_paths).unwrap();
        validate_source_inputs(&base_map, &inputs.states).unwrap();
        let parent_bytes =
            build_cove_o_with_source_states(&base_map, &inputs.rows, &inputs.states).unwrap();
        let parent_surface = read_object_surface_from_bytes(&parent_bytes).unwrap();
        let parent_states =
            reconstruct_object_states(&parent_surface, &Default::default()).unwrap();
        assert_eq!(state_keys(&parent_states).len(), 2);

        let alias_csv =
            br#"canonical_key,canonical_label,alias,authority,confidence_class,metadata_json
uk-company:acme,Acme,Acme Trading,curated,authoritative,{}
uk-company:acme,Acme,Acme Holdings,curated,authoritative,{}
"#;
        let (updated_map, alias_report) = alias_import::import_aliases_from_csv_bytes(
            &base_map,
            alias_csv,
            &alias_import::AliasImportOptions {
                catalog_id: "company_aliases".into(),
                resolver_id: "uk_company_name_resolver".into(),
            },
        )
        .unwrap();
        assert_eq!(alias_report["alias_count"], json!(2));
        validate_source_inputs(&updated_map, &inputs.states).unwrap();
        let rebuilt_bytes =
            build_cove_o_with_source_states(&updated_map, &inputs.rows, &inputs.states).unwrap();
        let rebuilt_surface = read_object_surface_from_bytes(&rebuilt_bytes).unwrap();
        let rebuilt_states =
            reconstruct_object_states(&rebuilt_surface, &Default::default()).unwrap();
        assert_eq!(state_keys(&rebuilt_states).len(), 1);

        let (updated_map_path, updated_source_paths, updated_dir) =
            write_map_and_sources("semantic-delta-alias-remap", &updated_map, &sources);
        let out = updated_dir.join("semantic.covedelta");
        let result = build_semantic_delta_from_paths(
            &updated_map_path,
            &updated_source_paths,
            semantic_delta_options_from_parent_bytes(out, &parent_bytes),
        )
        .unwrap();
        assert!(
            result.report["object_delta_validation"]["tombstone_object_ranges"]
                .as_u64()
                .unwrap_or_default()
                > 0,
            "report={}",
            result.report
        );
        let delta_bytes = fs::read(updated_dir.join("semantic.covedelta")).unwrap();
        assert_eq!(
            composed_delta_state_keys(&parent_bytes, &delta_bytes),
            state_keys(&rebuilt_states)
        );
        assert_eq!(
            composed_delta_evidence_keys(&parent_bytes, &delta_bytes),
            evidence_keys_from_cove_o_bytes(&rebuilt_bytes)
        );

        fs::remove_dir_all(base_dir).unwrap();
        fs::remove_dir_all(updated_dir).unwrap();
    }

    #[test]
    fn replay_verify_rejects_stale_reviewed_decision_digest() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let decision = reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        );
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        add_reviewed_decisions(&mut file, vec![decision.clone()]);
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        verify_replay_report(&file, &materialized.conversion_report).unwrap();

        let mut changed_decision = decision;
        changed_decision["reason"] = json!("post-run adjudication changed");
        let mut changed = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut changed, true);
        add_reviewed_decisions(&mut changed, vec![changed_decision]);
        let err = verify_replay_report(&changed, &materialized.conversion_report).unwrap_err();
        assert!(err.contains("MAP_REPLAY_STALE_REVIEW"));
    }

    #[test]
    fn reviewed_do_not_merge_rejects_conflicting_reviewed_merge() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        let left = identity_alias_ref("Person", "crm:0");
        let right = identity_alias_ref("Person", "support:0");
        add_reviewed_decisions(
            &mut file,
            vec![
                reviewed_same_object_decision(left.clone(), right.clone(), None),
                reviewed_do_not_merge_decision(left, right),
            ],
        );

        let err = plan_identities(&file, &rows).unwrap_err();
        assert!(err.contains("reviewed do-not-merge"));
    }

    #[test]
    fn reviewed_same_object_transitive_closure_is_deterministic() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2"), ("ops", "3")]);
        let mut file = three_source_identity_map();
        set_person_reviewed_equivalence(&mut file, true);
        let mut crm_support = reviewed_same_object_decision(
            identity_alias_ref("Person", "crm:0"),
            identity_alias_ref("Person", "support:0"),
            None,
        );
        crm_support["decision_id"] = json!("review:crm-support");
        let mut support_ops = reviewed_same_object_decision(
            identity_alias_ref("Person", "support:0"),
            identity_alias_ref("Person", "ops:0"),
            None,
        );
        support_ops["decision_id"] = json!("review:support-ops");
        add_reviewed_decisions(&mut file, vec![crm_support, support_ops]);

        let first = plan_identities(&file, &rows).unwrap();
        let second = plan_identities(&file, &rows).unwrap();
        let first_goids = first
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        let second_goids = second
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(first_goids.len(), 1);
        assert_eq!(first_goids, second_goids);
    }

    #[test]
    fn reviewed_source_row_references_bind_snapshot_and_schema() {
        let rows = reviewed_rows(&[("crm", "1"), ("support", "2")]);
        let crm_snapshot =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let support_snapshot =
            "sha256:2222222222222222222222222222222222222222222222222222222222222222";
        let mut file = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut file, true);
        mutate_section_payload(&mut file, 0, |payload| {
            payload["sources"][0]["snapshot_digest"] = json!(crm_snapshot);
            payload["sources"][1]["snapshot_digest"] = json!(support_snapshot);
        });
        let left = json!({
            "kind": "source_row",
            "object_type": "Person",
            "identity_rule_id": "person_by_id",
            "source_id": "crm",
            "source_row_identity": "crm:0",
            "source_snapshot_digest": crm_snapshot,
            "schema_fingerprint": schema_fingerprint(&rows[0])
        });
        let right = json!({
            "kind": "source_row",
            "object_type": "Person",
            "identity_rule_id": "person_by_id",
            "source_id": "support",
            "source_row_identity": "support:0",
            "source_snapshot_digest": support_snapshot,
            "schema_fingerprint": schema_fingerprint(&rows[1])
        });
        add_reviewed_decisions(
            &mut file,
            vec![reviewed_same_object_decision(
                left.clone(),
                right.clone(),
                None,
            )],
        );
        let planned = plan_identities(&file, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);

        let mut wrong_digest = two_source_identity_map(Vec::new());
        set_person_reviewed_equivalence(&mut wrong_digest, true);
        mutate_section_payload(&mut wrong_digest, 0, |payload| {
            payload["sources"][0]["snapshot_digest"] = json!(crm_snapshot);
            payload["sources"][1]["snapshot_digest"] = json!(support_snapshot);
        });
        let mut wrong_right = right;
        wrong_right["source_snapshot_digest"] =
            json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        add_reviewed_decisions(
            &mut wrong_digest,
            vec![reviewed_same_object_decision(left, wrong_right, None)],
        );
        let err = plan_identities(&wrong_digest, &rows).unwrap_err();
        assert!(err.contains("did not match"));
    }

    #[test]
    fn cross_rule_reviewed_same_object_requires_and_uses_canonical_anchor() {
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("1")),
                ("email".into(), json!("ada@example.test")),
            ]),
        }];
        let base = cross_rule_reviewed_identity_map();
        let planned = plan_identities(&base, &rows).unwrap();
        assert_eq!(planned.canonical.len(), 2);
        let person_by_id = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_id")
            .unwrap();
        let person_by_email = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_email")
            .unwrap();
        let left = json!({
            "kind": "identity_join_key",
            "object_type": "Person",
            "identity_rule_id": "person_by_id",
            "join_key_sha256": person_by_id.join_key_sha256
        });
        let right = json!({
            "kind": "identity_join_key",
            "object_type": "Person",
            "identity_rule_id": "person_by_email",
            "join_key_sha256": person_by_email.join_key_sha256
        });

        let mut missing_anchor = base.clone();
        add_reviewed_decisions(
            &mut missing_anchor,
            vec![reviewed_same_object_decision(
                left.clone(),
                right.clone(),
                None,
            )],
        );
        let err = plan_identities(&missing_anchor, &rows).unwrap_err();
        assert!(err.contains("requires canonical_anchor"));

        let mut wrong_shape = base.clone();
        add_reviewed_decisions(
            &mut wrong_shape,
            vec![reviewed_same_object_decision(
                left.clone(),
                right.clone(),
                Some(json!({
                    "kind": "resolved_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_id",
                    "components": [{
                        "role_id": "email",
                        "logical_type": "utf8",
                        "resolved_value": "ada@example.test"
                    }]
                })),
            )],
        );
        let err = plan_identities(&wrong_shape, &rows).unwrap_err();
        assert!(err.contains("join key shape"));

        let mut anchored = base;
        add_reviewed_decisions(
            &mut anchored,
            vec![reviewed_same_object_decision(
                left,
                right,
                Some(json!({
                    "kind": "resolved_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_id",
                    "components": [{
                        "role_id": "person_id",
                        "logical_type": "utf8",
                        "resolved_value": "1"
                    }]
                })),
            )],
        );
        let planned = plan_identities(&anchored, &rows).unwrap();
        let goids = planned
            .canonical
            .iter()
            .map(|identity| identity.goid)
            .collect::<BTreeSet<_>>();
        assert_eq!(goids.len(), 1);
        assert!(planned
            .canonical
            .iter()
            .all(|identity| identity.canonical_anchor.starts_with("person_by_id:")));
    }

    #[test]
    fn reviewed_same_object_rejects_cross_object_type_components() {
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("1")),
                ("email".into(), json!("ada@example.test")),
            ]),
        }];
        let mut base = cross_rule_reviewed_identity_map();
        mutate_section_payload(&mut base, 2, |payload| {
            payload["identity_rules"][1]["object_type"] = json!("Company");
        });
        let planned = plan_identities(&base, &rows).unwrap();
        let person_by_id = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_id")
            .unwrap();
        let company_by_email = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_email")
            .unwrap();
        assert_eq!(person_by_id.object_type, "Person");
        assert_eq!(company_by_email.object_type, "Company");

        let mut reviewed = base;
        add_reviewed_decisions(
            &mut reviewed,
            vec![reviewed_same_object_decision(
                json!({
                    "kind": "identity_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_id",
                    "join_key_sha256": person_by_id.join_key_sha256
                }),
                json!({
                    "kind": "identity_join_key",
                    "object_type": "Company",
                    "identity_rule_id": "person_by_email",
                    "join_key_sha256": company_by_email.join_key_sha256
                }),
                Some(json!({
                    "kind": "resolved_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_id",
                    "components": [{
                        "role_id": "person_id",
                        "logical_type": "utf8",
                        "resolved_value": "1"
                    }]
                })),
            )],
        );

        let err = plan_identities(&reviewed, &rows).unwrap_err();
        assert!(err.contains("crosses object types"));
    }

    #[test]
    fn reviewed_same_object_rejects_canonical_anchor_object_type_mismatch() {
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("1")),
                ("email".into(), json!("ada@example.test")),
            ]),
        }];
        let mut base = cross_rule_reviewed_identity_map();
        mutate_section_payload(&mut base, 2, |payload| {
            payload["identity_rules"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "rule_id": "company_by_id",
                    "object_type": "Company",
                    "semantic_role": "subject",
                    "confidence_class": "authoritative",
                    "candidate_only": false,
                    "property_conflicts_declared": true,
                    "allow_reviewed_equivalence": true,
                    "function_ids": ["identity"],
                    "join_keys": [{
                        "role_id": "company_id",
                        "source_column": "id",
                        "logical_type": "utf8",
                        "canonicalization": "identity",
                        "null_policy": "reject",
                        "ordering": "declared"
                    }]
                }));
        });
        let planned = plan_identities(&base, &rows).unwrap();
        let person_by_id = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_id")
            .unwrap();
        let person_by_email = planned
            .canonical
            .iter()
            .find(|identity| identity.identity_rule_id == "person_by_email")
            .unwrap();

        let mut reviewed = base;
        add_reviewed_decisions(
            &mut reviewed,
            vec![reviewed_same_object_decision(
                json!({
                    "kind": "identity_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_id",
                    "join_key_sha256": person_by_id.join_key_sha256
                }),
                json!({
                    "kind": "identity_join_key",
                    "object_type": "Person",
                    "identity_rule_id": "person_by_email",
                    "join_key_sha256": person_by_email.join_key_sha256
                }),
                Some(json!({
                    "kind": "resolved_join_key",
                    "object_type": "Company",
                    "identity_rule_id": "company_by_id",
                    "components": [{
                        "role_id": "company_id",
                        "logical_type": "utf8",
                        "resolved_value": "1"
                    }]
                })),
            )],
        );

        let err = plan_identities(&reviewed, &rows).unwrap_err();
        assert!(err.contains("canonical anchor object type"));
    }

    #[test]
    fn property_conflict_rejects_unequal_cross_source_values() {
        let file = two_source_property_map("reject_conflict", None, None);
        let rows = conflict_rows(json!("Ada"), json!("Ada Lovelace"));
        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("unresolved property conflict"));
    }

    #[test]
    fn property_conflict_accepts_equal_duplicate_values() {
        let file = two_source_property_map("reject_conflict", None, None);
        let rows = conflict_rows(json!("Ada"), json!("Ada"));
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let name_values = materialized
            .rows
            .iter()
            .flat_map(|row| row.properties.values())
            .filter(|property| property.entry.property_name == "name")
            .map(|property| property.value.clone())
            .collect::<Vec<_>>();
        assert_eq!(name_values, vec![json!("Ada"), json!("Ada")]);
    }

    #[test]
    fn null_property_candidate_does_not_overwrite_non_null_value() {
        let file = two_source_property_map("reject_conflict", None, None);
        let rows = conflict_rows(Value::Null, json!("Ada"));
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let name_values = materialized
            .rows
            .iter()
            .flat_map(|row| row.properties.values())
            .filter(|property| property.entry.property_name == "name")
            .map(|property| property.value.clone())
            .collect::<Vec<_>>();
        assert_eq!(name_values, vec![json!("Ada")]);
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry.get("suppressed_reason").and_then(Value::as_str)
                == Some("null_does_not_overwrite_non_null")
        }));
    }

    #[test]
    fn source_priority_wins_suppresses_losing_property_values() {
        let file = two_source_property_map("source_priority_wins", Some(10), Some(1));
        let rows = conflict_rows(json!("CRM"), json!("Support"));
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let name_values = materialized
            .rows
            .iter()
            .flat_map(|row| row.properties.values())
            .filter(|property| property.entry.property_name == "name")
            .map(|property| property.value.clone())
            .collect::<Vec<_>>();
        assert_eq!(name_values, vec![json!("Support")]);
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry.get("suppressed_reason").and_then(Value::as_str) == Some("source_priority_wins")
                && entry.get("suppressed_value") == Some(&json!("CRM"))
        }));
    }

    #[test]
    fn patch_operation_sets_delta_metadata_and_round_trips_evidence() {
        let mut file = two_source_property_map("reject_conflict", None, None);
        mutate_section_payload(&mut file, 3, |payload| {
            let rule = payload["rules"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap();
            rule.insert("source_operation_kind".into(), json!("PatchProperty"));
        });
        let rows = vec![SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("1")),
                ("name".into(), json!("Ada")),
                ("correction_of".into(), json!("crm:previous")),
                ("replacement_of".into(), json!("goid:previous")),
            ]),
        }];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert_eq!(materialized.rows[0].record_kind, RecordKind::Delta);
        let evidence = materialized
            .evidence_entries
            .iter()
            .find(|entry| entry["rule_id"] == json!("crm_person"))
            .unwrap();
        assert_eq!(evidence["source_operation_kind"], json!("PatchProperty"));
        assert_eq!(evidence["operation_effect"], json!("patch_property"));
        assert_eq!(evidence["operation_target"], json!("property"));
        assert_eq!(evidence["correction_of"], json!("crm:previous"));
        assert_eq!(evidence["replacement_of"], json!("goid:previous"));
        assert_eq!(
            materialized.conversion_report["operation_counts"]["PatchProperty"],
            json!(1)
        );

        let bytes = build_cove_o(&file, &rows).unwrap();
        let surface = read_object_surface_from_bytes(&bytes).unwrap();
        let persisted = surface
            .evidence_index
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.rule_id == "crm_person")
            .unwrap();
        assert_eq!(
            persisted.operation_metadata["source_operation_kind"],
            json!("PatchProperty")
        );
        assert_eq!(
            persisted.operation_metadata["correction_of"],
            json!("crm:previous")
        );
    }

    #[test]
    fn close_association_operation_marks_association_delta_and_policy_metadata() {
        let mut file = association_readback_map();
        mutate_section_payload(&mut file, 3, |payload| {
            let rule = payload["rules"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap();
            rule.insert("source_operation_kind".into(), json!("CloseAssociation"));
        });
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
                ("closes_association".into(), json!("member_of:p1:t1")),
            ]),
        }];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let association = materialized
            .rows
            .iter()
            .find(|row| row.object_type == "Association:member_of")
            .unwrap();
        assert_eq!(association.record_kind, RecordKind::Delta);
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry["source_operation_kind"] == json!("CloseAssociation")
                && entry["operation_effect"] == json!("close_association")
                && entry["operation_target"] == json!("association")
                && entry["closes_association"] == json!("member_of:p1:t1")
        }));
    }

    #[test]
    fn evidence_only_operation_emits_evidence_without_object_rows() {
        let mut file = two_source_identity_map(Vec::new());
        mutate_section_payload(&mut file, 3, |payload| {
            let rule = payload["rules"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap();
            rule.insert("row_semantics_kind".into(), json!("EvidenceOnly"));
            rule.insert("source_operation_kind".into(), json!("RedactEvidence"));
            rule.insert("assertion_kinds".into(), json!(["evidence"]));
        });
        let rows = vec![SourceRow {
            source_id: "crm".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("id".into(), json!("1")),
                ("redaction_scope".into(), json!("source_evidence")),
            ]),
        }];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        assert!(materialized.rows.is_empty());
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry["source_operation_kind"] == json!("RedactEvidence")
                && entry["operation_effect"] == json!("redact_evidence")
                && entry["operation_target"] == json!("evidence")
                && entry["redaction_scope"] == json!("source_evidence")
        }));
    }

    #[test]
    fn association_readback_preserves_roles_validity_and_cardinality() {
        let file = association_readback_map();
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let association = materialized
            .rows
            .iter()
            .find(|row| row.object_type == "Association:member_of")
            .unwrap();
        assert_eq!(
            property_by_name(association, "source_role"),
            json!("member")
        );
        assert_eq!(property_by_name(association, "target_role"), json!("team"));
        assert_eq!(
            property_by_name(association, "valid_from"),
            json!("2026-01-01")
        );
        assert_eq!(
            property_by_name(association, "valid_to"),
            json!("2026-12-31")
        );
        assert_eq!(
            property_by_name(association, "cardinality_policy"),
            json!("many_to_one")
        );
    }

    #[test]
    fn association_endpoint_resolution_uses_alias_backed_target_identity() {
        let file = alias_backed_association_map();
        let rows = vec![
            SourceRow {
                source_id: "memberships".into(),
                row_index: 0,
                values: BTreeMap::from([
                    ("person_id".into(), json!("p1")),
                    ("team_name".into(), json!("Alpha Team Ltd")),
                    ("valid_from".into(), json!("2026-01-01")),
                    ("valid_to".into(), json!("2026-12-31")),
                ]),
            },
            SourceRow {
                source_id: "teams".into(),
                row_index: 0,
                values: BTreeMap::from([("team_name".into(), json!("Team Alpha"))]),
            },
        ];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let team = materialized
            .rows
            .iter()
            .find(|row| row.object_type == "Team")
            .unwrap();
        let association = materialized
            .rows
            .iter()
            .find(|row| row.object_type == "Association:member_of")
            .unwrap();

        assert_eq!(
            property_by_name(association, "target_goid"),
            json!(hex_encode(&team.goid))
        );
        assert!(materialized.evidence_entries.iter().any(|entry| {
            entry["source_id"] == json!("teams")
                && entry["rule_id"] == json!("team_row")
                && entry["alias_hit"] == json!(true)
                && entry["canonical_key"] == json!("team:alpha")
        }));
    }

    #[test]
    fn association_endpoint_rejects_source_scoped_join_key_ambiguity() {
        let file = source_scoped_ambiguous_association_map();
        let rows = vec![
            SourceRow {
                source_id: "memberships".into(),
                row_index: 0,
                values: BTreeMap::from([
                    ("person_id".into(), json!("p1")),
                    ("team_id".into(), json!("team-1")),
                    ("valid_from".into(), json!("2026-01-01")),
                    ("valid_to".into(), json!("2026-12-31")),
                ]),
            },
            SourceRow {
                source_id: "teams_a".into(),
                row_index: 0,
                values: BTreeMap::from([("team_id".into(), json!("team-1"))]),
            },
            SourceRow {
                source_id: "teams_b".into(),
                row_index: 0,
                values: BTreeMap::from([("team_id".into(), json!("team-1"))]),
            },
        ];

        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("ambiguous across 2 GOIDs"));
    }

    #[test]
    fn cove_o_readback_decodes_association_surface_from_persisted_bytes() {
        let file = association_readback_map();
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];
        let bytes = build_cove_o(&file, &rows).unwrap();
        let surface = read_object_surface_from_bytes(&bytes).unwrap();
        let association_records = surface
            .records
            .iter()
            .filter(|record| record.association.is_some())
            .collect::<Vec<_>>();
        assert_eq!(surface.records.len(), 3);
        assert_eq!(association_records.len(), 1);

        let association = association_records[0];
        let metadata = association.association.as_ref().unwrap();
        assert_eq!(metadata.association_type.as_deref(), Some("member_of"));
        let source = association
            .properties
            .iter()
            .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_FROM_GOID != 0)
            .unwrap();
        let target = association
            .properties
            .iter()
            .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_TO_GOID != 0)
            .unwrap();
        let association_type = association
            .properties
            .iter()
            .find(|property| property.flags & PROPERTY_FLAG_ASSOCIATION_TYPE != 0)
            .unwrap();
        let evidence = association
            .properties
            .iter()
            .find(|property| property.flags & PROPERTY_FLAG_EVIDENCE_REF != 0)
            .unwrap();
        assert_eq!(source.value.as_str().unwrap().len(), 32);
        assert_eq!(target.value.as_str().unwrap().len(), 32);
        assert_eq!(association_type.value, json!("member_of"));
        assert_eq!(evidence.value, json!("people:0"));
        assert_eq!(
            metadata.source_goid,
            source.value.as_str().map(str::to_string)
        );
        assert_eq!(
            metadata.target_goid,
            target.value.as_str().map(str::to_string)
        );
        assert_eq!(metadata.evidence_ref.as_deref(), Some("people:0"));
    }

    #[test]
    fn project_cove_o_matches_source_projection_for_objects_associations_and_evidence() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [
                    {
                        "projection_id": "person_objects.v1",
                        "output_table": "person_objects",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "goid", "value": "object.goid"},
                            {"name": "object_type", "value": "object.type"}
                        ],
                        "output_modes": ["json", "cove-o"]
                    },
                    {
                        "projection_id": "member_links.v1",
                        "output_table": "member_links",
                        "row_grain": "one_row_per_association",
                        "anchor": {"association_type": "member_of"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "explode",
                        "columns": [
                            {"name": "source_goid", "value": "association.source_goid"},
                            {"name": "target_goid", "value": "association.target_goid"},
                            {"name": "association_type", "value": "association.association_type"},
                            {"name": "evidence_id", "value": "association.source_evidence_id"}
                        ],
                        "output_modes": ["json"]
                    },
                    {
                        "projection_id": "evidence_rows.v1",
                        "output_table": "evidence_rows",
                        "row_grain": "one_row_per_evidence_assertion",
                        "anchor": {"object_type": "Person"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "columns": [
                            {"name": "source_id", "value": "evidence.source_id"},
                            {"name": "rule_id", "value": "evidence.rule_id"},
                            {"name": "assertion_id", "value": "evidence.assertion_id"},
                            {"name": "output_object_id", "value": "evidence.output_object_id"}
                        ],
                        "output_modes": ["json"]
                    }
                ]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];
        let source_projected = project_rows(&file, &rows).unwrap();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "cove-map-project-cove-o-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let object_path = dir.join("object.cove");
        fs::write(&object_path, bytes).unwrap();
        let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
        assert_eq!(persisted_projected["rows"], source_projected["rows"]);
        assert_eq!(
            persisted_projected["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["projection_id"] == json!("member_links.v1"))
                .count(),
            1
        );
        assert!(persisted_projected["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["projection_id"] == json!("evidence_rows.v1")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn source_projection_matches_persisted_projection_after_conflict_resolution() {
        let mut file = two_source_property_map("source_priority_wins", Some(10), Some(1));
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_projection",
                    "output_table": "people_projection",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "person_goid", "value": "object.goid"},
                        {"name": "name", "value": "name", "logical_type": "utf8"}
                    ],
                    "output_modes": ["json", "cove-o"]
                }]
            }),
        ));
        let rows = conflict_rows(json!("CRM Name"), json!("Support Name"));
        let source_projected = project_rows(&file, &rows).unwrap();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "cove-map-project-conflict-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let object_path = dir.join("object.cove");
        fs::write(&object_path, bytes).unwrap();
        let persisted_projected = project_cove_o_path(&object_path, None).unwrap();
        assert_eq!(persisted_projected["rows"], source_projected["rows"]);
        let rows = source_projected["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["projection_id"], json!("person_projection"));
        assert_eq!(rows[0]["output_table"], json!("people_projection"));
        assert_eq!(rows[0]["name"], json!("Support Name"));
        assert!(rows[0]["person_goid"].as_str().is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_writes_object_report_manifest_readme_and_projection() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-success", &file);
        let out_dir = dir.join("bundle");
        let result = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
        assert_eq!(result.manifest["mapping_id"], json!("people-map"));
        assert_eq!(result.manifest["counts"]["object_count"], json!(1));
        let expected_evidence_count = result.manifest["counts"]["evidence_entry_count"]
            .as_u64()
            .unwrap() as usize;
        let object_path = out_dir.join("people_map.cove");
        let report_path = out_dir.join("map-build-report.json");
        let manifest_path = out_dir.join("map-build-manifest.json");
        let readme_path = out_dir.join("README.md");
        let index_path = out_dir.join("indexes/object_properties.covi");
        let projection_path = out_dir.join("projections/people_projection.cove");
        assert!(object_path.exists());
        assert!(report_path.exists());
        assert!(manifest_path.exists());
        assert!(readme_path.exists());
        assert!(index_path.exists());
        assert!(projection_path.exists());
        validate_bytes_with_options(
            &fs::read(&object_path).unwrap(),
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let object_bytes = fs::read(&object_path).unwrap();
        let object_report = validate_bytes_with_options(
            &object_bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let evidence_entry = object_report
            .validated
            .footer
            .sections
            .iter()
            .find(|entry| entry.section_kind == SectionKind::MapEvidenceIndex as u16)
            .unwrap();
        let evidence_bytes = compression::section_payload(&object_bytes, evidence_entry).unwrap();
        assert!(is_compact_evidence_index_bytes(&evidence_bytes));
        let evidence_index = MapEvidenceIndex::parse(&evidence_bytes).unwrap();
        assert_eq!(evidence_index.entries.len(), expected_evidence_count);
        validate_bytes_with_options(
            &fs::read(&projection_path).unwrap(),
            ValidationOptions {
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let index = cove_index::CoviArtifactV2::parse(&fs::read(&index_path).unwrap()).unwrap();
        assert!(index.header.index_root_count > 0);
        let manifest: Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["evidence_encoding"], json!("compact"));
        assert_eq!(
            manifest["evidence"]["logical_entry_count"],
            json!(expected_evidence_count)
        );
        assert!(
            manifest["evidence"]["compact_binary_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(
            manifest["artifacts"]["indexes"][0]["target"],
            json!("cove-o-object-properties")
        );
        assert_eq!(manifest["section_compression"], json!("zstd"));
        assert_eq!(
            manifest["compression_summary"]["format"],
            json!("cove-map-section-compression-summary-v1")
        );
        assert_eq!(
            manifest["cache"]["key_material"]["section_compression"],
            json!("zstd")
        );
        assert_eq!(
            manifest["artifacts"]["projections"][0]["projection_id"],
            json!("person_projection")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_compresses_large_sections_and_none_disables_it() {
        let file = build_projection_map();
        let dir = temp_build_dir("build-section-compression");
        let map = dir.join("people.covemap");
        fs::write(&map, file.serialize().unwrap()).unwrap();
        let crm = dir.join("crm.csv");
        let support = dir.join("support.csv");
        let mut crm_csv = String::from("id,name\n");
        let mut support_csv = String::from("id,name\n");
        let repeated = "same-overlap-payload-".repeat(16);
        for index in 0..256 {
            crm_csv.push_str(&format!("{index},CRM {repeated}{index}\n"));
            support_csv.push_str(&format!("{index},Support {repeated}{index}\n"));
        }
        fs::write(&crm, crm_csv).unwrap();
        fs::write(&support, support_csv).unwrap();
        let sources = vec![crm, support];

        let compressed_out = dir.join("compressed");
        let compressed =
            build_from_paths(&map, &sources, MapBuildOptions::new(&compressed_out)).unwrap();
        assert_eq!(compressed.manifest["section_compression"], json!("zstd"));
        assert!(
            compressed.manifest["compression_summary"]["compressed_section_count"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            compressed.manifest["compression_summary"]["saved_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        let compressed_object_path = compressed_out.join("people_map.cove");
        let compressed_bytes = fs::read(&compressed_object_path).unwrap();
        let compressed_report = validate_bytes_with_options(
            &compressed_bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        assert_ne!(
            compressed_report.validated.header.required_features & FEATURE_CODEC_ZSTD,
            0
        );
        assert!(compressed_report
            .validated
            .footer
            .sections
            .iter()
            .any(|entry| entry.compression == CompressionCodec::Zstd as u8));

        let uncompressed_out = dir.join("uncompressed");
        let mut uncompressed_options = MapBuildOptions::new(&uncompressed_out);
        uncompressed_options.section_compression = MapBuildSectionCompression::None;
        let uncompressed = build_from_paths(&map, &sources, uncompressed_options).unwrap();
        assert_eq!(uncompressed.manifest["section_compression"], json!("none"));
        assert_eq!(
            uncompressed.manifest["compression_summary"]["compressed_section_count"],
            json!(0)
        );
        let uncompressed_object_path = uncompressed_out.join("people_map.cove");
        let uncompressed_bytes = fs::read(&uncompressed_object_path).unwrap();
        let uncompressed_report = validate_bytes_with_options(
            &uncompressed_bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        assert!(uncompressed_report
            .validated
            .footer
            .sections
            .iter()
            .all(|entry| entry.compression == CompressionCodec::None as u8));
        assert!(compressed_bytes.len() < uncompressed_bytes.len());

        let compressed_projection = project_cove_o_path(&compressed_object_path, None).unwrap();
        let uncompressed_projection = project_cove_o_path(&uncompressed_object_path, None).unwrap();
        assert_eq!(
            compressed_projection["rows"],
            uncompressed_projection["rows"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_can_emit_expanded_evidence_index_for_compatibility() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-expanded-evidence", &file);
        let out_dir = dir.join("bundle");
        let mut options = MapBuildOptions::new(&out_dir);
        options.evidence_encoding = MapEvidenceEncoding::Expanded;
        let result = build_from_paths(&map, &sources, options).unwrap();
        assert_eq!(result.manifest["evidence_encoding"], json!("expanded"));
        assert!(result.manifest["evidence"]["compact_binary_bytes"].is_null());
        let expected_evidence_count = result.manifest["counts"]["evidence_entry_count"]
            .as_u64()
            .unwrap() as usize;

        let object_path = out_dir.join("people_map.cove");
        let object_bytes = fs::read(&object_path).unwrap();
        let object_report = validate_bytes_with_options(
            &object_bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let evidence_entry = object_report
            .validated
            .footer
            .sections
            .iter()
            .find(|entry| entry.section_kind == SectionKind::MapEvidenceIndex as u16)
            .unwrap();
        let evidence_bytes = compression::section_payload(&object_bytes, evidence_entry).unwrap();
        assert!(!is_compact_evidence_index_bytes(&evidence_bytes));
        let evidence_index = MapEvidenceIndex::parse(&evidence_bytes).unwrap();
        assert_eq!(evidence_index.entries.len(), expected_evidence_count);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_verify_runs_doctor_and_writes_projection_lineage() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-verify", &file);
        let out_dir = dir.join("bundle");
        let mut options = MapBuildOptions::new(&out_dir);
        options.verify = true;
        let result = build_from_paths(&map, &sources, options).unwrap();
        assert_eq!(
            result.report["verification"]["format"],
            json!("cove-map-doctor-report-v1")
        );
        assert_eq!(result.report["verification"]["status"], json!("ok"));
        assert!(!report_has_failures(&result.report["verification"], false));

        let doctor = verify_bundle_dir(&out_dir).unwrap();
        assert_eq!(doctor["status"], json!("ok"));
        assert!(!report_has_failures(&doctor, false));
        assert_eq!(
            doctor["acceleration"]["projection_covi"]["available"],
            json!(true)
        );

        let projection_path = out_dir.join("projections/people_projection.cove");
        let projection_bytes = fs::read(projection_path).unwrap();
        let report = validate_bytes_with_options(
            &projection_bytes,
            ValidationOptions {
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        let lineage: Value =
            serde_json::from_slice(&report.validated.footer.metadata_json).unwrap();
        assert_eq!(lineage["format"], json!("cove-map-projection-lineage-v1"));
        assert_eq!(lineage["mapping_id"], json!("people-map"));
        assert_eq!(lineage["mapping_version"], json!("test/v1"));
        assert_eq!(lineage["projection_id"], json!("person_projection"));
        assert_eq!(lineage["projection_version"], json!("test/v1"));
        assert_eq!(lineage["source_cove_o"]["path"], json!("people_map.cove"));
        assert!(lineage["source_cove_o"]["digest"].as_str().is_some());
        assert!(lineage["mapping_artifact_digest"].as_str().is_some());
        assert!(lineage["covm_manifest"].is_null());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn doctor_reports_invalid_bundle_artifacts() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("doctor-invalid", &file);
        let out_dir = dir.join("bundle");
        let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
        fs::write(
            out_dir.join("projections/people_projection.cove"),
            b"not cove",
        )
        .unwrap();
        fs::write(out_dir.join("indexes/object_properties.covi"), b"not covi").unwrap();

        let doctor = verify_bundle_dir(&out_dir).unwrap();
        assert!(report_has_failures(&doctor, false));
        assert!(doctor["errors"].as_array().unwrap().iter().any(|error| {
            error["code"] == json!("invalid_cove_t_projection")
                && error["projection_id"] == json!("person_projection")
        }));
        assert!(doctor["errors"].as_array().unwrap().iter().any(|error| {
            error["code"] == json!("invalid_covi_index")
                && error["index_id"] == json!("object_properties")
        }));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn doctor_reports_projection_covi_missing_or_invalid_readiness() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("doctor-projection-covi-readiness", &file);
        let out_dir = dir.join("bundle");
        let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();

        match fs::remove_file(out_dir.join("indexes/projection_columns.covi")) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => panic!("cannot remove projection_columns.covi: {err}"),
        }
        let doctor = verify_bundle_dir(&out_dir).unwrap();
        assert_eq!(
            doctor["acceleration"]["projection_covi"]["sidecar_status"],
            json!("missing")
        );
        assert!(doctor["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == json!("missing_projection_covi_sidecar")));

        let mut options = MapBuildOptions::new(&out_dir);
        options.force = true;
        let _ = build_from_paths(&map, &sources, options).unwrap();
        fs::write(out_dir.join("indexes/projection_columns.covi"), b"not covi").unwrap();
        let doctor = verify_bundle_dir(&out_dir).unwrap();
        assert_eq!(
            doctor["acceleration"]["projection_covi"]["sidecar_status"],
            json!("invalid")
        );
        assert!(doctor["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == json!("missing_projection_covi_sidecar")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn doctor_strict_treats_skipped_projection_warning_as_failure() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("doctor-skipped-projection", &file);
        let out_dir = dir.join("bundle");
        let mut options = MapBuildOptions::new(&out_dir);
        options.projection_output = MapBuildProjectionOutput::None;
        let _ = build_from_paths(&map, &sources, options).unwrap();

        let doctor = verify_bundle_dir(&out_dir).unwrap();
        assert!(!report_has_failures(&doctor, false));
        assert!(report_has_failures(&doctor, true));
        assert!(doctor["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| {
                warning["code"] == json!("skipped_projection")
                    && warning["details"]["projection_id"] == json!("person_projection")
            }));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn suggest_outputs_non_authoritative_identity_and_join_candidates() {
        let dir = temp_build_dir("suggest");
        let crm = dir.join("crm.csv");
        let support = dir.join("support.csv");
        fs::write(
            &crm,
            "customer_id,email,name\n1,a@example.com,Ada\n2,b@example.com,Bo\n",
        )
        .unwrap();
        fs::write(
            &support,
            "customer_id,email,ticket_count\n1,a@example.com,3\n3,c@example.com,1\n",
        )
        .unwrap();

        let suggestions = suggest_from_paths(&[crm, support]).unwrap();
        assert_eq!(suggestions["format"], json!("cove-map-suggestions-v1"));
        assert_eq!(suggestions["non_authoritative"], json!(true));
        assert!(suggestions["identity_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| {
                source["source_id"] == json!("crm")
                    && source["candidates"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|candidate| {
                            candidate["column"] == json!("customer_id")
                                && candidate["non_authoritative"] == json!(true)
                        })
            }));
        assert!(suggestions["join_key_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| {
                candidate["left_column"] == json!("customer_id")
                    && candidate["right_column"] == json!("customer_id")
            }));
        assert!(suggestions["starter_projections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|projection| projection["projection_id"] == json!("crm_starter.v1")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parity_reports_matches_and_keyed_differences() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("parity", &file);
        let expected = dir.join("expected.csv");
        fs::write(&expected, "name\nSupport Name\n").unwrap();

        let options = ParityOptions {
            projection_id: "person_projection".into(),
            expected: expected.clone(),
            expected_query: None,
            key: vec!["name".into()],
        };
        let report = parity_from_paths(&map, &sources, &options).unwrap();
        assert_eq!(report["status"], json!("ok"));
        assert!(!parity_has_failures(&report));

        fs::write(&expected, "name\nWrong Name\n").unwrap();
        let options = ParityOptions {
            projection_id: "person_projection".into(),
            expected,
            expected_query: None,
            key: vec!["name".into()],
        };
        let report = parity_from_paths(&map, &sources, &options).unwrap();
        assert_eq!(report["status"], json!("mismatch"));
        assert_eq!(report["diff"]["missing_count"], json!(1));
        assert_eq!(report["diff"]["extra_count"], json!(1));
        assert!(parity_has_failures(&report));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parity_cove_o_supports_expected_query_and_unordered_warning() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("parity-cove-o", &file);
        let object = dir.join("object.cove");
        let bytes = cove_o_from_paths(&map, &sources).unwrap();
        fs::write(&object, bytes).unwrap();
        let expected = dir.join("expected.csv");
        fs::write(&expected, "name\nIgnored Name\nSupport Name\n").unwrap();

        let report = parity_from_cove_o_path(
            &object,
            &ParityOptions {
                projection_id: "person_projection".into(),
                expected,
                expected_query: Some(r#"where(name == "Support Name")"#.into()),
                key: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(report["status"], json!("ok"));
        assert!(report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| { warning["code"] == json!("ordered_comparison_without_key") }));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_collision_requires_force() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-collision", &file);
        let out_dir = dir.join("bundle");
        let _ = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap();
        let err = build_from_paths(&map, &sources, MapBuildOptions::new(&out_dir)).unwrap_err();
        assert!(err.contains("--force"));
        let mut options = MapBuildOptions::new(&out_dir);
        options.force = true;
        let _ = build_from_paths(&map, &sources, options).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_rejects_duplicate_projection_output_names() {
        let mut file = build_projection_map();
        mutate_section_payload(&mut file, 4, |value| {
            value["projections"] = json!([
                {
                    "projection_id": "person_projection",
                    "output_table": "people_projection",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [{"name": "name", "value": "name", "logical_type": "utf8"}],
                    "output_modes": ["json", "cove-t"]
                },
                {
                    "projection_id": "person_projection_copy",
                    "output_table": "people_projection",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [{"name": "name", "value": "name", "logical_type": "utf8"}],
                    "output_modes": ["json", "cove-t"]
                }
            ]);
        });
        let (map, sources, dir) = write_build_fixture("build-duplicate-projection", &file);
        let err =
            build_from_paths(&map, &sources, MapBuildOptions::new(dir.join("bundle"))).unwrap_err();
        assert!(err.contains("both map to output file"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_rejects_unsupported_source_extension_and_missing_source() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-source-errors", &file);
        let unsupported = dir.join("crm.txt");
        fs::write(&unsupported, "id,name\n1,Ada\n").unwrap();
        let err = build_from_paths(
            &map,
            &[unsupported, sources[1].clone()],
            MapBuildOptions::new(dir.join("unsupported")),
        )
        .unwrap_err();
        assert!(err.contains("must be .jsonl, .csv"));

        let err = build_from_paths(
            &map,
            &[sources[0].clone()],
            MapBuildOptions::new(dir.join("missing-source")),
        )
        .unwrap_err();
        assert!(err.contains("source 'support' is required"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn map_build_surfaces_projection_generation_errors() {
        let mut file = build_projection_map();
        mutate_section_payload(&mut file, 4, |value| {
            value["projections"][0]["row_grain"] = json!("unsupported_row_grain");
        });
        let (map, sources, dir) = write_build_fixture("build-projection-error", &file);
        let err =
            build_from_paths(&map, &sources, MapBuildOptions::new(dir.join("bundle"))).unwrap_err();
        assert!(err.contains("projection"), "err={err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn projected_record_batches_from_cove_o_bytes_chunks_arrow_output() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                        {"name": "object_type", "value": "object.type", "logical_type": "utf8"}
                    ],
                    "output_modes": ["arrow"]
                }]
            }),
        ));
        let rows = vec![
            SourceRow {
                source_id: "people".into(),
                row_index: 0,
                values: BTreeMap::from([
                    ("person_id".into(), json!("p1")),
                    ("team_id".into(), json!("t1")),
                    ("valid_from".into(), json!("2026-01-01")),
                    ("valid_to".into(), json!("2026-12-31")),
                ]),
            },
            SourceRow {
                source_id: "people".into(),
                row_index: 1,
                values: BTreeMap::from([
                    ("person_id".into(), json!("p2")),
                    ("team_id".into(), json!("t2")),
                    ("valid_from".into(), json!("2026-01-01")),
                    ("valid_to".into(), json!("2026-12-31")),
                ]),
            },
        ];
        let bytes = build_cove_o(&file, &rows).unwrap();
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "person_objects.v1",
            &ProjectionBatchOptions {
                batch_size: Some(1),
                ..ProjectionBatchOptions::default()
            },
        )
        .unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].num_rows(), 1);
        assert_eq!(batches[1].num_rows(), 1);
    }

    #[test]
    fn projection_catalog_readback_enriches_direct_property_lineage() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let catalog = projection_catalog_from_cove_o_bytes(&bytes, None).unwrap();
        let projection = catalog
            .projections
            .iter()
            .find(|projection| projection.projection_id == "people_primitives.v1")
            .unwrap();
        let goid = projection
            .columns
            .iter()
            .find(|column| column.name == "goid")
            .unwrap();
        assert!(goid.lineage.is_none());
        let score = projection
            .columns
            .iter()
            .find(|column| column.name == "score")
            .unwrap();
        let lineage = score.lineage.as_ref().unwrap();
        assert_eq!(lineage.source, "object_property");
        assert_eq!(lineage.object_type_name, "Person");
        assert_eq!(lineage.property_name, "score");
        assert_eq!(lineage.projection_table_id, 1);
        assert_eq!(lineage.projection_column_id, 3);
        assert_eq!(lineage.filter_pushdown, "projection_covi_prefilter");
    }

    #[test]
    fn projection_covi_filter_plan_reports_stable_reason_codes() {
        let descriptor = ProjectionDescriptor {
            projection_id: "people_primitives.v1".into(),
            output_table: Some("people_primitives".into()),
            output_modes: vec!["arrow".into()],
            columns: vec![
                ProjectionColumnDescriptor {
                    name: "score".into(),
                    logical_type: "int64".into(),
                    nested_shape: None,
                    lineage: Some(ProjectionColumnLineageDescriptor {
                        source: "object_property".into(),
                        object_type_id: 1,
                        object_type_name: "Person".into(),
                        property_id: 3,
                        property_name: "score".into(),
                        projection_table_id: 1,
                        projection_column_id: 3,
                        expression: "score".into(),
                        transform: "identity".into(),
                        filter_pushdown: "projection_covi_prefilter".into(),
                    }),
                },
                ProjectionColumnDescriptor {
                    name: "computed".into(),
                    logical_type: "utf8".into(),
                    nested_shape: None,
                    lineage: None,
                },
            ],
        };
        let filters = vec![
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Int64(10),
            },
            ProjectionFilter::InList {
                column: "score".into(),
                literals: vec![
                    ProjectionFilterLiteral::Int64(10),
                    ProjectionFilterLiteral::Int64(20),
                ],
            },
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::GtEq,
                literal: ProjectionFilterLiteral::Int64(10),
            },
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::Ne,
                literal: ProjectionFilterLiteral::Int64(10),
            },
            ProjectionFilter::IsNull {
                column: "score".into(),
                negated: false,
            },
            ProjectionFilter::Compare {
                column: "score".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Null,
            },
            ProjectionFilter::Compare {
                column: "computed".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Utf8("x".into()),
            },
            ProjectionFilter::Compare {
                column: "missing".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Utf8("x".into()),
            },
        ];
        let plan = projection_covi_filter_plan(&descriptor, &filters);
        assert_eq!(plan.lookups.len(), 3);
        let reasons = plan
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.reason.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            reasons,
            vec![
                "eligible",
                "eligible",
                "eligible",
                "not_equal",
                "is_null",
                "null_literal",
                "missing_lineage",
                "column_not_found",
            ]
        );
        assert!(plan.diagnostics[0].eligible);
        assert_eq!(plan.diagnostics[0].op, "eq");
        assert_eq!(plan.diagnostics[0].lineage_status, "present");
        assert_eq!(plan.diagnostics[0].projection_table_id, Some(1));
        assert_eq!(plan.unsupported_filters.len(), 5);
    }

    #[test]
    fn projection_candidate_rows_prefilter_before_residual_filters() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives.v1",
            &ProjectionBatchOptions {
                max_rows: None,
                output_columns: Some(vec!["score".into()]),
                pushed_filters: vec![ProjectionFilter::Compare {
                    column: "active".into(),
                    op: ProjectionFilterOp::Eq,
                    literal: ProjectionFilterLiteral::Boolean(true),
                }],
                batch_size: None,
                candidate_projection_rows: Some(ProjectionCandidateRows::from_ordinals([0, 2])),
            },
        )
        .unwrap();
        assert_eq!(int64_column_values(&batches, "score"), vec![10, 30]);
    }

    #[test]
    fn map_build_emits_projection_column_covi_sidecar() {
        let file = build_projection_map();
        let (map, sources, dir) = write_build_fixture("build-projection-column-covi", &file);
        let out = dir.join("bundle");
        let result = build_from_paths(&map, &sources, MapBuildOptions::new(&out)).unwrap();
        assert!(out
            .join("indexes")
            .join("projection_columns.covi")
            .is_file());
        let indexes = result
            .manifest
            .pointer("/artifacts/indexes")
            .and_then(Value::as_array)
            .unwrap();
        assert!(indexes.iter().any(|artifact| {
            artifact.get("path").and_then(Value::as_str) == Some("indexes/projection_columns.covi")
        }));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn projected_record_batches_filter_primitives_without_leaking_filter_columns() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives.v1",
            &ProjectionBatchOptions {
                output_columns: Some(vec!["score".into()]),
                pushed_filters: vec![ProjectionFilter::Compare {
                    column: "active".into(),
                    op: ProjectionFilterOp::Eq,
                    literal: ProjectionFilterLiteral::Boolean(true),
                }],
                ..ProjectionBatchOptions::default()
            },
        )
        .unwrap();

        assert_eq!(int64_column_values(&batches, "score"), vec![10, 30, 40]);
        for batch in batches {
            assert_eq!(batch.schema().fields().len(), 1);
            assert_eq!(batch.schema().field(0).name(), "score");
        }
    }

    #[test]
    fn projected_record_batches_filter_primitives_match_ordered_fallback() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let options = ProjectionBatchOptions {
            output_columns: Some(vec!["score".into()]),
            pushed_filters: vec![ProjectionFilter::Compare {
                column: "active".into(),
                op: ProjectionFilterOp::Eq,
                literal: ProjectionFilterLiteral::Boolean(true),
            }],
            ..ProjectionBatchOptions::default()
        };
        let fast = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives.v1",
            &options,
        )
        .unwrap();
        let fallback = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives_ordered.v1",
            &options,
        )
        .unwrap();

        assert_eq!(
            int64_column_values(&fast, "score"),
            int64_column_values(&fallback, "score")
        );
    }

    #[test]
    fn projected_record_batches_filter_primitives_honor_limit_after_filtering() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "people_primitives.v1",
            &ProjectionBatchOptions {
                max_rows: Some(2),
                output_columns: Some(vec!["score".into()]),
                pushed_filters: vec![ProjectionFilter::Compare {
                    column: "active".into(),
                    op: ProjectionFilterOp::Eq,
                    literal: ProjectionFilterLiteral::Boolean(true),
                }],
                batch_size: Some(1),
                candidate_projection_rows: None,
            },
        )
        .unwrap();

        assert_eq!(batches.len(), 2);
        assert_eq!(int64_column_values(&batches, "score"), vec![10, 30]);
    }

    #[test]
    fn projected_record_batches_filter_primitives_cover_exact_ops_and_nulls() {
        let file = primitive_projection_map();
        let rows = primitive_projection_rows();
        let bytes = build_cove_o(&file, &rows).unwrap();
        let cases = [
            (
                ProjectionFilter::Compare {
                    column: "score".into(),
                    op: ProjectionFilterOp::GtEq,
                    literal: ProjectionFilterLiteral::Int64(30),
                },
                vec![30, 40],
            ),
            (
                ProjectionFilter::Compare {
                    column: "score".into(),
                    op: ProjectionFilterOp::Lt,
                    literal: ProjectionFilterLiteral::Float64(30.0),
                },
                vec![10, 20],
            ),
            (
                ProjectionFilter::Compare {
                    column: "status".into(),
                    op: ProjectionFilterOp::Ne,
                    literal: ProjectionFilterLiteral::Utf8("closed".into()),
                },
                vec![10, 30],
            ),
            (
                ProjectionFilter::InList {
                    column: "status".into(),
                    literals: vec![ProjectionFilterLiteral::Utf8("open".into())],
                },
                vec![10, 30],
            ),
            (
                ProjectionFilter::IsNull {
                    column: "nickname".into(),
                    negated: false,
                },
                vec![20, 30],
            ),
        ];

        for (filter, expected) in cases {
            let batches = projected_record_batches_from_cove_o_bytes(
                &bytes,
                None,
                "people_primitives.v1",
                &ProjectionBatchOptions {
                    output_columns: Some(vec!["score".into()]),
                    pushed_filters: vec![filter],
                    ..ProjectionBatchOptions::default()
                },
            )
            .unwrap();
            assert_eq!(int64_column_values(&batches, "score"), expected);
        }
    }

    #[test]
    fn projection_rejects_undeclared_runtime_function() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "normalized_type", "value": "lower(object.type)"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];

        let err = project_rows(&file, &rows).unwrap_err();
        assert!(err.contains("undeclared projection function 'lower'"));
    }

    #[test]
    fn projection_rejects_undeclared_function_inside_predicate_argument() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "label", "value": "if(unknown(object.type) == \"Person\", object.type, \"Other\")"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];

        let err = project_rows(&file, &rows).unwrap_err();
        assert!(err.contains("undeclared projection function 'unknown'"));
    }

    #[test]
    fn projection_rejects_aggregate_without_aggregate_policy() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_memberships.v1",
                    "output_table": "person_memberships",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "membership_count", "value": "count(association(member_of))"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];

        let err = project_rows(&file, &rows).unwrap_err();
        assert!(err.contains(
            "projection 'person_memberships.v1' aggregate 'count' requires multi_value_policy='aggregate'"
        ));
    }

    #[test]
    fn projection_cove_o_output_materializes_projected_objects() {
        let mut file = association_readback_map();
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_objects.v1",
                    "output_table": "person_objects",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "list",
                    "columns": [
                        {"name": "goid", "value": "object.goid"},
                        {"name": "object_type", "value": "object.type"}
                    ],
                    "output_modes": ["json", "cove-o"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
            ]),
        }];
        let bytes = crate::project::project_rows_with_source_states_output(
            &file,
            &rows,
            &[],
            crate::project::ProjectionFormat::CoveO,
            Some("person_objects.v1"),
        )
        .unwrap();
        let surface = read_object_surface_from_bytes(&bytes).unwrap();
        assert_eq!(
            surface.projection_catalog.as_ref().unwrap().projections[0].projection_id,
            "person_objects.v1"
        );
        let projected = surface
            .records
            .iter()
            .find(|record| record.object_type_name == "person_objects")
            .unwrap();
        assert!(projected
            .properties
            .iter()
            .any(|property| property.property_name == "object_type"
                && property.value == json!("Person")));
    }

    #[test]
    fn projection_cove_o_output_stores_nested_properties_as_filecodes() {
        let mut file = association_readback_map();
        mutate_section_payload(&mut file, 3, |payload| {
            let rule = payload["rules"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap();
            rule.insert(
                "property_bindings".into(),
                json!([
                    {
                        "assertion_id": "person_tags",
                        "property_id": "tags",
                        "property_name": "tags",
                        "source_column": "tags",
                        "logical_type": "list",
                        "physical_kind": "auto",
                        "nullable": true,
                        "missing_policy": "null",
                        "conflict_policy": "reject_conflict"
                    },
                    {
                        "assertion_id": "person_profile",
                        "property_id": "profile",
                        "property_name": "profile",
                        "source_column": "profile",
                        "logical_type": "struct",
                        "physical_kind": "auto",
                        "nullable": true,
                        "missing_policy": "null",
                        "conflict_policy": "reject_conflict"
                    },
                    {
                        "assertion_id": "person_scores",
                        "property_id": "scores",
                        "property_name": "scores",
                        "source_column": "scores",
                        "logical_type": "map",
                        "physical_kind": "auto",
                        "nullable": true,
                        "missing_policy": "null",
                        "conflict_policy": "reject_conflict"
                    }
                ]),
            );
        });
        file.sections.push(test_section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "person_nested.v1",
                    "output_table": "person_nested",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "list",
                    "columns": [
                        {
                            "name": "tags",
                            "value": "tags",
                            "logical_type": "list",
                            "nested_shape": {
                                "type": "list",
                                "item": {"logical_type": "utf8"}
                            }
                        },
                        {
                            "name": "profile",
                            "value": "profile",
                            "logical_type": "struct",
                            "nested_shape": {
                                "type": "struct",
                                "fields": [
                                    {"name": "active", "logical_type": "bool"},
                                    {"name": "level", "logical_type": "int64"}
                                ]
                            }
                        },
                        {
                            "name": "scores",
                            "value": "scores",
                            "logical_type": "map",
                            "nested_shape": {
                                "type": "map",
                                "key": {"logical_type": "utf8"},
                                "value": {"logical_type": "int64"}
                            }
                        }
                    ],
                    "output_modes": ["json", "cove-o"]
                }]
            }),
        ));
        let rows = vec![SourceRow {
            source_id: "people".into(),
            row_index: 0,
            values: BTreeMap::from([
                ("person_id".into(), json!("p1")),
                ("team_id".into(), json!("t1")),
                ("valid_from".into(), json!("2026-01-01")),
                ("valid_to".into(), json!("2026-12-31")),
                ("tags".into(), json!(["alpha", "beta"])),
                ("profile".into(), json!({"active": true, "level": 7})),
                ("scores".into(), json!({"logic": 100, "math": 99})),
            ]),
        }];
        let bytes = crate::project::project_rows_with_source_states_output(
            &file,
            &rows,
            &[],
            crate::project::ProjectionFormat::CoveO,
            Some("person_nested.v1"),
        )
        .unwrap();
        let report = validate_bytes_with_options(&bytes, ValidationOptions::default()).unwrap();
        assert!(report
            .validated
            .footer
            .sections
            .iter()
            .any(|entry| { entry.section_kind == SectionKind::FileDictionaryIndex as u16 }));
        let surface = read_object_surface_from_bytes(&bytes).unwrap();
        let object_type = surface
            .object_types
            .iter()
            .find(|object_type| object_type.type_name == "person_nested")
            .unwrap();
        for property_name in ["tags", "profile", "scores"] {
            let property = object_type
                .properties
                .iter()
                .find(|property| property.property_name == property_name)
                .unwrap();
            assert_eq!(property.physical_kind, CovePhysicalKind::FileCode);
        }
        assert_eq!(
            object_type
                .properties
                .iter()
                .find(|property| property.property_name == "tags")
                .unwrap()
                .logical_type,
            CoveLogicalType::List
        );
        assert_eq!(
            object_type
                .properties
                .iter()
                .find(|property| property.property_name == "profile")
                .unwrap()
                .logical_type,
            CoveLogicalType::Struct
        );
        assert_eq!(
            object_type
                .properties
                .iter()
                .find(|property| property.property_name == "scores")
                .unwrap()
                .logical_type,
            CoveLogicalType::Map
        );
        let projected = surface
            .records
            .iter()
            .find(|record| record.object_type_name == "person_nested")
            .unwrap();
        let projected_property = |name: &str| {
            projected
                .properties
                .iter()
                .find(|property| property.property_name == name)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(projected_property("tags"), json!(["alpha", "beta"]));
        assert_eq!(
            projected_property("profile"),
            json!({"active": true, "level": 7})
        );
        assert_eq!(
            projected_property("scores"),
            json!({"logic": 100, "math": 99})
        );
    }

    #[test]
    fn governance_metadata_emits_effective_policy_by_default() {
        let file = governance_map("emit_effective_policy");
        let rows = vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("2"))]),
            },
        ];
        let materialized = materialize_with_source_states(&file, &rows, &[]).unwrap();
        let governance = &materialized.conversion_report["governance"];
        assert_eq!(governance["effective_sensitivity_rank"], json!(5));
        assert_eq!(
            governance["effective_sensitivity_labels"],
            json!(["restricted"])
        );
        assert_eq!(
            governance["access_policy_ids"],
            json!(["hipaa", "internal"])
        );
    }

    #[test]
    fn governance_policy_rejects_mixed_sensitivity_when_requested() {
        let file = governance_map("reject_on_mixed_sensitivity");
        let rows = vec![
            SourceRow {
                source_id: "crm".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1"))]),
            },
            SourceRow {
                source_id: "support".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("2"))]),
            },
        ];
        let err = materialize_with_source_states(&file, &rows, &[]).unwrap_err();
        assert!(err.contains("mixed source sensitivity"));
    }

    #[test]
    fn replay_claimed_source_validates_fingerprints() {
        let dir = std::env::temp_dir().join(format!("cove-map-replay-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("crm.csv");
        fs::write(&path, "id\n1\n").unwrap();
        let inputs = read_source_inputs(&[path]).unwrap();
        let state = &inputs.states[0];
        let mut file = two_source_identity_map(Vec::new());
        file.sections[0] = test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "crm",
                    "row_identity_rules": ["person_by_id"],
                    "schema_fingerprint": state.schema_fingerprint,
                    "snapshot_digest": state.snapshot_digest,
                    "replay_claimed": true
                }]
            }),
        );
        validate_source_inputs(&file, &inputs.states).unwrap();
        file.sections[0] = test_section(
            SectionKind::MapSourceCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "sources": [{
                    "source_id": "crm",
                    "row_identity_rules": ["person_by_id"],
                    "schema_fingerprint": state.schema_fingerprint,
                    "snapshot_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "replay_claimed": true
                }]
            }),
        );
        assert!(validate_source_inputs(&file, &inputs.states).is_err());
        assert!(validate_source_inputs(&file, &[]).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn build_cove_o_emits_valid_object_temporal_file() {
        fn section(kind: SectionKind, value: Value) -> CovemapSection {
            let payload = serde_json::to_vec_pretty(&covemap_payload_value(kind, value))
                .expect("serializing serde_json::Value cannot fail");
            CovemapSection {
                entry: CovemapSectionEntryV1 {
                    section_id: kind as u32,
                    offset: 0,
                    length: payload.len() as u64,
                    uncompressed_length: payload.len() as u64,
                    compression: 0,
                    payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
                    required: true,
                    reserved: 0,
                    checksum: 0,
                },
                payload,
            }
        }
        let file = CovemapFile {
            header: CovemapHeaderV1::new([0x42; 16], 0),
            mapping_version: "test/v1".into(),
            sections: vec![
                section(
                    SectionKind::MapSourceCatalog,
                    json!({
                        "mapping_id": "people-map",
                        "mapping_version": "test/v1",
                        "sources": [{
                            "source_id": "people",
                            "row_identity_rules": ["person_by_id"]
                        }]
                    }),
                ),
                section(
                    SectionKind::MapFunctionRegistry,
                    json!({
                        "mapping_id": "people-map",
                        "mapping_version": "test/v1",
                        "functions": [{
                            "function_id": "identity",
                            "version": "1",
                            "deterministic": true,
                            "dependency": "pure"
                        }]
                    }),
                ),
                section(
                    SectionKind::MapIdentityRuleCatalog,
                    json!({
                        "mapping_id": "people-map",
                        "mapping_version": "test/v1",
                        "identity_rules": [{
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        }],
                        "do_not_merge": []
                    }),
                ),
                section(
                    SectionKind::MapRowSemanticsCatalog,
                    json!({
                        "mapping_id": "people-map",
                        "mapping_version": "test/v1",
                        "rules": [{
                            "rule_id": "upsert_person",
                            "source_id": "people",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": [],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "name_assertion",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8"
                            }]
                        }]
                    }),
                ),
            ],
            postscript: CovemapPostscriptV1 {
                required_features: FEATURE_SEMANTIC_MAP,
                optional_features: 0,
                file_len: 0,
                header_offset: 0,
                header_length: 0,
                checksum: 0,
            },
        };
        let rows = vec![
            SourceRow {
                source_id: "people".into(),
                row_index: 0,
                values: BTreeMap::from([("id".into(), json!("1")), ("name".into(), json!("Ada"))]),
            },
            SourceRow {
                source_id: "people".into(),
                row_index: 1,
                values: BTreeMap::from([
                    ("id".into(), json!("2")),
                    ("name".into(), json!("Linus")),
                ]),
            },
        ];
        let bytes = build_cove_o(&file, &rows).unwrap();
        let report = validate_bytes_with_options(
            &bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            report.validated.header.required_features & FEATURE_SEMANTIC_MAP,
            0
        );
        assert_ne!(
            report.validated.header.optional_features & FEATURE_SEMANTIC_MAP,
            0
        );
        assert!(report
            .validated
            .footer
            .sections
            .iter()
            .filter(|entry| {
                matches!(
                    SectionKind::from_u16(entry.section_kind),
                    Some(
                        SectionKind::MapSourceCatalog
                            | SectionKind::MapFunctionRegistry
                            | SectionKind::MapIdentityRuleCatalog
                            | SectionKind::MapRowSemanticsCatalog
                            | SectionKind::MapAssertionLog
                            | SectionKind::MapIdentityEquivalenceIndex
                            | SectionKind::MapEvidenceIndex
                            | SectionKind::MapConversionReport
                    )
                )
            })
            .all(|entry| entry.required_features & FEATURE_SEMANTIC_MAP == 0
                && entry.optional_features & FEATURE_SEMANTIC_MAP != 0));
        let kinds = report
            .validated
            .footer
            .sections
            .iter()
            .map(|entry| SectionKind::from_u16(entry.section_kind).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                SectionKind::MapSourceCatalog,
                SectionKind::MapFunctionRegistry,
                SectionKind::MapIdentityRuleCatalog,
                SectionKind::MapRowSemanticsCatalog,
                SectionKind::ObjectTypeCatalog,
                SectionKind::TemporalSegmentIndex,
                SectionKind::TemporalSegmentData,
                SectionKind::TrustManifest,
                SectionKind::MapAssertionLog,
                SectionKind::MapIdentityEquivalenceIndex,
                SectionKind::MapEvidenceIndex,
                SectionKind::MapConversionReport,
            ]
        );
        let segment_entry = report
            .validated
            .footer
            .sections
            .iter()
            .find(|entry| entry.section_kind == SectionKind::TemporalSegmentData as u16)
            .unwrap();
        let segment_bytes = compression::section_payload(&bytes, segment_entry).unwrap();
        let segment = TemporalSegmentData::parse(&segment_bytes).unwrap();
        assert_eq!(segment.header.column_count, 1);
        assert_eq!(segment.property_columns.len(), 1);
        assert_eq!(segment.property_columns[0].page_index.entries.len(), 1);

        let mut projected_file = file.clone();
        projected_file.sections.push(section(
            SectionKind::MapProjectionCatalog,
            json!({
                "mapping_id": "people-map",
                "mapping_version": "test/v1",
                "projections": [{
                    "projection_id": "people_names.v1",
                    "output_table": "people_names",
                    "row_grain": "one_row_per_object",
                    "anchor": {"object_type": "Person"},
                    "temporal_mode": {"as_of": "latest_committed"},
                    "multi_value_policy": "reject",
                    "columns": [
                        {"name": "person_goid", "value": "object.goid"},
                        {"name": "name", "value": "Person.name"}
                    ],
                    "output_modes": ["json"]
                }]
            }),
        ));
        let projected = project_rows(&projected_file, &rows).unwrap();
        assert_eq!(projected["rows"].as_array().unwrap().len(), 2);
        assert_eq!(projected["rows"][0]["name"], json!("Ada"));
    }

    #[test]
    fn cove_o_conversion_accepts_parquet_orc_and_arrow_ipc_sources() {
        let dir = std::env::temp_dir().join(format!(
            "cove-map-multi-source-ingest-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let map_path = dir.join("mapping.covemap");
        fs::write(&map_path, association_readback_map().serialize().unwrap()).unwrap();
        let batch = people_batch();
        let cases = [
            ("people.parquet", write_parquet(&batch)),
            ("people.orc", write_orc(&batch)),
            ("people.arrow", write_arrow_ipc(&batch)),
        ];
        for (file_name, bytes) in cases {
            let source_path = dir.join(file_name);
            fs::write(&source_path, bytes).unwrap();
            let cove_bytes =
                cove_o_from_paths(&map_path, std::slice::from_ref(&source_path)).unwrap();
            let surface = read_object_surface_from_bytes(&cove_bytes).unwrap();
            assert_eq!(surface.records.len(), 3, "{file_name}");
            assert_eq!(
                surface
                    .records
                    .iter()
                    .filter(|record| record.association.is_some())
                    .count(),
                1,
                "{file_name}"
            );
            fs::remove_file(&source_path).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
