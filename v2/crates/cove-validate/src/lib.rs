//! Library entrypoint for COVE validation command behavior.

mod args;
mod format;
mod validate;

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<bool, String> {
    let args = args::parse_args(args)?;
    Ok(validate::validate_paths(&args))
}

pub fn usage() -> &'static str {
    args::USAGE
}
