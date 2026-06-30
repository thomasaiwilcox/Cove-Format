use super::*;

pub(super) fn validate_feature_scope_use_fixture(
    entry: &Entry,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let mut request = FeatureUseRequestV2::new();
    if let Some(profile) = entry.raw.get("requested_profile").and_then(Value::as_u64) {
        request.requested_profile = Some(u8::try_from(profile).map_err(|_| {
            CoveError::BadSection("feature-scope fixture profile out of range".into())
        })?);
    }
    if let Some(operation) = entry.raw.get("requested_operation").and_then(Value::as_u64) {
        let operation = u16::try_from(operation).map_err(|_| {
            CoveError::BadSection("feature-scope fixture operation out of range".into())
        })?;
        request.requested_operation =
            Some(OperationKindV2::from_u16(operation).ok_or_else(|| {
                CoveError::BadSection("feature-scope fixture operation is unknown".into())
            })?);
    }
    if let Some(sections) = entry.raw.get("needed_sections").and_then(Value::as_array) {
        for section in sections {
            let section_id = section.as_u64().ok_or_else(|| {
                CoveError::BadSection("feature-scope fixture section id is not numeric".into())
            })?;
            request
                .needed_section_ids
                .insert(u32::try_from(section_id).map_err(|_| {
                    CoveError::BadSection("feature-scope fixture section id out of range".into())
                })?);
        }
    }
    if let Some(pages) = entry.raw.get("needed_pages").and_then(Value::as_array) {
        for page in pages {
            let pair = page.as_array().ok_or_else(|| {
                CoveError::BadSection("feature-scope fixture page ref must be an array".into())
            })?;
            if pair.len() != 2 {
                return Err(CoveError::BadSection(
                    "feature-scope fixture page ref must have two values".into(),
                ));
            }
            let section_id = pair[0].as_u64().ok_or_else(|| {
                CoveError::BadSection("feature-scope fixture page section is not numeric".into())
            })?;
            let target = pair[1].as_u64().ok_or_else(|| {
                CoveError::BadSection("feature-scope fixture page target is not numeric".into())
            })?;
            request
                .needed_page_refs
                .insert(cove_core::feature_scope::FeatureTargetRefV2::new(
                    u32::try_from(section_id).map_err(|_| {
                        CoveError::BadSection(
                            "feature-scope fixture page section out of range".into(),
                        )
                    })?,
                    target,
                ));
        }
    }
    let validator = EmbeddedOptionalProfileValidator::default_builtins();
    let report = reader::validate_bytes_for_feature_use_with_optional_profile_validator(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            optional_pushdown_policy: OptionalPushdownPolicy::FailOpen,
        },
        request.clone(),
        &validator,
    )?;
    validator.validate_embedded_optional_profile_sections(
        bytes,
        &report,
        OptionalPushdownPolicy::FailOpen,
        Some(&request),
        false,
    )
}

pub(super) fn validate_extension_registry_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let registry = ExtensionRegistry::parse(bytes)?;
    registry.validate_known(true)
}

struct HarborFixtureResolver;

impl ExecutionCodeResolver for HarborFixtureResolver {
    fn resolve(&self, request: ExecutionCodeRequest<'_>) -> Result<ExecutionCodeValue, CoveError> {
        Ok(ExecutionCodeValue::Unsigned(
            10_000 + u64::from(request.file_code),
        ))
    }
}

