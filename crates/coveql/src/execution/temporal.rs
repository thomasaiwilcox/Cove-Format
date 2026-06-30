use super::*;

pub(crate) fn temporal_object_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    if let Some(changes) = &planned.resolved.method_chain.changes {
        return match changes.mode {
            AstChangeMode::Records => Ok(change_records(surface, planned)?
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .map(MaterializedObjectRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::ChangeRecord))
                .map(ExecutionRow::Object)
                .collect()),
            AstChangeMode::PropertyDiffs => {
                object_property_diff_rows(surface, planned, object_type_id)
            }
            AstChangeMode::StateTransitions => states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::ChangeStateTransition,
            ),
            AstChangeMode::FinalRows => {
                final_object_rows_for_change_window(surface, planned, object_type_id)
            }
        };
    }
    match planned
        .resolved
        .method_chain
        .history
        .unwrap_or(AstHistoryMode::States)
    {
        AstHistoryMode::Records => Ok(history_records(surface, planned)
            .into_iter()
            .filter(|record| record.object_type_id == object_type_id)
            .filter(|record| include_record_tombstone(record, planned))
            .map(MaterializedObjectRow::from_record)
            .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
            .map(ExecutionRow::Object)
            .collect()),
        AstHistoryMode::States => {
            states_for_records(surface, planned, object_type_id, OutputGrain::HistoryState)
        }
        AstHistoryMode::RecordsAndStates => {
            let mut rows = history_records(surface, planned)
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .map(MaterializedObjectRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
                .map(ExecutionRow::Object)
                .collect::<Vec<_>>();
            rows.extend(states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::HistoryState,
            )?);
            Ok(rows)
        }
    }
}

pub(crate) fn temporal_association_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    if let Some(changes) = &planned.resolved.method_chain.changes {
        return match changes.mode {
            AstChangeMode::Records => Ok(change_records(surface, planned)?
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .filter_map(MaterializedAssociationRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::ChangeRecord))
                .map(ExecutionRow::Association)
                .collect()),
            AstChangeMode::PropertyDiffs => {
                association_property_diff_rows(surface, planned, object_type_id)
            }
            AstChangeMode::StateTransitions => association_states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::ChangeStateTransition,
            ),
            AstChangeMode::FinalRows => {
                final_association_rows_for_change_window(surface, planned, object_type_id)
            }
        };
    }
    match planned
        .resolved
        .method_chain
        .history
        .unwrap_or(AstHistoryMode::States)
    {
        AstHistoryMode::Records => Ok(history_records(surface, planned)
            .into_iter()
            .filter(|record| record.object_type_id == object_type_id)
            .filter(|record| include_record_tombstone(record, planned))
            .filter_map(MaterializedAssociationRow::from_record)
            .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
            .map(ExecutionRow::Association)
            .collect()),
        AstHistoryMode::States => association_states_for_records(
            surface,
            planned,
            object_type_id,
            OutputGrain::HistoryState,
        ),
        AstHistoryMode::RecordsAndStates => {
            let mut rows = history_records(surface, planned)
                .into_iter()
                .filter(|record| record.object_type_id == object_type_id)
                .filter(|record| include_record_tombstone(record, planned))
                .filter_map(MaterializedAssociationRow::from_record)
                .map(|row| row.with_output_grain(OutputGrain::HistoryRecord))
                .map(ExecutionRow::Association)
                .collect::<Vec<_>>();
            rows.extend(association_states_for_records(
                surface,
                planned,
                object_type_id,
                OutputGrain::HistoryState,
            )?);
            Ok(rows)
        }
    }
}

pub(super) fn history_records<'a>(
    surface: &'a CoveObjectSurface,
    planned: &PlannedQuery,
) -> Vec<&'a CoveObjectRecord> {
    let branch_key = concrete_branch_key(planned);
    let mut records = surface
        .records
        .iter()
        .filter(|record| branch_key.is_none_or(|branch_key| record.branch_key == branch_key))
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record_sort_key(record));
    records
}

