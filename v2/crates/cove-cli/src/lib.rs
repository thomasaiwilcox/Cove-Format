use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use cove_core::{
    artifact::covm::CovmFile,
    constants::SectionKind,
    durable,
    reader::{validate_bytes_with_options, ValidationOptions},
    table::TableCatalog,
    utility::{build_covm_artifact, build_covx_artifact},
    writer::ScanProfileCoveWriter,
};
use coveql::{
    acceleration_report_json, apply_acceleration_bundle, coveql_identifier,
    discover_acceleration_bundle, discover_query_surfaces, execute_query_from_artifact,
    generate_acceleration_sidecars, plan_acceleration, suggest_queries, AccelerationBundleOptions,
    ArtifactExecutionEngine, AstEvidenceGrain, CoveAccelerationBundle, CoveOptimizationOptions,
    ExecuteArtifactOptions, ExecuteArtifactQueryError, ExplainDisclosurePolicy,
    GraphTraversalContract, GraphTraversalDistinctPolicy, GraphTraversalMode, KernelExecutionMode,
    KernelExecutionOptions, PhysicalPlanOptions, PhysicalSidecarInputs, QueryArtifactMember,
    QuerySurfaceDiscovery, QuerySurfaceDiscoveryOptions, TableExecutionAuthority,
    TableSurfaceAuthority, TableSurfaceAuthorityKind, TableSurfaceColumnContract,
    TableSurfaceContract, TableSurfaceRow, TableTemporalAuthority, COVEQL_PROFILE_CONTRACT_VERSION,
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Examples {
        json: bool,
    },
    Doctor {
        file: PathBuf,
        json: bool,
    },
    Inspect {
        file: PathBuf,
        queries: bool,
        json: bool,
        performance: bool,
    },
    Optimize {
        file: PathBuf,
        out_dir: Option<PathBuf>,
        full: bool,
        json: bool,
    },
    Query(Box<QueryCommand>),
    Convert {
        format: ConvertFormat,
        args: Vec<String>,
    },
    Validate {
        args: Vec<String>,
    },
    InspectDetailed {
        args: Vec<String>,
    },
    Dump {
        args: Vec<String>,
    },
    Map {
        args: Vec<String>,
    },
    Export {
        format: ExportFormat,
        args: Vec<String>,
    },
    Perf {
        command: PerfCommand,
        args: Vec<String>,
    },
    Sidecar {
        args: Vec<String>,
    },
    Digest {
        args: Vec<String>,
    },
    Profile {
        args: Vec<String>,
    },
    Canonicalise {
        args: Vec<String>,
    },
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvertFormat {
    Parquet,
    Arrow,
    Orc,
    Csv,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Arrow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PerfCommand {
    ExplainPruning,
    PlanCost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryCommand {
    file: Option<PathBuf>,
    query: String,
    query_file: Option<PathBuf>,
    format: OutputFormat,
    take: Option<usize>,
    explain: Option<String>,
    mapping: Option<PathBuf>,
    members: Vec<(String, PathBuf)>,
    dataset: Option<PathBuf>,
    engine: QueryEngine,
    batch_size: Option<usize>,
    graph_budget: GraphBudgetOverrides,
    enable_graph_traversal: bool,
    allow_index_only: bool,
    allow_zero_copy: bool,
    physical_sidecars: QueryPhysicalSidecarPaths,
    external_tables: Vec<ExternalTableSpec>,
    no_auto_sidecars: bool,
    strict_performance: bool,
    perf_report: bool,
    json_diagnostics: bool,
    max_cell_width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Table,
    Json,
    Jsonl,
    Csv,
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    match parse_args(args)? {
        Command::Help => {
            print_usage();
            Ok(())
        }
        Command::Examples { json } => run_examples(json),
        Command::Doctor { file, json } => run_doctor(&file, json),
        Command::Inspect {
            file,
            queries,
            json,
            performance,
        } => run_inspect(&file, queries, json, performance),
        Command::Optimize {
            file,
            out_dir,
            full,
            json,
        } => run_optimize(&file, out_dir.as_deref(), full, json),
        Command::Query(command) => run_query(
            command.file.as_deref(),
            &command.query,
            QueryCommandOptions {
                query_file: command.query_file,
                format: command.format,
                take: command.take,
                explain: command.explain,
                mapping: command.mapping,
                members: command.members,
                dataset: command.dataset,
                engine: command.engine,
                batch_size: command.batch_size,
                graph_budget: command.graph_budget,
                enable_graph_traversal: command.enable_graph_traversal,
                allow_index_only: command.allow_index_only,
                allow_zero_copy: command.allow_zero_copy,
                physical_sidecars: command.physical_sidecars,
                external_tables: command.external_tables,
                no_auto_sidecars: command.no_auto_sidecars,
                strict_performance: command.strict_performance,
                perf_report: command.perf_report,
                json_diagnostics: command.json_diagnostics,
                max_cell_width: command.max_cell_width,
            },
        ),
        Command::Convert { format, args } => run_convert(format, args),
        Command::Validate { args } => run_validate(args),
        Command::InspectDetailed { args } => run_inspect_detailed(args),
        Command::Dump { args } => cove_dump::run_cli(args),
        Command::Map { args } => cove_map::run_cli(args),
        Command::Export { format, args } => run_export(format, args),
        Command::Perf { command, args } => run_perf(command, args),
        Command::Sidecar { args } => run_sidecar(args),
        Command::Digest { args } => run_digest(args),
        Command::Profile { args } => run_profile(args),
        Command::Canonicalise { args } => run_canonicalise(args),
    }
}

struct QueryCommandOptions {
    query_file: Option<PathBuf>,
    format: OutputFormat,
    take: Option<usize>,
    explain: Option<String>,
    mapping: Option<PathBuf>,
    members: Vec<(String, PathBuf)>,
    dataset: Option<PathBuf>,
    engine: QueryEngine,
    batch_size: Option<usize>,
    graph_budget: GraphBudgetOverrides,
    enable_graph_traversal: bool,
    allow_index_only: bool,
    allow_zero_copy: bool,
    physical_sidecars: QueryPhysicalSidecarPaths,
    external_tables: Vec<ExternalTableSpec>,
    no_auto_sidecars: bool,
    strict_performance: bool,
    perf_report: bool,
    json_diagnostics: bool,
    max_cell_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalTableSpec {
    table_name: String,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum QueryEngine {
    #[default]
    Auto,
    Materialized,
    Physical,
    Compare,
    Kernel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GraphBudgetOverrides {
    max_depth: Option<u32>,
    max_paths: Option<usize>,
    max_fanout: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QueryPhysicalSidecarPaths {
    coverage_plan_candidate: Option<PathBuf>,
    coverage_proof_record: Option<PathBuf>,
    coverage_set: Option<PathBuf>,
    covi_artifact: Option<PathBuf>,
    covx_artifact: Option<PathBuf>,
    layout_plan: Option<PathBuf>,
    scan_split_index: Option<PathBuf>,
    page_cluster_directory: Option<PathBuf>,
    zero_copy_buffer_map: Option<PathBuf>,
    coverage_cache: Option<PathBuf>,
    cove_e_artifact: Option<PathBuf>,
}

impl QueryPhysicalSidecarPaths {
    fn has_any(&self) -> bool {
        self.coverage_plan_candidate.is_some()
            || self.coverage_proof_record.is_some()
            || self.coverage_set.is_some()
            || self.covi_artifact.is_some()
            || self.covx_artifact.is_some()
            || self.layout_plan.is_some()
            || self.scan_split_index.is_some()
            || self.page_cluster_directory.is_some()
            || self.zero_copy_buffer_map.is_some()
            || self.coverage_cache.is_some()
            || self.cove_e_artifact.is_some()
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter().collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(Command::Help);
    }
    let command = args.remove(0);
    match command.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "examples" => parse_examples(args),
        "doctor" => parse_doctor(args),
        "inspect" => parse_inspect(args),
        "optimize" => parse_optimize(args),
        "query" => parse_query(args),
        "convert" => parse_convert(args),
        "validate" => Ok(Command::Validate { args }),
        "dump" => Ok(Command::Dump { args }),
        "map" => Ok(Command::Map { args }),
        "export" => parse_export(args),
        "perf" => parse_perf(args),
        "sidecar" => Ok(Command::Sidecar { args }),
        "digest" => parse_digest(args),
        "profile" => Ok(Command::Profile { args }),
        "canonicalise" | "canonicalize" => Ok(Command::Canonicalise { args }),
        other => Err(format!("unknown command '{other}'\n\n{}", usage())),
    }
}

fn parse_examples(args: Vec<String>) -> Result<Command, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            arg if arg.starts_with("--") => return Err(format!("unknown examples option '{arg}'")),
            _ => return Err("examples does not accept positional arguments".into()),
        }
    }
    Ok(Command::Examples { json })
}

fn parse_doctor(args: Vec<String>) -> Result<Command, String> {
    let mut json = false;
    let mut file = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            arg if arg.starts_with("--") => return Err(format!("unknown doctor option '{arg}'")),
            path => {
                if file.replace(PathBuf::from(path)).is_some() {
                    return Err("doctor accepts one file path".into());
                }
            }
        }
    }
    Ok(Command::Doctor {
        file: file.ok_or_else(|| "usage: cove doctor [--json] <file>".to_string())?,
        json,
    })
}

fn parse_inspect(args: Vec<String>) -> Result<Command, String> {
    if wants_detailed_inspect(&args) {
        return Ok(Command::InspectDetailed { args });
    }
    let mut queries = false;
    let mut json = false;
    let mut performance = false;
    let mut file = None;
    for arg in args {
        match arg.as_str() {
            "--queries" => queries = true,
            "--json" => json = true,
            "--performance" => performance = true,
            "-h" | "--help" => return Ok(Command::Help),
            arg if arg.starts_with("--") => return Err(format!("unknown inspect option '{arg}'")),
            path => {
                if file.replace(PathBuf::from(path)).is_some() {
                    return Err("inspect accepts one file path".into());
                }
            }
        }
    }
    Ok(Command::Inspect {
        file: file.ok_or_else(|| {
            "usage: cove inspect [--queries] [--performance] [--json] <file>".to_string()
        })?,
        queries,
        json,
        performance,
    })
}

fn wants_detailed_inspect(args: &[String]) -> bool {
    let mut positional = 0usize;
    for arg in args {
        match arg.as_str() {
            "--sections" => return true,
            "--queries" | "--json" | "--performance" | "-h" | "--help" => {}
            _ if arg.starts_with("--") => {}
            _ => positional += 1,
        }
    }
    positional > 1
}

fn parse_convert(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err("usage: cove convert <parquet|arrow|orc|csv|report> [options]".into());
    }
    let raw = args.remove(0);
    let format = match raw.as_str() {
        "parquet" => ConvertFormat::Parquet,
        "arrow" => ConvertFormat::Arrow,
        "orc" => ConvertFormat::Orc,
        "csv" => ConvertFormat::Csv,
        "report" => ConvertFormat::Report,
        other => {
            return Err(format!(
                "unknown convert format '{other}'; expected parquet, arrow, orc, csv, or report"
            ))
        }
    };
    Ok(Command::Convert { format, args })
}

