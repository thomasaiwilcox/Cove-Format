use super::*;

pub(super) fn validate_covm_delta_chain_selection_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVM delta-chain selection fixture json: {error}"
        ))
    })?;
    let extension_bytes = required_u8_array(&value, "extension")?;
    let summary_bytes = match value.get("summary") {
        Some(summary) => Some(value_u8_array(summary)?),
        None => None,
    };
    let delta_values = value
        .get("deltas")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("delta-chain selection fixture missing deltas".into())
        })?;
    let deltas = delta_values
        .iter()
        .map(value_u8_array)
        .collect::<Result<Vec<_>, _>>()?;
    let delta_refs = deltas.iter().map(Vec::as_slice).collect::<Vec<&[u8]>>();

    let extension = CovmDeltaChainExtensionV1::parse(&extension_bytes)?;
    validate_selected_delta_chain_with_summary_bytes(
        &extension,
        summary_bytes.as_deref(),
        &delta_refs,
    )?;
    if let Some(base_bytes) = parse_optional_fixture_byte_vector(&value, "base")? {
        let summary = summary_bytes
            .as_deref()
            .map(CovmDeltaChainSummaryV1::parse)
            .transpose()?;
        validate_selected_delta_chain_with_base(
            &extension,
            summary.as_ref(),
            Some(&base_bytes),
            &delta_refs,
        )?;
    }
    Ok(())
}

pub(super) fn validate_covm_delta_pruning_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid COVM delta pruning fixture json: {error}"))
    })?;
    let summary_bytes = required_u8_array(&value, "summary")?;
    let summary = CovmDeltaChainSummaryV1::parse(&summary_bytes)?;
    let decision = summary.prune_delta_chain(CovmDeltaPruneRequest {
        as_of_csn: optional_u64(&value, "as_of_csn")?,
        as_of_commit_timestamp_us: optional_i64(&value, "as_of_commit_timestamp_us")?,
        as_of_valid_time_us: optional_i64(&value, "as_of_valid_time_us")?,
        source_publish_range_us: optional_i64_range(
            &value,
            "source_publish_range_start_us",
            "source_publish_range_end_us",
        )?,
    })?;

    let expected_selected = required_u32_array(&value, "expect_selected")?;
    if decision.selected_chain_ordinals != expected_selected {
        return Err(CoveError::BadSection(format!(
            "delta pruning selected ordinals mismatch: expected {:?}, got {:?}",
            expected_selected, decision.selected_chain_ordinals
        )));
    }

    let expected_skipped = required_prune_skips(&value)?;
    if decision.skipped != expected_skipped {
        return Err(CoveError::BadSection(format!(
            "delta pruning skipped ordinals mismatch: expected {:?}, got {:?}",
            expected_skipped, decision.skipped
        )));
    }
    if let Some(metrics_value) = value.get("expect_metrics") {
        validate_prune_metrics(decision.metrics(), metrics_value)?;
    }

    Ok(())
}

pub(super) fn validate_covedelta_sparse_patch_state_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVEDELTA sparse patch state fixture json: {error}"
        ))
    })?;
    let record_values = value
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("sparse patch state fixture missing records".into())
        })?;
    let mut records = Vec::with_capacity(record_values.len());
    for (index, record_value) in record_values.iter().enumerate() {
        let field = format!("records[{index}]");
        let record_bytes = parse_fixture_byte_vector(Some(record_value), &field)?;
        records.push(DeltaSparsePatchRecordV1::parse(&record_bytes)?);
    }

    let states = reconstruct_sparse_patch_state_table(&records)?;
    let expected = value.get("expect_state").ok_or_else(|| {
        CoveError::BadSection("sparse patch state fixture missing expect_state".into())
    })?;
    let key = sparse_object_key_from_json(expected)?;
    let actual = states.get(&key).ok_or_else(|| {
        CoveError::BadSection("sparse patch state fixture expected state is absent".into())
    })?;

    let expected_latest_record_id = fixture_array_16(expected, "latest_record_id")?;
    if actual.latest_record_id != expected_latest_record_id {
        return Err(CoveError::BadSection(
            "sparse patch latest_record_id mismatch".into(),
        ));
    }
    let expected_latest_timestamp_us = required_i64_field(expected, "latest_timestamp_us")?;
    if actual.latest_timestamp_us != expected_latest_timestamp_us {
        return Err(CoveError::BadSection(
            "sparse patch latest_timestamp_us mismatch".into(),
        ));
    }
    let expected_latest_csn = required_u64_field(expected, "latest_csn")?;
    if actual.latest_csn != expected_latest_csn {
        return Err(CoveError::BadSection(
            "sparse patch latest_csn mismatch".into(),
        ));
    }
    let expected_record_kind =
        record_kind_from_name(json_field_str(expected, "latest_record_kind")?)?;
    if actual.latest_record_kind != expected_record_kind {
        return Err(CoveError::BadSection(
            "sparse patch latest_record_kind mismatch".into(),
        ));
    }
    let expected_tombstone_status =
        sparse_tombstone_status_from_name(json_field_str(expected, "tombstone_status")?)?;
    if actual.tombstone_status != expected_tombstone_status {
        return Err(CoveError::BadSection(
            "sparse patch tombstone_status mismatch".into(),
        ));
    }

    let expected_properties = expected
        .get("properties")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("sparse patch state fixture missing properties".into())
        })?;
    if actual.properties.len() != expected_properties.len() {
        return Err(CoveError::BadSection(format!(
            "sparse patch property count mismatch: expected {}, got {}",
            expected_properties.len(),
            actual.properties.len()
        )));
    }
    for property in expected_properties {
        let property_id = json_field_u32(property, "property_id")?;
        let expected_state = sparse_property_state_from_json(property)?;
        if actual.properties.get(&property_id) != Some(&expected_state) {
            return Err(CoveError::BadSection(format!(
                "sparse patch property {property_id} state mismatch"
            )));
        }
    }
    Ok(())
}

