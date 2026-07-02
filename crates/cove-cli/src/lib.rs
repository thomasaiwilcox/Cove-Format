//! Beginner-friendly CLI facade for the COVE reference workspace.
//!
//! The command surface keeps user-facing stderr stable. Rust callers receive
//! typed `CliError` values so argument/usage failures can be distinguished from
//! command execution failures without parsing display text.

mod arrow_export;
mod commands;
pub mod customer360;
mod delta;
mod error;
mod external_tables;
mod help;
mod output;
mod perf;
mod sidecar;

pub use error::CliError;

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), CliError> {
    commands::run_cli(args)
}