fn parse_export(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err("usage: cove export <arrow> [options]".into());
    }
    let raw = args.remove(0);
    let format = match raw.as_str() {
        "arrow" => ExportFormat::Arrow,
        other => return Err(format!("unknown export format '{other}'; expected arrow")),
    };
    Ok(Command::Export { format, args })
}

fn parse_perf(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err("usage: cove perf <explain-pruning|plan-cost> [options]".into());
    }
    let raw = args.remove(0);
    let command = match raw.as_str() {
        "explain-pruning" => PerfCommand::ExplainPruning,
        "plan-cost" => PerfCommand::PlanCost,
        other => {
            return Err(format!(
                "unknown perf command '{other}'; expected explain-pruning or plan-cost"
            ))
        }
    };
    Ok(Command::Perf { command, args })
}

fn parse_digest(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err("usage: cove digest verify <file.cove> [--require]".into());
    }
    let command = args.remove(0);
    match command.as_str() {
        "verify" => Ok(Command::Digest { args }),
        other => Err(format!("unknown digest command '{other}'; expected verify")),
    }
}

fn parse_optimize(args: Vec<String>) -> Result<Command, String> {
    let mut out_dir = None;
    let mut full = false;
    let mut json = false;
    let mut file = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out-dir" => {
                out_dir =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out-dir requires a directory path".to_string()
                    })?));
            }
            "--full" => full = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            arg if arg.starts_with("--") => return Err(format!("unknown optimize option '{arg}'")),
            path => {
                if file.replace(PathBuf::from(path)).is_some() {
                    return Err("optimize accepts one file path".into());
                }
            }
        }
    }
    Ok(Command::Optimize {
        file: file.ok_or_else(|| {
            "usage: cove optimize <file> [--out-dir dir] [--full] [--json]".to_string()
        })?,
        out_dir,
        full,
        json,
    })
}

fn parse_query(args: Vec<String>) -> Result<Command, String> {
    let mut format = OutputFormat::Table;
    let mut take = None;
    let mut explain = None;
    let mut mapping = None;
    let mut members = Vec::new();
    let mut dataset = None;
    let mut engine = QueryEngine::Auto;
    let mut batch_size = None;
    let mut graph_budget = GraphBudgetOverrides::default();
    let mut enable_graph_traversal = false;
    let mut allow_index_only = false;
    let mut allow_zero_copy = false;
    let mut physical_sidecars = QueryPhysicalSidecarPaths::default();
    let mut external_tables = Vec::new();
    let mut no_auto_sidecars = false;
    let mut strict_performance = false;
    let mut perf_report = false;
    let mut json_diagnostics = false;
    let mut query_file = None;
    let mut max_cell_width = 32usize;
    let mut positionals = Vec::new();
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--format requires table, json, jsonl, or csv".to_string())?;
                format = parse_output_format(&value)?;
            }
            "--take" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--take requires a positive integer".to_string())?;
                take = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| "--take requires a positive integer".to_string())?,
                );
            }
            "--max-cell-width" => {
                let value = iter.next().ok_or_else(|| {
                    "--max-cell-width requires an integer from 8 to 256".to_string()
                })?;
                max_cell_width = value.parse::<usize>().map_err(|_| {
                    "--max-cell-width requires an integer from 8 to 256".to_string()
                })?;
                if !(8..=256).contains(&max_cell_width) {
                    return Err("--max-cell-width requires an integer from 8 to 256".into());
                }
            }
            "--explain" => {
                let mode = iter
                    .peek()
                    .filter(|value| is_explain_mode(value))
                    .cloned()
                    .unwrap_or_else(|| "public".into());
                if iter.peek().is_some_and(|value| is_explain_mode(value)) {
                    iter.next();
                }
                explain = Some(mode);
            }
            "--mapping" => {
                mapping =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--mapping requires a .covemap path".to_string()
                    })?));
            }
            "--external-table" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--external-table requires <table=path>".to_string())?;
                let (table_name, path) = raw
                    .split_once('=')
                    .ok_or_else(|| "--external-table requires <table=path>".to_string())?;
                if table_name.trim().is_empty() {
                    return Err("--external-table requires a non-empty table name".into());
                }
                external_tables.push(ExternalTableSpec {
                    table_name: table_name.to_string(),
                    path: PathBuf::from(path),
                });
            }
            "--engine" => {
                let value = iter.next().ok_or_else(|| {
                    "--engine requires auto, materialized, physical, compare, or kernel".to_string()
                })?;
                engine = parse_query_engine(&value)?;
            }
            "--physical" => engine = QueryEngine::Physical,
            "--compare" => engine = QueryEngine::Compare,
            "--force-kernel" => engine = QueryEngine::Kernel,
            "--batch-size" => {
                batch_size = Some(parse_positive_usize(
                    iter.next().as_deref(),
                    "--batch-size",
                )?);
            }
            "--enable-graph-traversal" => enable_graph_traversal = true,
            "--max-graph-depth" => {
                graph_budget.max_depth = Some(parse_positive_u32(
                    iter.next().as_deref(),
                    "--max-graph-depth",
                )?);
                enable_graph_traversal = true;
            }
            "--max-graph-paths" => {
                graph_budget.max_paths = Some(parse_positive_usize(
                    iter.next().as_deref(),
                    "--max-graph-paths",
                )?);
                enable_graph_traversal = true;
            }
            "--max-graph-fanout" => {
                graph_budget.max_fanout = Some(parse_positive_usize(
                    iter.next().as_deref(),
                    "--max-graph-fanout",
                )?);
                enable_graph_traversal = true;
            }
            "--allow-index-only" => allow_index_only = true,
            "--allow-zero-copy" => allow_zero_copy = true,
            "--coverage-plan" => {
                physical_sidecars.coverage_plan_candidate =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--coverage-plan requires a file path".to_string()
                    })?));
            }
            "--coverage-proof" => {
                physical_sidecars.coverage_proof_record =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--coverage-proof requires a file path".to_string()
                    })?));
            }
            "--coverage-set" => {
                physical_sidecars.coverage_set =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--coverage-set requires a file path".to_string()
                    })?));
            }
            "--covi" | "--index" => {
                physical_sidecars.covi_artifact = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--covi requires a file path".to_string())?,
                ));
            }
            "--covx" => {
                physical_sidecars.covx_artifact = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--covx requires a file path".to_string())?,
                ));
            }
            "--layout-plan" => {
                physical_sidecars.layout_plan =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--layout-plan requires a file path".to_string()
                    })?));
            }
            "--scan-split-index" => {
                physical_sidecars.scan_split_index =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--scan-split-index requires a file path".to_string()
                    })?));
            }
            "--page-cluster-directory" => {
                physical_sidecars.page_cluster_directory =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--page-cluster-directory requires a file path".to_string()
                    })?));
            }
            "--zero-copy-buffer-map" => {
                physical_sidecars.zero_copy_buffer_map =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--zero-copy-buffer-map requires a file path".to_string()
                    })?));
            }
            "--coverage-cache" => {
                physical_sidecars.coverage_cache =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--coverage-cache requires a file path".to_string()
                    })?));
            }
            "--cove-e" | "--execution-codes" => {
                physical_sidecars.cove_e_artifact =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--cove-e requires a file path".to_string()
                    })?));
            }
            "--no-auto-sidecars" => no_auto_sidecars = true,
            "--strict-performance" => strict_performance = true,
            "--perf-report" => perf_report = true,
            "--member" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--member requires <manifest-uri=path>".to_string())?;
                let (id, path) = raw
                    .split_once('=')
                    .ok_or_else(|| "--member requires <manifest-uri=path>".to_string())?;
                members.push((id.to_string(), PathBuf::from(path)));
            }
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--query-file" => {
                query_file = Some(PathBuf::from(iter.next().ok_or_else(|| {
                    "--query-file requires a path or '-' for stdin".to_string()
                })?));
            }
            "--json-diagnostics" => json_diagnostics = true,
            "-h" | "--help" => return Ok(Command::Help),
            arg if arg.starts_with("--") => return Err(format!("unknown query option '{arg}'")),
            positional => positionals.push(positional.to_string()),
        }
    }
    let (file, query) = if query_file.is_some() {
        match positionals.len() {
            0 if !external_tables.is_empty() => (None, String::new()),
            1 => (Some(PathBuf::from(positionals.remove(0))), String::new()),
            _ => return Err("usage: cove query [options] --query-file <path|-> [file]".into()),
        }
    } else {
        if positionals.len() < 2 {
            if external_tables.is_empty() || positionals.len() != 1 {
                return Err("usage: cove query [options] [file] '<coveql>'".into());
            }
            (None, positionals.remove(0))
        } else {
            let file = PathBuf::from(positionals.remove(0));
            let query = positionals.join(" ");
            (Some(file), query)
        }
    };
    Ok(Command::Query(Box::new(QueryCommand {
        file,
        query,
        query_file,
        format,
        take,
        explain,
        mapping,
        members,
        dataset,
        engine,
        batch_size,
        graph_budget,
        enable_graph_traversal,
        allow_index_only,
        allow_zero_copy,
        physical_sidecars,
        external_tables,
        no_auto_sidecars,
        strict_performance,
        perf_report,
        json_diagnostics,
        max_cell_width,
    })))
}