pub(super) fn object_states_for_temporal_context(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
) -> Result<Vec<CoveObjectState>, BuildExecutionError> {
    if let TemporalMode::AsOfTimestampMicros(timestamp) = planned.resolved.temporal.mode {
        if let Some(binding) = planned.resolved.temporal.role_binding.as_deref() {
            return role_bound_states_at(surface, planned, binding, timestamp);
        }
    }
    let reconstruction = reconstruction_options(planned)?;
    reconstruct_object_states(surface, &reconstruction).map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O object reconstruction failed: {err}"),
            json!({}),
        )
    })
}

pub(crate) fn role_bound_states_at(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    binding: &str,
    timestamp_micros: i64,
) -> Result<Vec<CoveObjectState>, BuildExecutionError> {
    let branch_key = concrete_branch_key(planned);
    let mut selected = BTreeMap::new();
    for record in &surface.records {
        if branch_key.is_some_and(|branch_key| record.branch_key != branch_key) {
            continue;
        }
        if !include_record_tombstone(record, planned) {
            continue;
        }
        let Some(value) = temporal_binding_value(record, binding)? else {
            continue;
        };
        if value > timestamp_micros {
            continue;
        }
        let key = (record.object_type_id, record.branch_key, record.goid);
        let replace = selected
            .get(&key)
            .is_none_or(|current: &&CoveObjectRecord| {
                record_sort_key(record) > record_sort_key(current)
            });
        if replace {
            selected.insert(key, record);
        }
    }
    let mut states = Vec::new();
    for record in selected.values() {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let reconstructed = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O role-bound reconstruction failed: {err}"),
                json!({ "binding": binding }),
            )
        })?;
        states.extend(
            reconstructed
                .into_iter()
                .filter(|state| state.object_type_id == record.object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id),
        );
    }
    states.sort_by_key(|state| {
        (
            state.object_type_id,
            state.branch_key,
            state.goid,
            state.timestamp_us,
            state.csn,
        )
    });
    Ok(states)
}

pub(super) fn change_records<'a>(
    surface: &'a CoveObjectSurface,
    planned: &PlannedQuery,
) -> Result<Vec<&'a CoveObjectRecord>, BuildExecutionError> {
    let changes = planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .ok_or_else(|| exec_error("E_EXECUTION", "missing changes context", json!({})))?;
    let branch_key = concrete_branch_key(planned);
    let mut records = Vec::new();
    for record in &surface.records {
        if branch_key.is_some_and(|branch_key| record.branch_key != branch_key) {
            continue;
        }
        if record_in_half_open_bound(record, &changes.from, &changes.to)? {
            records.push(record);
        }
    }
    records.sort_by_key(|record| record_sort_key(record));
    Ok(records)
}

pub(super) fn states_for_records(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
    output_grain: OutputGrain,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let records = if planned.resolved.method_chain.changes.is_some() {
        change_records(surface, planned)?
    } else {
        history_records(surface, planned)
    };
    let mut rows = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let states = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O history state reconstruction failed: {err}"),
                json!({}),
            )
        })?;
        rows.extend(
            states
                .iter()
                .filter(|state| state.object_type_id == object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id)
                .map(MaterializedObjectRow::from_state)
                .map(|row| row.with_output_grain(output_grain))
                .map(ExecutionRow::Object),
        );
    }
    Ok(rows)
}

pub(super) fn association_states_for_records(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
    output_grain: OutputGrain,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let records = if planned.resolved.method_chain.changes.is_some() {
        change_records(surface, planned)?
    } else {
        history_records(surface, planned)
    };
    let mut rows = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let options = CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        };
        let states = reconstruct_object_states(surface, &options).map_err(|err| {
            exec_error(
                "E_RECONSTRUCT",
                format!("COVE-O association history state reconstruction failed: {err}"),
                json!({}),
            )
        })?;
        rows.extend(
            states
                .iter()
                .filter(|state| state.object_type_id == object_type_id)
                .filter(|state| state.branch_key == record.branch_key && state.goid == record.goid)
                .filter(|state| state.latest_record_id == record.record_id)
                .filter_map(MaterializedAssociationRow::from_state)
                .map(|row| row.with_output_grain(output_grain))
                .map(ExecutionRow::Association),
        );
    }
    Ok(rows)
}

