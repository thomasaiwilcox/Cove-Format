use super::*;

pub(super) fn validate_cove_map_convert_fixture(
    corpus: &Path,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid cove-map convert fixture json: {error}"))
    })?;
    let (map, sources) = cove_map_fixture_paths(corpus, &value)?;
    let summary = cove_map::conversion_summary_from_paths(&map, &sources)
        .map_err(|_| CoveError::MapInvalid)?;
    let report = summary
        .get("report")
        .ok_or_else(|| CoveError::BadSection("conversion summary missing report".into()))?;
    if let Some(expected_report) = value.get("expected_conversion") {
        validate_expected_json_fields(report, expected_report)?;
    }
    if let Some(expected_summary) = value.get("expected_conversion_summary") {
        validate_expected_json_fields(&summary, expected_summary)?;
    }
    if let Some(expected_entries) = value.get("expected_evidence_entries") {
        validate_expected_evidence_entries(&summary, expected_entries)?;
    }
    if value
        .get("expect_cove_o_valid")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let bytes =
            cove_map::cove_o_from_paths(&map, &sources).map_err(|_| CoveError::MapInvalid)?;
        let report = reader::validate_bytes_with_options(
            &bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: false,
                allow_unknown_optional_extensions: true,
                ..ValidationOptions::default()
            },
        )?;
        if value
            .get("expect_semantic_map_optional")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let required = report.validated.header.required_features;
            let optional = report.validated.header.optional_features;
            if required & cove_core::constants::FEATURE_SEMANTIC_MAP != 0
                || optional & cove_core::constants::FEATURE_SEMANTIC_MAP == 0
            {
                return Err(CoveError::MapInvalid);
            }
            if report.validated.footer.sections.iter().any(|entry| {
                entry.required_features & cove_core::constants::FEATURE_SEMANTIC_MAP != 0
            }) {
                return Err(CoveError::MapInvalid);
            }
        }
        if value
            .get("expect_association_readback_flags")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            validate_association_readback_flags(&bytes, &report.validated)?;
        }
    }
    Ok(())
}

pub(super) fn validate_cove_map_candidates_fixture(
    corpus: &Path,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid cove-map candidates fixture json: {error}"))
    })?;
    let (map, sources) = cove_map_fixture_paths(corpus, &value)?;
    let candidates = cove_map::candidate_matches_from_paths(&map, &sources)
        .map_err(|_| CoveError::MapInvalid)?;

    if let Some(expected) = value.get("expected_candidates") {
        validate_expected_candidate_matches(&candidates, expected)?;
    }
    Ok(())
}

pub(super) fn validate_cove_map_replay_fixture(
    corpus: &Path,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid cove-map replay fixture json: {error}"))
    })?;
    let (map, sources) = cove_map_fixture_paths(corpus, &value)?;
    let mut report = cove_map::conversion_report_from_paths(&map, &sources)
        .map_err(|_| CoveError::MapInvalid)?;
    if let Some(mutation) = value.get("mutate_report").and_then(Value::as_str) {
        mutate_replay_report(&mut report, mutation)?;
    }

    match cove_map::verify_replay_report_from_paths(&map, &report) {
        Ok(actual) => {
            if let Some(expected) = value.get("expected_replay") {
                validate_expected_json_fields(&actual, expected)?;
            }
            Ok(())
        }
        Err(error) => {
            if let Some(expected) = value.get("expected_error_contains").and_then(Value::as_str) {
                let error = error.to_string();
                if !error.contains(expected) {
                    return Err(CoveError::BadSection(format!(
                        "unexpected cove-map replay error: {error}"
                    )));
                }
            }
            Err(CoveError::MapInvalid)
        }
    }
}