pub(super) fn validate_harbor_mount_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let mounted = mount_cove_h_file(
        bytes,
        MountOptions::default(),
        HarborMountOptions::default(),
        Some(&HarborFixtureResolver),
    )?;
    if mounted.harbor_maps.is_empty() {
        return Err(CoveError::BadSection(
            "harbor mount fixture did not build any maps".into(),
        ));
    }
    let reused = mount_cove_h_file(
        bytes,
        MountOptions::default(),
        HarborMountOptions {
            existing_maps: Some(&mounted.harbor_maps),
            rebuild_missing_or_stale: false,
        },
        None,
    )?;
    if reused.harbor_maps != mounted.harbor_maps {
        return Err(CoveError::BadSection(
            "harbor mount fixture did not reuse valid maps".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_lakehouse_overlay_guard_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let hints = LakehouseHints::parse(bytes)?;
    if hints.visibility_overlay.is_none() {
        return Err(CoveError::BadSection(
            "lakehouse overlay guard fixture missing overlay".into(),
        ));
    }
    let expected = [
        (
            LakehouseMetadataUse::PhysicalPruning,
            false,
            false,
            LakehouseOverlayDecision::Allow,
        ),
        (
            LakehouseMetadataUse::LookupOrInvertedCandidates,
            false,
            false,
            LakehouseOverlayDecision::RequireOverlayApplication,
        ),
        (
            LakehouseMetadataUse::VisibleExactDomain,
            false,
            false,
            LakehouseOverlayDecision::ForbidVisibleExactness,
        ),
        (
            LakehouseMetadataUse::VisibleAggregateAnswer,
            false,
            true,
            LakehouseOverlayDecision::Allow,
        ),
    ];
    for (metadata_use, overlay_empty, overlay_aware, decision) in expected {
        if hints.overlay_decision(metadata_use, overlay_empty, overlay_aware) != decision {
            return Err(CoveError::BadSection(
                "lakehouse overlay guard decision mismatch".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_zero_copy_compat_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid zero-copy fixture json: {error}"))
    })?;
    let section_bytes = json_byte_vec(&value, "section")?;
    let map = ZeroCopyBufferMapV2::parse(&section_bytes)?;
    let entry = map
        .entries
        .first()
        .ok_or_else(|| CoveError::BadSection("zero-copy fixture has no entries".into()))?;
    let context = ZeroCopyCompatibilityContext {
        active_visibility_overlay: value
            .get("active_visibility_overlay")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        accepts_cove_null_bitmap_polarity: value
            .get("accepts_cove_null_bitmap_polarity")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        expected_dictionary_semantics: ZeroCopyCompatibilityContext::default()
            .expected_dictionary_semantics,
        expected_nested_layout_kind: ZeroCopyCompatibilityContext::default()
            .expected_nested_layout_kind,
        required_lifetime_scope: if value
            .get("require_reader_session_lifetime")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            cove_layout::ZeroCopyLifetimeScopeV2::ReaderSession
        } else {
            ZeroCopyCompatibilityContext::default().required_lifetime_scope
        },
    };
    let actual = match entry.compatibility(&context) {
        ZeroCopyCompatibilityV2::Compatible => "Compatible",
        ZeroCopyCompatibilityV2::MaterializeRequired(reason) => materialization_reason_name(reason),
    };
    let expected = value
        .get("expect")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("zero-copy fixture missing expect".into()))?;
    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "zero-copy compatibility mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn materialization_reason_name(reason: ZeroCopyMaterializationReasonV2) -> &'static str {
    match reason {
        ZeroCopyMaterializationReasonV2::UnknownRole => "UnknownRole",
        ZeroCopyMaterializationReasonV2::NullPolarityMismatch => "NullPolarityMismatch",
        ZeroCopyMaterializationReasonV2::CompressedBuffer => "CompressedBuffer",
        ZeroCopyMaterializationReasonV2::DictionaryMismatch => "DictionaryMismatch",
        ZeroCopyMaterializationReasonV2::NestedLayoutMismatch => "NestedLayoutMismatch",
        ZeroCopyMaterializationReasonV2::InsufficientLifetime => "InsufficientLifetime",
        ZeroCopyMaterializationReasonV2::ActiveVisibilityOverlay => "ActiveVisibilityOverlay",
    }
}

pub(super) fn validate_coverage_proof_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid coverage proof fixture json: {error}"))
    })?;
    let coverage_set_bytes = json_byte_vec(&value, "coverage_set")?;
    let proof_record_bytes = json_byte_vec(&value, "proof_record")?;
    let record = CoverageProofRecordV2::parse(&proof_record_bytes)?;
    record.validate_against_coverage_set_bytes(&coverage_set_bytes)?;

    if let Some(selected_snapshot) = value
        .get("selected_snapshot_validity_ref")
        .and_then(Value::as_u64)
    {
        if selected_snapshot > u32::MAX as u64
            || selected_snapshot as u32 != record.snapshot_validity_ref
        {
            return Err(CoveError::CoverageStale);
        }
    }

    if let Some(expected_pruning_safe) = value.get("expect_pruning_safe").and_then(Value::as_bool) {
        let actual = can_use_proof_for_pruning(&record);
        if actual != expected_pruning_safe {
            return Err(CoveError::BadCoverage);
        }
    }
    Ok(())
}

