use std::fs;

use cove_arrow::convert::convert_parquet_bytes;

use crate::{
    cli::{parse_conversion_args, publish_conversion_result, set_source_identity, usage},
    source::{convert_file_to_cove, ConversionOptions, CsvReadOptions, SourceFormat},
};

pub fn run_parquet(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let Some(mut command) = parse_conversion_args(args, "input.parquet", "parquet_import")? else {
        println!("{}", usage("cove convert parquet", "input.parquet"));
        return Ok(());
    };
    let input = fs::read(&command.input)
        .map_err(|err| format!("cannot read {}: {err}", command.input.display()))?;
    set_source_identity(&mut command.options, &command.input, &input)?;
    let result = convert_parquet_bytes(&input, &command.options).map_err(|err| err.to_string())?;
    publish_conversion_result(command, result)
}

pub fn run_arrow(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let Some(command) = parse_conversion_args(args, "input.arrow|input.feather", "arrow_import")?
    else {
        println!(
            "{}",
            usage("cove convert arrow", "input.arrow|input.feather")
        );
        return Ok(());
    };
    let input = command.input.clone();
    let result = convert_file_to_cove(
        &input,
        ConversionOptions {
            source_format: Some(SourceFormat::ArrowIpc),
            cove: command.options.clone(),
            ..ConversionOptions::default()
        },
    )?;
    publish_conversion_result(command, result)
}

pub fn run_orc(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let Some(command) = parse_conversion_args(args, "input.orc", "orc_import")? else {
        println!("{}", usage("cove convert orc", "input.orc"));
        return Ok(());
    };
    let input = command.input.clone();
    let result = convert_file_to_cove(
        &input,
        ConversionOptions {
            source_format: Some(SourceFormat::Orc),
            cove: command.options.clone(),
            ..ConversionOptions::default()
        },
    )?;
    publish_conversion_result(command, result)
}

pub fn run_csv(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let (csv_options, conversion_args) = parse_csv_options(args.into_iter().collect())?;
    let Some(command) = parse_conversion_args(conversion_args, "input.csv", "csv_import")? else {
        println!("{}", csv_usage());
        return Ok(());
    };
    let input = command.input.clone();
    let result = convert_file_to_cove(
        &input,
        ConversionOptions {
            source_format: Some(SourceFormat::Csv),
            cove: command.options.clone(),
            csv: csv_options,
        },
    )?;
    publish_conversion_result(command, result)
}

pub fn run_report(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    crate::conversion_report::run(args.into_iter().collect())
}

fn parse_csv_options(args: Vec<String>) -> Result<(CsvReadOptions, Vec<String>), String> {
    let mut csv = CsvReadOptions::default();
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--csv-header" => csv.has_header = true,
            "--no-csv-header" => csv.has_header = false,
            "--csv-delimiter" => csv.delimiter = parse_delimiter(&next_value(&mut iter, &arg)?)?,
            "--csv-infer-rows" => {
                let raw = next_value(&mut iter, &arg)?;
                csv.infer_rows = if raw == "all" {
                    None
                } else {
                    Some(
                        raw.parse::<usize>()
                            .map_err(|_| "--csv-infer-rows must be a usize or `all`".to_string())?,
                    )
                };
            }
            "--csv-batch-size" => {
                csv.batch_size = next_value(&mut iter, &arg)?
                    .parse::<usize>()
                    .map_err(|_| "--csv-batch-size must be a usize".to_string())?;
                if csv.batch_size == 0 {
                    return Err("--csv-batch-size must be greater than zero".into());
                }
            }
            "--csv-allow-truncated-rows" => csv.allow_truncated_rows = true,
            _ => rest.push(arg),
        }
    }
    Ok((csv, rest))
}

fn parse_delimiter(value: &str) -> Result<u8, String> {
    match value {
        "tab" | "\\t" => Ok(b'\t'),
        _ if value.len() == 1 => Ok(value.as_bytes()[0]),
        _ => Err("--csv-delimiter must be one byte, `tab`, or `\\t`".into()),
    }
}

fn next_value(iter: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn csv_usage() -> String {
    format!(
        "{}\nCSV options: [--csv-header|--no-csv-header] [--csv-delimiter <byte|tab>] [--csv-infer-rows <n|all>] [--csv-batch-size <n>] [--csv-allow-truncated-rows]",
        usage("cove convert csv", "input.csv")
    )
}
