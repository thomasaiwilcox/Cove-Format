//! # cove-datafusion -- DataFusion integration for COVE
//!
//! Reference DataFusion SQL, FileFormat, and execution integration for COVE v2.

use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatafusionCliError {
    message: String,
}

impl DatafusionCliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DatafusionCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DatafusionCliError {}

impl From<String> for DatafusionCliError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for DatafusionCliError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<crate::delta_snapshot::DeltaSnapshotError> for DatafusionCliError {
    fn from(error: crate::delta_snapshot::DeltaSnapshotError) -> Self {
        Self::new(error.to_string())
    }
}

pub mod adapter_v53;
pub mod bootstrap;
pub mod coverage_plan;
pub mod dataset_state;
pub mod decode;
pub mod delta_snapshot;
pub mod execution_code;
pub mod explain;
pub mod expr_lowering;
pub mod metadata_aggregate;
pub mod options;
pub mod overlay;
pub mod planner;
pub mod projection_provider;
pub mod prune;
pub mod range_reader;
pub mod register;
pub mod scan_program;
pub mod task_graph;

pub mod arrow_export_cli;
pub mod explain_pruning_cli;
pub mod plan_cost_cli;