pub(super) fn validate_sidecar_validity_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid sidecar validity fixture json: {error}"))
    })?;
    let covi_bytes = json_byte_vec(&value, "covi")?;
    let artifact = CoviArtifactV2::parse(&covi_bytes)?;
    let dataset_matches = value
        .get("dataset_matches")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let snapshot_matches = value
        .get("snapshot_matches")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let digest_matches = value
        .get("digest_matches")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let schema_matches = value
        .get("schema_matches")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let semantic_map_matches = value
        .get("semantic_map_matches")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let valid_identity = artifact.header.dataset_id == [0x11; 16]
        && artifact.header.snapshot_id == [0x22; 16]
        && dataset_matches
        && snapshot_matches
        && digest_matches
        && schema_matches
        && semantic_map_matches;
    let actual = if valid_identity {
        "Valid"
    } else {
        "StaleIgnored"
    };
    let expected = value
        .get("expect")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("sidecar validity fixture missing expect".into()))?;
    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "sidecar validity mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

pub(super) fn covi_validation_context_from_fixture_value(
    context_value: &Value,
) -> Result<CoviValidationContextV2, CoveError> {
    let mut context = CoviValidationContextV2::for_file(
        json_uuid(context_value, "file_id")?,
        json_u64(context_value, "file_len")?,
        json_u64(context_value, "footer_crc32c")? as u32,
    );
    if let Some(dataset_id) = optional_uuid(context_value, "dataset_id")? {
        context = context.with_dataset_id(dataset_id);
    }
    if let Some(snapshot_id) = optional_uuid(context_value, "snapshot_id")? {
        context = context.with_snapshot_id(snapshot_id);
    }
    if let Some(value) = optional_u32(context_value, "schema_fingerprint_ref")? {
        context = context.with_schema_fingerprint_ref(value);
    }
    if let Some(value) = optional_u32(context_value, "semantic_map_fingerprint_ref")? {
        context = context.with_semantic_map_fingerprint_ref(value);
    }
    if let Some(value) = optional_u32(context_value, "external_visibility_ref")? {
        context = context.with_external_visibility_ref(value);
    }
    if let Some(value) = context_value.get("now_us").and_then(Value::as_i64) {
        context = context.with_now_us(value);
    }
    if context_value
        .get("allow_file_code_keys")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        context = context.with_file_code_keys(true);
    }
    if let Some(digest) = context_value.get("file_digest") {
        let algorithm = optional_u32(context_value, "file_digest_algorithm")?
            .and_then(|value| DigestAlgorithm::from_u16(value as u16))
            .ok_or_else(|| CoveError::BadSection("invalid file_digest_algorithm".into()))?;
        let bytes: Vec<u8> = serde_json::from_value(digest.clone()).map_err(|error| {
            CoveError::BadSection(format!("fixture field file_digest is not bytes: {error}"))
        })?;
        context = context.with_file_digest(algorithm, bytes);
    }
    if let Some(digest) = context_value.get("delta_chain_digest") {
        let algorithm = optional_u32(context_value, "delta_chain_digest_algorithm")?
            .and_then(|value| DigestAlgorithm::from_u16(value as u16))
            .ok_or_else(|| CoveError::BadSection("invalid delta_chain_digest_algorithm".into()))?;
        let bytes: Vec<u8> = serde_json::from_value(digest.clone()).map_err(|error| {
            CoveError::BadSection(format!(
                "fixture field delta_chain_digest is not bytes: {error}"
            ))
        })?;
        context = context.with_delta_chain_digest(algorithm, bytes);
    }
    Ok(context)
}

