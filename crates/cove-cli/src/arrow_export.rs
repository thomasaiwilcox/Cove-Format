use std::path::PathBuf;

use arrow_ipc::writer::FileWriter;
use arrow_json::writer::{LineDelimited, WriterBuilder};
use arrow_schema::SchemaRef;
use cove_core::artifact::covm::CovmDeltaPruneRequest;
use cove_core::durable;
use cove_core::profile::cove_map::{MapProjectionCatalog, MapProjectionEntry};
use cove_datafusion::decode::DecodeStats;
use cove_datafusion::delta_snapshot::{
    delta_chain_required, delta_snapshot_plan_json, load_validated_delta_snapshot,
    materialize_validated_delta_snapshot, read_validated_delta_object_surface,
    ValidatedDeltaSnapshot,
};
use cove_datafusion::explain::{
    execute_planned_scan, parse_filter_dsl, parse_projection_dsl, plan_bytes, plan_local_file,
    ExplainOptions, FilterDsl, FilterOp,
};
use cove_map::{
    ProjectionBatchOptions, ProjectionFilter, ProjectionFilterLiteral, ProjectionFilterOp,
};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Ipc,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReportTarget {
    Stdout,
    Path(PathBuf),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DeltaExportOptions {
    dataset: Option<PathBuf>,
    request: CovmDeltaPruneRequest,
    plan_json: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let Some(parsed) = parse_args(args)? else {
        print_usage();
        return Ok(());
    };
    let input_bytes = std::fs::read(&parsed.input)
        .map_err(|err| format!("cannot read {}: {err}", parsed.input.display()))?;
    let delta_manifest = delta_chain_required(&input_bytes).unwrap_or(false);
    let mut delta_report = None;
    let mut delta_execution = None;
    let (source, schema, batches, rows, columns, decode_stats) = if delta_manifest {
        let dataset = parsed
            .delta_options
            .dataset
            .as_deref()
            .ok_or_else(|| "delta manifest export requires --dataset <dir>".to_string())?;
        let snapshot =
            load_validated_delta_snapshot(&parsed.input, dataset, parsed.delta_options.request)
                .map_err(|error| error.to_string())?;
        let plan_json =
            delta_snapshot_plan_json(Some(&parsed.input), &snapshot.plan, &snapshot.extension);
        if parsed.delta_options.plan_json {
            let text = serde_json::to_string_pretty(&plan_json)
                .map_err(|err| format!("cannot serialize delta plan report: {err}"))?;
            eprintln!("{text}");
        }
        delta_report = Some(plan_json);
        if let Some(direct) = try_direct_delta_projection_export(&snapshot, &parsed.options)? {
            delta_execution = Some("direct_projection_surface");
            (
                format!("{}#delta-projection-surface", parsed.input.display()),
                direct.schema,
                direct.batches,
                direct.rows,
                direct.columns,
                direct_projection_decode_stats(direct.rows),
            )
        } else {
            let materialized = materialize_validated_delta_snapshot(&snapshot)
                .map_err(|error| error.to_string())?;
            let planned = plan_bytes(
                format!("{}#delta-snapshot", parsed.input.display()),
                materialized.bytes,
                parsed.options,
            )
            .map_err(|err| err.to_string())?;
            let decoded = execute_planned_scan(&planned).map_err(|err| err.to_string())?;
            let schema = planned.plan.output_schema.clone();
            let rows = decoded.stats.rows_materialized;
            let columns = schema.fields().len();
            (
                planned.state.source().to_string(),
                schema,
                decoded.batches,
                rows,
                columns,
                decode_stats_json(&decoded.stats),
            )
        }
    } else {
        if parsed.delta_options.request != CovmDeltaPruneRequest::default()
            || parsed.delta_options.plan_json
        {
            return Err("delta snapshot options require a COVM delta manifest input".into());
        }
        let planned =
            plan_local_file(&parsed.input, parsed.options).map_err(|err| err.to_string())?;
        let decoded = execute_planned_scan(&planned).map_err(|err| err.to_string())?;
        let schema = planned.plan.output_schema.clone();
        let rows = decoded.stats.rows_materialized;
        let columns = schema.fields().len();
        (
            planned.state.source().to_string(),
            schema,
            decoded.batches,
            rows,
            columns,
            decode_stats_json(&decoded.stats),
        )
    };
    let bytes = match parsed.format {
        OutputFormat::Ipc => write_ipc(&schema, &batches)?,
        OutputFormat::Json => write_json(&batches)?,
    };
    durable::durable_replace(&parsed.output, &bytes)
        .map_err(|err| format!("cannot durably publish {}: {err}", parsed.output.display()))?;

    let report_json = json!({
        "version": 1,
        "source": source,
        "output": parsed.output.display().to_string(),
        "format": match parsed.format { OutputFormat::Ipc => "ipc", OutputFormat::Json => "json" },
        "execution": "native_arrow_export",
        "delta_execution": delta_execution,
        "batches": batches.len(),
        "rows": rows,
        "columns": columns,
        "decode_stats": decode_stats,
        "delta_snapshot": delta_report,
    });
    if let Some(target) = parsed.report {
        let text = serde_json::to_string_pretty(&report_json)
            .map_err(|err| format!("cannot serialize export report: {err}"))?;
        match target {
            ReportTarget::Stdout => println!("{text}"),
            ReportTarget::Path(path) => std::fs::write(&path, text)
                .map_err(|err| format!("cannot write {}: {err}", path.display()))?,
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportArgs {
    input: PathBuf,
    output: PathBuf,
    options: ExplainOptions,
    format: OutputFormat,
    report: Option<ReportTarget>,
    delta_options: DeltaExportOptions,
}

fn parse_args(args: Vec<String>) -> Result<Option<ExportArgs>, String> {
    let mut options = ExplainOptions::default();
    let mut format = OutputFormat::Ipc;
    let mut report = None;
    let mut delta_options = DeltaExportOptions::default();
    let mut positional = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--columns" | "--projection" => {
                let raw = next_value(&mut iter, &arg)?;
                options.projection = Some(parse_projection_dsl(&raw));
            }
            "--filter" => {
                let raw = next_value(&mut iter, "--filter")?;
                options
                    .filters
                    .push(parse_filter_dsl(&raw).map_err(|err| err.to_string())?);
            }
            "--format" => {
                format = parse_format(&next_value(&mut iter, "--format")?)?;
            }
            "--report" => {
                let raw = next_value(&mut iter, "--report")?;
                report = Some(if raw == "-" {
                    ReportTarget::Stdout
                } else {
                    ReportTarget::Path(PathBuf::from(raw))
                });
            }
            "--dataset" => {
                delta_options.dataset = Some(PathBuf::from(next_value(&mut iter, "--dataset")?));
            }
            "--as-of-csn" => {
                delta_options.request.as_of_csn = Some(parse_u64(
                    &next_value(&mut iter, "--as-of-csn")?,
                    "--as-of-csn",
                )?);
            }
            "--as-of-commit-us" => {
                delta_options.request.as_of_commit_timestamp_us = Some(parse_i64(
                    &next_value(&mut iter, "--as-of-commit-us")?,
                    "--as-of-commit-us",
                )?);
            }
            "--source-publish-range" => {
                let raw = next_value(&mut iter, "--source-publish-range")?;
                delta_options.request.source_publish_range_us =
                    Some(parse_i64_range(&raw, "--source-publish-range")?);
            }
            "--delta-plan-json" => delta_options.plan_json = true,
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if positional.len() != 2 {
        return Err("expected <input.cove> and <output.arrow|output.json>".into());
    }
    Ok(Some(ExportArgs {
        input: positional.remove(0),
        output: positional.remove(0),
        options,
        format,
        report,
        delta_options,
    }))
}

struct DirectDeltaProjectionExport {
    schema: SchemaRef,
    batches: Vec<arrow_array::RecordBatch>,
    rows: usize,
    columns: usize,
}

fn try_direct_delta_projection_export(
    snapshot: &ValidatedDeltaSnapshot,
    options: &ExplainOptions,
) -> Result<Option<DirectDeltaProjectionExport>, String> {
    let surface = match read_validated_delta_object_surface(snapshot) {
        Ok(surface) => surface,
        Err(_) => return Ok(None),
    };
    if surface.object_types.is_empty() {
        return Ok(None);
    }
    let Some(catalog) = surface.projection_catalog.as_ref() else {
        return Err(
            "native table-style delta export over COVE-O requires a MAP projection catalog; use `cove export arrow --query ...` for object roots"
                .into(),
        );
    };
    let projection = single_arrow_projection(catalog)?;
    let projection_options = projection_batch_options(projection, options)?;
    let batches = cove_map::projected_record_batches_from_cove_o_surface_with_catalog(
        &surface,
        catalog,
        &projection.projection_id,
        &projection_options,
    )
    .map_err(|err| err.to_string())?;
    let schema = batches
        .first()
        .map(arrow_array::RecordBatch::schema)
        .ok_or_else(|| "direct delta projection export produced no Arrow batches".to_string())?;
    let rows = batches
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .sum::<usize>();
    let columns = schema.fields().len();
    Ok(Some(DirectDeltaProjectionExport {
        schema,
        batches,
        rows,
        columns,
    }))
}

fn single_arrow_projection(catalog: &MapProjectionCatalog) -> Result<&MapProjectionEntry, String> {
    let projections = catalog
        .projections
        .iter()
        .filter(|projection| projection.output_modes.iter().any(|mode| mode == "arrow"))
        .collect::<Vec<_>>();
    match projections.as_slice() {
        [projection] => Ok(*projection),
        [] => Err(
            "native table-style delta export found no Arrow projection in the selected COVE-O snapshot; use `cove export arrow --query ...` with an explicit CoveQL root"
                .into(),
        ),
        many => Err(format!(
            "native table-style delta export found multiple Arrow projections ({}); use `cove export arrow --query ...` to select one explicitly",
            many.iter()
                .map(|projection| projection.projection_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn projection_batch_options(
    projection: &MapProjectionEntry,
    options: &ExplainOptions,
) -> Result<ProjectionBatchOptions, String> {
    let output_columns = options
        .projection
        .as_ref()
        .map(|columns| {
            columns
                .iter()
                .map(|column| resolve_projection_column(projection, column))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let pushed_filters = options
        .filters
        .iter()
        .map(|filter| projection_filter(projection, filter))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProjectionBatchOptions {
        output_columns,
        pushed_filters,
        ..ProjectionBatchOptions::default()
    })
}

fn resolve_projection_column(projection: &MapProjectionEntry, raw: &str) -> Result<String, String> {
    if let Ok(index) = raw.parse::<usize>() {
        return projection
            .columns
            .get(index)
            .map(|column| column.name.clone())
            .ok_or_else(|| {
                format!(
                    "column index {index} is out of bounds for projection '{}' with {} columns",
                    projection.projection_id,
                    projection.columns.len()
                )
            });
    }
    projection
        .columns
        .iter()
        .find(|column| column.name == raw)
        .map(|column| column.name.clone())
        .ok_or_else(|| {
            format!(
                "unknown projection column {raw:?} in projection '{}'",
                projection.projection_id
            )
        })
}

fn projection_filter(
    projection: &MapProjectionEntry,
    filter: &FilterDsl,
) -> Result<ProjectionFilter, String> {
    let column = resolve_projection_column(projection, &filter.column)?;
    match filter.op {
        FilterOp::IsNull => Ok(ProjectionFilter::IsNull {
            column,
            negated: false,
        }),
        FilterOp::IsNotNull => Ok(ProjectionFilter::IsNull {
            column,
            negated: true,
        }),
        FilterOp::In => Ok(ProjectionFilter::InList {
            column,
            literals: filter
                .value
                .as_deref()
                .unwrap_or_default()
                .split('|')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(parse_projection_filter_literal)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        FilterOp::Eq | FilterOp::Lt | FilterOp::Lte | FilterOp::Gt | FilterOp::Gte => {
            let raw = filter
                .value
                .as_deref()
                .ok_or_else(|| "comparison filter requires value=<literal>".to_string())?;
            Ok(ProjectionFilter::Compare {
                column,
                op: match filter.op {
                    FilterOp::Eq => ProjectionFilterOp::Eq,
                    FilterOp::Lt => ProjectionFilterOp::Lt,
                    FilterOp::Lte => ProjectionFilterOp::LtEq,
                    FilterOp::Gt => ProjectionFilterOp::Gt,
                    FilterOp::Gte => ProjectionFilterOp::GtEq,
                    _ => unreachable!("covered by outer match"),
                },
                literal: parse_projection_filter_literal(raw)?,
            })
        }
    }
}

fn parse_projection_filter_literal(raw: &str) -> Result<ProjectionFilterLiteral, String> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return Ok(ProjectionFilterLiteral::Null);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return match value {
            serde_json::Value::Null => Ok(ProjectionFilterLiteral::Null),
            serde_json::Value::Bool(value) => Ok(ProjectionFilterLiteral::Boolean(value)),
            serde_json::Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    Ok(ProjectionFilterLiteral::Int64(value))
                } else if let Some(value) = number.as_u64() {
                    Ok(ProjectionFilterLiteral::UInt64(value))
                } else if let Some(value) = number.as_f64() {
                    Ok(ProjectionFilterLiteral::Float64(value))
                } else {
                    Err(format!("unsupported numeric filter literal {trimmed:?}"))
                }
            }
            serde_json::Value::String(value) => Ok(ProjectionFilterLiteral::Utf8(value)),
            _ => Err("projection filters require scalar literals".into()),
        };
    }
    Ok(ProjectionFilterLiteral::Utf8(trimmed.to_string()))
}

fn decode_stats_json(stats: &DecodeStats) -> serde_json::Value {
    json!({
        "metadata_bytes_read": stats.metadata_bytes_read,
        "data_bytes_read": stats.data_bytes_read,
        "range_requests": stats.range_requests,
        "pages_decoded": stats.pages_decoded,
        "rows_selected": stats.rows_selected,
        "rows_materialized": stats.rows_materialized,
    })
}

fn direct_projection_decode_stats(rows: usize) -> serde_json::Value {
    json!({
        "metadata_bytes_read": 0,
        "data_bytes_read": 0,
        "range_requests": 0,
        "pages_decoded": 0,
        "rows_selected": rows,
        "rows_materialized": rows,
    })
}

pub fn write_ipc(
    schema: &arrow_schema::SchemaRef,
    batches: &[arrow_array::RecordBatch],
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut writer =
            FileWriter::try_new(&mut bytes, schema).map_err(|err| format!("IPC writer: {err}"))?;
        for batch in batches {
            writer
                .write(batch)
                .map_err(|err| format!("cannot write IPC batch: {err}"))?;
        }
        writer
            .finish()
            .map_err(|err| format!("cannot finish IPC writer: {err}"))?;
    }
    Ok(bytes)
}

pub fn write_json(batches: &[arrow_array::RecordBatch]) -> Result<Vec<u8>, String> {
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, LineDelimited>(Vec::new());
    for batch in batches {
        writer
            .write(batch)
            .map_err(|err| format!("cannot write JSON batch: {err}"))?;
    }
    writer
        .finish()
        .map_err(|err| format!("cannot finish JSON writer: {err}"))?;
    Ok(writer.into_inner())
}

fn parse_format(raw: &str) -> Result<OutputFormat, String> {
    match raw {
        "ipc" => Ok(OutputFormat::Ipc),
        "json" => Ok(OutputFormat::Json),
        _ => Err("--format must be ipc or json".into()),
    }
}

fn next_value(iter: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} requires an unsigned integer"))
}

fn parse_i64(value: &str, flag: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| format!("{flag} requires an integer"))
}

fn parse_i64_range(raw: &str, flag: &str) -> Result<(i64, i64), String> {
    let (start, end) = raw
        .split_once(':')
        .ok_or_else(|| format!("{flag} requires start:end"))?;
    Ok((parse_i64(start, flag)?, parse_i64(end, flag)?))
}

fn print_usage() {
    eprintln!(
        "usage: cove export arrow [--columns a,b] [--filter column=<name|index>,op=<eq|lt|lte|gt|gte|is-null|is-not-null>,value=<literal>] [--format ipc|json] [--report -|path] [--dataset dir] [--as-of-csn n|--as-of-commit-us n] [--delta-plan-json] <input.cove|manifest.covm> <output.arrow|output.json>\n       cove export arrow --query '<coveql>' [--format ipc|json] [--report -|path] [--dataset dir] [--as-of-csn n|--as-of-commit-us n] [--delta-plan|--delta-plan-json] <input.cove|manifest.covm> <output.arrow|output.json>"
    );
}