pub(super) fn object_property_diff_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let mut rows = Vec::new();
    for record in change_records(surface, planned)?
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let old_properties = previous_object_properties(surface, planned, record)?;
        let current_row = object_row_for_record_state(surface, planned, record)?
            .unwrap_or_else(|| MaterializedObjectRow::from_record(record))
            .with_output_grain(OutputGrain::ChangePropertyDiff);
        let new_properties =
            row_properties_by_id(&current_row.properties, &current_row.property_ids);
        for change in property_diffs(old_properties, new_properties) {
            rows.push(ExecutionRow::Object(
                current_row.clone().with_change(change),
            ));
        }
    }
    Ok(rows)
}

pub(super) fn association_property_diff_rows(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let mut rows = Vec::new();
    for record in change_records(surface, planned)?
        .into_iter()
        .filter(|record| record.object_type_id == object_type_id)
        .filter(|record| include_record_tombstone(record, planned))
    {
        let Some(current_row) = association_row_for_record_state(surface, planned, record)?
            .or_else(|| MaterializedAssociationRow::from_record(record))
        else {
            continue;
        };
        let current_row = current_row.with_output_grain(OutputGrain::ChangePropertyDiff);
        let old_properties = previous_object_properties(surface, planned, record)?;
        let new_properties =
            row_properties_by_id(&current_row.properties, &current_row.property_ids);
        for change in property_diffs(old_properties, new_properties) {
            rows.push(ExecutionRow::Association(
                current_row.clone().with_change(change),
            ));
        }
    }
    Ok(rows)
}

pub(super) fn previous_object_properties(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<BTreeMap<u32, (String, Value)>, BuildExecutionError> {
    if record.csn == 0 {
        return Ok(BTreeMap::new());
    }
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn.saturating_sub(1)),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O property diff previous-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states
        .iter()
        .find(|state| {
            state.object_type_id == record.object_type_id
                && state.branch_key == record.branch_key
                && state.goid == record.goid
        })
        .map(|state| properties_by_id(&state.properties))
        .unwrap_or_default())
}

pub(super) fn object_row_for_record_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<MaterializedObjectRow>, BuildExecutionError> {
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O property diff current-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states
        .iter()
        .find(|state| {
            state.object_type_id == record.object_type_id
                && state.branch_key == record.branch_key
                && state.goid == record.goid
                && state.latest_record_id == record.record_id
        })
        .map(MaterializedObjectRow::from_state))
}

pub(super) fn association_row_for_record_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<MaterializedAssociationRow>, BuildExecutionError> {
    Ok(object_row_state(surface, planned, record)?
        .as_ref()
        .and_then(MaterializedAssociationRow::from_state))
}

pub(super) fn object_row_state(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    record: &CoveObjectRecord,
) -> Result<Option<CoveObjectState>, BuildExecutionError> {
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: CoveObjectTemporalCut::Csn(record.csn),
            branch_key: Some(record.branch_key),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O association property diff current-state reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    Ok(states.into_iter().find(|state| {
        state.object_type_id == record.object_type_id
            && state.branch_key == record.branch_key
            && state.goid == record.goid
            && state.latest_record_id == record.record_id
    }))
}

pub(super) fn properties_by_id(
    properties: &[CoveObjectPropertyValue],
) -> BTreeMap<u32, (String, Value)> {
    properties
        .iter()
        .map(|property| {
            (
                property.property_id,
                (property.property_name.clone(), property.value.clone()),
            )
        })
        .collect()
}

pub(super) fn row_properties_by_id(
    properties: &BTreeMap<String, Value>,
    property_ids: &BTreeMap<u32, String>,
) -> BTreeMap<u32, (String, Value)> {
    property_ids
        .iter()
        .filter_map(|(property_id, name)| {
            properties
                .get(name)
                .cloned()
                .map(|value| (*property_id, (name.clone(), value)))
        })
        .collect()
}

