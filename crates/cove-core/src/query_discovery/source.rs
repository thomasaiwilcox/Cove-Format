use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::{
    artifact::covm::{CovmFile, CovmFileEntryV1},
    constants::DigestAlgorithm,
    digest::compute_digest,
    mount::dictionary_crc32c,
    utility::hex_encode,
    CoveError,
};

use super::{
    helpers::{disclosure_label, rounded_count_hint},
    model::{QueryDiscoveryOptions, QueryDiscoveryValidationContext},
};

pub(super) fn cove_file_source_binding(
    bytes: &[u8],
    mounted: &crate::mount::MountedCoveFile,
    options: &QueryDiscoveryOptions,
) -> Result<Value, CoveError> {
    let file_digest = compute_digest(DigestAlgorithm::Sha256, bytes)?;
    let mut binding = json!({
        "source_kind": "cove_file",
        "source_uri_hint": options.source_name,
        "file_digest": prefixed_sha256(&file_digest),
        "file_length": bytes.len().to_string(),
        "footer_crc32c": format!("{:08x}", mounted.footer.compute_crc()),
        "header_crc32c": format!("{:08x}", mounted.header.checksum),
        "file_id": hex_encode(&mounted.header.file_id),
        "primary_profile": mounted.header.primary_profile.to_string(),
        "required_features": mounted.header.required_features.to_string(),
        "optional_features": mounted.header.optional_features.to_string(),
        "schema_fingerprint": format!(
            "sha256:{}",
            hex_encode(&compute_digest(DigestAlgorithm::Sha256, &schema_fingerprint_seed(mounted))?)
        ),
        "visibility_scope": disclosure_label(options.disclosure_mode),
        "redaction_scope": disclosure_label(options.disclosure_mode),
    });
    if let Some(dictionary) = &mounted.dictionary {
        binding["dictionary_crc32c"] = json!(format!("{:08x}", dictionary_crc32c(dictionary)));
    }
    if let Some(fingerprint) = &options.policy_fingerprint {
        binding["policy_fingerprint"] = json!(fingerprint);
    }
    if let Some(principal_class) = &options.principal_class {
        binding["principal_class"] = json!(principal_class);
    }
    if let Some(audience) = &options.audience {
        binding["audience"] = json!(audience);
    }
    Ok(binding)
}

pub(super) fn covm_source_binding(
    bytes: &[u8],
    covm: &CovmFile,
    options: &QueryDiscoveryOptions,
) -> Result<Value, CoveError> {
    let manifest_digest = compute_digest(DigestAlgorithm::Sha256, bytes)?;
    let member_digest = compute_digest(
        DigestAlgorithm::Sha256,
        &covm_member_digest_seed(&covm.files),
    )?;
    let mut binding = json!({
        "source_kind": "covm_dataset_snapshot",
        "source_uri_hint": options.source_name,
        "dataset_id": hex_encode(&covm.header.dataset_id),
        "covm_snapshot_digest": prefixed_sha256(&manifest_digest),
        "member_set_digest": prefixed_sha256(&member_digest),
        "file_length": bytes.len().to_string(),
        "postscript_checksum": format!("{:08x}", covm.postscript.checksum),
        "header_checksum": format!("{:08x}", covm.header.checksum),
        "created_at_us": covm.header.created_at_us.to_string(),
        "table_count": covm.header.table_count.to_string(),
        "file_count": covm.files.len().to_string(),
        "visibility_scope": disclosure_label(options.disclosure_mode),
        "redaction_scope": disclosure_label(options.disclosure_mode),
        "members": covm.files.iter().map(covm_member_binding_json).collect::<Vec<_>>(),
    });
    if let Some(fingerprint) = &options.policy_fingerprint {
        binding["policy_fingerprint"] = json!(fingerprint);
    }
    if let Some(principal_class) = &options.principal_class {
        binding["principal_class"] = json!(principal_class);
    }
    if let Some(audience) = &options.audience {
        binding["audience"] = json!(audience);
    }
    Ok(binding)
}