fn parse_query_engine(value: &str) -> Result<QueryEngine, String> {
    match value {
        "auto" => Ok(QueryEngine::Auto),
        "materialized" => Ok(QueryEngine::Materialized),
        "physical" => Ok(QueryEngine::Physical),
        "compare" => Ok(QueryEngine::Compare),
        "kernel" | "force-kernel" => Ok(QueryEngine::Kernel),
        other => Err(format!(
            "unsupported --engine '{other}'; expected auto, materialized, physical, compare, or kernel"
        )),
    }
}

fn parse_positive_usize(value: Option<&str>, flag: &str) -> Result<usize, String> {
    let parsed = value
        .ok_or_else(|| format!("{flag} requires a positive integer"))?
        .parse::<usize>()
        .map_err(|_| format!("{flag} requires a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{flag} requires a positive integer"));
    }
    Ok(parsed)
}

fn parse_positive_u32(value: Option<&str>, flag: &str) -> Result<u32, String> {
    let parsed = value
        .ok_or_else(|| format!("{flag} requires a positive integer"))?
        .parse::<u32>()
        .map_err(|_| format!("{flag} requires a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{flag} requires a positive integer"));
    }
    Ok(parsed)
}

fn parse_output_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "table" => Ok(OutputFormat::Table),
        "json" => Ok(OutputFormat::Json),
        "jsonl" => Ok(OutputFormat::Jsonl),
        "csv" => Ok(OutputFormat::Csv),
        other => Err(format!(
            "unsupported --format '{other}'; expected table, json, jsonl, or csv"
        )),
    }
}

fn is_explain_mode(value: &str) -> bool {
    matches!(
        value,
        "public" | "developer" | "proof" | "coded" | "forensic"
    )
}

fn explain_policy_for_cli(mode: &str) -> ExplainDisclosurePolicy {
    match mode {
        "developer" => ExplainDisclosurePolicy::Developer,
        "proof" | "coded" => ExplainDisclosurePolicy::Proof,
        "forensic" => ExplainDisclosurePolicy::Forensic,
        _ => ExplainDisclosurePolicy::PublicOnly,
    }
}

fn run_convert(format: ConvertFormat, args: Vec<String>) -> Result<(), String> {
    match format {
        ConvertFormat::Parquet => cove_convert_parquet::commands::run_parquet(args),
        ConvertFormat::Arrow => cove_convert_parquet::commands::run_arrow(args),
        ConvertFormat::Orc => cove_convert_parquet::commands::run_orc(args),
        ConvertFormat::Csv => cove_convert_parquet::commands::run_csv(args),
        ConvertFormat::Report => cove_convert_parquet::commands::run_report(args),
    }
}

fn run_validate(args: Vec<String>) -> Result<(), String> {
    if cove_validate::run_cli(args)? {
        Ok(())
    } else {
        Err("validation failed".into())
    }
}

fn run_inspect_detailed(args: Vec<String>) -> Result<(), String> {
    if cove_inspect::run_cli(args)? {
        Ok(())
    } else {
        Err("inspection failed".into())
    }
}

fn run_export(format: ExportFormat, args: Vec<String>) -> Result<(), String> {
    match format {
        ExportFormat::Arrow => cove_datafusion::arrow_export_cli::run(args),
    }
}

fn run_perf(command: PerfCommand, args: Vec<String>) -> Result<(), String> {
    match command {
        PerfCommand::ExplainPruning => cove_datafusion::explain_pruning_cli::run(args),
        PerfCommand::PlanCost => cove_datafusion::plan_cost_cli::run(args),
    }
}

fn run_profile(args: Vec<String>) -> Result<(), String> {
    if cove_core::profile_cli::run(args)? {
        Ok(())
    } else {
        Err("profile command failed".into())
    }
}

fn run_canonicalise(args: Vec<String>) -> Result<(), String> {
    if cove_core::canonicalise_cli::run(args)? {
        Ok(())
    } else {
        Err("canonicalise command failed".into())
    }
}

fn run_digest(args: Vec<String>) -> Result<(), String> {
    if run_digest_verify(args)? {
        Ok(())
    } else {
        Err("digest verification failed".into())
    }
}

fn run_digest_verify(args: Vec<String>) -> Result<bool, String> {
    let mut require = false;
    let mut input = None;
    for arg in args {
        match arg.as_str() {
            "--require" => require = true,
            "-h" | "--help" => {
                println!("usage: cove digest verify <file.cove> [--require]");
                return Ok(true);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown digest option {arg}")),
            _ => {
                if input.replace(PathBuf::from(arg)).is_some() {
                    return Err("expected one <file.cove>".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "expected <file.cove>".to_string())?;
    let bytes =
        fs::read(&input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    let structural = validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: false,
            verify_digests: false,
            ..ValidationOptions::default()
        },
    )
    .map_err(|error| format!("cannot validate {}: {error}", input.display()))?;
    let has_manifest = structural
        .validated
        .footer
        .sections
        .iter()
        .any(|entry| entry.section_kind == SectionKind::DigestManifest as u16);
    let (status, success, error) = if !has_manifest {
        ("missing_manifest", !require, None)
    } else {
        match validate_bytes_with_options(
            &bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: true,
                ..ValidationOptions::default()
            },
        ) {
            Ok(_) => ("verified", true, None),
            Err(error) => ("mismatch", false, Some(error.to_string())),
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "file": input.display().to_string(),
            "status": status,
            "require": require,
            "digest_manifest_present": has_manifest,
            "error": error,
        }))
        .map_err(|error| format!("cannot serialize report: {error}"))?
    );
    Ok(success)
}

fn run_sidecar(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        println!("usage: cove sidecar <inspect|build> ...");
        return Ok(());
    }
    let command = args.remove(0);
    match command.as_str() {
        "inspect" => run_sidecar_inspect(args),
        "build" => run_sidecar_build(args),
        other => Err(format!(
            "unknown sidecar command '{other}'; expected inspect or build"
        )),
    }
}