fn sparse_object_key_from_json(value: &Value) -> Result<DeltaSparseObjectKeyV1, CoveError> {
    Ok(DeltaSparseObjectKeyV1 {
        scope_kind: json_field_u16(value, "scope_kind")?,
        scope_id: fixture_array_16(value, "scope_id")?,
        branch_identity_ref: json_field_u32(value, "branch_identity_ref")?,
        object_type_id: json_field_u32(value, "object_type_id")?,
        goid: fixture_array_16(value, "goid")?,
    })
}

fn sparse_property_state_from_json(
    value: &Value,
) -> Result<DeltaSparsePatchPropertyStateV1, CoveError> {
    match json_field_str(value, "state")? {
        "value_ref" => Ok(DeltaSparsePatchPropertyStateV1::ValueRef(json_field_u32(
            value,
            "value_ref",
        )?)),
        "null" => Ok(DeltaSparsePatchPropertyStateV1::Null),
        "clear" => Ok(DeltaSparsePatchPropertyStateV1::Clear),
        "tombstone" => Ok(DeltaSparsePatchPropertyStateV1::Tombstone(json_field_u8(
            value,
            "tombstone_kind",
        )?)),
        "redacted" => Ok(DeltaSparsePatchPropertyStateV1::Redacted {
            redaction_ref: json_field_u32(value, "redaction_ref")?,
        }),
        other => Err(CoveError::BadSection(format!(
            "unknown sparse patch property state {other}"
        ))),
    }
}

fn sparse_tombstone_status_from_name(
    value: &str,
) -> Result<DeltaSparseObjectTombstoneStatusV1, CoveError> {
    match value {
        "live" => Ok(DeltaSparseObjectTombstoneStatusV1::Live),
        "tombstoned" => Ok(DeltaSparseObjectTombstoneStatusV1::Tombstoned),
        other => Err(CoveError::BadSection(format!(
            "unknown sparse patch tombstone_status {other}"
        ))),
    }
}

fn record_kind_from_name(value: &str) -> Result<RecordKind, CoveError> {
    match value {
        "Delta" => Ok(RecordKind::Delta),
        "Snapshot" => Ok(RecordKind::Snapshot),
        "Baseline" => Ok(RecordKind::Baseline),
        "Tombstone" => Ok(RecordKind::Tombstone),
        other => Err(CoveError::BadSection(format!(
            "unknown sparse patch record kind {other}"
        ))),
    }
}

fn fixture_array_16(value: &Value, field: &str) -> Result<[u8; 16], CoveError> {
    parse_fixture_byte_vector(value.get(field), field)?
        .try_into()
        .map_err(|_| CoveError::BadSection(format!("{field} must contain exactly 16 bytes")))
}

