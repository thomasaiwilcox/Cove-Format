use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn main() {
    let sample_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/coveql");
    let events = sample_dir.join("events.cove");
    let people = sample_dir.join("people.cove");
    let cove = cove_bin();

    let cases = [
        (
            "inspect_events",
            vec![
                "inspect".to_string(),
                events.display().to_string(),
                "--queries".to_string(),
            ],
        ),
        (
            "query_events_table",
            vec![
                "query".to_string(),
                events.display().to_string(),
                "--format".to_string(),
                "jsonl".to_string(),
                "table(events).where(score >= 20).select(id, score)".to_string(),
            ],
        ),
        (
            "query_people_mapped_table",
            vec![
                "query".to_string(),
                people.display().to_string(),
                "--format".to_string(),
                "jsonl".to_string(),
                "table(people).where(score >= 20).select(score, status).take(2)".to_string(),
            ],
        ),
        (
            "query_events_compare",
            vec![
                "query".to_string(),
                "--engine".to_string(),
                "compare".to_string(),
                events.display().to_string(),
                "--format".to_string(),
                "jsonl".to_string(),
                "table(events).where(score >= 20).select(id, score)".to_string(),
            ],
        ),
    ];

    println!("cove CLI regression benchmark");
    println!("binary: {}", cove.display());
    for (name, args) in cases {
        let duration = time_case(&cove, &args, 10);
        println!("{name}: {:.3} ms/iter", duration.as_secs_f64() * 1000.0);
    }
}

fn cove_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_cove") {
        return PathBuf::from(path);
    }
    let mut path = std::env::current_exe().expect("benchmark executable path");
    path.pop();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.push(if cfg!(windows) { "cove.exe" } else { "cove" });
    path
}

fn time_case(cove: &Path, args: &[String], iterations: u32) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        let start = Instant::now();
        let output = Command::new(cove)
            .args(args)
            .output()
            .expect("run cove benchmark command");
        assert!(
            output.status.success(),
            "command failed: {} {}\nstdout={}\nstderr={}",
            cove.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        total += start.elapsed();
    }
    total / iterations
}