fn run_sidecar_inspect(mut args: Vec<String>) -> Result<(), String> {
    if args.len() != 2 || args[0] == "-h" || args[0] == "--help" {
        return Err(
            "usage: cove sidecar inspect <index|coverage|layout|cache|runtime> <file>".into(),
        );
    }
    let kind = args.remove(0);
    let path = PathBuf::from(args.remove(0));
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    match kind.as_str() {
        "index" | "covi" => inspect_index_sidecar(&path, &bytes),
        "coverage" => inspect_coverage_sidecar(&path, &bytes),
        "layout" => inspect_layout_sidecar(&path, &bytes),
        "cache" => inspect_cache_sidecar(&path, &bytes),
        "runtime" => inspect_runtime_sidecar(&path, &bytes),
        other => Err(format!(
            "unknown sidecar kind '{other}'; expected index, coverage, layout, cache, or runtime"
        )),
    }
}

fn run_sidecar_build(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err("usage: cove sidecar build <covi|covx|covm> ...".into());
    }
    let kind = args.remove(0);
    match kind.as_str() {
        "covi" => build_covi_sidecar(args),
        "covx" => build_covx_or_covm_sidecar(args, true),
        "covm" => build_covx_or_covm_sidecar(args, false),
        other => Err(format!(
            "unknown sidecar build kind '{other}'; expected covi, covx, or covm"
        )),
    }
}