pub(super) fn validate_covedelta_object_membership_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVEDELTA object membership fixture json: {error}"
        ))
    })?;
    let delta_bytes = parse_fixture_byte_vector(value.get("delta"), "delta")?;
    let delta = CoveDeltaFile::parse(&delta_bytes)?;
    let validation = delta.validate_object_delta()?;
    let checks = value
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("COVEDELTA object membership fixture missing checks".into())
        })?;

    for (index, check) in checks.iter().enumerate() {
        let point = delta_object_point_from_json(check)?;
        let expected_touched =
            exact_membership_from_name(json_field_str(check, "expect_touched")?)?;
        let actual_touched = validation.exact_touched_membership(point);
        if actual_touched != expected_touched {
            return Err(CoveError::BadSection(format!(
                "check {index} touched membership mismatch: expected {:?}, got {:?}",
                expected_touched, actual_touched
            )));
        }

        let expected_can_skip = json_field_bool(check, "expect_can_skip")?;
        let actual_can_skip = validation.can_skip_delta_for_point_lookup(point);
        if actual_can_skip != expected_can_skip {
            return Err(CoveError::BadSection(format!(
                "check {index} skip decision mismatch: expected {expected_can_skip}, got {actual_can_skip}"
            )));
        }

        let expected_tombstone =
            exact_membership_from_name(json_field_str(check, "expect_tombstone")?)?;
        let actual_tombstone = validation.exact_tombstone_membership(point);
        if actual_tombstone != expected_tombstone {
            return Err(CoveError::BadSection(format!(
                "check {index} tombstone membership mismatch: expected {:?}, got {:?}",
                expected_tombstone, actual_tombstone
            )));
        }

        let expected_suppress = json_field_bool(check, "expect_suppress_parent_latest_state")?;
        let actual_suppress = validation.should_suppress_parent_latest_state_for_tombstone(point);
        if actual_suppress != expected_suppress {
            return Err(CoveError::BadSection(format!(
                "check {index} tombstone suppression mismatch: expected {expected_suppress}, got {actual_suppress}"
            )));
        }

        if check.get("expect_projection_property_skip").is_some() {
            let expected_projection_skip =
                json_field_bool(check, "expect_projection_property_skip")?;
            let requested_property_ids =
                optional_u32_array(check, "requested_property_ids")?.ok_or_else(|| {
                    CoveError::BadSection(format!(
                        "check {index} projection-property skip expectation missing requested_property_ids"
                    ))
                })?;
            let actual_projection_skip =
                validation.can_skip_delta_for_projection_properties(point, &requested_property_ids);
            if actual_projection_skip != expected_projection_skip {
                return Err(CoveError::BadSection(format!(
                    "check {index} projection-property skip mismatch: expected {expected_projection_skip}, got {actual_projection_skip}"
                )));
            }
        }
    }
    Ok(())
}

fn delta_object_point_from_json(value: &Value) -> Result<DeltaObjectPointLookupV1, CoveError> {
    Ok(DeltaObjectPointLookupV1 {
        scope_kind: json_field_u16(value, "scope_kind")?,
        scope_id: fixture_array_16(value, "scope_id")?,
        object_type_id: json_field_u32(value, "object_type_id")?,
        branch_identity_ref: json_field_u32(value, "branch_identity_ref")?,
        goid: fixture_array_16(value, "goid")?,
    })
}

fn exact_membership_from_name(value: &str) -> Result<DeltaExactObjectSetMembershipV1, CoveError> {
    match value {
        "present" => Ok(DeltaExactObjectSetMembershipV1::Present),
        "absent" => Ok(DeltaExactObjectSetMembershipV1::Absent),
        "unavailable" => Ok(DeltaExactObjectSetMembershipV1::Unavailable),
        other => Err(CoveError::BadSection(format!(
            "unknown exact object-set membership {other}"
        ))),
    }
}

