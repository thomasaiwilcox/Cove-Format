use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use cove_arrow::convert::{
    ParquetAccelerationPolicy, ParquetAggregatePolicy, ParquetClusteringPolicy,
    ParquetConversionOptions, ParquetConversionResult, ParquetDictionaryPolicy, ParquetStatsPolicy,
};
use cove_core::{constants::CompressionCodec, durable, CoveError};

use crate::source::ConvertError;

#[derive(Debug)]
#[non_exhaustive]
pub enum ConvertCliError {
    MissingValue {
        flag: &'static str,
    },
    InvalidValue {
        message: &'static str,
    },
    UnknownOption {
        option: String,
    },
    InvalidArity {
        input_label: String,
    },
    Publish {
        path: PathBuf,
        source: CoveError,
    },
    SerializeReport {
        source: serde_json::Error,
    },
    WriteReport {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ConvertCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertCliError::MissingValue { flag } => write!(f, "{flag} requires a value"),
            ConvertCliError::InvalidValue { message } => f.write_str(message),
            ConvertCliError::UnknownOption { option } => write!(f, "unknown option {option}"),
            ConvertCliError::InvalidArity { input_label } => {
                write!(f, "expected <{input_label}> and <output.cove>")
            }
            ConvertCliError::Publish { path, source } => {
                write!(f, "cannot durably publish {}: {source}", path.display())
            }
            ConvertCliError::SerializeReport { source } => {
                write!(f, "cannot serialize conversion report: {source}")
            }
            ConvertCliError::WriteReport { path, source } => {
                write!(f, "cannot write {}: {source}", path.display())
            }
        }
    }
}

