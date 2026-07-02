fn format_execution_error(error: coveql::BuildExecutionError, json_diagnostics: bool) -> String {
    if json_diagnostics {
        return serde_json::to_string_pretty(&error.diagnostics)
            .unwrap_or_else(|_| error.to_string());
    }
    if let Some(diagnostic) = error.diagnostics.first() {
        format!(
            "{} [{}]: {}\n{}",
            diagnostic.code,
            diagnostic.phase,
            diagnostic.message,
            beginner_suggestion(&diagnostic.code)
        )
    } else {
        error.to_string()
    }
}

fn json_pretty_string<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| format!("cannot serialize JSON output: {error}"))
}

fn json_pretty_bytes<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot serialize JSON output: {error}"))
}

fn print_json_pretty<T: serde::Serialize + ?Sized>(value: &T) -> Result<(), String> {
    println!("{}", json_pretty_string(value)?);
    Ok(())
}

fn eprint_json_pretty<T: serde::Serialize + ?Sized>(value: &T) -> Result<(), String> {
    eprintln!("{}", json_pretty_string(value)?);
    Ok(())
}

fn beginner_suggestion(code: &str) -> &'static str {
    match code {
        "E_UNKNOWN_TABLE_SURFACE" => {
            "Try `cove inspect --queries <file>` to see table names this file exposes."
        }
        "E_UNKNOWN_ROOT" | "E_UNKNOWN_OBJECT_TYPE" => {
            "Try `cove inspect --queries <file>` to see object types this file exposes."
        }
        "E_PARSE" => "Check the CoveQL syntax or start from a suggested query.",
        _ => "Run with `--json-diagnostics` for structured diagnostic details.",
    }
}