pub(super) fn validate_covedelta_covi_tombstone_overlay_fixture(
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVEDelta COVE-I tombstone overlay fixture json: {error}"
        ))
    })?;
    let base_bytes = json_byte_vec(&value, "base")?;
    let base_surface = read_object_surface_from_bytes(&base_bytes)?;
    let delta_bytes = json_byte_vec(&value, "delta")?;
    let delta = CoveDeltaFile::parse(&delta_bytes)?;
    let delta_validation = delta.validate_object_delta()?;
    let covi_bytes = json_byte_vec(&value, "covi")?;
    let context_value = value
        .get("context")
        .ok_or_else(|| CoveError::BadSection("COVE-I overlay fixture missing context".into()))?;
    let context = covi_validation_context_from_fixture_value(context_value)?;
    let validated_covi = ValidatedCoviArtifactV2::parse_and_validate(&covi_bytes, context)?;
    let operation = value
        .get("operation")
        .ok_or_else(|| CoveError::BadSection("COVE-I overlay fixture missing operation".into()))?;
    let object_type_id = json_field_u32(operation, "object_type_id")?;
    let path_ref = json_field_u32(operation, "path_ref")?;
    let key = json_byte_vec(operation, "key")?;
    let request = CoviLookupRequestV2::eq_target(
        CoviLookupTargetV2::ObjectPath {
            object_type_id,
            path_ref,
        },
        CoviLookupKeyV2::ObjectPathTuple(key),
    );
    let candidates = validated_covi.lookup(&request)?;
    if let Some(expected) = optional_usize(&value, "expect_base_candidate_count")? {
        if candidates.object_paths.len() != expected {
            return Err(CoveError::BadSection(
                "COVE-I base object-path candidate count mismatch".into(),
            ));
        }
    }

    let mut corrected_goids = Vec::<[u8; 16]>::new();
    for candidate in &candidates.object_paths {
        if candidate.file_ref != 0 {
            return Err(CoveError::BadCovi);
        }
        let end = candidate
            .row_start
            .checked_add(candidate.row_count)
            .ok_or(CoveError::ArithOverflow)?;
        for row_index in candidate.row_start..end {
            let record = base_surface
                .records
                .iter()
                .find(|record| {
                    record.object_type_id == candidate.object_type_id
                        && record.segment_id == candidate.segment_id
                        && u64::from(record.row_index) == row_index
                })
                .ok_or_else(|| {
                    CoveError::BadSection(
                        "COVE-I object-path candidate does not resolve to a base object record"
                            .into(),
                    )
                })?;
            let branch_identity_ref =
                u32::try_from(record.branch_key).map_err(|_| CoveError::ArithOverflow)?;
            let point = DeltaObjectPointLookupV1 {
                scope_kind: delta.header.scope_kind,
                scope_id: delta.header.scope_id,
                object_type_id: record.object_type_id,
                branch_identity_ref,
                goid: record.goid,
            };
            if !delta_validation.should_suppress_parent_latest_state_for_tombstone(point) {
                corrected_goids.push(record.goid);
            }
        }
    }
    if let Some(expected) = optional_usize(&value, "expect_corrected_candidate_count")? {
        if corrected_goids.len() != expected {
            return Err(CoveError::BadSection(
                "COVE-I delta-corrected candidate count mismatch".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_covedelta_reconstruction_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVEDELTA reconstruction fixture json: {error}"
        ))
    })?;
    let base_bytes = parse_fixture_byte_vector(value.get("base"), "base")?;
    let delta_values = value
        .get("deltas")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("COVEDELTA reconstruction fixture missing deltas".into())
        })?;
    let mut deltas = Vec::with_capacity(delta_values.len());
    for (index, delta_value) in delta_values.iter().enumerate() {
        let field = format!("deltas[{index}]");
        let delta_bytes = parse_fixture_byte_vector(Some(delta_value), &field)?;
        deltas.push(CoveDeltaFile::parse(&delta_bytes)?);
    }

    let states = reconstruct_object_states_from_base_and_delta_files(
        &base_bytes,
        &deltas,
        &CoveObjectReadOptions::default(),
        &CoveObjectReconstructionOptions::default(),
    )?;
    let expected_states = value
        .get("expect_states")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("COVEDELTA reconstruction fixture missing expect_states".into())
        })?;
    if states.len() != expected_states.len() {
        return Err(CoveError::BadSection(format!(
            "COVEDELTA reconstruction state count mismatch: expected {}, got {}",
            expected_states.len(),
            states.len()
        )));
    }

    validate_expected_reconstructed_states(&states, expected_states)?;
    validate_expected_delta_validation(&deltas, &value)?;

    if let Some(expected_evidence) = value.get("expect_evidence_index") {
        let surface = read_object_surface_from_base_and_delta_files_with_options(
            &base_bytes,
            &deltas,
            &CoveObjectReadOptions::default(),
        )?;
        validate_expected_evidence_index(&surface, expected_evidence)?;
    }

    if let Some(expected_with_tombstones) = value
        .get("expect_states_with_tombstones")
        .and_then(Value::as_array)
    {
        let states_with_tombstones = reconstruct_object_states_from_base_and_delta_files(
            &base_bytes,
            &deltas,
            &CoveObjectReadOptions::default(),
            &CoveObjectReconstructionOptions {
                include_tombstones: true,
                ..CoveObjectReconstructionOptions::default()
            },
        )?;
        if states_with_tombstones.len() != expected_with_tombstones.len() {
            return Err(CoveError::BadSection(format!(
                "COVEDELTA reconstruction tombstone-inclusive state count mismatch: expected {}, got {}",
                expected_with_tombstones.len(),
                states_with_tombstones.len()
            )));
        }
        validate_expected_reconstructed_states(&states_with_tombstones, expected_with_tombstones)?;
    }

    if let Some(compacted_bytes) = parse_optional_fixture_byte_vector(&value, "compacted")? {
        let compacted_surface = read_object_surface_from_bytes(&compacted_bytes)?;
        let compacted_states = reconstruct_object_states(
            &compacted_surface,
            &CoveObjectReconstructionOptions::default(),
        )?;
        validate_expected_reconstructed_states(&compacted_states, expected_states)?;
        validate_compacted_state_equivalence(&states, &compacted_states)?;
    }

    Ok(())
}

