use super::*;

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

pub(crate) fn push_unique_assertion(
    assertions: &mut Vec<Value>,
    assertion_id: &str,
    output_object_id: &str,
) {
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

pub(crate) fn candidate_rule_report_entry(candidate: &Value) -> Value {
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

pub(crate) fn candidate_rule_evidence_entry(candidate: &Value) -> Result<Value, String> {
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

pub(crate) fn nested_candidate_value(object: Option<&Map<String, Value>>, key: &str) -> Value {
    object
        .and_then(|object| object.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

pub(crate) fn conversion_report_sources(
    rows: &[SourceRow],
    source_states: &[ObservedSourceState],
) -> Value {
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

pub(crate) fn add_operation_metadata(
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

pub(crate) fn copy_operation_policy_value(
    object: &mut Map<String, Value>,
    source_row: &SourceRow,
    key: &str,
) {
    if let Some(value) = source_row.values.get(key).filter(|value| !value.is_null()) {
        object.insert(key.to_string(), value.clone());
    }
}

pub(crate) fn operation_counts(evidence_entries: &[Value]) -> Value {
    let mut counts = BTreeMap::<String, u64>::new();
    for entry in evidence_entries {
        if let Some(kind) = entry.get("source_operation_kind").and_then(Value::as_str) {
            *counts.entry(kind.to_string()).or_default() += 1;
        }
    }
    json!(counts)
}

pub(crate) fn evidence_bool_count(evidence_entries: &[Value], key: &str) -> usize {
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

pub(crate) fn resolver_catalog_digests(evidence_entries: &[Value]) -> Value {
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

pub(crate) fn resolver_goid_impact(evidence_entries: &[Value]) -> Value {
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

pub(crate) fn operation_effect(kind: SourceOperationKind) -> &'static str {
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

pub(crate) fn operation_target(row_rule: &MapRowSemanticRule) -> &'static str {
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

pub(crate) fn row_rule_emits_non_object_evidence(row_rule: &MapRowSemanticRule) -> bool {
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

pub(crate) fn governance_report(
    context: &MappingContext,
    rows: &[SourceRow],
) -> Result<Value, String> {
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

pub(crate) fn materialize_properties(
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

pub(crate) fn validate_property_conflict_policy(policy: &str) -> Result<(), String> {
    match policy {
        "reject_conflict" | "source_priority_wins" => Ok(()),
        other => Err(format!("unsupported property conflict_policy '{other}'")),
    }
}

pub(crate) fn prune_empty_shadow_rows(rows: &mut Vec<ObjectRow>) {
    let populated = rows
        .iter()
        .filter(|row| !row.properties.is_empty())
        .map(|row| (row.object_type_id, row.goid))
        .collect::<BTreeSet<_>>();
    rows.retain(|row| {
        !(row.properties.is_empty() && populated.contains(&(row.object_type_id, row.goid)))
    });
}

pub(crate) fn resolve_property_conflicts(
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

pub(crate) fn source_value_for_binding(
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

pub(crate) fn source_value_for_expression(
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

pub(crate) fn association_validity_value(
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
pub(crate) fn materialize_associations(
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
pub(crate) fn resolve_association_endpoint<'a>(
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

pub(crate) fn row_rule_materializes_object(row_rule: &MapRowSemanticRule) -> Result<bool, String> {
    match row_rule.row_semantics_kind.as_str() {
        "Object" | "EventObject" | "LinkObject" | "Composite" | "Dispatched"
        | "KeyValueFragment" | "Tombstone" => Ok(true),
        "AssociationOnly" | "EvidenceOnly" | "ProjectionOnly" => Ok(false),
        other => Err(format!("unsupported row_semantics_kind '{other}'")),
    }
}

pub(crate) fn row_rule_materializes_associations(
    row_rule: &MapRowSemanticRule,
) -> Result<bool, String> {
    match row_rule.row_semantics_kind.as_str() {
        "Object" | "EventObject" | "LinkObject" | "AssociationOnly" | "Composite"
        | "Dispatched" | "KeyValueFragment" => Ok(true),
        "EvidenceOnly" | "ProjectionOnly" | "Tombstone" => Ok(false),
        other => Err(format!("unsupported row_semantics_kind '{other}'")),
    }
}

pub(crate) fn record_kind_for_row_rule(
    row_rule: &MapRowSemanticRule,
) -> Result<RecordKind, String> {
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

pub(crate) fn association_record_kind_for_row_rule(row_rule: &MapRowSemanticRule) -> RecordKind {
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

pub(crate) fn record_id_for(
    source_id: &str,
    row_index: usize,
    rule_id: &str,
    goid: &[u8; 16],
) -> [u8; 16] {
    let record_material = format!("{source_id}:{row_index}:{rule_id}:{}", hex_encode(goid));
    first_16(&sha256_array(record_material.as_bytes()))
}

pub(crate) fn association_goid(
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

pub(crate) fn property_id_from_binding(binding: &MapPropertyBinding, fallback: u32) -> u32 {
    stable_u32(&binding.property_id, fallback)
}

pub(crate) fn physical_kind_from_binding(
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

pub(crate) fn association_properties() -> Vec<PropertyEntryV1> {
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

pub(crate) fn build_temporal_segments(
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

pub(crate) fn reconstructed_temporal_segments(
    object_types: &[ObjectTypeEntryV1],
    states: &[CoveObjectState],
) -> Result<Vec<ReconstructedTemporalSegmentBuild>, String> {
    reconstructed_temporal_segments_with_record_kind(object_types, states, None)
}

pub(crate) fn reconstructed_temporal_segments_with_record_kind(
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

pub(crate) fn reconstructed_temporal_segment_payload(
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

pub(crate) fn reconstructed_property_page_payload(
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

pub(crate) fn reconstructed_property_value(
    state: &CoveObjectState,
    property_id: u32,
) -> Option<&Value> {
    state
        .properties
        .iter()
        .find(|property| property.property_id == property_id)
        .map(|property| &property.value)
}

pub(crate) fn reconstructed_temporal_segment_index(
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

pub(crate) fn reconstructed_row_kind_counts(rows: &[CoveObjectState]) -> (u32, u32, u32, u32) {
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

pub(crate) fn reconstructed_trust_manifest(
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

pub(crate) fn temporal_segment_payload(
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

pub(crate) fn build_property_page_payload(
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

pub(crate) fn nested_shapes_for_model(
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

pub(crate) fn file_dictionary_for_model(
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

pub(crate) fn file_dictionary_index_bytes(dictionary: &FileDictionary) -> Vec<u8> {
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

pub(crate) fn file_dictionary_key_for_property(
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

pub(crate) fn canonical_value_for_logical<'a>(
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

pub(crate) fn canonical_value_for_nested_shape<'a>(
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

pub(crate) fn canonical_map_object_key_for_shape<'a>(
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

pub(crate) fn canonical_value_from_json<'a>(
    value: &'a Value,
) -> Result<CanonicalValue<'a>, String> {
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

pub(crate) fn canonical_list_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| "list property value must be an array".to_string())?
        .iter()
        .map(canonical_value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CanonicalValue::List(items))
}

pub(crate) fn canonical_struct_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
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

pub(crate) fn canonical_map_value<'a>(value: &'a Value) -> Result<CanonicalValue<'a>, String> {
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

pub(crate) fn append_property_value_bytes(
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

pub(crate) fn append_null_placeholder(
    property: &PropertyEntryV1,
    out: &mut Vec<u8>,
) -> Result<(), String> {
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

pub(crate) fn temporal_segment_index(
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

pub(crate) fn row_kind_counts(rows: &[ObjectRow]) -> (u32, u32, u32, u32) {
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

pub(crate) fn temporal_prev_refs(
    segment_id: u32,
    rows: &[ObjectRow],
) -> Vec<Option<CoveRecordRefV1>> {
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

pub(crate) fn trust_manifest(
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

pub(crate) fn object_section(
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

pub(crate) fn dictionary_section(
    kind: SectionKind,
    item_count: u64,
    data: Vec<u8>,
) -> SectionPayload {
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

pub(crate) fn map_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
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

pub(crate) fn ensure_covemap_payload_envelope(kind: SectionKind, data: Vec<u8>) -> Vec<u8> {
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

pub(crate) fn map_passthrough_sections(
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

pub(crate) fn enriched_projection_catalog_payload(
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
pub(crate) struct JoinKeyComponent<'a> {
    pub(crate) role_id: &'a str,
    pub(crate) logical_type_id: &'a str,
    pub(crate) value: Option<&'a [u8]>,
}

pub(crate) fn join_key_tuple(
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

pub(crate) fn resolver_error_observed_value(
    resolver: &MapResolver,
    observed_value: &str,
) -> String {
    if resolver.evidence_policy == "redact_raw" {
        "<redacted>".to_string()
    } else {
        observed_value.to_string()
    }
}

pub(crate) fn resolution_metadata_base(
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

pub(crate) fn apply_resolution_pipeline(
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

pub(crate) fn strip_legal_suffix(
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

pub(crate) fn most_restrictive_confidence(left: &str, right: &str) -> String {
    if identity_confidence_rank(left) >= identity_confidence_rank(right) {
        left.to_string()
    } else {
        right.to_string()
    }
}

pub(crate) fn identity_confidence_rank(class: &str) -> u8 {
    match class {
        "authoritative" | "reviewed_authoritative" => 0,
        "strong_deterministic" => 1,
        "source_scoped" => 2,
        "weak_deterministic" => 3,
        "candidate_only" | "candidate" => 4,
        _ => 5,
    }
}

pub(crate) fn apply_canonicalization(
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

pub(crate) fn deterministic_builtin_function_ids() -> &'static [&'static str] {
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

pub(crate) fn string_arg<'a>(value: &'a Value, function_id: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("{function_id} requires a string value"))
}

pub(crate) fn validate_decimal_text(text: &str) -> Result<(), String> {
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

pub(crate) fn validate_utc_timestamp_text(text: &str) -> Result<(), String> {
    let has_utc_suffix = text.ends_with('Z') || text.ends_with("+00:00");
    if has_utc_suffix && text.contains('T') {
        Ok(())
    } else {
        Err("parse_timestamp_utc requires an ISO-8601 UTC timestamp".into())
    }
}

pub(crate) fn canonical_component_bytes(
    logical_type: &str,
    value: &Value,
) -> Result<Vec<u8>, String> {
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

pub(crate) fn mapped_goid(
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

pub(crate) fn goid16_parts(parts: &[&[u8]]) -> [u8; 16] {
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
