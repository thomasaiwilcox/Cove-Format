//! Reader facade for stable COVE v2 read and mount APIs.

#[cfg(feature = "datafusion")]
use std::sync::Arc;
use std::{fs, path::Path};

#[cfg(feature = "datafusion")]
use arrow_array::RecordBatch;

pub use cove_core::{
    artifact, constants, dictionary, footer, header, mount, profile, reader, table, CoveError,
};
#[cfg(feature = "datafusion")]
pub use cove_datafusion::{dataset_state, options};

#[cfg(feature = "datafusion")]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DumpRowsOptions {
    pub projection: Option<Vec<String>>,
    pub table_options: options::CoveTableOptions,
}

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

#[cfg(feature = "datafusion")]
pub fn open_table(path: impl AsRef<Path>) -> Result<Arc<dataset_state::DatasetState>, CoveError> {
    cove_datafusion::bootstrap::bootstrap_local_file(path)
}

#[cfg(feature = "datafusion")]
pub fn open_table_with_options(
    path: impl AsRef<Path>,
    table_options: options::CoveTableOptions,
) -> Result<Arc<dataset_state::DatasetState>, CoveError> {
    cove_datafusion::bootstrap::bootstrap_local_file_with_options(path, table_options)
}

#[cfg(feature = "datafusion")]
pub fn dump_rows(
    path: impl AsRef<Path>,
    options: DumpRowsOptions,
) -> Result<Vec<RecordBatch>, CoveError> {
    let planned = cove_datafusion::explain::plan_local_file(
        path,
        cove_datafusion::explain::ExplainOptions {
            projection: options.projection,
            table_options: options.table_options,
            ..cove_datafusion::explain::ExplainOptions::default()
        },
    )?;
    Ok(cove_datafusion::explain::execute_planned_scan(&planned)?.batches)
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