pub(super) fn validate_covi_validation_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid COVI validation fixture json: {error}"))
    })?;
    let covi_bytes = json_byte_vec(&value, "covi")?;
    let context_value = value
        .get("context")
        .ok_or_else(|| CoveError::BadSection("COVI fixture missing context".into()))?;
    let context = covi_validation_context_from_fixture_value(context_value)?;
    let expected = value
        .get("expected_result")
        .and_then(Value::as_str)
        .unwrap_or("valid");
    let validated = match ValidatedCoviArtifactV2::parse_and_validate(&covi_bytes, context) {
        Ok(validated) => {
            if expected != "valid" {
                return Err(CoveError::BadSection(format!(
                    "COVI fixture expected {expected}, got valid"
                )));
            }
            validated
        }
        Err(error) => {
            if expected == "stale_ignored" {
                return Ok(());
            }
            return Err(error);
        }
    };
    match value
        .get("operation")
        .and_then(Value::as_object)
        .and_then(|object| object.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("validate")
    {
        "validate" => Ok(()),
        "lookup_eq" => {
            let operation = value
                .get("operation")
                .ok_or_else(|| CoveError::BadSection("COVI fixture missing operation".into()))?;
            let key = json_byte_vec(operation, "key")?;
            let request = CoviLookupRequestV2::eq(
                json_u64(operation, "table_id")? as u32,
                json_u64(operation, "column_id")? as u32,
                CoviLookupKeyV2::CanonicalValueBytes(key),
            );
            let candidates = validated.lookup(&request)?;
            if let Some(expected_count) =
                value.get("expect_row_range_count").and_then(Value::as_u64)
            {
                if candidates.row_ranges.len() as u64 != expected_count {
                    return Err(CoveError::BadCovi);
                }
            }
            Ok(())
        }
        "index_only" => {
            let operation = value
                .get("operation")
                .ok_or_else(|| CoveError::BadSection("COVI fixture missing operation".into()))?;
            let aggregate_kind = match operation
                .get("aggregate_kind")
                .and_then(Value::as_str)
                .unwrap_or("count")
            {
                "count" => CoviAggregateKindV2::Count,
                "min" => CoviAggregateKindV2::Min,
                "max" => CoviAggregateKindV2::Max,
                "exists" => CoviAggregateKindV2::Exists,
                "sum" => CoviAggregateKindV2::Sum,
                "avg" => CoviAggregateKindV2::Avg,
                other => {
                    return Err(CoveError::BadSection(format!(
                        "unsupported aggregate_kind {other}"
                    )));
                }
            };
            let request = CoviIndexOnlyRequestV2 {
                table_id: json_u64(operation, "table_id")? as u32,
                column_id: optional_u32(operation, "column_id")?,
                aggregate_kind,
                predicate_form_ref: optional_u32(operation, "predicate_form_ref")?,
                require_exact: operation
                    .get("require_exact")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            };
            let answer = validated
                .index_only_answer(&request)?
                .ok_or(CoveError::BadCovi)?;
            if let Some(expected_row_count) = value.get("expect_row_count").and_then(Value::as_u64)
            {
                if answer.row_count != expected_row_count {
                    return Err(CoveError::BadCovi);
                }
            }
            Ok(())
        }
        other => Err(CoveError::BadSection(format!(
            "unsupported COVI validation operation {other}"
        ))),
    }
}

fn json_uuid(value: &Value, field: &str) -> Result<[u8; 16], CoveError> {
    let bytes: Vec<u8> = serde_json::from_value(
        value
            .get(field)
            .cloned()
            .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field}")))?,
    )
    .map_err(|error| {
        CoveError::BadSection(format!("fixture field {field} is not bytes: {error}"))
    })?;
    bytes
        .try_into()
        .map_err(|_| CoveError::BadSection(format!("fixture field {field} must be 16 bytes")))
}

fn optional_uuid(value: &Value, field: &str) -> Result<Option<[u8; 16]>, CoveError> {
    if value.get(field).is_none() {
        return Ok(None);
    }
    json_uuid(value, field).map(Some)
}

pub(super) fn json_u64(value: &Value, field: &str) -> Result<u64, CoveError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing integer {field}")))
}

pub(super) fn optional_u32(value: &Value, field: &str) -> Result<Option<u32>, CoveError> {
    let Some(raw) = value.get(field).and_then(Value::as_u64) else {
        return Ok(None);
    };
    u32::try_from(raw)
        .map(Some)
        .map_err(|_| CoveError::BadSection(format!("fixture field {field} exceeds u32")))
}

