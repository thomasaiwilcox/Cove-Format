use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    run_with_platform_stack(args)
}

#[cfg(windows)]
fn run_with_platform_stack(args: Vec<String>) -> ExitCode {
    std::thread::Builder::new()
        .name("cove-cli".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || run(args))
        .expect("failed to start cove CLI thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

#[cfg(not(windows))]
fn run_with_platform_stack(args: Vec<String>) -> ExitCode {
    run(args)
}

fn run(args: Vec<String>) -> ExitCode {
    match cove_cli::run_cli(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
