//! Library entrypoint for detailed COVE inspection command behavior.

mod format;
mod inspect;

use std::path::Path;

pub use inspect::InspectSections;

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<bool, String> {
    let args = parse_args(args.into_iter().collect::<Vec<_>>())?;
    if args.file_paths.is_empty() {
        return Err(usage().into());
    }

    if args.json {
        let mut values = Vec::new();
        let mut all_ok = true;
        for path in &args.file_paths {
            match inspect::inspect_file_json(Path::new(path), &args.sections) {
                Ok(value) => values.push(value),
                Err(error) => {
                    all_ok = false;
                    eprintln!("ERROR: {error}");
                }
            }
        }
        if all_ok {
            if values.len() == 1 {
                println!("{}", serde_json::to_string_pretty(&values[0]).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&values).unwrap());
            }
        }
        return Ok(all_ok);
    }

    let mut all_ok = true;
    for path in &args.file_paths {
        if let Err(error) = inspect::inspect_file(Path::new(path)) {
            all_ok = false;
            eprintln!("ERROR: {error}");
        }
    }
    Ok(all_ok)
}

pub fn usage() -> &'static str {
    "Usage: cove inspect [--json] [--sections <stats,dictionary,execution,indexes,optional>] <file.cove> [<file2.cove> ...]"
}

struct Args {
    json: bool,
    sections: inspect::InspectSections,
    file_paths: Vec<String>,
}

fn parse_args(raw_args: Vec<String>) -> Result<Args, String> {
    let mut json = false;
    let mut sections = inspect::InspectSections::All;
    let mut file_paths = Vec::new();
    let mut index = 0usize;
    while index < raw_args.len() {
        match raw_args[index].as_str() {
            "--json" => json = true,
            "--sections" => {
                let Some(raw_sections) = raw_args.get(index + 1) else {
                    return Err("--sections requires a comma-separated group list".into());
                };
                sections = inspect::InspectSections::parse(raw_sections)?;
                index += 1;
            }
            arg if arg.starts_with("--") => return Err(format!("unknown argument: {arg}")),
            path => file_paths.push(path.to_string()),
        }
        index += 1;
    }
    Ok(Args {
        json,
        sections,
        file_paths,
    })
}
