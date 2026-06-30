use super::*;

pub(super) fn validate_extension_logical_type_fixture(
    entry: &Entry,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let descriptor = ExtensionLogicalTypeV1::parse(bytes)?;
    let collation_count = entry
        .raw
        .get("collation_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    descriptor.validate(ExtensionValidationContext { collation_count })
}

pub(super) fn validate_extension_index_descriptor_fixture(
    entry: &Entry,
    bytes: &[u8],
) -> Result<(), CoveError> {
    let descriptor = ExtensionIndexDescriptorV1::parse(bytes)?;
    descriptor.validate()?;
    if let Some(expected) = entry.raw.get("expect_can_skip").and_then(Value::as_bool) {
        if descriptor.can_skip_data() != expected {
            return Err(CoveError::BadExtension);
        }
    }
    Ok(())
}

pub(super) fn validate_durable_publish_fixture(bytes: &[u8]) -> Result<(), CoveError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        CoveError::BadSection(format!("invalid durable-publish fixture json: {error}"))
    })?;
    let payload = value
        .get("payload")
        .and_then(Value::as_str)
        .unwrap_or("durable-cove-candidate")
        .as_bytes()
        .to_vec();
    let manifest_payload = value
        .get("manifest_payload")
        .and_then(Value::as_str)
        .map(|value| value.as_bytes().to_vec());
    let dir = std::env::temp_dir().join(format!(
        "cove-conformance-durable-{}-{}",
        std::process::id(),
        value
            .get("case_id")
            .and_then(Value::as_str)
            .unwrap_or("case")
    ));
    std::fs::create_dir_all(&dir).map_err(CoveError::from)?;
    let path = dir.join("published.cove");
    let actual = if let Some(manifest_payload) = manifest_payload {
        let manifest_path = dir.join("published.covm");
        std::fs::write(&manifest_path, b"old-authoritative").map_err(CoveError::from)?;
        durable::durable_publish_delta_then_manifest(
            &path,
            &payload,
            &manifest_path,
            &manifest_payload,
        )?;
        let delta_actual = std::fs::read(&path).map_err(CoveError::from)?;
        let manifest_actual = std::fs::read(&manifest_path).map_err(CoveError::from)?;
        [delta_actual, manifest_actual].concat()
    } else {
        std::fs::write(&path, b"old-authoritative").map_err(CoveError::from)?;
        durable::durable_replace(&path, &payload)?;
        std::fs::read(&path).map_err(CoveError::from)?
    };
    let _ = std::fs::remove_dir_all(&dir);
    let expected = if let Some(manifest_payload) = value
        .get("manifest_payload")
        .and_then(Value::as_str)
        .map(str::as_bytes)
    {
        [payload.as_slice(), manifest_payload].concat()
    } else {
        payload
    };
    if actual != expected {
        return Err(CoveError::BadSection(
            "durable publish fixture did not replace destination bytes".into(),
        ));
    }
    Ok(())
}