fn inspect_index_sidecar(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() >= 4 && bytes[bytes.len() - 4..] == *b"CVI2" {
        let artifact = cove_index::CoviArtifactV2::parse(bytes)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        println!(
            "valid COVE-I artifact: sections={} roots={} files={} capabilities={} key_blocks={} entry_blocks={} postings_blocks={}",
            artifact.sections.len(),
            artifact.header.index_root_count,
            artifact.header.referenced_file_count,
            artifact.header.capability_count,
            artifact.key_blocks.len(),
            artifact.entry_blocks.len(),
            artifact.postings_blocks.len()
        );
        return Ok(());
    }
    if let Ok(capabilities) = cove_index::IndexCapabilityV2::parse_many(bytes) {
        println!(
            "valid COVE-I index capability section: {} capabilities",
            capabilities.len()
        );
        return Ok(());
    }
    if let Ok(capabilities) = cove_index::IndexOnlyCapabilityV2::parse_many(bytes) {
        println!(
            "valid COVE-I index-only capability section: {} capabilities",
            capabilities.len()
        );
        return Ok(());
    }
    let artifact = cove_index::CoviArtifactV2::parse(bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    println!(
        "valid COVE-I artifact: sections={} roots={} files={} capabilities={} key_blocks={} entry_blocks={} postings_blocks={}",
        artifact.sections.len(),
        artifact.header.index_root_count,
        artifact.header.referenced_file_count,
        artifact.header.capability_count,
        artifact.key_blocks.len(),
        artifact.entry_blocks.len(),
        artifact.postings_blocks.len()
    );
    Ok(())
}

fn inspect_coverage_sidecar(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(providers) = cove_coverage::CoverageProviderDescriptorV2::parse_many(bytes) {
        println!(
            "valid COVE-COVERAGE provider registry: {} providers",
            providers.len()
        );
        return Ok(());
    }
    if let Ok(set) = cove_coverage::CoverageSetV2::parse(bytes) {
        println!(
            "valid COVE-COVERAGE set: id={} provider={} entries={} pruning_safe={}",
            set.header.coverage_set_id,
            set.header.provider_id,
            set.entries.len(),
            cove_coverage::can_use_for_pruning(&set.header)
        );
        return Ok(());
    }
    if let Ok(records) = cove_coverage::CoverageProofRecordV2::parse_many(bytes) {
        println!(
            "valid COVE-COVERAGE proof records: {} pruning_safe={}",
            records.len(),
            records.iter().all(cove_coverage::can_use_proof_for_pruning)
        );
        return Ok(());
    }
    if let Ok(candidates) = cove_coverage::CoveragePlanCandidateV2::parse_many(bytes) {
        println!("valid COVE-COVERAGE plan candidates: {}", candidates.len());
        return Ok(());
    }
    if let Ok(forms) = cove_coverage::PredicateNormalFormV2::parse_many(bytes) {
        println!("valid COVE-COVERAGE predicate forms: {}", forms.len());
        return Ok(());
    }
    match cove_coverage::IntervalPredicateV2::parse_many(bytes) {
        Ok(intervals) => {
            println!(
                "valid COVE-COVERAGE interval predicates: {}",
                intervals.len()
            );
            Ok(())
        }
        Err(error) => Err(format!(
            "{}: not a valid provider registry, coverage set, proof record, predicate form, interval predicate, or plan candidate: {error}",
            path.display()
        )),
    }
}

fn inspect_layout_sidecar(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Ok(plan) = cove_layout::LayoutPlanV2::parse(bytes) {
        println!(
            "valid COVE-L layout plan: layout_id={} nodes={} root={}",
            plan.header.layout_id,
            plan.nodes.len(),
            plan.header.root_node_id
        );
        return Ok(());
    }
    if let Ok(index) = cove_layout::ScanSplitIndexV2::parse(bytes) {
        println!(
            "valid COVE-L scan split index: splits={}",
            index.entries.len()
        );
        return Ok(());
    }
    match cove_layout::ZeroCopyBufferMapV2::parse(bytes) {
        Ok(map) => {
            println!(
                "valid COVE-L zero-copy buffer map: targets={} entries={}",
                map.targets.len(),
                map.entries.len()
            );
            Ok(())
        }
        Err(error) => Err(format!(
            "{}: not a valid COVE-L layout plan, scan split index, or zero-copy map: {error}",
            path.display()
        )),
    }
}

fn inspect_cache_sidecar(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match cove_cache::CoverageCacheV2::parse(bytes) {
        Ok(cache) => {
            println!(
                "valid COVE-CACHE diagnostic record: entries={} version={}.{}",
                cache.entries.len(),
                cache.header.cache_format_version_major,
                cache.header.cache_format_version_minor
            );
            Ok(())
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn inspect_runtime_sidecar(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match cove_runtime::RuntimeCompatibilityHintV2::parse_many(bytes) {
        Ok(hints) => {
            println!("valid COVE-R runtime hints: {} hints", hints.len());
            for hint in hints {
                println!(
                    "hint_id={} kind={:?} required={} {}::{} v{}.{}",
                    hint.hint_id,
                    hint.hint_kind,
                    hint.required,
                    hint.namespace,
                    hint.name,
                    hint.version_major,
                    hint.version_minor
                );
            }
            Ok(())
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn build_covx_or_covm_sidecar(mut args: Vec<String>, covx: bool) -> Result<(), String> {
    if args.len() < 2 {
        return Err(if covx {
            "usage: cove sidecar build covx <output.covx> <input.cove>...".into()
        } else {
            "usage: cove sidecar build covm <output.covm> <input.cove>...".into()
        });
    }
    let output = PathBuf::from(args.remove(0));
    let inputs = args.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let (bytes, report) = if covx {
        build_covx_artifact(&output, &inputs).map_err(|error| error.to_string())?
    } else {
        build_covm_artifact(&output, &inputs).map_err(|error| error.to_string())?
    };
    durable::durable_replace(&output, &bytes)
        .map_err(|error| format!("cannot durably publish {}: {error}", output.display()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report.to_json_value())
            .map_err(|error| format!("cannot serialize report: {error}"))?
    );
    Ok(())
}

fn build_covi_sidecar(args: Vec<String>) -> Result<(), String> {
    use cove_index::build::{build_covi_from_cove_bytes, CoviBuildOptions};

    if args.len() == 1 {
        let output = &args[0];
        let artifact = cove_index::CoviArtifactV2::new_empty([0u8; 16], [0u8; 16]);
        let bytes = artifact
            .serialize_empty()
            .map_err(|error| format!("failed to build empty COVE-I artifact: {error}"))?;
        fs::write(output, bytes).map_err(|error| format!("{output}: {error}"))?;
        println!("wrote empty COVE-I artifact to {output}");
        return Ok(());
    }

    let mut positionals = Vec::new();
    let mut options = CoviBuildOptions::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--table-id" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--table-id requires a value".to_string())?;
                options.table_id = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --table-id value: {value}"))?,
                );
            }
            "--column-id" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--column-id requires a value".to_string())?;
                options.column_ids.push(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --column-id value: {value}"))?,
                );
            }
            "--all-columns" => options.all_columns = true,
            "--index-only-counts" => options.include_index_only_counts = true,
            "--index-only-exists" => options.include_index_only_exists = true,
            "--index-only-min-max" => options.include_index_only_min_max = true,
            "--index-only-distinct-count" => options.include_index_only_distinct_count = true,
            "--index-only-sum-avg" => options.include_index_only_sum_avg = true,
            "-h" | "--help" => {
                println!("usage: cove sidecar build covi <input.cove> <output.covi> [--table-id <id>] [--column-id <id> ... | --all-columns] [--index-only-counts] [--index-only-exists] [--index-only-min-max] [--index-only-distinct-count] [--index-only-sum-avg]");
                return Ok(());
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option: {arg}")),
            _ => positionals.push(arg),
        }
    }
    if positionals.len() != 2 {
        return Err("usage: cove sidecar build covi <input.cove> <output.covi> [options]".into());
    }
    if options.all_columns && !options.column_ids.is_empty() {
        return Err("--all-columns cannot be combined with --column-id".into());
    }
    let input_path = positionals.remove(0);
    let output_path = positionals.remove(0);
    let input = fs::read(&input_path).map_err(|error| format!("{input_path}: {error}"))?;
    let bytes = build_covi_from_cove_bytes(&input, &options)
        .map_err(|error| format!("{input_path}: {error}"))?;
    fs::write(&output_path, bytes).map_err(|error| format!("{output_path}: {error}"))?;
    println!("wrote COVE-I artifact to {output_path}");
    Ok(())
}

fn run_examples(json: bool) -> Result<(), String> {
    let sample_dir = "examples/coveql";
    let examples = vec![
        (
            "Inspect an object sample",
            "cove inspect --queries --performance examples/coveql/people.cove",
        ),
        (
            "Query mapped object rows as a table",
            "cove query examples/coveql/people.cove 'table(people).select(score, status, nickname).take(5)'",
        ),
        (
            "Query a COVE-T table",
            "cove query examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'",
        ),
        (
            "Check acceleration decisions",
            "cove query --engine compare --perf-report examples/coveql/events.cove 'table(events).where(score >= 20).select(id, score)'",
        ),
        (
            "Join an external CSV file",
            "cove query --external-table people=/tmp/people.csv 'table(people).where(score >= 20).select(id, score)'",
        ),
    ];
    if json {
        let value = serde_json::json!({
            "sample_dir": sample_dir,
            "examples": examples.iter().map(|(title, command)| {
                serde_json::json!({
                    "title": title,
                    "command": command,
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }

    println!("CoveQL examples");
    println!("Sample directory: {sample_dir}");
    println!();
    for (title, command) in examples {
        println!("{title}:");
        println!("  {command}");
    }
    println!();
    println!("Regenerate samples from v2/ with:");
    println!("  cargo run -p cove-cli --example generate_beginner_samples -- examples/coveql");
    Ok(())
}

fn run_doctor(file: &Path, json: bool) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    let discovery = discover_query_surfaces(
        &bytes,
        QuerySurfaceDiscoveryOptions {
            source_name: Some(file.display().to_string()),
        },
    );
    let bundle = discover_acceleration_bundle(
        &bytes,
        file,
        AccelerationBundleOptions {
            auto_discover: true,
            strict_source_digest: true,
        },
    );
    let suggestions = suggest_queries(&discovery);
    let mut findings = Vec::new();
    if discovery.queryable {
        findings.push("artifact exposes queryable rows".to_string());
    } else {
        findings.push(discovery.guidance.clone());
    }
    if bundle.has_usable_sidecars() {
        findings.push("validated acceleration sidecars are available".to_string());
    } else if discovery.queryable {
        findings.push(format!(
            "no validated acceleration bundle found; run `cove optimize {}`",
            file.display()
        ));
    }
    for diagnostic in &discovery.diagnostics {
        findings.push(format!("{}: {}", diagnostic.code, diagnostic.message));
    }
    for diagnostic in &bundle.diagnostics {
        findings.push(format!("{}: {}", diagnostic.code, diagnostic.message));
    }

    if json {
        let value = serde_json::json!({
            "file": file.display().to_string(),
            "artifact": discovery.artifact_label,
            "queryable": discovery.queryable,
            "guidance": discovery.guidance,
            "findings": findings,
            "suggested_queries": suggestions,
            "performance": acceleration_report_json(&bundle),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }

    println!("Doctor: {}", file.display());
    println!("Artifact: {}", discovery.artifact_label);
    println!(
        "Queryable: {}",
        if discovery.queryable { "yes" } else { "no" }
    );
    println!("Guidance: {}", discovery.guidance);
    println!();
    println!("Findings:");
    for finding in &findings {
        println!("  - {finding}");
    }
    if !suggestions.is_empty() {
        println!();
        println!("Try next:");
        for suggestion in suggestions.iter().take(3) {
            println!("  - {}", suggestion.query);
        }
    }
    println!();
    println!("Useful commands:");
    println!("  cove inspect --queries --performance {}", file.display());
    if discovery.queryable && !bundle.has_usable_sidecars() {
        println!("  cove optimize {}", file.display());
    }
    println!("  cove query --help");
    Ok(())
}

fn run_inspect(file: &Path, queries: bool, json: bool, performance: bool) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    let discovery = discover_query_surfaces(
        &bytes,
        QuerySurfaceDiscoveryOptions {
            source_name: Some(file.display().to_string()),
        },
    );
    if json {
        let mut value = serde_json::to_value(&discovery)
            .map_err(|error| format!("cannot serialize discovery: {error}"))?;
        if queries {
            value["suggested_queries"] = serde_json::to_value(suggest_queries(&discovery))
                .map_err(|error| format!("cannot serialize suggested queries: {error}"))?;
        }
        if performance {
            let bundle = discover_acceleration_bundle(
                &bytes,
                file,
                AccelerationBundleOptions {
                    auto_discover: true,
                    strict_source_digest: true,
                },
            );
            value["performance"] = acceleration_report_json(&bundle);
        }
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }
    print_discovery(&discovery, queries);
    if performance {
        let bundle = discover_acceleration_bundle(
            &bytes,
            file,
            AccelerationBundleOptions {
                auto_discover: true,
                strict_source_digest: true,
            },
        );
        print_performance_discovery(&bundle);
    }
    Ok(())
}

fn run_optimize(file: &Path, out_dir: Option<&Path>, full: bool, json: bool) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    let options = CoveOptimizationOptions {
        source_path: Some(file.to_path_buf()),
        out_dir: out_dir.map(Path::to_path_buf),
        full,
    };
    let plan = plan_acceleration(&bytes, options);
    let out_dir = plan.out_dir.clone();
    let report = generate_acceleration_sidecars(&bytes, plan, &out_dir)
        .map_err(|error| format!("cannot optimize {}: {error}", file.display()))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }
    println!("Optimized: {}", file.display());
    println!("Manifest: {}", report.manifest_path.display());
    if !report.generated.is_empty() {
        println!("\nGenerated sidecars:");
        for generated in &report.generated {
            println!(
                "  - {}: {} ({} bytes)",
                generated.kind,
                generated.path.display(),
                generated.bytes
            );
        }
    }
    if !report.skipped.is_empty() {
        println!("\nSkipped / not applicable:");
        for skipped in &report.skipped {
            println!("  - {}: {}", skipped.kind, skipped.reason);
        }
    }
    if !report.diagnostics.is_empty() {
        println!("\nDiagnostics:");
        for diagnostic in &report.diagnostics {
            println!("  - {}: {}", diagnostic.code, diagnostic.message);
        }
    }
    Ok(())
}

fn run_query(file: Option<&Path>, query: &str, options: QueryCommandOptions) -> Result<(), String> {
    let bytes = match file {
        Some(file) => {
            fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?
        }
        None => external_only_context_bytes(),
    };
    let mut execute_options = ExecuteArtifactOptions::default();
    register_external_tables(&mut execute_options, &options.external_tables)?;
    if let Some(mapping) = &options.mapping {
        execute_options.execution_options.mapping_path = Some(mapping.clone());
    }
    if let Some(batch_size) = options.batch_size {
        execute_options.execution_options.batch_size = Some(batch_size);
    }
    apply_graph_budget(&mut execute_options, options.graph_budget);
    if options.enable_graph_traversal {
        execute_options.resolve_options.graph_traversal_contract =
            Some(cli_graph_traversal_contract(&execute_options));
    }
    if let Some(explain) = options.explain.as_deref() {
        execute_options.resolve_options.security.explain_policy = explain_policy_for_cli(explain);
    }
    configure_execution_engine(&mut execute_options, &options)?;
    let acceleration_bundle = if !options.no_auto_sidecars {
        file.map(|file| {
            discover_acceleration_bundle(
                &bytes,
                file,
                AccelerationBundleOptions {
                    auto_discover: true,
                    strict_source_digest: true,
                },
            )
        })
    } else {
        None
    };
    if let Some(bundle) = &acceleration_bundle {
        if options.engine != QueryEngine::Materialized
            && (bundle.has_usable_sidecars() || options.physical_sidecars.has_any())
        {
            execute_options = apply_acceleration_bundle(bundle, execute_options);
        }
        if options.strict_performance
            && options.engine != QueryEngine::Materialized
            && !bundle.has_usable_sidecars()
            && !options.physical_sidecars.has_any()
        {
            return Err(format!(
                "strict performance requested, but no validated acceleration sidecars were found for {}",
                bundle.source_path.display()
            ));
        }
    }
    execute_options.manifest_members = manifest_members_for(file, &bytes, &options)?;
    let query = match &options.query_file {
        Some(query_file) => read_query_file(query_file)?,
        None => query.to_string(),
    };
    let query = prepare_query_text(&query, options.take, options.explain.as_deref())?;
    let executed = match execute_query_from_artifact(&bytes, &query, execute_options.clone()) {
        Ok(executed) => {
            if options.perf_report {
                print_query_perf_report(acceleration_bundle.as_ref(), None);
            }
            executed
        }
        Err(error) if options.engine == QueryEngine::Auto && !options.strict_performance => {
            let mut fallback_options = execute_options;
            fallback_options.execution_engine = ArtifactExecutionEngine::Materialized;
            match execute_query_from_artifact(&bytes, &query, fallback_options) {
                Ok(executed) => {
                    if options.perf_report {
                        print_query_perf_report(
                            acceleration_bundle.as_ref(),
                            Some(&format_artifact_query_error(
                                error,
                                options.json_diagnostics,
                            )),
                        );
                    }
                    executed
                }
                Err(fallback_error) => {
                    return Err(format_artifact_query_error(
                        fallback_error,
                        options.json_diagnostics,
                    ))
                }
            }
        }
        Err(error) => return Err(format_artifact_query_error(error, options.json_diagnostics)),
    };
    if options.explain.is_some() {
        println!("{}", executed.explain_text());
        return Ok(());
    }
    let value = executed
        .result_json()
        .map_err(|error| format_execution_error(error, options.json_diagnostics))?;
    write_result(&value, options.format, options.max_cell_width)
}

fn external_only_context_bytes() -> Vec<u8> {
    ScanProfileCoveWriter::new(TableCatalog {
        flags: 0,
        tables: Vec::new(),
    })
    .write()
    .expect("empty COVE-T context file is valid")
}

fn register_external_tables(
    execute_options: &mut ExecuteArtifactOptions,
    specs: &[ExternalTableSpec],
) -> Result<(), String> {
    for spec in specs {
        let rows = read_external_table_rows(&spec.path)?;
        let authority = external_table_authority(&spec.table_name, &spec.path, rows)?;
        execute_options
            .resolve_options
            .table_authorities
            .insert(spec.table_name.clone(), authority);
    }
    Ok(())
}

fn read_external_table_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => read_external_csv_rows(path),
        Some("jsonl") | Some("ndjson") => read_external_jsonl_rows(path),
        Some("json") => read_external_json_rows(path),
        _ => {
            let text = fs::read_to_string(path).map_err(|error| {
                format!("cannot read external table {}: {error}", path.display())
            })?;
            if text.trim_start().starts_with('[') || text.trim_start().starts_with('{') {
                rows_from_json_text(&text, path)
            } else {
                read_external_jsonl_text(&text, path)
            }
        }
    }
}

fn read_external_csv_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("cannot read CSV external table {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("cannot read CSV headers {}: {error}", path.display()))?
        .iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|error| format!("cannot read CSV row {}: {error}", path.display()))?;
        let mut row = BTreeMap::new();
        for (header, value) in headers.iter().zip(record.iter()) {
            row.insert(header.clone(), parse_external_csv_cell(value));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn parse_external_csv_cell(value: &str) -> Value {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Value::Number(value.into());
    }
    if let Ok(value) = trimmed.parse::<u64>() {
        return Value::Number(value.into());
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(value) {
            return Value::Number(number);
        }
    }
    Value::String(value.to_string())
}

fn read_external_json_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read JSON external table {}: {error}",
            path.display()
        )
    })?;
    rows_from_json_text(&text, path)
}

