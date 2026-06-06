use std::{
    collections::BTreeSet,
    io::{self, Write},
};

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Table,
    Json,
    Jsonl,
    Csv,
}

pub(crate) fn write_result(
    value: &Value,
    format: OutputFormat,
    max_cell_width: usize,
) -> Result<(), String> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value).unwrap());
            Ok(())
        }
        OutputFormat::Jsonl => write_jsonl(value),
        OutputFormat::Csv => write_csv(value),
        OutputFormat::Table => write_table(value, max_cell_width),
    }
}

fn rows_array(value: &Value) -> Result<&[Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "query output is not a row array".to_string())
}

fn write_jsonl(value: &Value) -> Result<(), String> {
    for row in rows_array(value)? {
        println!("{}", serde_json::to_string(row).unwrap());
    }
    Ok(())
}

fn write_csv(value: &Value) -> Result<(), String> {
    let rows = rows_array(value)?;
    let columns = output_columns(rows);
    let mut out = io::BufWriter::new(io::stdout());
    writeln_csv_row(&mut out, &columns)?;
    for row in rows {
        let values = columns
            .iter()
            .map(|column| {
                row.as_object()
                    .and_then(|object| object.get(column))
                    .map(cell_text)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        writeln_csv_row(&mut out, &values)?;
    }
    Ok(())
}

fn writeln_csv_row(out: &mut impl Write, values: &[String]) -> Result<(), String> {
    let line = values
        .iter()
        .map(|value| csv_escape(value))
        .collect::<Vec<_>>()
        .join(",");
    writeln!(out, "{line}").map_err(|error| format!("cannot write CSV: {error}"))
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn write_table(value: &Value, max_cell_width: usize) -> Result<(), String> {
    let rows = rows_array(value)?;
    let columns = output_columns(rows);
    if columns.is_empty() {
        println!("(0 columns, {} rows)", rows.len());
        return Ok(());
    }
    let rendered = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| {
                    row.as_object()
                        .and_then(|object| object.get(column))
                        .map(cell_text)
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let widths = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let value_width = rendered
                .iter()
                .map(|row| character_count(&display_cell(&row[index], max_cell_width)))
                .max()
                .unwrap_or_default();
            character_count(column).max(value_width).min(max_cell_width)
        })
        .collect::<Vec<_>>();
    print_table_separator(&widths);
    print_table_row(&columns, &widths);
    print_table_separator(&widths);
    for row in &rendered {
        print_table_row(row, &widths);
    }
    print_table_separator(&widths);
    let truncated = rendered
        .iter()
        .flatten()
        .any(|cell| character_count(&display_cell(cell, max_cell_width)) < character_count(cell));
    println!(
        "{} row{}{}",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        if truncated {
            " (long cells truncated)"
        } else {
            ""
        }
    );
    Ok(())
}

fn output_columns(rows: &[Value]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut columns = Vec::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if seen.insert(key.clone()) {
                    columns.push(key.clone());
                }
            }
        }
    }
    columns
}

fn print_table_separator(widths: &[usize]) {
    print!("+");
    for width in widths {
        print!("{}+", "-".repeat(width + 2));
    }
    println!();
}

fn print_table_row(values: &[String], widths: &[usize]) {
    print!("|");
    for (value, width) in values.iter().zip(widths) {
        let cell = display_cell(value, *width);
        print!(" {cell:<width$} |");
    }
    println!();
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn display_cell(value: &str, max_cell_width: usize) -> String {
    let clean = value.replace(['\n', '\r', '\t'], " ");
    if character_count(&clean) <= max_cell_width {
        return clean;
    }
    let mut out = clean
        .chars()
        .take(max_cell_width.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn character_count(value: &str) -> usize {
    value.chars().count()
}
