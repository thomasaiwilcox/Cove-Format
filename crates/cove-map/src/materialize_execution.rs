use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ReviewedDecisionReplayBinding {
    pub(crate) count: usize,
    pub(crate) digest: String,
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

pub(crate) fn materialize_with_source_states(
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

pub(crate) fn reviewed_decision_replay_binding(
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

pub(crate) fn identity_equivalence_index(
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

pub(crate) fn object_types_from_mapping(
    context: &MappingContext,
) -> Result<Vec<ObjectTypeEntryV1>, String> {
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
