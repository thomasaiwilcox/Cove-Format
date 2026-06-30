use super::*;

pub(super) fn validate_suite_contract_fixture(
    corpus: &Path,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid suite-contract fixture json: {error}"))
    })?;
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("suite-contract fixture missing op".into()))?;

    match op {
        "manifest_sections_present" => validate_suite_manifest_contract(corpus, &value),
        "release_gate_contains" => validate_release_gate_contract(corpus, &value),
        "workspace_members_present" => validate_workspace_contract(corpus, &value),
        "governance_docs_present" => validate_governance_docs_contract(corpus, &value),
        other => Err(CoveError::BadSection(format!(
            "unsupported suite-contract op {other}"
        ))),
    }
}

fn validate_suite_manifest_contract(corpus: &Path, value: &Value) -> Result<(), CoveError> {
    let manifest_path = corpus.join("manifest.jsonl");
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|error| {
        CoveError::BadSection(format!(
            "cannot read suite manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    let required_sections = parse_fixture_string_vector(value.get("sections"), "sections")?;
    let minimum_accept = value
        .get("minimum_accept")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let minimum_reject = value
        .get("minimum_reject")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;

    let mut seen_sections = BTreeSet::new();
    let mut accept_count = 0usize;
    let mut reject_count = 0usize;
    for (line_number, line) in manifest.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry: Value = serde_json::from_str(line).map_err(|error| {
            CoveError::BadSection(format!(
                "invalid manifest line {} for suite contract: {error}",
                line_number + 1
            ))
        })?;
        match entry.get("expect").and_then(Value::as_str) {
            Some("accept") => accept_count += 1,
            Some("reject") => reject_count += 1,
            _ => {}
        }
        if let Some(sections) = entry.get("sections").and_then(Value::as_array) {
            for section in sections.iter().filter_map(Value::as_str) {
                seen_sections.insert(section.to_string());
            }
        }
    }

    if accept_count < minimum_accept {
        return Err(CoveError::BadSection(format!(
            "suite contract requires at least {minimum_accept} accept fixtures, found {accept_count}"
        )));
    }
    if reject_count < minimum_reject {
        return Err(CoveError::BadSection(format!(
            "suite contract requires at least {minimum_reject} reject fixtures, found {reject_count}"
        )));
    }
    for section in required_sections {
        let matched = seen_sections
            .iter()
            .any(|seen| seen == &section || seen.starts_with(&format!("{section}.")));
        if !matched {
            return Err(CoveError::BadSection(format!(
                "suite contract missing manifest coverage for {section}"
            )));
        }
    }

    Ok(())
}

fn validate_release_gate_contract(corpus: &Path, value: &Value) -> Result<(), CoveError> {
    let repo_root = corpus.parent().ok_or_else(|| {
        CoveError::BadSection("cannot locate repository root from conformance corpus".into())
    })?;
    let gate_path = repo_root.join("scripts/release-gates.sh");
    validate_script_executable(&gate_path)?;
    let contents = std::fs::read_to_string(&gate_path).map_err(|error| {
        CoveError::BadSection(format!(
            "cannot read release-gate script {}: {error}",
            gate_path.display()
        ))
    })?;
    for needle in parse_fixture_string_vector(value.get("needles"), "needles")? {
        if !contents.contains(&needle) {
            return Err(CoveError::BadSection(format!(
                "release-gate script missing required command: {needle}"
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn validate_script_executable(path: &Path) -> Result<(), CoveError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|error| {
        CoveError::BadSection(format!(
            "cannot stat release-gate script {}: {error}",
            path.display()
        ))
    })?;
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(CoveError::BadSection(format!(
            "release-gate script {} is not executable",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_script_executable(_path: &Path) -> Result<(), CoveError> {
    Ok(())
}

fn validate_workspace_contract(corpus: &Path, value: &Value) -> Result<(), CoveError> {
    let repo_root = corpus.parent().ok_or_else(|| {
        CoveError::BadSection("cannot locate repository root from conformance corpus".into())
    })?;
    let cargo_toml_path = repo_root.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path).map_err(|error| {
        CoveError::BadSection(format!(
            "cannot read workspace manifest {}: {error}",
            cargo_toml_path.display()
        ))
    })?;
    for member in parse_fixture_string_vector(value.get("members"), "members")? {
        let needle = format!("\"{member}\"");
        if !cargo_toml.contains(&needle) {
            return Err(CoveError::BadSection(format!(
                "workspace manifest missing required member {member}"
            )));
        }
    }
    Ok(())
}

fn validate_governance_docs_contract(corpus: &Path, value: &Value) -> Result<(), CoveError> {
    let repo_root = corpus.parent().ok_or_else(|| {
        CoveError::BadSection("cannot locate repository root from conformance corpus".into())
    })?;
    let docs = parse_fixture_string_vector(value.get("docs"), "docs")?;
    let mut combined = String::new();
    for doc in docs {
        let path = repo_root.join(&doc);
        let contents = std::fs::read_to_string(&path).map_err(|error| {
            CoveError::BadSection(format!(
                "cannot read governance doc {}: {error}",
                path.display()
            ))
        })?;
        combined.push_str(&contents);
        combined.push('\n');
    }
    for needle in parse_fixture_string_vector(value.get("needles"), "needles")? {
        if !combined.contains(&needle) {
            return Err(CoveError::BadSection(format!(
                "governance docs missing required text: {needle}"
            )));
        }
    }
    Ok(())
}

pub(super) fn parse_fixture_string_vector(
    value: Option<&Value>,
    field: &str,
) -> Result<Vec<String>, CoveError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| CoveError::BadSection(format!("fixture missing {field}")))?;
    let mut out = Vec::with_capacity(values.len());
    for (index, item) in values.iter().enumerate() {
        let string = item.as_str().ok_or_else(|| {
            CoveError::BadSection(format!("fixture field {field}[{index}] is not a string"))
        })?;
        out.push(string.to_string());
    }
    Ok(out)
}

pub(super) fn validate_error_surface_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid error-surface fixture json: {error}"))
    })?;
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| CoveError::BadSection("error-surface fixture missing code".into()))?;
    let error = synthetic_error_surface_error(code).ok_or_else(|| {
        CoveError::BadSection(format!("unsupported error-surface fixture code {code}"))
    })?;
    if error.spec_code() != Some(code) {
        return Err(CoveError::BadSection(format!(
            "error-surface fixture code {code} does not match spec_code()"
        )));
    }
    if !error.to_string().contains(code) {
        return Err(CoveError::BadSection(format!(
            "error-surface fixture code {code} is not present in display output"
        )));
    }
    Err(error)
}

fn synthetic_error_surface_error(code: &str) -> Option<CoveError> {
    match code {
        "COVE_E_BAD_VERSION" => Some(CoveError::BadVersion),
        "COVE_E_ARITH_OVERFLOW" => Some(CoveError::ArithOverflow),
        "COVE_E_DICT_MISS" => Some(CoveError::DictMiss),
        "COVE_E_BAD_FILECODE" => Some(CoveError::BadFileCode),
        "COVE_E_BAD_NUMCODE" => Some(CoveError::BadNumCode),
        "COVE_E_BAD_EXTENSION" => Some(CoveError::BadExtension),
        "COVE_E_EXECUTION_CODE_MAP" => Some(CoveError::ExecutionCodeMap),
        "COVE_E_HARBOR_MOUNT_LEASE" => Some(CoveError::HarborMountLease),
        "COVE_E_NOT_SELF_CONTAINED" => Some(CoveError::NotSelfContained),
        "COVE_E_REDACTION_POLICY" => Some(CoveError::RedactionPolicy),
        "COVE_E_SIDECAR_STALE" => Some(CoveError::SidecarStale),
        "COVE_E_MAP_INVALID" => Some(CoveError::MapInvalid),
        "COVE_E_MAP_FUNCTION_UNDECLARED" => Some(CoveError::MapFunctionUndeclared),
        "COVE_E_MAP_IDENTITY_CONFLICT" => Some(CoveError::MapIdentityConflict),
        "COVE_E_MAP_SOURCE_STALE" => Some(CoveError::MapSourceStale),
        "COVE_E_MAP_EVIDENCE_INVALID" => Some(CoveError::MapEvidenceInvalid),
        "COVE_E_BAD_CODEC_EXTENSION" => Some(CoveError::BadCodecExtension),
        "COVE_E_CODEC_UNSUPPORTED" => Some(CoveError::CodecUnsupported),
        "COVE_E_BAD_LAYOUT_PLAN" => Some(CoveError::BadLayoutPlan),
        "COVE_E_RUNTIME_HINT_UNSUPPORTED" => Some(CoveError::RuntimeHintUnsupported),
        "COVE_E_BAD_COVERAGE" => Some(CoveError::BadCoverage),
        "COVE_E_COVERAGE_STALE" => Some(CoveError::CoverageStale),
        "COVE_E_BAD_COVI" => Some(CoveError::BadCovi),
        "COVE_E_INDEX_ONLY_UNSAFE" => Some(CoveError::IndexOnlyUnsafe),
        "COVE_E_CACHE_STALE" => Some(CoveError::CacheStale),
        _ => None,
    }
}
