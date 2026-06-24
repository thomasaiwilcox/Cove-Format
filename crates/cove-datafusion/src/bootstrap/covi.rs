use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use cove_core::{checksum, constants::DigestAlgorithm, digest::compute_digest, CoveError};
use cove_index::execution::{CoviValidationContextV2, ValidatedCoviArtifactV2};

use crate::{
    dataset_state::{DatasetBootstrapStats, FileMetadata},
    options::{CoveTableOptions, CoviDiscovery},
};

pub(super) fn discover_covi_path(path: &Path, discovery: CoviDiscovery) -> Option<PathBuf> {
    if discovery != CoviDiscovery::SiblingExtension {
        return None;
    }
    let appended = PathBuf::from(format!("{}.covi", path.display()));
    if appended.is_file() {
        return Some(appended);
    }
    let replaced = path.with_extension("covi");
    replaced.is_file().then_some(replaced)
}

pub(super) fn validate_covi_for_file(
    cove_path: &Path,
    file: &FileMetadata,
    options: CoveTableOptions,
    stats: &mut DatasetBootstrapStats,
) -> Option<Arc<ValidatedCoviArtifactV2>> {
    let path = discover_covi_path(cove_path, options.covi_discovery())?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            stats.covi_sidecars_ignored += 1;
            return None;
        }
    };
    let mut context = base_covi_validation_context(file);
    if let Ok(cove_bytes) = fs::read(cove_path) {
        context = context.with_snapshot_id(derived_covi_snapshot_id(
            &cove_bytes,
            file.identity().footer_crc32c,
        ));
        if let Ok(digest) = compute_digest(DigestAlgorithm::Sha256, &cove_bytes) {
            context = context.with_file_digest(DigestAlgorithm::Sha256, digest);
        }
    }
    match ValidatedCoviArtifactV2::parse_and_validate(&bytes, context) {
        Ok(validated) => {
            stats.covi_sidecars_loaded += 1;
            Some(Arc::new(validated))
        }
        Err(CoveError::BadCovi)
        | Err(CoveError::ChecksumMismatch)
        | Err(CoveError::DigestMismatch)
        | Err(CoveError::BadMagic) => {
            stats.covi_sidecars_stale += 1;
            None
        }
        Err(_) => {
            stats.covi_sidecars_ignored += 1;
            None
        }
    }
}

pub(super) fn base_covi_validation_context(file: &FileMetadata) -> CoviValidationContextV2 {
    let mut context = CoviValidationContextV2::for_file(
        file.identity().file_id,
        file.identity().file_len,
        file.identity().footer_crc32c,
    )
    .with_dataset_id(file.identity().file_id)
    .with_schema_fingerprint_ref(u32::MAX)
    .with_semantic_map_fingerprint_ref(u32::MAX)
    .with_external_visibility_ref(u32::MAX)
    .with_file_code_keys(true);
    if let Ok(scopes) = file.covi_row_range_scopes() {
        context = context.with_row_range_scopes(scopes);
    }
    context
}

pub(super) fn derived_covi_snapshot_id(bytes: &[u8], footer_crc32c: u32) -> [u8; 16] {
    derived_covi_snapshot_id_from_parts(checksum::crc32c(bytes), footer_crc32c, bytes.len() as u64)
}

pub(super) fn derived_covi_snapshot_id_from_parts(
    file_crc32c: u32,
    footer_crc32c: u32,
    file_len: u64,
) -> [u8; 16] {
    let mut snapshot_id = [0u8; 16];
    snapshot_id[0..4].copy_from_slice(&file_crc32c.to_le_bytes());
    snapshot_id[4..8].copy_from_slice(&footer_crc32c.to_le_bytes());
    snapshot_id[8..16].copy_from_slice(&file_len.to_le_bytes());
    snapshot_id
}