impl Error for ConvertCliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ConvertCliError::Publish { source, .. } => Some(source),
            ConvertCliError::WriteReport { source, .. } => Some(source),
            ConvertCliError::SerializeReport { source } => Some(source),
            ConvertCliError::MissingValue { .. }
            | ConvertCliError::InvalidValue { .. }
            | ConvertCliError::UnknownOption { .. }
            | ConvertCliError::InvalidArity { .. } => None,
        }
    }
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionCommand {
    pub input: PathBuf,
    pub output: PathBuf,
    pub options: ParquetConversionOptions,
    pub report: Option<ReportTarget>,
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportTarget {
    Stdout,
    Path(PathBuf),
}

pub fn parse_conversion_args(
    args: impl IntoIterator<Item = String>,
    input_label: &str,
    default_table_name: &str,
) -> Result<Option<ConversionCommand>, ConvertCliError> {
    let mut options = ParquetConversionOptions {
        table_name: default_table_name.to_string(),
        ..ParquetConversionOptions::default()
    };
    let mut report = None;
    let mut positional = Vec::new();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--table-name" => options.table_name = next_value(&mut iter, "--table-name")?,
            "--namespace" => options.namespace = next_value(&mut iter, "--namespace")?,
            "--morsel-row-count" => {
                let raw = next_value(&mut iter, "--morsel-row-count")?;
                options.morsel_row_count = raw
                    .parse::<u32>()
                    .map_err(|_| invalid_value("--morsel-row-count must be a u32"))?;
                if options.morsel_row_count == 0 {
                    return Err(invalid_value(
                        "--morsel-row-count must be greater than zero",
                    ));
                }
            }
            "--segment-row-count" => {
                let raw = next_value(&mut iter, "--segment-row-count")?;
                options.segment_row_count = raw
                    .parse::<u32>()
                    .map_err(|_| invalid_value("--segment-row-count must be a u32"))?;
                if options.segment_row_count == 0 {
                    return Err(invalid_value(
                        "--segment-row-count must be greater than zero",
                    ));
                }
            }
            "--compression" => {
                options.page_compression =
                    parse_compression(&next_value(&mut iter, "--compression")?)?;
            }
            "--dictionary-policy" => {
                options.dictionary_policy =
                    parse_dictionary_policy(&next_value(&mut iter, "--dictionary-policy")?)?;
            }
            "--stats-policy" => {
                options.stats_policy =
                    parse_stats_policy(&next_value(&mut iter, "--stats-policy")?)?;
            }
            "--acceleration-policy" => {
                options.acceleration_policy =
                    parse_acceleration_policy(&next_value(&mut iter, "--acceleration-policy")?)?;
            }
            "--point-lookup-columns" => {
                options.point_lookup_columns =
                    parse_csv_list(&next_value(&mut iter, "--point-lookup-columns")?);
            }
            "--cluster-columns" => {
                options.cluster_columns =
                    parse_csv_list(&next_value(&mut iter, "--cluster-columns")?);
            }
            "--topn-columns" => {
                options.topn_columns = parse_csv_list(&next_value(&mut iter, "--topn-columns")?);
            }
            "--aggregate-synopsis" => {
                options.aggregate_policy =
                    parse_aggregate_policy(&next_value(&mut iter, "--aggregate-synopsis")?)?;
            }
            "--aggregate-columns" => {
                options.aggregate_columns =
                    parse_csv_list(&next_value(&mut iter, "--aggregate-columns")?);
            }
            "--aggregate-topk-columns" => {
                options.aggregate_topk_columns =
                    parse_csv_list(&next_value(&mut iter, "--aggregate-topk-columns")?);
            }
            "--distinct-sketch-columns" => {
                options.distinct_sketch_columns =
                    parse_csv_list(&next_value(&mut iter, "--distinct-sketch-columns")?);
            }
            "--quantile-sketch-columns" => {
                options.quantile_sketch_columns =
                    parse_csv_list(&next_value(&mut iter, "--quantile-sketch-columns")?);
            }
            "--aggregate-topk-k" => {
                let raw = next_value(&mut iter, "--aggregate-topk-k")?;
                options.aggregate_topk_k = raw
                    .parse::<u32>()
                    .map_err(|_| invalid_value("--aggregate-topk-k must be a u32"))?;
                if options.aggregate_topk_k == 0 {
                    return Err(invalid_value(
                        "--aggregate-topk-k must be greater than zero",
                    ));
                }
            }
            "--hll-precision" => {
                let raw = next_value(&mut iter, "--hll-precision")?;
                options.hll_precision = raw
                    .parse::<u8>()
                    .map_err(|_| invalid_value("--hll-precision must be a u8"))?;
            }
            "--kll-k" => {
                let raw = next_value(&mut iter, "--kll-k")?;
                options.kll_k = raw
                    .parse::<u32>()
                    .map_err(|_| invalid_value("--kll-k must be a u32"))?;
                if options.kll_k < 8 {
                    return Err(invalid_value("--kll-k must be at least 8"));
                }
            }
            "--composite-zone" => {
                options
                    .composite_zone_groups
                    .push(parse_csv_list(&next_value(&mut iter, "--composite-zone")?));
            }
            "--emit-covx" => options.emit_covx = true,
            "--emit-covm" => options.emit_covm = true,
            "--stable-clustering" => {
                options.clustering_policy = ParquetClusteringPolicy::StableClusterDeclaredColumns;
            }
            "--report" => {
                let raw = next_value(&mut iter, "--report")?;
                report = Some(if raw == "-" {
                    ReportTarget::Stdout
                } else {
                    ReportTarget::Path(PathBuf::from(raw))
                });
            }
            _ if arg.starts_with('-') => {
                return Err(ConvertCliError::UnknownOption { option: arg })
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }

    if positional.len() != 2 {
        return Err(ConvertCliError::InvalidArity {
            input_label: input_label.to_string(),
        });
    }
    Ok(Some(ConversionCommand {
        input: positional.remove(0),
        output: positional.remove(0),
        options,
        report,
    }))
}

pub fn publish_conversion_result(
    command: ConversionCommand,
    result: ParquetConversionResult,
) -> Result<(), ConvertCliError> {
    publish_bytes(&command.output, &result.cove_bytes)?;
    if let Some(covx_bytes) = &result.covx_bytes {
        let path = command.output.with_extension("covx");
        publish_bytes(&path, covx_bytes)?;
    }
    if let Some(covm_bytes) = &result.covm_bytes {
        let path = command.output.with_extension("covm");
        publish_bytes(&path, covm_bytes)?;
    }

    if let Some(target) = command.report {
        let report = serde_json::to_string_pretty(&result.report.to_json_value())
            .map_err(|source| ConvertCliError::SerializeReport { source })?;
        match target {
            ReportTarget::Stdout => println!("{report}"),
            ReportTarget::Path(path) => fs::write(&path, report)
                .map_err(|source| ConvertCliError::WriteReport { path, source })?,
        }
    } else {
        eprintln!(
            "converted {} rows and {} columns to {}",
            result.report.row_count,
            result.report.column_count,
            command.output.display()
        );
    }
    Ok(())
}

pub fn set_source_identity(
    options: &mut ParquetConversionOptions,
    input: &std::path::Path,
    bytes: &[u8],
) -> Result<(), ConvertError> {
    options.source_identifier = Some(input.display().to_string());
    options.source_digest = Some(source_digest(bytes)?);
    Ok(())
}

pub fn source_digest(bytes: &[u8]) -> Result<String, ConvertError> {
    crate::source::source_digest(bytes)
}

fn next_value(
    iter: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<String, ConvertCliError> {
    iter.next().ok_or(ConvertCliError::MissingValue { flag })
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_compression(raw: &str) -> Result<CompressionCodec, ConvertCliError> {
    match raw {
        "none" => Ok(CompressionCodec::None),
        "lz4" => Ok(CompressionCodec::Lz4),
        "zstd" => Ok(CompressionCodec::Zstd),
        _ => Err(invalid_value(
            "--compression must be one of: none, lz4, zstd",
        )),
    }
}

fn parse_dictionary_policy(raw: &str) -> Result<ParquetDictionaryPolicy, ConvertCliError> {
    match raw {
        "auto" => Ok(ParquetDictionaryPolicy::Auto),
        "never" => Ok(ParquetDictionaryPolicy::Never),
        "always" => Ok(ParquetDictionaryPolicy::Always),
        _ => Err(invalid_value(
            "--dictionary-policy must be one of: auto, never, always",
        )),
    }
}

fn parse_stats_policy(raw: &str) -> Result<ParquetStatsPolicy, ConvertCliError> {
    match raw {
        "none" => Ok(ParquetStatsPolicy::None),
        "recompute" => Ok(ParquetStatsPolicy::Recompute),
        _ => Err(invalid_value(
            "--stats-policy must be one of: none, recompute",
        )),
    }
}

fn parse_acceleration_policy(raw: &str) -> Result<ParquetAccelerationPolicy, ConvertCliError> {
    match raw {
        "none" => Ok(ParquetAccelerationPolicy::None),
        "declared-only" => Ok(ParquetAccelerationPolicy::DeclaredOnly),
        "auto" => Ok(ParquetAccelerationPolicy::Auto),
        _ => Err(invalid_value(
            "--acceleration-policy must be one of: none, declared-only, auto",
        )),
    }
}

fn parse_aggregate_policy(raw: &str) -> Result<ParquetAggregatePolicy, ConvertCliError> {
    match raw {
        "none" => Ok(ParquetAggregatePolicy::None),
        "declared-only" => Ok(ParquetAggregatePolicy::DeclaredOnly),
        "auto" => Ok(ParquetAggregatePolicy::Auto),
        _ => Err(invalid_value(
            "--aggregate-synopsis must be one of: none, declared-only, auto",
        )),
    }
}

fn publish_bytes(path: &Path, bytes: &[u8]) -> Result<(), ConvertCliError> {
    durable::durable_replace(path, bytes).map_err(|source| ConvertCliError::Publish {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn invalid_value(message: &'static str) -> ConvertCliError {
    ConvertCliError::InvalidValue { message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_reports_typed_usage_errors_with_stable_text() {
        let missing_value =
            parse_conversion_args(["--table-name".to_string()], "input.csv", "csv_import")
                .unwrap_err();
        assert!(matches!(
            missing_value,
            ConvertCliError::MissingValue {
                flag: "--table-name"
            }
        ));
        assert_eq!(missing_value.to_string(), "--table-name requires a value");

        let unknown = parse_conversion_args(
            [
                "--surprise".to_string(),
                "in.csv".to_string(),
                "out.cove".to_string(),
            ],
            "input.csv",
            "csv_import",
        )
        .unwrap_err();
        assert!(matches!(unknown, ConvertCliError::UnknownOption { .. }));
        assert_eq!(unknown.to_string(), "unknown option --surprise");

        let invalid_arity =
            parse_conversion_args(["in.csv".to_string()], "input.csv", "csv_import").unwrap_err();
        assert!(matches!(
            invalid_arity,
            ConvertCliError::InvalidArity { .. }
        ));
        assert_eq!(
            invalid_arity.to_string(),
            "expected <input.csv> and <output.cove>"
        );
    }
}
