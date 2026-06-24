use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match cove_cli::run_cli(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
