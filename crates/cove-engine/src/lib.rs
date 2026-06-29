//! Engine facade for stable COVE v2 runtime and DataFusion APIs.

use std::{fs, path::Path, sync::Arc};

use arrow_array::RecordBatch;

pub use cove_core::mount;
pub use cove_core::CoveError;
pub use cove_datafusion::register::register_cove_file_format as register_datafusion;
pub use cove_datafusion::{
    coverage_plan, dataset_state, execution_code, options, planner, prune, register,
};
pub use cove_runtime as runtime;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DumpRowsOptions {
    pub projection: Option<Vec<String>>,
    pub table_options: options::CoveTableOptions,
}

pub fn validate_execution_profile(
    path: impl AsRef<Path>,
) -> Result<mount::EngineMetadata, CoveError> {
    inspect_execution_metadata(path)
}

pub fn inspect_execution_metadata(
    path: impl AsRef<Path>,
) -> Result<mount::EngineMetadata, CoveError> {
    let data = fs::read(path)?;
    Ok(mount::mount_cove_file(&data, mount::MountOptions::default(), None)?.engine_metadata)
}

pub fn open_table(path: impl AsRef<Path>) -> Result<Arc<dataset_state::DatasetState>, CoveError> {
    cove_datafusion::bootstrap::bootstrap_local_file(path)
}

pub fn open_table_with_options(
    path: impl AsRef<Path>,
    table_options: options::CoveTableOptions,
) -> Result<Arc<dataset_state::DatasetState>, CoveError> {
    cove_datafusion::bootstrap::bootstrap_local_file_with_options(path, table_options)
}

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
