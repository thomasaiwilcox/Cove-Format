//! Library entrypoint for COVE dump command behavior.

mod args;
mod dump;

use std::path::Path;

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let args = args::parse_args(args)?;
    dump::dump_file(Path::new(&args.path), args.mode, args.max_bytes)
}

pub fn usage() -> &'static str {
    args::USAGE
}
