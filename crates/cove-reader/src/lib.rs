//! Reader facade for stable COVE v2 read and mount APIs.

use std::{fs, path::Path};

pub use cove_core::{
    artifact, constants, dictionary, footer, header, mount, profile, reader, table, CoveError,
};

pub fn validate_file(path: impl AsRef<Path>) -> Result<reader::ValidationReport, CoveError> {
    validate_file_with_options(
        path,
        reader::ValidationOptions {
            semantic: true,
            verify_digests: false,
            allow_unknown_optional_extensions: true,
            ..reader::ValidationOptions::default()
        },
    )
}

pub fn validate_file_with_options(
    path: impl AsRef<Path>,
    options: reader::ValidationOptions,
) -> Result<reader::ValidationReport, CoveError> {
    let data = fs::read(path)?;
    reader::validate_bytes_with_options(&data, options)
}

pub fn inspect_file(path: impl AsRef<Path>) -> Result<mount::MountedCoveFile, CoveError> {
    let data = fs::read(path)?;
    mount::mount_cove_file(&data, mount::MountOptions::default(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_file_with_options_exposes_digest_verification() {
        let path = std::env::temp_dir().join(format!(
            "cove_reader_validate_options_{}.cove",
            std::process::id()
        ));
        std::fs::write(
            &path,
            cove_core::writer::MinimalCoveWriter::write_empty_file().unwrap(),
        )
        .unwrap();

        let report = validate_file_with_options(
            &path,
            reader::ValidationOptions {
                semantic: true,
                verify_digests: true,
                allow_unknown_optional_extensions: true,
                ..reader::ValidationOptions::default()
            },
        )
        .unwrap();
        assert!(report.semantic_checked);

        let _ = std::fs::remove_file(path);
    }
}
