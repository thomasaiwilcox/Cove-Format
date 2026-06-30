//! Library entrypoint for COVE dump command behavior.

mod args;
mod dump;

use std::path::Path;
use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpCliError {
    message: String,
}

impl DumpCliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DumpCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for DumpCliError {}

impl From<String> for DumpCliError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for DumpCliError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), DumpCliError> {
    let args = args::parse_args(args).map_err(DumpCliError::from)?;
    dump::dump_file(Path::new(&args.path), args.mode, args.max_bytes).map_err(DumpCliError::from)
}

pub fn usage() -> &'static str {
    args::USAGE
}