fn mutate_replay_report(report: &mut Value, mutation: &str) -> Result<(), CoveError> {
    const BAD_DIGEST: &str =
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    match mutation {
        "resolver_digest" => {
            let Some(digests) = report
                .get_mut("resolver_catalog_digests")
                .and_then(Value::as_array_mut)
            else {
                return Err(CoveError::BadSection(
                    "replay fixture report missing resolver digests".into(),
                ));
            };
            let Some(first) = digests.first_mut().and_then(Value::as_object_mut) else {
                return Err(CoveError::BadSection(
                    "replay fixture report has no resolver digest entry".into(),
                ));
            };
            first.insert("resolver_digest".into(), Value::String(BAD_DIGEST.into()));
        }
        "reviewed_decision_digest" => {
            let Some(object) = report.as_object_mut() else {
                return Err(CoveError::BadSection(
                    "replay fixture report must be an object".into(),
                ));
            };
            object.insert(
                "reviewed_decision_catalog_digest".into(),
                Value::String(BAD_DIGEST.into()),
            );
        }
        "source_snapshot_digest" => {
            let Some(sources) = report.get_mut("sources").and_then(Value::as_array_mut) else {
                return Err(CoveError::BadSection(
                    "replay fixture report missing sources".into(),
                ));
            };
            let Some(first) = sources.first_mut().and_then(Value::as_object_mut) else {
                return Err(CoveError::BadSection(
                    "replay fixture report has no source entry".into(),
                ));
            };
            first.insert("snapshot_digest".into(), Value::String(BAD_DIGEST.into()));
        }
        other => {
            return Err(CoveError::BadSection(format!(
                "unknown replay report mutation '{other}'"
            )));
        }
    }
    Ok(())
}