pub(super) fn property_diffs(
    old_properties: BTreeMap<u32, (String, Value)>,
    new_properties: BTreeMap<u32, (String, Value)>,
) -> Vec<MaterializedChangeDetail> {
    let property_ids = old_properties
        .keys()
        .chain(new_properties.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    property_ids
        .into_iter()
        .filter_map(|property_id| {
            let old = old_properties.get(&property_id);
            let new = new_properties.get(&property_id);
            match (old, new) {
                (None, None) => None,
                (None, Some((name, new_value))) => Some(MaterializedChangeDetail {
                    property_id,
                    property_name: name.clone(),
                    old_value: Value::Null,
                    new_value: new_value.clone(),
                    diff_kind: MaterializedChangeDiffKind::Added,
                }),
                (Some((name, old_value)), None) => Some(MaterializedChangeDetail {
                    property_id,
                    property_name: name.clone(),
                    old_value: old_value.clone(),
                    new_value: Value::Null,
                    diff_kind: MaterializedChangeDiffKind::Removed,
                }),
                (Some((name, old_value)), Some((new_name, new_value))) => (old_value != new_value)
                    .then(|| MaterializedChangeDetail {
                        property_id,
                        property_name: non_empty_property_name(new_name, name),
                        old_value: old_value.clone(),
                        new_value: new_value.clone(),
                        diff_kind: MaterializedChangeDiffKind::Changed,
                    }),
            }
        })
        .collect()
}

pub(super) fn non_empty_property_name(preferred: &str, fallback: &str) -> String {
    if preferred.is_empty() {
        fallback.to_string()
    } else {
        preferred.to_string()
    }
}

pub(super) fn final_object_rows_for_change_window(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let cut = change_to_reconstruction_cut(planned)?;
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: cut,
            branch_key: concrete_branch_key(planned),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O final change reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    let changed_keys = change_records(surface, planned)?
        .into_iter()
        .map(|record| (record.object_type_id, record.branch_key, record.goid))
        .collect::<BTreeSet<_>>();
    Ok(states
        .iter()
        .filter(|state| state.object_type_id == object_type_id)
        .filter(|state| {
            changed_keys.contains(&(state.object_type_id, state.branch_key, state.goid))
        })
        .map(MaterializedObjectRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::FinalObject))
        .map(ExecutionRow::Object)
        .collect())
}

pub(super) fn final_association_rows_for_change_window(
    surface: &CoveObjectSurface,
    planned: &PlannedQuery,
    object_type_id: u32,
) -> Result<Vec<ExecutionRow>, BuildExecutionError> {
    let cut = change_to_reconstruction_cut(planned)?;
    let states = reconstruct_object_states(
        surface,
        &CoveObjectReconstructionOptions {
            temporal_cut: cut,
            branch_key: concrete_branch_key(planned),
            include_tombstones: planned.resolved.tombstone.include_tombstones,
        },
    )
    .map_err(|err| {
        exec_error(
            "E_RECONSTRUCT",
            format!("COVE-O final association change reconstruction failed: {err}"),
            json!({}),
        )
    })?;
    let changed_keys = change_records(surface, planned)?
        .into_iter()
        .map(|record| (record.object_type_id, record.branch_key, record.goid))
        .collect::<BTreeSet<_>>();
    Ok(states
        .iter()
        .filter(|state| state.object_type_id == object_type_id)
        .filter(|state| {
            changed_keys.contains(&(state.object_type_id, state.branch_key, state.goid))
        })
        .filter_map(MaterializedAssociationRow::from_state)
        .map(|row| row.with_output_grain(OutputGrain::FinalObject))
        .map(ExecutionRow::Association)
        .collect())
}

pub(super) fn include_record_tombstone(record: &CoveObjectRecord, planned: &PlannedQuery) -> bool {
    planned.resolved.tombstone.include_tombstones
        || record.record_kind != cove_core::profile::cove_o::RecordKind::Tombstone
}

pub(super) fn concrete_branch_key(planned: &PlannedQuery) -> Option<u64> {
    match planned.resolved.branch.selector {
        crate::BranchSelector::BranchKey(branch) => Some(branch),
        crate::BranchSelector::Default | crate::BranchSelector::RejectAmbiguous => None,
    }
}

pub(super) fn record_sort_key(
    record: &CoveObjectRecord,
) -> (u32, u64, [u8; 16], i64, u64, u32, u32, [u8; 16]) {
    (
        record.object_type_id,
        record.branch_key,
        record.goid,
        record.timestamp_us,
        record.csn,
        record.segment_id,
        record.row_index,
        record.record_id,
    )
}

