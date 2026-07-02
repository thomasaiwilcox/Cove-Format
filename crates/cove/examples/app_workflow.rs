use std::error::Error;

use cove::prelude::*;

fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::temp_dir().join("cove_app_workflow_example");
    std::fs::create_dir_all(&dir)?;
    let csv_path = dir.join("people.csv");
    let cove_path = dir.join("people.cove");

    std::fs::write(&csv_path, "id,name\n1,Ada\n2,Linus\n")?;

    let converted = cove::convert_file(
        &csv_path,
        cove::convert::ConversionOptions {
            source_format: Some(cove::convert::SourceFormat::Csv),
            ..cove::convert::ConversionOptions::default()
        },
    )?;
    std::fs::write(&cove_path, &converted.cove_bytes)?;

    let discovery = discover_query_surfaces_file(&cove_path)?;
    println!(
        "artifact={} queryable={}",
        discovery.artifact_label, discovery.queryable
    );

    let rows = query_file(
        &cove_path,
        "table(people).select(id, name)",
        QueryOptions {
            take: Some(2),
            ..QueryOptions::default()
        },
    )?
    .result_json()?;
    println!("{}", serde_json::to_string_pretty(&rows)?);

    let explain = explain_query_file(
        &cove_path,
        "table(people).select(id, name)",
        ExplainOptions {
            mode: ExplainMode::Developer,
            ..ExplainOptions::default()
        },
    )?;
    println!("{}", explain.text);

    Ok(())
}