fn validate_expected_delta_validation(
    deltas: &[CoveDeltaFile],
    value: &Value,
) -> Result<(), CoveError> {
    let expected_schema_fingerprints = value
        .get("expect_delta_effective_schema_fingerprint_refs")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let expected_object_catalog_fingerprints = value
        .get("expect_delta_effective_object_catalog_fingerprint_refs")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let expected_semantic_map_fingerprints = value
        .get("expect_delta_effective_semantic_map_fingerprint_refs")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let expected_projection_fingerprints = value
        .get("expect_delta_effective_projection_fingerprint_refs")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let expected_patch_counts = value
        .get("expect_delta_evidence_patch_counts")
        .and_then(Value::as_array);
    let expected_projection_patch_counts = value
        .get("expect_delta_projection_patch_counts")
        .and_then(Value::as_array);
    if expected_schema_fingerprints.is_none()
        && expected_object_catalog_fingerprints.is_none()
        && expected_semantic_map_fingerprints.is_none()
        && expected_projection_fingerprints.is_none()
        && expected_patch_counts.is_none()
        && expected_projection_patch_counts.is_none()
    {
        return Ok(());
    }

    validate_expected_delta_array_len(expected_schema_fingerprints, deltas.len())?;
    validate_expected_delta_array_len(expected_object_catalog_fingerprints, deltas.len())?;
    validate_expected_delta_array_len(expected_semantic_map_fingerprints, deltas.len())?;
    validate_expected_delta_array_len(expected_projection_fingerprints, deltas.len())?;
    if let Some(expected_patch_counts) = expected_patch_counts {
        if expected_patch_counts.len() != deltas.len() {
            return Err(CoveError::BadSection(
                "COVEDELTA expected evidence patch count must match deltas".into(),
            ));
        }
    }
    if let Some(expected_projection_patch_counts) = expected_projection_patch_counts {
        if expected_projection_patch_counts.len() != deltas.len() {
            return Err(CoveError::BadSection(
                "COVEDELTA expected projection patch count must match deltas".into(),
            ));
        }
    }

    for (index, delta) in deltas.iter().enumerate() {
        let validation = delta.validate_object_delta()?;
        validate_expected_effective_fingerprint(
            expected_schema_fingerprints,
            index,
            validation.effective_schema_fingerprint_ref,
            "schema",
        )?;
        validate_expected_effective_fingerprint(
            expected_object_catalog_fingerprints,
            index,
            validation.effective_object_catalog_fingerprint_ref,
            "object catalog",
        )?;
        validate_expected_effective_fingerprint(
            expected_semantic_map_fingerprints,
            index,
            validation.effective_semantic_map_fingerprint_ref,
            "semantic map",
        )?;
        validate_expected_effective_fingerprint(
            expected_projection_fingerprints,
            index,
            validation.effective_projection_fingerprint_ref,
            "projection",
        )?;
        if let Some(expected_patch_counts) = expected_patch_counts {
            let expected = expected_patch_counts[index]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(
                        "COVEDELTA expected evidence patch count must be usize".into(),
                    )
                })?;
            if validation.evidence_patches.len() != expected {
                return Err(CoveError::BadSection(
                    "COVEDELTA evidence patch count mismatch".into(),
                ));
            }
        }
        if let Some(expected_projection_patch_counts) = expected_projection_patch_counts {
            let expected = expected_projection_patch_counts[index]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(
                        "COVEDELTA expected projection patch count must be usize".into(),
                    )
                })?;
            if validation.projection_patches.len() != expected {
                return Err(CoveError::BadSection(
                    "COVEDELTA projection patch count mismatch".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_expected_delta_array_len(
    expected: Option<&[Value]>,
    delta_count: usize,
) -> Result<(), CoveError> {
    if let Some(expected) = expected {
        if expected.len() != delta_count {
            return Err(CoveError::BadSection(
                "COVEDELTA expected fingerprint count must match deltas".into(),
            ));
        }
    }
    Ok(())
}

fn validate_expected_effective_fingerprint(
    expected_fingerprints: Option<&[Value]>,
    index: usize,
    actual: u32,
    label: &str,
) -> Result<(), CoveError> {
    let Some(expected_fingerprints) = expected_fingerprints else {
        return Ok(());
    };
    let expected = expected_fingerprints[index]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            CoveError::BadSection(format!(
                "COVEDELTA expected {label} fingerprint ref must be u32"
            ))
        })?;
    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "COVEDELTA effective {label} fingerprint mismatch"
        )));
    }
    Ok(())
}