fn rows_from_json_text(text: &str, path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| {
        format!(
            "cannot parse JSON external table {}: {error}",
            path.display()
        )
    })?;
    let rows = match value {
        Value::Array(rows) => rows,
        Value::Object(mut object) => match object.remove("rows") {
            Some(Value::Array(rows)) => rows,
            _ => {
                return Err(format!(
                "JSON external table {} must be an array of objects or an object with a rows array",
                path.display()
            ))
            }
        },
        _ => {
            return Err(format!(
                "JSON external table {} must be an array of objects",
                path.display()
            ))
        }
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, value)| value_to_table_row(value, path, index + 1))
        .collect()
}

fn read_external_jsonl_rows(path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "cannot read JSONL external table {}: {error}",
            path.display()
        )
    })?;
    read_external_jsonl_text(&text, path)
}

fn read_external_jsonl_text(text: &str, path: &Path) -> Result<Vec<TableSurfaceRow>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value = serde_json::from_str::<Value>(line).map_err(|error| {
                format!(
                    "cannot parse JSONL external table {} line {}: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            value_to_table_row(value, path, index + 1)
        })
        .collect()
}

fn value_to_table_row(
    value: Value,
    path: &Path,
    row_number: usize,
) -> Result<TableSurfaceRow, String> {
    let Value::Object(object) = value else {
        return Err(format!(
            "external table {} row {} must be a JSON object",
            path.display(),
            row_number
        ));
    };
    Ok(object.into_iter().collect::<BTreeMap<_, _>>())
}

fn external_table_authority(
    table_name: &str,
    path: &Path,
    mut rows: Vec<TableSurfaceRow>,
) -> Result<TableSurfaceAuthority, String> {
    let columns = external_table_columns(&rows);
    if columns.is_empty() {
        return Err(format!(
            "external table {table_name} from {} has no columns",
            path.display()
        ));
    }
    let row_identity = if columns.iter().any(|column| column.name == "id") {
        vec!["id".into()]
    } else {
        vec![columns[0].name.clone()]
    };
    let canonical_order = row_identity.clone();
    let provider_id = format!("external-file:{}", path.display());
    let table_id = format!("external:{}", coveql_identifier(table_name));
    for row in &mut rows {
        for column in &columns {
            row.entry(column.name.clone()).or_insert(Value::Null);
        }
    }
    Ok(TableSurfaceAuthority {
        contract: TableSurfaceContract {
            table_id: table_id.clone(),
            table_name: table_name.to_string(),
            contract_version: COVEQL_PROFILE_CONTRACT_VERSION.into(),
            authority_kind: TableSurfaceAuthorityKind::ExternalRegisteredTable,
            authority_fingerprint: external_table_fingerprint(
                table_name,
                path,
                rows.len(),
                &columns,
            ),
            schema_fingerprint: external_table_schema_fingerprint(table_name, &columns),
            logical_column_map: columns,
            row_grain: "external_file_row".into(),
            row_identity,
            canonical_order,
            visibility_authority: "external_file_visible_rows".into(),
            redaction_authority: "external_file_no_redaction_metadata".into(),
            temporal_authority: TableTemporalAuthority::StaticTableSnapshot,
            evidence_capabilities: vec![AstEvidenceGrain::Row],
            null_missing_nan_policy: "json_null_and_missing_are_null".into(),
            collation_policy: "binary_utf8".into(),
            code_domain_contexts: Vec::new(),
            code_domain_bridges: Vec::new(),
            projection_dependency_contract_id: None,
            datafusion_interop_contract: Some("cli_external_file_rows".into()),
        },
        execution_authority: TableExecutionAuthority::ExternalRows { provider_id, rows },
    })
}

fn external_table_columns(rows: &[TableSurfaceRow]) -> Vec<TableSurfaceColumnContract> {
    let mut names = BTreeSet::new();
    for row in rows {
        names.extend(row.keys().cloned());
    }
    names
        .into_iter()
        .map(|name| {
            let values = rows
                .iter()
                .map(|row| row.get(&name).unwrap_or(&Value::Null))
                .collect::<Vec<_>>();
            TableSurfaceColumnContract {
                name,
                logical_type: Some(infer_external_logical_type(&values).into()),
                nullable: values.iter().any(|value| value.is_null()),
                source_path: None,
                code_domain: None,
                collation: None,
            }
        })
        .collect()
}