fn validate_expected_candidate_matches(actual: &Value, expected: &Value) -> Result<(), CoveError> {
    let expected = expected.as_object().ok_or_else(|| {
        CoveError::BadSection("expected candidate fixture fields must be an object".into())
    })?;
    let matches = actual
        .get("candidate_matches")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("candidate output missing matches".into()))?;
    let diagnostics = actual
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("candidate output missing diagnostics".into()))?;

    if let Some(expected_count) = expected
        .get("candidate_match_count")
        .and_then(Value::as_u64)
    {
        if matches.len() as u64 != expected_count {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    if let Some(expected_count) = expected.get("diagnostic_count").and_then(Value::as_u64) {
        if diagnostics.len() as u64 != expected_count {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    if let Some(expected_first) = expected.get("first_match") {
        let first = matches.first().ok_or(CoveError::MapEvidenceInvalid)?;
        validate_expected_json_fields(first, expected_first)?;
    }
    if let Some(expected_left) = expected.get("first_left") {
        let first = matches.first().ok_or(CoveError::MapEvidenceInvalid)?;
        let left = first
            .get("left")
            .ok_or_else(|| CoveError::BadSection("candidate match missing left".into()))?;
        validate_expected_json_fields(left, expected_left)?;
    }
    if let Some(expected_right) = expected.get("first_right") {
        let first = matches.first().ok_or(CoveError::MapEvidenceInvalid)?;
        let right = first
            .get("right")
            .ok_or_else(|| CoveError::BadSection("candidate match missing right".into()))?;
        validate_expected_json_fields(right, expected_right)?;
    }
    if let Some(expected_order) = expected.get("match_order") {
        let expected_order = expected_order.as_array().ok_or_else(|| {
            CoveError::BadSection("expected candidate match_order must be an array".into())
        })?;
        if matches.len() != expected_order.len() {
            return Err(CoveError::MapEvidenceInvalid);
        }
        for (actual_match, expected_match) in matches.iter().zip(expected_order) {
            validate_expected_candidate_order_entry(actual_match, expected_match)?;
        }
    }
    Ok(())
}

fn validate_expected_candidate_order_entry(
    actual: &Value,
    expected: &Value,
) -> Result<(), CoveError> {
    if let Some(fields) = expected.get("match") {
        validate_expected_json_fields(actual, fields)?;
    }
    if let Some(fields) = expected.get("left") {
        let left = actual
            .get("left")
            .ok_or_else(|| CoveError::BadSection("candidate match missing left".into()))?;
        validate_expected_json_fields(left, fields)?;
    }
    if let Some(fields) = expected.get("right") {
        let right = actual
            .get("right")
            .ok_or_else(|| CoveError::BadSection("candidate match missing right".into()))?;
        validate_expected_json_fields(right, fields)?;
    }
    Ok(())
}

fn validate_association_readback_flags(
    bytes: &[u8],
    validated: &reader::ValidatedCoveFile,
) -> Result<(), CoveError> {
    let entry = validated
        .footer
        .sections
        .iter()
        .find(|entry| entry.section_kind == SectionKind::ObjectTypeCatalog as u16)
        .ok_or_else(|| CoveError::BadSection("missing object type catalog".into()))?;
    let payload = section_payload(bytes, entry)?;
    let catalog = ObjectTypeCatalog::parse(&payload)?;
    let association = catalog
        .types
        .iter()
        .find(|ty| ty.type_name.starts_with("Association:"))
        .ok_or(CoveError::MapEvidenceInvalid)?;
    let required_type_flags = OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT;
    if association.flags & required_type_flags != required_type_flags {
        return Err(CoveError::MapEvidenceInvalid);
    }
    let required_property_flags = [
        ("source_goid", PROPERTY_FLAG_ASSOCIATION_FROM_GOID),
        ("target_goid", PROPERTY_FLAG_ASSOCIATION_TO_GOID),
        ("association_type", PROPERTY_FLAG_ASSOCIATION_TYPE),
        ("mapping_rule_id", PROPERTY_FLAG_MAPPING_RULE_REF),
        ("source_evidence_id", PROPERTY_FLAG_EVIDENCE_REF),
    ];
    for (name, flag) in required_property_flags {
        let property = association
            .properties
            .iter()
            .find(|property| property.property_name == name)
            .ok_or(CoveError::MapEvidenceInvalid)?;
        if property.flags & flag != flag {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    let required_metadata = [
        ("source_role", CoveLogicalType::Utf8, false),
        ("target_role", CoveLogicalType::Utf8, false),
        ("valid_from", CoveLogicalType::Json, true),
        ("valid_to", CoveLogicalType::Json, true),
        ("cardinality_policy", CoveLogicalType::Utf8, false),
    ];
    for (name, logical_type, nullable) in required_metadata {
        let property = association
            .properties
            .iter()
            .find(|property| property.property_name == name)
            .ok_or(CoveError::MapEvidenceInvalid)?;
        if property.logical_type != logical_type
            || property.physical_kind != CovePhysicalKind::VarBytes
            || property.nullable != nullable
        {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    Ok(())
}

pub(super) fn validate_cove_map_project_fixture(
    corpus: &Path,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid cove-map project fixture json: {error}"))
    })?;
    let (map, sources) = cove_map_fixture_paths(corpus, &value)?;
    let projected =
        cove_map::projected_rows_from_paths(&map, &sources).map_err(|_| CoveError::MapInvalid)?;
    if let Some(expected_rows) = value.get("expected_projected_rows") {
        if projected.get("rows") != Some(expected_rows) {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    if let Some(expected) = value.get("expected_projection") {
        validate_expected_json_fields(&projected, expected)?;
    }
    if let Some(outputs) = value
        .get("expected_projection_outputs")
        .and_then(Value::as_array)
    {
        for output in outputs {
            validate_projection_output_fixture(&map, &sources, output)?;
        }
    }
    if value
        .get("expect_persisted_projection_rows")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        validate_persisted_projection_rows(&map, &sources, &projected)?;
    }
    Ok(())
}

fn validate_persisted_projection_rows(
    map: &Path,
    sources: &[PathBuf],
    projected: &Value,
) -> Result<(), CoveError> {
    struct TempDirGuard(PathBuf);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let bytes = cove_map::cove_o_from_paths(map, sources).map_err(|_| CoveError::MapInvalid)?;
    let dir =
        std::env::temp_dir().join(format!("cove-conformance-project-cove-o-{}", process::id()));
    std::fs::create_dir_all(&dir).map_err(CoveError::from)?;
    let _guard = TempDirGuard(dir.clone());
    let path = dir.join("projected.cove");
    std::fs::write(&path, bytes).map_err(CoveError::from)?;
    let persisted = cove_map::projected_rows_from_cove_o_path(&path, None)
        .map_err(|_| CoveError::MapInvalid)?;
    if persisted.get("rows") != projected.get("rows") {
        return Err(CoveError::MapEvidenceInvalid);
    }
    Ok(())
}

fn validate_projection_output_fixture(
    map: &Path,
    sources: &[PathBuf],
    output: &Value,
) -> Result<(), CoveError> {
    let format = output
        .get("format")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("projection output fixture missing format".into()))?;
    let projection_id = output.get("projection_id").and_then(Value::as_str);
    let format = match format {
        "json" => ProjectionFormat::Json,
        "cove-o" => ProjectionFormat::CoveO,
        "arrow" => ProjectionFormat::Arrow,
        "cove-t" => ProjectionFormat::CoveT,
        "sql" => ProjectionFormat::Sql,
        _ => return Err(CoveError::MapInvalid),
    };
    let bytes = cove_map::projected_output_from_paths(map, sources, format, projection_id)
        .map_err(|_| CoveError::MapInvalid)?;
    if bytes.is_empty() {
        return Err(CoveError::MapEvidenceInvalid);
    }
    match format {
        ProjectionFormat::Json => {
            let _: Value = serde_json::from_slice(&bytes).map_err(|_| CoveError::MapInvalid)?;
        }
        ProjectionFormat::CoveO => {
            reader::validate_bytes_with_options(
                &bytes,
                ValidationOptions {
                    semantic: true,
                    verify_digests: false,
                    allow_unknown_optional_extensions: true,
                    ..ValidationOptions::default()
                },
            )?;
        }
        ProjectionFormat::Arrow => {
            if !bytes.starts_with(b"ARROW1") || !bytes.ends_with(b"ARROW1") {
                return Err(CoveError::MapEvidenceInvalid);
            }
        }
        ProjectionFormat::CoveT => {
            let report = reader::validate_bytes_with_options(
                &bytes,
                ValidationOptions {
                    semantic: true,
                    verify_digests: false,
                    allow_unknown_optional_extensions: true,
                    ..ValidationOptions::default()
                },
            )?;
            if !report.validated.footer.sections.iter().any(|entry| {
                SectionKind::from_u16(entry.section_kind) == Some(SectionKind::TableCatalog)
            }) {
                return Err(CoveError::MapEvidenceInvalid);
            }
        }
        ProjectionFormat::Sql => {
            let sql = std::str::from_utf8(&bytes).map_err(|_| CoveError::MapInvalid)?;
            if !sql.contains("CREATE TABLE") || !sql.contains("INSERT INTO") {
                return Err(CoveError::MapEvidenceInvalid);
            }
        }
    }
    Ok(())
}

fn cove_map_fixture_paths(
    corpus: &Path,
    value: &Value,
) -> Result<(PathBuf, Vec<PathBuf>), CoveError> {
    let mapping = value
        .get("mapping")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("cove-map fixture missing mapping".into()))?;
    let sources = value
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection("cove-map fixture sources must be an array".into()))?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| {
                    CoveError::BadSection("cove-map fixture source is not a string".into())
                })
                .map(|path| resolve_corpus_path(corpus, path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((resolve_corpus_path(corpus, mapping), sources))
}

fn resolve_corpus_path(corpus: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        corpus.join(path)
    }
}

fn validate_expected_json_fields(actual: &Value, expected: &Value) -> Result<(), CoveError> {
    let expected = expected
        .as_object()
        .ok_or_else(|| CoveError::BadSection("expected fixture fields must be an object".into()))?;
    for (key, expected_value) in expected {
        if actual.get(key) != Some(expected_value) {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    Ok(())
}

fn validate_expected_evidence_entries(summary: &Value, expected: &Value) -> Result<(), CoveError> {
    let entries = summary
        .get("evidence_entries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoveError::BadSection("conversion summary missing evidence entries".into())
        })?;
    let expected = expected.as_array().ok_or_else(|| {
        CoveError::BadSection("expected evidence entries must be an array".into())
    })?;

    for expectation in expected {
        if !entries
            .iter()
            .any(|entry| evidence_entry_matches(entry, expectation).unwrap_or(false))
        {
            return Err(CoveError::MapEvidenceInvalid);
        }
    }
    Ok(())
}

fn evidence_entry_matches(entry: &Value, expectation: &Value) -> Result<bool, CoveError> {
    let expectation = expectation.as_object().ok_or_else(|| {
        CoveError::BadSection("expected evidence entry matcher must be an object".into())
    })?;

    if let Some(contains) = expectation.get("contains") {
        let contains = contains.as_object().ok_or_else(|| {
            CoveError::BadSection("expected evidence contains matcher must be an object".into())
        })?;
        for (key, expected_value) in contains {
            if entry.get(key) != Some(expected_value) {
                return Ok(false);
            }
        }
    }

    if let Some(present) = expectation.get("present") {
        let present = present.as_array().ok_or_else(|| {
            CoveError::BadSection("expected evidence present matcher must be an array".into())
        })?;
        for key in present {
            let key = key.as_str().ok_or_else(|| {
                CoveError::BadSection("expected evidence present key must be a string".into())
            })?;
            if entry.get(key).is_none_or(Value::is_null) {
                return Ok(false);
            }
        }
    }

    if let Some(absent) = expectation.get("absent") {
        let absent = absent.as_array().ok_or_else(|| {
            CoveError::BadSection("expected evidence absent matcher must be an array".into())
        })?;
        for key in absent {
            let key = key.as_str().ok_or_else(|| {
                CoveError::BadSection("expected evidence absent key must be a string".into())
            })?;
            if entry.get(key).is_some() {
                return Ok(false);
            }
        }
    }

    Ok(true)
}
