//! Library entrypoint for COVE validation command behavior.

use std::{error::Error, fmt};

mod args;
mod format;
mod validate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidateCliError {
    message: String,
}

impl ValidateCliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidateCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ValidateCliError {}

impl From<String> for ValidateCliError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for ValidateCliError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<bool, ValidateCliError> {
    let args = args::parse_args(args).map_err(ValidateCliError::from)?;
    Ok(validate::validate_paths(&args))
}

pub fn usage() -> &'static str {
    args::USAGE
}