pub(super) fn validate_visibility_safety_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid visibility fixture json: {error}"))
    })?;
    let active_overlay = value
        .get("active_visibility_overlay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let overlay_aware = value
        .get("overlay_aware")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let exact_answers_allowed = !active_overlay || overlay_aware;
    for field in ["expect_index_only_allowed", "expect_metadata_only_allowed"] {
        let expected = value
            .get(field)
            .and_then(Value::as_bool)
            .ok_or_else(|| CoveError::BadSection(format!("visibility fixture missing {field}")))?;
        if expected != exact_answers_allowed {
            return Err(CoveError::BadSection(format!(
                "visibility fixture {field} mismatch"
            )));
        }
    }
    let section_bytes = json_byte_vec(&value, "zero_copy_section")?;
    let map = ZeroCopyBufferMapV2::parse(&section_bytes)?;
    let entry = map.entries.first().ok_or_else(|| {
        CoveError::BadSection("visibility fixture has no zero-copy entries".into())
    })?;
    let actual = match entry.compatibility(&ZeroCopyCompatibilityContext {
        active_visibility_overlay: active_overlay && !overlay_aware,
        ..ZeroCopyCompatibilityContext::default()
    }) {
        ZeroCopyCompatibilityV2::Compatible => "Compatible",
        ZeroCopyCompatibilityV2::MaterializeRequired(reason) => materialization_reason_name(reason),
    };
    let expected = value
        .get("expect_zero_copy")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CoveError::BadSection("visibility fixture missing expect_zero_copy".into())
        })?;
    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "visibility zero-copy mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

pub(super) fn json_byte_vec(value: &Value, field: &str) -> Result<Vec<u8>, CoveError> {
    serde_json::from_value(
        value
            .get(field)
            .cloned()
            .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field}")))?,
    )
    .map_err(|error| CoveError::BadSection(format!("fixture field {field} is not bytes: {error}")))
}

pub(super) fn validate_runtime_operation_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| CoveError::BadSection(format!("invalid runtime fixture json: {error}")))?;
    let hints = value
        .get("hints")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("runtime fixture missing hints".into()))?
        .iter()
        .map(runtime_hint_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    validate_hints(&hints)?;

    let supported = value
        .get("supported")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("runtime fixture missing supported list".into()))?
        .iter()
        .map(runtime_supported_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let unsupported = unsupported_required_hints(
        &hints,
        supported
            .iter()
            .map(|(kind, namespace, name, major, minor)| {
                (*kind, namespace.as_str(), name.as_str(), *major, *minor)
            }),
    );
    let actual = unsupported
        .into_iter()
        .map(|hint| hint.hint_id)
        .collect::<BTreeSet<_>>();
    let expected = value
        .get("expect_unsupported_required_hint_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("runtime fixture missing expected ids".into()))?
        .iter()
        .map(|item| json_u32(item, "runtime expected hint id"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if actual != expected {
        return Err(CoveError::BadSection(format!(
            "runtime unsupported hint ids mismatch: actual={actual:?} expected={expected:?}"
        )));
    }
    Ok(())
}

fn runtime_hint_from_json(value: &Value) -> Result<RuntimeCompatibilityHintV2, CoveError> {
    Ok(RuntimeCompatibilityHintV2 {
        hint_id: json_field_u32(value, "hint_id")?,
        hint_kind: runtime_hint_kind_from_str(json_field_str(value, "kind")?)?,
        required: value
            .get("required")
            .and_then(Value::as_bool)
            .ok_or_else(|| CoveError::BadSection("runtime hint missing required".into()))?,
        flags: 0,
        namespace: json_field_str(value, "namespace")?.to_string(),
        name: json_field_str(value, "name")?.to_string(),
        version_major: json_field_u16(value, "version_major")?,
        version_minor: json_field_u16(value, "version_minor")?,
        payload_ref: u32::MAX,
        checksum: 0,
    })
}

fn runtime_supported_from_json(
    value: &Value,
) -> Result<(RuntimeHintKindV2, String, String, u16, u16), CoveError> {
    Ok((
        runtime_hint_kind_from_str(json_field_str(value, "kind")?)?,
        json_field_str(value, "namespace")?.to_string(),
        json_field_str(value, "name")?.to_string(),
        json_field_u16(value, "version_major")?,
        json_field_u16(value, "version_minor")?,
    ))
}

fn runtime_hint_kind_from_str(value: &str) -> Result<RuntimeHintKindV2, CoveError> {
    match value {
        "codec_registry" => Ok(RuntimeHintKindV2::CodecRegistry),
        "layout_registry" => Ok(RuntimeHintKindV2::LayoutRegistry),
        "predicate_kernel" => Ok(RuntimeHintKindV2::PredicateKernel),
        "engine_adapter" => Ok(RuntimeHintKindV2::EngineAdapter),
        "ffi_surface" => Ok(RuntimeHintKindV2::FfiSurface),
        "language_binding" => Ok(RuntimeHintKindV2::LanguageBinding),
        "wasm_or_external_kernel_package" => Ok(RuntimeHintKindV2::WasmOrExternalKernelPackage),
        _ => Err(CoveError::BadSection(format!(
            "unknown runtime hint kind {value}"
        ))),
    }
}

pub(super) fn json_field_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, CoveError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection(format!("missing string field {field}")))
}

pub(super) fn json_field_u16(value: &Value, field: &str) -> Result<u16, CoveError> {
    let raw = value
        .get(field)
        .ok_or_else(|| CoveError::BadSection(format!("missing integer field {field}")))?;
    let number = json_u32(raw, field)?;
    u16::try_from(number)
        .map_err(|_| CoveError::BadSection(format!("integer field {field} exceeds u16::MAX")))
}

pub(super) fn json_field_u32(value: &Value, field: &str) -> Result<u32, CoveError> {
    let raw = value
        .get(field)
        .ok_or_else(|| CoveError::BadSection(format!("missing integer field {field}")))?;
    json_u32(raw, field)
}

pub(super) fn json_field_u8(value: &Value, field: &str) -> Result<u8, CoveError> {
    let raw = value
        .get(field)
        .ok_or_else(|| CoveError::BadSection(format!("missing integer field {field}")))?;
    raw.as_u64()
        .and_then(|number| u8::try_from(number).ok())
        .ok_or_else(|| CoveError::BadSection(format!("{field} is not a u8")))
}

pub(super) fn json_field_bool(value: &Value, field: &str) -> Result<bool, CoveError> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| CoveError::BadSection(format!("{field} is not a bool")))
}