fn validate_expected_evidence_index(
    surface: &CoveObjectSurface,
    expected: &Value,
) -> Result<(), CoveError> {
    let evidence_index = surface.evidence_index.as_ref().ok_or_else(|| {
        CoveError::BadSection("COVEDELTA expected evidence index is absent".into())
    })?;
    if let Some(mapping_id) = expected.get("mapping_id").and_then(Value::as_str) {
        if evidence_index.mapping_id != mapping_id {
            return Err(CoveError::BadSection(
                "COVEDELTA evidence mapping_id mismatch".into(),
            ));
        }
    }
    if let Some(mapping_version) = expected.get("mapping_version").and_then(Value::as_str) {
        if evidence_index.mapping_version != mapping_version {
            return Err(CoveError::BadSection(
                "COVEDELTA evidence mapping_version mismatch".into(),
            ));
        }
    }
    if let Some(entry_count) = optional_usize(expected, "entry_count")? {
        if evidence_index.entries.len() != entry_count {
            return Err(CoveError::BadSection(
                "COVEDELTA evidence entry count mismatch".into(),
            ));
        }
    }
    if let Some(embedded_count) = optional_usize(expected, "embedded_evidence_section_count")? {
        let actual = surface
            .embedded_map_sections
            .iter()
            .filter(|section| matches!(section, EmbeddedMapSection::EvidenceIndex(_)))
            .count();
        if actual != embedded_count {
            return Err(CoveError::BadSection(
                "COVEDELTA embedded evidence section count mismatch".into(),
            ));
        }
    }
    if let Some(expected_entries) = expected.get("contains").and_then(Value::as_array) {
        for expected_entry in expected_entries {
            let source_id = json_field_str(expected_entry, "source_id")?;
            let source_row_identity = json_field_str(expected_entry, "source_row_identity")?;
            let rule_id = json_field_str(expected_entry, "rule_id")?;
            let assertion_id = json_field_str(expected_entry, "assertion_id")?;
            let output_object_id = json_field_str(expected_entry, "output_object_id")?;
            let entry = evidence_index
                .entries
                .iter()
                .find(|entry| {
                    entry.source_id == source_id
                        && entry.source_row_identity == source_row_identity
                        && entry.rule_id == rule_id
                        && entry.assertion_id == assertion_id
                        && entry.output_object_id == output_object_id
                })
                .ok_or_else(|| {
                    CoveError::BadSection("COVEDELTA expected evidence entry is absent".into())
                })?;
            if let Some(metadata) = expected_entry
                .get("operation_metadata")
                .and_then(Value::as_object)
            {
                for (key, expected_value) in metadata {
                    if entry.operation_metadata.get(key) != Some(expected_value) {
                        return Err(CoveError::BadSection(
                            "COVEDELTA evidence operation metadata mismatch".into(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_expected_reconstructed_states(
    states: &[CoveObjectState],
    expected_states: &[Value],
) -> Result<(), CoveError> {
    for expected in expected_states {
        let object_type_id = json_field_u32(expected, "object_type_id")?;
        let branch_key = required_u64_field(expected, "branch_key")?;
        let goid = fixture_array_16(expected, "goid")?;
        let state = states
            .iter()
            .find(|state| {
                state.object_type_id == object_type_id
                    && state.branch_key == branch_key
                    && state.goid == goid
            })
            .ok_or_else(|| {
                CoveError::BadSection(
                    "COVEDELTA reconstruction expected object state is absent".into(),
                )
            })?;

        if let Some(expected_object_type_name) =
            expected.get("object_type_name").and_then(Value::as_str)
        {
            if state.object_type_name != expected_object_type_name {
                return Err(CoveError::BadSection(
                    "COVEDELTA reconstruction object_type_name mismatch".into(),
                ));
            }
        }

        if let Some(expected_association_type) =
            expected.get("association_type").and_then(Value::as_str)
        {
            let actual_association_type = state
                .association
                .as_ref()
                .and_then(|association| association.association_type.as_deref());
            if actual_association_type != Some(expected_association_type) {
                return Err(CoveError::BadSection(
                    "COVEDELTA reconstruction association_type mismatch".into(),
                ));
            }
        }

        let expected_latest_record_id = fixture_array_16(expected, "latest_record_id")?;
        if state.latest_record_id != expected_latest_record_id {
            return Err(CoveError::BadSection(
                "COVEDELTA reconstruction latest_record_id mismatch".into(),
            ));
        }
        let expected_latest_timestamp_us = required_i64_field(expected, "latest_timestamp_us")?;
        if state.timestamp_us != expected_latest_timestamp_us {
            return Err(CoveError::BadSection(
                "COVEDELTA reconstruction latest timestamp mismatch".into(),
            ));
        }
        let expected_latest_csn = required_u64_field(expected, "latest_csn")?;
        if state.csn != expected_latest_csn {
            return Err(CoveError::BadSection(
                "COVEDELTA reconstruction latest csn mismatch".into(),
            ));
        }
        let expected_record_kind =
            record_kind_from_name(json_field_str(expected, "latest_record_kind")?)?;
        if state.record_kind != expected_record_kind {
            return Err(CoveError::BadSection(
                "COVEDELTA reconstruction latest record kind mismatch".into(),
            ));
        }
        let expected_tombstone_status =
            object_tombstone_status_from_name(json_field_str(expected, "tombstone_status")?)?;
        if state.tombstone_status != expected_tombstone_status {
            return Err(CoveError::BadSection(
                "COVEDELTA reconstruction tombstone_status mismatch".into(),
            ));
        }
        let expected_property_count = required_usize_field(expected, "property_count")?;
        if state.properties.len() != expected_property_count {
            return Err(CoveError::BadSection(format!(
                "COVEDELTA reconstruction property count mismatch: expected {}, got {}",
                expected_property_count,
                state.properties.len()
            )));
        }
    }

    Ok(())
}

fn validate_compacted_state_equivalence(
    delta_states: &[CoveObjectState],
    compacted_states: &[CoveObjectState],
) -> Result<(), CoveError> {
    if delta_states.len() != compacted_states.len() {
        return Err(CoveError::BadSection(format!(
            "COVEDELTA compaction state count mismatch: delta chain {}, compacted {}",
            delta_states.len(),
            compacted_states.len()
        )));
    }
    for state in delta_states {
        if !compacted_states
            .iter()
            .any(|compacted| object_state_logically_equal(state, compacted))
        {
            return Err(CoveError::BadSection(
                "COVEDELTA compacted state is not equivalent to base plus deltas".into(),
            ));
        }
    }
    Ok(())
}

fn object_state_logically_equal(left: &CoveObjectState, right: &CoveObjectState) -> bool {
    left.object_type_id == right.object_type_id
        && left.object_type_name == right.object_type_name
        && left.object_type_flags == right.object_type_flags
        && left.branch_key == right.branch_key
        && left.goid == right.goid
        && left.latest_record_id == right.latest_record_id
        && left.timestamp_us == right.timestamp_us
        && left.csn == right.csn
        && left.record_kind == right.record_kind
        && left.tombstone_status == right.tombstone_status
        && left.properties == right.properties
        && left.association == right.association
}

fn object_tombstone_status_from_name(value: &str) -> Result<CoveObjectTombstoneStatus, CoveError> {
    match value {
        "live" => Ok(CoveObjectTombstoneStatus::Live),
        "tombstoned" => Ok(CoveObjectTombstoneStatus::Tombstoned),
        other => Err(CoveError::BadSection(format!(
            "unknown COVE-O tombstone_status {other}"
        ))),
    }
}

pub(super) fn validate_covedelta_state_hash_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!(
            "invalid COVEDELTA state hash fixture json: {error}"
        ))
    })?;
    let descriptor_bytes = parse_fixture_byte_vector(value.get("descriptor"), "descriptor")?;
    let descriptor = DeltaStateHashDescriptorV1::parse(&descriptor_bytes)?;
    descriptor.validate_cove_object_delta_state_hash()?;
    let algorithm = DigestAlgorithm::from_u16(descriptor.hash_algorithm).ok_or_else(|| {
        CoveError::BadSection("state hash fixture descriptor algorithm is unknown".into())
    })?;
    let stored_hash = parse_fixture_byte_vector(value.get("stored_hash"), "stored_hash")?;
    if stored_hash.len() != usize::from(descriptor.hash_len) {
        return Err(CoveError::BadSection(
            "state hash fixture stored_hash length does not match descriptor".into(),
        ));
    }

    let state_value = value
        .get("state")
        .ok_or_else(|| CoveError::BadSection("state hash fixture missing state material".into()))?;
    let state = state_hash_material_from_json(state_value)?;
    let computed_hash = state.compute_hash(algorithm)?;
    if computed_hash != stored_hash {
        return Err(CoveError::BadSection(
            "state hash fixture stored_hash does not match recomputed canonical hash".into(),
        ));
    }
    Ok(())
}

fn state_hash_material_from_json(value: &Value) -> Result<CoveObjectDeltaStateHashV1, CoveError> {
    let property_values = value
        .get("properties")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("state hash fixture missing properties".into()))?;
    let mut properties = Vec::with_capacity(property_values.len());
    for property in property_values {
        properties.push(state_hash_property_from_json(property)?);
    }
    Ok(CoveObjectDeltaStateHashV1 {
        scope_kind: json_field_u16(value, "scope_kind")?,
        scope_id: fixture_array_16(value, "scope_id")?,
        canonical_branch_identity: parse_fixture_byte_vector(
            value.get("canonical_branch_identity"),
            "canonical_branch_identity",
        )?,
        object_type_id: json_field_u32(value, "object_type_id")?,
        goid: fixture_array_16(value, "goid")?,
        predecessor_record_id: fixture_array_16(value, "predecessor_record_id")?,
        predecessor_csn: required_u64_field(value, "predecessor_csn")?,
        predecessor_timestamp_us: required_i64_field(value, "predecessor_timestamp_us")?,
        record_kind: record_kind_from_name(json_field_str(value, "record_kind")?)?,
        tombstone_state: json_field_u8(value, "tombstone_state")?,
        properties,
    })
}

fn state_hash_property_from_json(
    value: &Value,
) -> Result<CoveObjectDeltaStateHashPropertyV1, CoveError> {
    Ok(CoveObjectDeltaStateHashPropertyV1 {
        property_id: json_field_u32(value, "property_id")?,
        logical_type: json_field_u16(value, "logical_type")?,
        collation_id: json_field_u32(value, "collation_id")?,
        value_state: json_field_u8(value, "value_state")?,
        canonical_value: parse_fixture_byte_vector(
            value.get("canonical_value"),
            "canonical_value",
        )?,
        redaction_commitment: parse_fixture_byte_vector(
            value.get("redaction_commitment"),
            "redaction_commitment",
        )?,
        hidden_value_commitment: parse_optional_fixture_byte_vector(
            value,
            "hidden_value_commitment",
        )?,
    })
}

fn required_u8_array(value: &Value, key: &str) -> Result<Vec<u8>, CoveError> {
    let array = value.get(key).ok_or_else(|| {
        CoveError::BadSection(format!("delta-chain selection fixture missing {key}"))
    })?;
    value_u8_array(array)
}

fn value_u8_array(value: &Value) -> Result<Vec<u8>, CoveError> {
    let array = value.as_array().ok_or_else(|| {
        CoveError::BadSection("delta-chain selection byte field must be an array".into())
    })?;
    array
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| {
                    CoveError::BadSection(
                        "delta-chain selection byte array contains non-u8 value".into(),
                    )
                })
        })
        .collect()
}

fn optional_u64(value: &Value, key: &str) -> Result<Option<u64>, CoveError> {
    match value.get(key) {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| CoveError::BadSection(format!("{key} must be a u64"))),
        None => Ok(None),
    }
}