fn infer_external_logical_type(values: &[&Value]) -> &'static str {
    let non_null = values
        .iter()
        .copied()
        .filter(|value| !value.is_null())
        .collect::<Vec<_>>();
    if non_null.is_empty() {
        return "null";
    }
    if non_null.iter().all(|value| value.is_boolean()) {
        return "bool";
    }
    if non_null.iter().all(|value| {
        value.as_i64().is_some()
            || value
                .as_u64()
                .is_some_and(|value| i64::try_from(value).is_ok())
    }) {
        return "int64";
    }
    if non_null.iter().all(|value| value.is_number()) {
        return "float64";
    }
    if non_null.iter().all(|value| value.is_string()) {
        return "utf8";
    }
    "json"
}

fn external_table_fingerprint(
    table_name: &str,
    path: &Path,
    row_count: usize,
    columns: &[TableSurfaceColumnContract],
) -> String {
    format!(
        "external-file:{}:{}:{}:{}",
        table_name,
        path.display(),
        row_count,
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn external_table_schema_fingerprint(
    table_name: &str,
    columns: &[TableSurfaceColumnContract],
) -> String {
    format!(
        "external-schema:{}:{}",
        table_name,
        columns
            .iter()
            .map(|column| format!(
                "{}:{}:{}",
                column.name,
                column.logical_type.as_deref().unwrap_or("unknown"),
                column.nullable
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn apply_graph_budget(options: &mut ExecuteArtifactOptions, budget: GraphBudgetOverrides) {
    if let Some(max_depth) = budget.max_depth {
        options
            .parse_options
            .resource_budget
            .maximum_graph_traversal_depth = max_depth;
        options
            .resolve_options
            .resource_budget
            .maximum_graph_traversal_depth = max_depth;
        options
            .execution_options
            .resource_budget
            .maximum_graph_traversal_depth = max_depth;
    }
    if let Some(max_paths) = budget.max_paths {
        options
            .parse_options
            .resource_budget
            .maximum_graph_traversal_paths = max_paths;
        options
            .resolve_options
            .resource_budget
            .maximum_graph_traversal_paths = max_paths;
        options
            .execution_options
            .resource_budget
            .maximum_graph_traversal_paths = max_paths;
        options
            .parse_options
            .resource_budget
            .maximum_graph_traversal_frontier = max_paths;
        options
            .resolve_options
            .resource_budget
            .maximum_graph_traversal_frontier = max_paths;
        options
            .execution_options
            .resource_budget
            .maximum_graph_traversal_frontier = max_paths;
    }
    if let Some(max_fanout) = budget.max_fanout {
        options
            .parse_options
            .resource_budget
            .maximum_graph_traversal_fanout = max_fanout;
        options
            .resolve_options
            .resource_budget
            .maximum_graph_traversal_fanout = max_fanout;
        options
            .execution_options
            .resource_budget
            .maximum_graph_traversal_fanout = max_fanout;
    }
}

fn cli_graph_traversal_contract(options: &ExecuteArtifactOptions) -> GraphTraversalContract {
    let budget = &options.resolve_options.resource_budget;
    GraphTraversalContract {
        contract_version: COVEQL_PROFILE_CONTRACT_VERSION.into(),
        allow_variable_length: true,
        supported_modes: vec![
            GraphTraversalMode::Walk,
            GraphTraversalMode::Trail,
            GraphTraversalMode::SimplePath,
        ],
        supported_distinct_policies: vec![
            GraphTraversalDistinctPolicy::None,
            GraphTraversalDistinctPolicy::Path,
            GraphTraversalDistinctPolicy::EndNode,
        ],
        max_depth: budget.maximum_graph_traversal_depth,
        max_fanout_per_node: budget.maximum_graph_traversal_fanout,
        max_paths: budget.maximum_graph_traversal_paths,
        max_frontier: budget.maximum_graph_traversal_frontier,
        path_identity: vec![
            "start_goid".into(),
            "edge_goids".into(),
            "node_goids".into(),
        ],
        hidden_endpoint_policy: "suppress_path".into(),
        ordering_policy: "depth_start_edge_target".into(),
        execution_authority: "cli_bounded_materialized_visible_graph_oracle".into(),
    }
}

fn configure_execution_engine(
    execute_options: &mut ExecuteArtifactOptions,
    options: &QueryCommandOptions,
) -> Result<(), String> {
    let physical_requested = matches!(
        options.engine,
        QueryEngine::Physical | QueryEngine::Compare | QueryEngine::Kernel
    ) || options.physical_sidecars.has_any()
        || options.allow_index_only
        || options.allow_zero_copy;
    if !physical_requested {
        return Ok(());
    }
    let physical_options = PhysicalPlanOptions {
        allow_index_only_answers: options.allow_index_only,
        allow_zero_copy_output: options.allow_zero_copy,
        sidecars: physical_sidecars_from_paths(&options.physical_sidecars)?,
        ..Default::default()
    };

    let kernel_options = KernelExecutionOptions {
        batch_size: options.batch_size,
        mode: match options.engine {
            QueryEngine::Auto => KernelExecutionMode::Auto,
            QueryEngine::Materialized => KernelExecutionMode::Auto,
            QueryEngine::Physical => KernelExecutionMode::Auto,
            QueryEngine::Compare => KernelExecutionMode::CompareWithMaterialized,
            QueryEngine::Kernel => KernelExecutionMode::ForceKernel,
        },
        ..Default::default()
    };
    execute_options.execution_engine = ArtifactExecutionEngine::Physical {
        physical_options,
        kernel_options,
    };
    Ok(())
}

fn physical_sidecars_from_paths(
    paths: &QueryPhysicalSidecarPaths,
) -> Result<PhysicalSidecarInputs, String> {
    Ok(PhysicalSidecarInputs {
        coverage_plan_candidate_bytes: read_optional_bytes(&paths.coverage_plan_candidate)?,
        coverage_proof_record_bytes: read_optional_bytes(&paths.coverage_proof_record)?,
        coverage_set_bytes: read_optional_bytes(&paths.coverage_set)?,
        covi_artifact_bytes: read_optional_bytes(&paths.covi_artifact)?,
        covx_artifact_bytes: read_optional_bytes(&paths.covx_artifact)?,
        layout_plan_bytes: read_optional_bytes(&paths.layout_plan)?,
        scan_split_index_bytes: read_optional_bytes(&paths.scan_split_index)?,
        page_cluster_directory_bytes: read_optional_bytes(&paths.page_cluster_directory)?,
        zero_copy_buffer_map_bytes: read_optional_bytes(&paths.zero_copy_buffer_map)?,
        coverage_cache_bytes: read_optional_bytes(&paths.coverage_cache)?,
        cove_e_artifact_bytes: read_optional_bytes(&paths.cove_e_artifact)?,
    })
}

fn read_optional_bytes(path: &Option<PathBuf>) -> Result<Option<Vec<u8>>, String> {
    path.as_ref()
        .map(|path| {
            fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
        })
        .transpose()
}

fn read_query_file(path: &Path) -> Result<String, String> {
    if path == Path::new("-") {
        let mut query = String::new();
        io::stdin()
            .read_to_string(&mut query)
            .map_err(|error| format!("cannot read query from stdin: {error}"))?;
        return Ok(query);
    }
    fs::read_to_string(path)
        .map_err(|error| format!("cannot read query file {}: {error}", path.display()))
}

fn manifest_members_for(
    file: Option<&Path>,
    bytes: &[u8],
    options: &QueryCommandOptions,
) -> Result<Vec<QueryArtifactMember>, String> {
    let mut members = Vec::new();
    for (source, path) in &options.members {
        members.push(QueryArtifactMember {
            source: source.clone(),
            bytes: fs::read(path)
                .map_err(|error| format!("cannot read member {}: {error}", path.display()))?,
        });
    }
    if let Some(dataset_dir) = &options.dataset {
        let Some(file) = file else {
            return Err("--dataset requires a COVM manifest file argument".into());
        };
        let manifest = CovmFile::parse(bytes)
            .map_err(|error| format!("{} is not a valid COVM manifest: {error}", file.display()))?;
        for entry in manifest.files {
            if members.iter().any(|member| member.source == entry.uri) {
                continue;
            }
            let path = dataset_dir.join(&entry.uri);
            members.push(QueryArtifactMember {
                source: entry.uri,
                bytes: fs::read(&path)
                    .map_err(|error| format!("cannot read member {}: {error}", path.display()))?,
            });
        }
    }
    Ok(members)
}

fn prepare_query_text(
    query: &str,
    take: Option<usize>,
    explain: Option<&str>,
) -> Result<String, String> {
    let mut text = query.trim().to_string();
    if let Some(take) = take {
        if take == 0 {
            return Err("--take requires a positive integer".into());
        }
        if !text.contains(".take(") {
            text.push_str(&format!(".take({take})"));
        }
    }
    if let Some(mode) = explain {
        if !is_explain_mode(mode) {
            return Err(format!("unsupported explain mode '{mode}'"));
        }
        if !text.contains(".explain(") {
            text.push_str(&format!(".explain(\"{mode}\")"));
        }
    }
    Ok(text)
}

fn print_discovery(discovery: &QuerySurfaceDiscovery, queries: bool) {
    println!(
        "File: {}",
        discovery.source_name.as_deref().unwrap_or("<bytes>")
    );
    println!("Artifact: {}", discovery.artifact_label);
    if let Some(profile) = &discovery.primary_profile {
        println!("Profile: {profile}");
    }
    println!(
        "Queryable: {}",
        if discovery.queryable { "yes" } else { "no" }
    );
    println!("Guidance: {}", discovery.guidance);
    if !discovery.object_types.is_empty() {
        println!("\nObjects:");
        for object in &discovery.object_types {
            println!(
                "  - {} rows={} properties={} kind={}",
                object.type_name,
                object.row_count,
                object.properties.len(),
                object.kind
            );
            print_columns(&object.properties);
        }
    }
    if !discovery.tables.is_empty() {
        println!("\nTables:");
        for table in &discovery.tables {
            println!(
                "  - {} rows={} columns={} authority={}",
                table.table_name,
                table.row_count,
                table.columns.len(),
                table.authority_kind
            );
            print_columns(&table.columns);
        }
    }
    if !discovery.projections.is_empty() {
        println!("\nProjections:");
        for projection in &discovery.projections {
            println!(
                "  - {} table={} columns={}",
                projection.projection_id,
                projection.output_table.as_deref().unwrap_or("-"),
                projection.columns.len()
            );
        }
    }
    if !discovery.evidence.is_empty() {
        println!("\nEvidence:");
        for evidence in &discovery.evidence {
            println!("  - {} rows={}", evidence.grain, evidence.row_count);
        }
    }
    if !discovery.sidecars.is_empty() {
        println!("\nSidecars:");
        for sidecar in &discovery.sidecars {
            println!("  - {}: {}", sidecar.kind, sidecar.guidance);
        }
    }
    if !discovery.diagnostics.is_empty() {
        println!("\nDiagnostics:");
        for diagnostic in &discovery.diagnostics {
            println!("  - {}: {}", diagnostic.code, diagnostic.message);
        }
    }
    if queries {
        let suggestions = suggest_queries(discovery);
        if !suggestions.is_empty() {
            println!("\nSuggested queries:");
            for suggestion in &suggestions {
                println!("  - {}: {}", suggestion.title, suggestion.query);
            }
            if let Some(first) = suggestions.first() {
                println!("\nTry next:");
                println!(
                    "  cove query {} '{}'",
                    discovery.source_name.as_deref().unwrap_or("<file>"),
                    first.query
                );
            }
        }
    }
}

fn print_performance_discovery(bundle: &CoveAccelerationBundle) {
    println!("\nPerformance:");
    println!("  source digest: {}", bundle.source_digest);
    if let Some(manifest) = &bundle.manifest_path {
        println!("  manifest: {}", manifest.display());
    } else {
        println!("  manifest: not found");
    }
    if !bundle.sidecars.is_empty() {
        println!("  sidecars:");
        for sidecar in bundle.sidecars.values() {
            println!(
                "    - {}: {:?} ({})",
                sidecar.kind,
                sidecar.status,
                sidecar.path.display()
            );
        }
    }
    if !bundle.diagnostics.is_empty() {
        println!("  diagnostics:");
        for diagnostic in &bundle.diagnostics {
            println!("    - {}: {}", diagnostic.code, diagnostic.message);
        }
    }
    if !bundle.has_usable_sidecars() {
        println!(
            "  suggestion: run `cove optimize {}`",
            bundle.source_path.display()
        );
    }
}

fn print_query_perf_report(bundle: Option<&CoveAccelerationBundle>, fallback_reason: Option<&str>) {
    eprintln!("Performance report:");
    if let Some(bundle) = bundle {
        let usable = bundle
            .sidecars
            .values()
            .filter(|sidecar| {
                matches!(
                    sidecar.status,
                    coveql::CoveAccelerationSidecarStatus::Present
                )
            })
            .count();
        eprintln!("  source digest: {}", bundle.source_digest);
        eprintln!("  usable sidecars: {usable}");
        for sidecar in bundle.sidecars.values() {
            if matches!(
                sidecar.status,
                coveql::CoveAccelerationSidecarStatus::Present
            ) {
                eprintln!(
                    "  - used candidate {}: {}",
                    sidecar.kind,
                    sidecar.path.display()
                );
            }
        }
    } else {
        eprintln!("  usable sidecars: 0");
    }
    if let Some(reason) = fallback_reason {
        eprintln!("  materialized fallback: {reason}");
    } else {
        eprintln!("  materialized fallback: not required by CLI execution wrapper");
    }
    eprintln!("  detail: use `--explain coded` for proof-level acceleration decisions");
}

fn print_columns(columns: &[coveql::QueryColumnSurface]) {
    if columns.is_empty() {
        return;
    }
    let preview = columns
        .iter()
        .take(6)
        .map(|column| {
            format!(
                "{}:{}{}",
                column.name,
                column.logical_type.as_deref().unwrap_or("unknown"),
                if column.nullable { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>();
    println!(
        "      columns: {}{}",
        preview.join(", "),
        if columns.len() > preview.len() {
            ", ..."
        } else {
            ""
        }
    );
}

fn write_result(value: &Value, format: OutputFormat, max_cell_width: usize) -> Result<(), String> {
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
                .map(|row| display_cell(&row[index], max_cell_width).len())
                .max()
                .unwrap_or_default();
            column.len().max(value_width).min(max_cell_width)
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
        .any(|cell| display_cell(cell, max_cell_width).len() < cell.len());
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
    if clean.chars().count() <= max_cell_width {
        return clean;
    }
    let mut out = clean
        .chars()
        .take(max_cell_width.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn format_artifact_query_error(error: ExecuteArtifactQueryError, json_diagnostics: bool) -> String {
    match error {
        ExecuteArtifactQueryError::Execution(error) => {
            format_execution_error(error, json_diagnostics)
        }
        ExecuteArtifactQueryError::NotQueryable(discovery) if json_diagnostics => {
            serde_json::to_string_pretty(&discovery).unwrap_or(discovery.guidance)
        }
        other => other.to_string(),
    }
}

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

fn print_usage() {
    println!("{}", usage());
}

fn usage() -> String {
    "Usage:\n  cove examples [--json]\n  cove doctor [--json] <file>\n  cove inspect [--queries] [--performance] [--json] <file>\n  cove inspect [--json] [--sections stats,dictionary,execution,indexes,optional] <file...>\n  cove optimize <file> [--out-dir dir] [--full] [--json]\n  cove query [--format table|json|jsonl|csv] [--take n] [--max-cell-width n] [--explain [public|developer|proof|coded|forensic]] [--engine auto|materialized|physical|compare|kernel] [--no-auto-sidecars] [--strict-performance] [--perf-report] [--batch-size n] [--external-table name=path.csv|json|jsonl] [--enable-graph-traversal] [--max-graph-depth n] [--max-graph-paths n] [--max-graph-fanout n] [--mapping file.covemap] [--member id=path] [--dataset dir] [--covi file] [--covx file] [--cove-e file] [file] '<coveql>'\n  cove query [options] --query-file <path|-> [file]\n  cove convert <parquet|arrow|orc|csv|report> ...\n  cove validate ...\n  cove dump ...\n  cove map <validate|preview|plan-keys|convert|explain|diff|project|test> ...\n  cove export arrow ...\n  cove perf <explain-pruning|plan-cost> ...\n  cove sidecar inspect <index|coverage|layout|cache|runtime> <file>\n  cove sidecar build <covi|covx|covm> ...\n  cove digest verify <file.cove> [--require]\n  cove profile <inspect|generate|validate-section> ...\n  cove canonicalise <validate-payload|encode-json|check-domain|check-trust> ...\n\nExamples:\n  cove examples\n  cove doctor people.cove\n  cove inspect --queries --performance people.cove\n  cove convert parquet source.parquet output.cove\n  cove validate --semantic output.cove\n  cove optimize output.cove\n  cove query output.cove 'table(source).take(10)'\n  cove query --format jsonl people.cove 'table(people).where(active == true)'\n  cove map preview mapping.covemap\n  cove sidecar build covi output.cove output.covi --all-columns\n  cove query --query-file query.coveql people.cove".into()
}