pub(super) fn json_u32(value: &Value, field: &str) -> Result<u32, CoveError> {
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| CoveError::BadSection(format!("{field} is not a u32")))
}

pub(super) fn optional_u32_array(
    value: &Value,
    field: &str,
) -> Result<Option<Vec<u32>>, CoveError> {
    let Some(items) = value.get(field) else {
        return Ok(None);
    };
    let items = items
        .as_array()
        .ok_or_else(|| CoveError::BadSection(format!("{field} is not a u32 array")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| json_u32(item, &format!("{field}[{index}]")))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(super) fn validate_sidecar_freshness_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid sidecar-freshness fixture json: {error}"))
    })?;
    let cove = parse_fixture_byte_vector(value.get("cove"), "cove")?;
    let covx = parse_optional_fixture_byte_vector(&value, "covx")?;
    let covm = parse_optional_fixture_byte_vector(&value, "covm")?;
    let expected_covx = parse_sidecar_status(
        value
            .get("expect_covx")
            .and_then(Value::as_str)
            .ok_or_else(|| CoveError::BadSection("sidecar fixture missing expect_covx".into()))?,
    )?;
    let expected_covm = parse_sidecar_status(
        value
            .get("expect_covm")
            .and_then(Value::as_str)
            .ok_or_else(|| CoveError::BadSection("sidecar fixture missing expect_covm".into()))?,
    )?;
    let mounted = mount_cove_file(
        &cove,
        MountOptions {
            covx: covx.as_deref(),
            covm: covm.as_deref(),
            ..MountOptions::default()
        },
        None,
    )?;
    if mounted.covx_status != expected_covx {
        return Err(CoveError::BadSection(format!(
            "expected covx status {}, got {}",
            sidecar_status_name(expected_covx),
            sidecar_status_name(mounted.covx_status)
        )));
    }
    if mounted.covm_status != expected_covm {
        return Err(CoveError::BadSection(format!(
            "expected covm status {}, got {}",
            sidecar_status_name(expected_covm),
            sidecar_status_name(mounted.covm_status)
        )));
    }
    Ok(())
}

fn parse_sidecar_status(value: &str) -> Result<SidecarValidationStatus, CoveError> {
    match value {
        "NotProvided" => Ok(SidecarValidationStatus::NotProvided),
        "Valid" => Ok(SidecarValidationStatus::Valid),
        "StaleIgnored" => Ok(SidecarValidationStatus::StaleIgnored),
        other => Err(CoveError::BadSection(format!(
            "unknown sidecar status {other}"
        ))),
    }
}

fn sidecar_status_name(status: SidecarValidationStatus) -> &'static str {
    match status {
        SidecarValidationStatus::NotProvided => "NotProvided",
        SidecarValidationStatus::Valid => "Valid",
        SidecarValidationStatus::StaleIgnored => "StaleIgnored",
        _ => "Future",
    }
}
