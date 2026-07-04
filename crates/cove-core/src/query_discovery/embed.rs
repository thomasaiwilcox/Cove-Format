use crate::{
    artifact::covm::CovmFile,
    compression,
    constants::{PrimaryProfile, SectionKind, FEATURE_QUERY_DISCOVERY_METADATA},
    mount::{mount_cove_file, MountOptions},
    reader,
    writer::SectionPayload,
    CoveError,
};

use super::{
    model::{QueryDiscoveryManifest, QueryDiscoveryOptions, QueryDiscoveryValidationContext},
    source::{
        cove_file_source_binding, covm_source_binding, validation_context_from_source_binding,
    },
};

pub fn query_discovery_section_payload(manifest: &QueryDiscoveryManifest) -> SectionPayload {
    SectionPayload {
        section_kind: SectionKind::QueryDiscoveryManifest as u16,
        profile: PrimaryProfile::QueryDiscovery as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_QUERY_DISCOVERY_METADATA,
        data: manifest.raw_canonical_json().to_vec(),
    }
}

pub fn embedded_query_discovery_manifests(
    bytes: &[u8],
) -> Result<Vec<QueryDiscoveryManifest>, CoveError> {
    let validated = reader::validate_bytes(bytes)?;
    validated
        .footer
        .sections
        .iter()
        .filter(|entry| entry.section_kind == SectionKind::QueryDiscoveryManifest as u16)
        .map(|entry| {
            let payload = compression::section_payload(bytes, entry)?;
            QueryDiscoveryManifest::parse(payload.as_ref())
        })
        .collect()
}

pub fn query_discovery_validation_context_for_source(
    bytes: &[u8],
    options: &QueryDiscoveryOptions,
) -> Result<QueryDiscoveryValidationContext, CoveError> {
    let binding = if let Ok(covm) = CovmFile::parse_delta_aware(bytes) {
        covm_source_binding(bytes, &covm, options)?
    } else {
        let mounted = mount_cove_file(bytes, MountOptions::default(), None)?;
        cove_file_source_binding(bytes, &mounted, options)?
    };
    Ok(validation_context_from_source_binding(&binding, options))
}

pub fn query_discovery_validation_context_for_embedded_source(
    bytes: &[u8],
    options: &QueryDiscoveryOptions,
) -> Result<QueryDiscoveryValidationContext, CoveError> {
    let mut context = query_discovery_validation_context_for_source(bytes, options)?;
    context.expected_file_digest = None;
    context.expected_file_length = None;
    context.expected_header_crc32c = None;
    context.expected_footer_crc32c = None;
    context.expected_covm_snapshot_digest = None;
    Ok(context)
}