fn covm_member_binding_json(member: &CovmFileEntryV1) -> Value {
    json!({
        "member_id": member.uri,
        "uri_hint": member.uri,
        "file_id": hex_encode(&member.file_id),
        "file_digest": digest_algorithm_prefix(member.digest_algorithm, &member.digest),
        "file_length": member.file_len.to_string(),
        "footer_crc32c": format!("{:08x}", member.footer_crc32c),
        "row_count_hint": {
            "value": rounded_count_hint(member.row_count).to_string(),
            "precision": if member.row_count <= 1000 { "exact" } else { "rounded" }
        },
        "segment_count": member.segment_count.to_string(),
        "flags": member.flags.to_string()
    })
}

fn schema_fingerprint_seed(mounted: &crate::mount::MountedCoveFile) -> Vec<u8> {
    let mut seed = Vec::new();
    seed.extend_from_slice(&mounted.header.primary_profile.to_le_bytes());
    if let Some(catalog) = &mounted.table_catalog {
        seed.extend_from_slice(&catalog.flags.to_le_bytes());
        for table in &catalog.tables {
            seed.extend_from_slice(&table.table_id.to_le_bytes());
            push_len_prefixed(&mut seed, table.namespace.as_bytes());
            push_len_prefixed(&mut seed, table.name.as_bytes());
            seed.extend_from_slice(&table.row_count.to_le_bytes());
            for column in &table.columns {
                seed.extend_from_slice(&column.column_id.to_le_bytes());
                push_len_prefixed(&mut seed, column.name.as_bytes());
                seed.extend_from_slice(&(column.logical as u16).to_le_bytes());
                seed.extend_from_slice(&(column.physical as u16).to_le_bytes());
                seed.push(u8::from(column.nullable));
            }
        }
    }
    seed
}

fn covm_member_digest_seed(members: &[CovmFileEntryV1]) -> Vec<u8> {
    let mut seed = Vec::new();
    for member in members {
        seed.extend_from_slice(&member.file_id);
        push_len_prefixed(&mut seed, member.uri.as_bytes());
        seed.extend_from_slice(&member.file_len.to_le_bytes());
        seed.extend_from_slice(&member.footer_crc32c.to_le_bytes());
        seed.extend_from_slice(&member.digest_algorithm.to_le_bytes());
        push_len_prefixed(&mut seed, &member.digest);
        seed.extend_from_slice(&member.row_count.to_le_bytes());
        seed.extend_from_slice(&member.segment_count.to_le_bytes());
        seed.extend_from_slice(&member.flags.to_le_bytes());
    }
    seed
}

fn push_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn prefixed_sha256(digest: &[u8]) -> String {
    format!("sha256:{}", hex_encode(digest))
}

fn digest_algorithm_prefix(algorithm: u16, digest: &[u8]) -> String {
    let prefix = match DigestAlgorithm::from_u16(algorithm) {
        Some(DigestAlgorithm::Sha256) => "sha256",
        Some(DigestAlgorithm::Blake3) => "blake3",
        _ => "unknown",
    };
    format!("{prefix}:{}", hex_encode(digest))
}

pub(super) fn validation_context_from_source_binding(
    binding: &Value,
    options: &QueryDiscoveryOptions,
) -> QueryDiscoveryValidationContext {
    let expected_member_file_digests = binding
        .get("members")
        .and_then(Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|member| {
                    let id = member.get("member_id").and_then(Value::as_str)?;
                    let digest = member.get("file_digest").and_then(Value::as_str)?;
                    Some((id.to_string(), digest.to_string()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    QueryDiscoveryValidationContext {
        strict_discovery: true,
        source_binding_valid: Some(true),
        policy_scope_compatible: Some(true),
        expected_source_kind: binding
            .get("source_kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_file_digest: binding
            .get("file_digest")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_file_length: binding
            .get("file_length")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_header_crc32c: binding
            .get("header_crc32c")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_footer_crc32c: binding
            .get("footer_crc32c")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_schema_fingerprint: binding
            .get("schema_fingerprint")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_dictionary_crc32c: binding
            .get("dictionary_crc32c")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_covm_snapshot_digest: binding
            .get("covm_snapshot_digest")
            .and_then(Value::as_str)
            .map(str::to_string),
        expected_member_file_digests,
        expected_policy_fingerprint: options.policy_fingerprint.clone(),
        expected_principal_class: options.principal_class.clone(),
        expected_audience: options.audience.clone(),
        validation_flags: Vec::new(),
    }
}
