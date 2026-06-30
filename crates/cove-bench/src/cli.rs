use super::*;

pub(super) fn run(args: Vec<String>) -> Result<(), BenchError> {
    match args.first().map(String::as_str) {
        Some("gen") => {
            let profile = option_value(&args, "--profile").unwrap_or_else(|| "ci".into());
            let out = option_value(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(default_corpus_dir);
            generate_corpus(&profile, &out)?;
            println!("generated {profile} benchmark corpus at {}", out.display());
            Ok(())
        }
        Some("run") => {
            let corpus = option_value(&args, "--corpus")
                .map(PathBuf::from)
                .unwrap_or_else(default_corpus_dir);
            let report_json = option_value(&args, "--report-json")
                .map(PathBuf::from)
                .unwrap_or_else(|| corpus.join("report.json"));
            let report_md = option_value(&args, "--report-md")
                .map(PathBuf::from)
                .unwrap_or_else(|| corpus.join("report.md"));
            run_corpus(&corpus, &report_json, &report_md)?;
            println!("wrote benchmark report to {}", report_json.display());
            Ok(())
        }
        Some("check") => {
            let out = default_corpus_dir();
            generate_corpus("ci", &out)?;
            run_corpus(&out, &out.join("report.json"), &out.join("report.md"))?;
            println!("cove-bench check passed at {}", out.display());
            Ok(())
        }
        Some("-h" | "--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}").into()),
    }
}

pub(super) fn print_usage() {
    println!(
        "Usage:\n  cove-bench gen --profile ci|standard|publication --out <dir>\n  cove-bench run --corpus <dir> --report-json <path> --report-md <path>\n  cove-bench check"
    );
}

pub(super) fn option_value(args: &[String], option: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == option)
        .map(|window| window[1].clone())
}

pub(super) fn default_corpus_dir() -> PathBuf {
    PathBuf::from("target/cove-bench/ci")
}
