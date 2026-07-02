use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CliError {
    Usage(String),
    Command(String),
}

impl CliError {
    pub(crate) fn usage(message: String) -> Self {
        Self::Usage(message)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(message) | CliError::Command(message) => f.write_str(message),
        }
    }
}

impl Error for CliError {}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::Command(message)
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self::Command(message.to_string())
    }
}