fn optional_usize(value: &Value, key: &str) -> Result<Option<usize>, CoveError> {
    match value.get(key) {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| CoveError::BadSection(format!("{key} must be a usize"))),
        None => Ok(None),
    }
}

fn optional_i64(value: &Value, key: &str) -> Result<Option<i64>, CoveError> {
    match value.get(key) {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| CoveError::BadSection(format!("{key} must be an i64"))),
        None => Ok(None),
    }
}

fn optional_i64_range(
    value: &Value,
    start_key: &str,
    end_key: &str,
) -> Result<Option<(i64, i64)>, CoveError> {
    match (
        optional_i64(value, start_key)?,
        optional_i64(value, end_key)?,
    ) {
        (Some(start), Some(end)) => Ok(Some((start, end))),
        (None, None) => Ok(None),
        _ => Err(CoveError::BadSection(format!(
            "{start_key} and {end_key} must both be present or both be null"
        ))),
    }
}

fn required_u32_array(value: &Value, key: &str) -> Result<Vec<u32>, CoveError> {
    let array = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("{key} must be an array")))?;
    array
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| CoveError::BadSection(format!("{key} contains non-u32 value")))
        })
        .collect()
}

fn required_prune_skips(value: &Value) -> Result<Vec<CovmDeltaPruneSkip>, CoveError> {
    let array = value
        .get("expect_skipped")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("expect_skipped must be an array".into()))?;
    array
        .iter()
        .map(|skip| {
            let chain_ordinal = skip
                .get("chain_ordinal")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    CoveError::BadSection("expect_skipped chain_ordinal must be a u32".into())
                })?;
            let reason = match skip.get("reason").and_then(Value::as_str) {
                Some("as_of_csn_before_delta") => CovmDeltaPruneReason::AsOfCsnBeforeDelta,
                Some("as_of_commit_before_delta") => CovmDeltaPruneReason::AsOfCommitBeforeDelta,
                Some("source_publish_range_outside_delta") => {
                    CovmDeltaPruneReason::SourcePublishRangeOutsideDelta
                }
                Some(other) => {
                    return Err(CoveError::BadSection(format!(
                        "unknown delta pruning skip reason {other}"
                    )));
                }
                None => {
                    return Err(CoveError::BadSection(
                        "expect_skipped reason must be a string".into(),
                    ));
                }
            };
            Ok(CovmDeltaPruneSkip {
                chain_ordinal,
                reason,
            })
        })
        .collect()
}

