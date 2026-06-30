use std::{error::Error, fmt, fs, path::PathBuf};

use cove_arrow::convert::convert_parquet_bytes;
use cove_convert::{cli::ConvertCliError, source::ConvertError};

use crate::{
    cli::{parse_conversion_args, publish_conversion_result, set_source_identity},
    source::{convert_file_to_cove, ConversionOptions, CsvReadOptions, SourceFormat},
};

#[derive(Debug)]
#[non_exhaustive]
pub enum ConvertParquetError {
    Cli(ConvertCliError),
    Convert(ConvertError),
    ReadInput {
        path: PathBuf,
        source: std::io::Error,
    },
    Parquet {
        message: String,
    },
    InvalidInput {
        message: String,
    },
    Report {
        message: String,
    },
}

impl fmt::Display for ConvertParquetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli(source) => source.fmt(f),
            Self::Convert(source) => source.fmt(f),
            Self::ReadInput { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
            Self::Parquet { message }
            | Self::InvalidInput { message }
            | Self::Report { message } => f.write_str(message),
        }
    }
}

impl Error for ConvertParquetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cli(source) => Some(source),
            Self::Convert(source) => Some(source),
            Self::ReadInput { source, .. } => Some(source),
            Self::Parquet { .. } | Self::InvalidInput { .. } | Self::Report { .. } => None,
        }
    }
}

impl From<ConvertCliError> for ConvertParquetError {
    fn from(source: ConvertCliError) -> Self {
        Self::Cli(source)
    }
}

impl From<ConvertError> for ConvertParquetError {
    fn from(source: ConvertError) -> Self {
        Self::Convert(source)
    }
}

impl From<String> for ConvertParquetError {
    fn from(message: String) -> Self {
        Self::InvalidInput { message }
    }
}

impl From<&str> for ConvertParquetError {
    fn from(message: &str) -> Self {
        Self::InvalidInput {
            message: message.to_string(),
        }
    }
}

pub fn run_parquet(args: impl IntoIterator<Item = String>) -> Result<(), ConvertParquetError> {
    let Some(mut command) = parse_conversion_args(args, "input.parquet", "parquet_import")? else {
        println!("{}", usage("cove convert parquet", "input.parquet"));
        return Ok(());
    };
    let input = fs::read(&command.input).map_err(|source| ConvertParquetError::ReadInput {
        path: command.input.clone(),
        source,
    })?;
    set_source_identity(&mut command.options, &command.input, &input)?;
    let result = convert_parquet_bytes(&input, &command.options).map_err(|err| {
        ConvertParquetError::Parquet {
            message: err.to_string(),
        }
    })?;
    Ok(publish_conversion_result(command, result)?)
}

pub fn run_arrow(args: impl IntoIterator<Item = String>) -> Result<(), ConvertParquetError> {
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
    Ok(publish_conversion_result(command, result)?)
}

pub fn run_orc(args: impl IntoIterator<Item = String>) -> Result<(), ConvertParquetError> {
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
    Ok(publish_conversion_result(command, result)?)
}

pub fn run_csv(args: impl IntoIterator<Item = String>) -> Result<(), ConvertParquetError> {
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
    Ok(publish_conversion_result(command, result)?)
}

pub fn run_report(args: impl IntoIterator<Item = String>) -> Result<(), ConvertParquetError> {
    crate::conversion_report::run(args.into_iter().collect())
        .map_err(|message| ConvertParquetError::Report { message })
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

fn usage(binary: &str, input_label: &str) -> String {
    format!(
        "Usage: {binary} [options] <{input_label}> <output.cove>\n\n\
Options:\n  \
--table-name <name>         Output COVE table name\n  \
--namespace <name>          Output COVE namespace (default: interop)\n  \
--morsel-row-count <rows>   Rows per COVE morsel/page (default: 4096)\n  \
--segment-row-count <rows>  Rows per COVE segment (default: u32::MAX)\n  \
--compression <codec>       Page compression: none, lz4, zstd (default: none)\n  \
--dictionary-policy <mode>  Dictionary synthesis policy: auto, never, always\n  \
--stats-policy <mode>       Stats policy: none, recompute\n  \
--acceleration-policy <m>   Index policy: none, declared-only, auto\n  \
--point-lookup-columns <c>  Comma-separated columns eligible for lookup indexes\n  \
--cluster-columns <cols>    Comma-separated stable clustering columns\n  \
--topn-columns <cols>       Comma-separated ordered hot columns for Top-N summaries\n  \
--aggregate-synopsis <m>    Aggregate synopsis policy: none, declared-only, auto\n  \
--aggregate-columns <cols>  Comma-separated columns for declared aggregate synopsis\n  \
--aggregate-topk-columns <c> Columns for TopK aggregate synopsis payloads\n  \
--distinct-sketch-columns <c> Columns for HLL distinct sketch payloads\n  \
--quantile-sketch-columns <c> Columns for KLL quantile sketch payloads\n  \
--aggregate-topk-k <n>      TopK payload size (default: 64)\n  \
--hll-precision <p>         HLL precision for distinct sketches (default: 14)\n  \
--kll-k <n>                 KLL compactor k for quantile sketches (default: 200)\n  \
--composite-zone <cols>     Comma-separated composite zone group; may be repeated\n  \
--stable-clustering         Opt in to stable clustering when implemented\n  \
--emit-covx                 Request COVX artifact emission\n  \
--emit-covm                 Request COVM artifact emission\n  \
--report <path|->           Write the machine-readable conversion report\n  \
-h, --help                  Show this help"
    )
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