pub(super) fn record_in_half_open_bound(
    record: &CoveObjectRecord,
    from: &ResolvedTimeBound,
    to: &ResolvedTimeBound,
) -> Result<bool, BuildExecutionError> {
    match (from, to) {
        (ResolvedTimeBound::Csn(from), ResolvedTimeBound::Csn(to)) => {
            Ok(record.csn >= *from && record.csn < *to)
        }
        (
            ResolvedTimeBound::TimestampMicros {
                role: from_role,
                binding: from_binding,
                timestamp_micros: from,
                ..
            },
            ResolvedTimeBound::TimestampMicros {
                role: to_role,
                binding: to_binding,
                timestamp_micros: to,
                ..
            },
        ) if from_role == to_role && *from_role == TemporalRole::CommitTime => {
            Ok(record.timestamp_us >= *from && record.timestamp_us < *to)
        }
        (
            ResolvedTimeBound::TimestampMicros {
                binding: from_binding,
                timestamp_micros: from,
                ..
            },
            ResolvedTimeBound::TimestampMicros {
                binding: to_binding,
                timestamp_micros: to,
                ..
            },
        ) => {
            if from_binding != to_binding {
                return Err(exec_error(
                    "E_UNSUPPORTED_TEMPORAL_ROLE",
                    "change windows must use matching temporal role bindings",
                    json!({}),
                ));
            }
            let Some(binding) = from_binding.as_deref() else {
                return Ok(record.timestamp_us >= *from && record.timestamp_us < *to);
            };
            let Some(value) = temporal_binding_value(record, binding)? else {
                return Ok(false);
            };
            Ok(value >= *from && value < *to)
        }
        _ => Err(exec_error(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            "change windows must use matching CSN or timestamp bound types",
            json!({}),
        )),
    }
}

pub(super) fn temporal_binding_value(
    record: &CoveObjectRecord,
    binding: &str,
) -> Result<Option<i64>, BuildExecutionError> {
    let Some(property) = record
        .properties
        .iter()
        .find(|property| property.property_name == binding)
    else {
        return Ok(None);
    };
    match &property.value {
        Value::Number(number) => number.as_i64().map(Some).ok_or_else(|| {
            exec_error(
                "E_UNSUPPORTED_TEMPORAL_ROLE",
                "temporal role binding value must fit in timestamp micros",
                json!({ "binding": binding }),
            )
        }),
        Value::String(value) => {
            let (timestamp, _) = parse_execution_timestamp_micros(value)?;
            Ok(Some(timestamp))
        }
        Value::Null => Ok(None),
        _ => Err(exec_error(
            "E_UNSUPPORTED_TEMPORAL_ROLE",
            "temporal role binding value must be timestamp micros or RFC3339 text",
            json!({ "binding": binding }),
        )),
    }
}

pub(super) fn parse_execution_timestamp_micros(
    value: &str,
) -> Result<(i64, String), BuildExecutionError> {
    let parsed = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| {
            exec_error(
                "E_LITERAL",
                "timestamp literal must be RFC3339 with explicit offset",
                json!({}),
            )
        })?;
    let micros = parsed.unix_timestamp_nanos() / 1_000;
    let micros = i64::try_from(micros).map_err(|_| {
        exec_error(
            "E_LITERAL",
            "timestamp literal is outside supported microsecond range",
            json!({}),
        )
    })?;
    let canonical = parsed
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|err| exec_error("E_LITERAL", err.to_string(), json!({})))?;
    Ok((micros, canonical))
}

pub(super) fn change_to_reconstruction_cut(
    planned: &PlannedQuery,
) -> Result<CoveObjectTemporalCut, BuildExecutionError> {
    let changes = planned
        .resolved
        .method_chain
        .changes
        .as_ref()
        .ok_or_else(|| exec_error("E_EXECUTION", "missing changes context", json!({})))?;
    match &changes.to {
        ResolvedTimeBound::Csn(0) => Ok(CoveObjectTemporalCut::Csn(0)),
        ResolvedTimeBound::Csn(csn) => Ok(CoveObjectTemporalCut::Csn(csn.saturating_sub(1))),
        ResolvedTimeBound::TimestampMicros {
            role,
            timestamp_micros,
            ..
        } if *role == TemporalRole::CommitTime => Ok(CoveObjectTemporalCut::TimestampUs(
            timestamp_micros.saturating_sub(1),
        )),
        ResolvedTimeBound::TimestampMicros {
            timestamp_micros, ..
        } => Ok(CoveObjectTemporalCut::TimestampUs(
            timestamp_micros.saturating_sub(1),
        )),
    }
}