fn validate_prune_metrics(metrics: CovmDeltaPruneMetrics, value: &Value) -> Result<(), CoveError> {
    let expected = CovmDeltaPruneMetrics {
        delta_chain_depth: required_usize_field(value, "delta_chain_depth")?,
        selected_delta_count: required_usize_field(value, "selected_delta_count")?,
        skipped_delta_count: required_usize_field(value, "skipped_delta_count")?,
        delta_artifacts_planned_to_open: required_usize_field(
            value,
            "delta_artifacts_planned_to_open",
        )?,
        delta_artifacts_skipped_before_open: required_usize_field(
            value,
            "delta_artifacts_skipped_before_open",
        )?,
        as_of_csn_prunes: required_usize_field(value, "as_of_csn_prunes")?,
        commit_time_range_prunes: required_usize_field(value, "commit_time_range_prunes")?,
        source_publish_range_prunes: required_usize_field(value, "source_publish_range_prunes")?,
    };
    if metrics != expected {
        return Err(CoveError::BadSection(format!(
            "delta pruning metrics mismatch: expected {:?}, got {:?}",
            expected, metrics
        )));
    }
    Ok(())
}

fn required_usize_field(value: &Value, key: &str) -> Result<usize, CoveError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| CoveError::BadSection(format!("{key} must be a usize")))
}

fn required_u64_field(value: &Value, key: &str) -> Result<u64, CoveError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| CoveError::BadSection(format!("{key} must be a u64")))
}

fn required_i64_field(value: &Value, key: &str) -> Result<i64, CoveError> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| CoveError::BadSection(format!("{key} must be an i64")))
}
