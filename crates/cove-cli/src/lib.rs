pub mod customer360;
mod delta;
mod external_tables;
mod help;
mod output;
mod sidecar;

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use cove_core::{
    artifact::{
        coveai::{
            write_covev_filecode_vectors, CoveAiArtifactKind, CoveAiFile,
            CoveVecFileCodeVectorBuild,
        },
        covemap::CovemapFile,
        covm::{CovmDeltaPruneRequest, CovmFile},
    },
    compression,
    constants::{SectionKind, MAGIC_COVEAI, MAGIC_COVEMAP, MAGIC_COVEV},
    profile::{
        cove_map::{parse_embedded_section, EmbeddedMapSection},
        cove_o::CoveObjectSurface,
    },
    reader::{validate_bytes_with_options, ValidatedCoveFile, ValidationOptions},
    table::TableCatalog,
    writer::ScanProfileCoveWriter,
};
use coveql::{
    acceleration_report_json, apply_acceleration_bundle, discover_acceleration_bundle,
    discover_query_surfaces, execute_query_from_artifact, generate_acceleration_sidecars,
    parse_resolve_plan_and_execute_query_on_object_surface, plan_acceleration, suggest_queries,
    AccelerationBundleOptions, ArtifactExecutionEngine, CoveAccelerationBundle,
    CoveOptimizationOptions, CoveQlExecutionResult, CoveQlOutputMode, ExecuteArtifactOptions,
    ExecuteArtifactQueryError, ExecutedQuery, ExplainDisclosurePolicy, GraphTraversalContract,
    GraphTraversalDistinctPolicy, GraphTraversalMode, KernelExecutionMode, KernelExecutionOptions,
    PhysicalPlanOptions, PhysicalSidecarInputs, QueryArtifactMember, QuerySurfaceDiscovery,
    QuerySurfaceDiscoveryOptions, COVEQL_PROFILE_CONTRACT_VERSION,
};
use customer360::{
    generate_customer360, generate_proof_suite, Customer360Options, Customer360Profile,
    ProofSuiteOptions, ProofSuiteScenario,
};
use delta::run_delta;
use external_tables::{register_external_tables, ExternalTableSpec};
use help::{print_usage, usage, HelpTopic};
use output::{write_result, OutputFormat};
use sidecar::run_sidecar;

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
        ai: bool,
    },
    Optimize {
        file: PathBuf,
        out_dir: Option<PathBuf>,
        full: bool,
        json: bool,
    },
    ShowcaseCustomer360 {
        out_dir: PathBuf,
        profile: Customer360Profile,
        force: bool,
        json: bool,
    },
    ShowcaseProofSuite {
        out_dir: PathBuf,
        profile: Customer360Profile,
        scenario: ProofSuiteScenario,
        force: bool,
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
    VecCommand {
        args: Vec<String>,
    },
    Train {
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
    Delta {
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
    Help(HelpTopic),
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
enum ArrowQueryExportOutputFormat {
    Ipc,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArrowQueryExportReportTarget {
    Stdout,
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArrowQueryExportCommand {
    input: PathBuf,
    output: PathBuf,
    query: Option<String>,
    query_file: Option<PathBuf>,
    format: ArrowQueryExportOutputFormat,
    report: Option<ArrowQueryExportReportTarget>,
    dataset: Option<PathBuf>,
    delta_request: CovmDeltaPruneRequest,
    delta_plan: bool,
    delta_plan_json: bool,
    perf_report: bool,
    take: Option<usize>,
    graph_budget: GraphBudgetOverrides,
    enable_graph_traversal: bool,
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
    delta_request: CovmDeltaPruneRequest,
    delta_plan: bool,
    delta_plan_json: bool,
    max_cell_width: usize,
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    match parse_args(args)? {
        Command::Help(topic) => {
            print_usage(topic);
            Ok(())
        }
        Command::Examples { json } => run_examples(json),
        Command::Doctor { file, json } => run_doctor(&file, json),
        Command::Inspect {
            file,
            queries,
            json,
            performance,
            ai,
        } => run_inspect(&file, queries, json, performance, ai),
        Command::Optimize {
            file,
            out_dir,
            full,
            json,
        } => run_optimize(&file, out_dir.as_deref(), full, json),
        Command::ShowcaseCustomer360 {
            out_dir,
            profile,
            force,
            json,
        } => run_showcase_customer360(&out_dir, profile, force, json),
        Command::ShowcaseProofSuite {
            out_dir,
            profile,
            scenario,
            force,
            json,
        } => run_showcase_proof_suite(&out_dir, profile, scenario, force, json),
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
                delta_request: command.delta_request,
                delta_plan: command.delta_plan,
                delta_plan_json: command.delta_plan_json,
                max_cell_width: command.max_cell_width,
            },
        ),
        Command::Convert { format, args } => run_convert(format, args),
        Command::Validate { args } => run_validate(args),
        Command::VecCommand { args } => run_vec(args),
        Command::Train { args } => run_train(args),
        Command::InspectDetailed { args } => run_inspect_detailed(args),
        Command::Dump { args } => cove_dump::run_cli(args),
        Command::Map { args } => run_map(args),
        Command::Export { format, args } => run_export(format, args),
        Command::Perf { command, args } => run_perf(command, args),
        Command::Sidecar { args } => run_sidecar(args),
        Command::Delta { args } => run_delta(args),
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
    delta_request: CovmDeltaPruneRequest,
    delta_plan: bool,
    delta_plan_json: bool,
    max_cell_width: usize,
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
    cove_ai_artifact: Option<PathBuf>,
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
            || self.cove_ai_artifact.is_some()
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter().collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(Command::Help(HelpTopic::Global));
    }
    let command = args.remove(0);
    match command.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help(HelpTopic::Global)),
        "examples" => parse_examples(args),
        "doctor" => parse_doctor(args),
        "inspect" => parse_inspect(args),
        "optimize" => parse_optimize(args),
        "showcase" => parse_showcase(args),
        "query" => parse_query(args),
        "convert" => parse_convert(args),
        "validate" => Ok(Command::Validate { args }),
        "vec"
            if args
                .first()
                .is_some_and(|arg| arg == "-h" || arg == "--help") =>
        {
            Ok(Command::Help(HelpTopic::Vec))
        }
        "vec" => Ok(Command::VecCommand { args }),
        "train"
            if args
                .first()
                .is_some_and(|arg| arg == "-h" || arg == "--help") =>
        {
            Ok(Command::Help(HelpTopic::Train))
        }
        "train" => Ok(Command::Train { args }),
        "dump" => Ok(Command::Dump { args }),
        "map"
            if args
                .first()
                .is_some_and(|arg| arg == "-h" || arg == "--help") =>
        {
            Ok(Command::Help(HelpTopic::Map))
        }
        "map" => Ok(Command::Map { args }),
        "export" => parse_export(args),
        "perf" => parse_perf(args),
        "sidecar"
            if args
                .first()
                .is_some_and(|arg| arg == "-h" || arg == "--help") =>
        {
            Ok(Command::Help(HelpTopic::Sidecar))
        }
        "sidecar" => Ok(Command::Sidecar { args }),
        "delta"
            if args
                .first()
                .is_some_and(|arg| arg == "-h" || arg == "--help") =>
        {
            Ok(Command::Help(HelpTopic::Delta))
        }
        "delta" => Ok(Command::Delta { args }),
        "digest" => parse_digest(args),
        "profile" => Ok(Command::Profile { args }),
        "canonicalise" | "canonicalize" => Ok(Command::Canonicalise { args }),
        other => Err(format!(
            "unknown command '{other}'\n\n{}",
            usage(HelpTopic::Global)
        )),
    }
}

fn parse_examples(args: Vec<String>) -> Result<Command, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Global)),
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
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Global)),
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
    reject_mixed_inspect_modes(&args)?;
    if wants_detailed_inspect(&args) {
        return Ok(Command::InspectDetailed { args });
    }
    let mut queries = false;
    let mut json = false;
    let mut performance = false;
    let mut ai = false;
    let mut file = None;
    for arg in args {
        match arg.as_str() {
            "--queries" => queries = true,
            "--json" => json = true,
            "--performance" => performance = true,
            "--ai" => ai = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Inspect)),
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
            "usage: cove inspect [--queries] [--performance] [--ai] [--json] <file>".to_string()
        })?,
        queries,
        json,
        performance,
        ai,
    })
}

fn reject_mixed_inspect_modes(args: &[String]) -> Result<(), String> {
    let detailed = args.iter().any(|arg| arg == "--sections");
    let beginner = args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--queries" | "--performance" | "--ai"));
    if detailed && beginner {
        return Err(
            "`cove inspect --sections` cannot be combined with `--queries` or `--performance`; use beginner inspect without `--sections`, or detailed inspect without beginner-only options"
                .into(),
        );
    }
    Ok(())
}

fn wants_detailed_inspect(args: &[String]) -> bool {
    let mut positional = 0usize;
    for arg in args {
        match arg.as_str() {
            "--sections" => return true,
            "--queries" | "--json" | "--performance" | "--ai" | "-h" | "--help" => {}
            _ if arg.starts_with("--") => {}
            _ => positional += 1,
        }
    }
    positional > 1
}

fn parse_convert(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Ok(Command::Help(HelpTopic::Convert));
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
        return Ok(Command::Help(HelpTopic::Global));
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
        return Ok(Command::Help(HelpTopic::Global));
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
        return Ok(Command::Help(HelpTopic::Global));
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
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Optimize)),
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

fn parse_showcase(mut args: Vec<String>) -> Result<Command, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Ok(Command::Help(HelpTopic::Showcase));
    }
    let name = args.remove(0);
    if name == "customer360" {
        return parse_showcase_customer360(args);
    }
    if name == "proof-suite" {
        return parse_showcase_proof_suite(args);
    }
    Err(format!(
        "unknown showcase '{name}'; expected customer360 or proof-suite\n\n{}",
        usage(HelpTopic::Showcase)
    ))
}

fn parse_showcase_customer360(args: Vec<String>) -> Result<Command, String> {
    let mut out_dir = None;
    let mut profile = Customer360Profile::Quick;
    let mut force = false;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                out_dir =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a directory path".to_string()
                    })?));
            }
            "--profile" => {
                let value = iter.next().ok_or_else(|| {
                    "--profile requires quick, standard, or publication".to_string()
                })?;
                profile = Customer360Profile::parse(&value)?;
            }
            "--force" => force = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Showcase)),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown customer360 showcase option '{arg}'"))
            }
            _ => return Err("showcase customer360 does not accept positional arguments".into()),
        }
    }
    let out_dir = out_dir.ok_or_else(|| {
        format!(
            "--out is required for showcase customer360\n\n{}",
            usage(HelpTopic::Showcase)
        )
    })?;
    Ok(Command::ShowcaseCustomer360 {
        out_dir,
        profile,
        force,
        json,
    })
}

fn parse_showcase_proof_suite(args: Vec<String>) -> Result<Command, String> {
    let mut out_dir = None;
    let mut profile = Customer360Profile::Quick;
    let mut scenario = ProofSuiteScenario::All;
    let mut force = false;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                out_dir =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a directory path".to_string()
                    })?));
            }
            "--profile" => {
                let value = iter.next().ok_or_else(|| {
                    "--profile requires quick, standard, or publication".to_string()
                })?;
                profile = Customer360Profile::parse(&value)?;
            }
            "--scenario" => {
                let value = iter.next().ok_or_else(|| {
                    "--scenario requires customer360, claims, catalog, or all".to_string()
                })?;
                scenario = ProofSuiteScenario::parse(&value)?;
            }
            "--force" => force = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Showcase)),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown proof-suite showcase option '{arg}'"))
            }
            _ => return Err("showcase proof-suite does not accept positional arguments".into()),
        }
    }
    let out_dir = out_dir.ok_or_else(|| {
        format!(
            "--out is required for showcase proof-suite\n\n{}",
            usage(HelpTopic::Showcase)
        )
    })?;
    Ok(Command::ShowcaseProofSuite {
        out_dir,
        profile,
        scenario,
        force,
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
    let mut delta_request = CovmDeltaPruneRequest::default();
    let mut delta_plan = false;
    let mut delta_plan_json = false;
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
            "--cove-ai" | "--covev" => {
                physical_sidecars.cove_ai_artifact =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--cove-ai requires a file path".to_string()
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
            "--as-of-csn" => {
                delta_request.as_of_csn = Some(parse_u64(iter.next().as_deref(), "--as-of-csn")?);
            }
            "--as-of-commit-us" => {
                delta_request.as_of_commit_timestamp_us =
                    Some(parse_i64(iter.next().as_deref(), "--as-of-commit-us")?);
            }
            "--source-publish-range" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--source-publish-range requires start:end".to_string())?;
                delta_request.source_publish_range_us =
                    Some(parse_i64_range(&raw, "--source-publish-range")?);
            }
            "--delta-plan" => delta_plan = true,
            "--delta-plan-json" => {
                delta_plan = true;
                delta_plan_json = true;
            }
            "--query-file" => {
                query_file = Some(PathBuf::from(iter.next().ok_or_else(|| {
                    "--query-file requires a path or '-' for stdin".to_string()
                })?));
            }
            "--json-diagnostics" => json_diagnostics = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Query)),
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
        delta_request,
        delta_plan,
        delta_plan_json,
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

fn parse_u32_arg(value: Option<&str>, flag: &str) -> Result<u32, String> {
    value
        .ok_or_else(|| format!("{flag} requires an unsigned 32-bit integer"))?
        .parse::<u32>()
        .map_err(|_| format!("{flag} requires an unsigned 32-bit integer"))
}

fn parse_u64(value: Option<&str>, flag: &str) -> Result<u64, String> {
    value
        .ok_or_else(|| format!("{flag} requires an unsigned integer"))?
        .parse::<u64>()
        .map_err(|_| format!("{flag} requires an unsigned integer"))
}

fn parse_i64(value: Option<&str>, flag: &str) -> Result<i64, String> {
    value
        .ok_or_else(|| format!("{flag} requires an integer"))?
        .parse::<i64>()
        .map_err(|_| format!("{flag} requires an integer"))
}

fn parse_i64_range(raw: &str, flag: &str) -> Result<(i64, i64), String> {
    let (start, end) = raw
        .split_once(':')
        .ok_or_else(|| format!("{flag} requires start:end"))?;
    Ok((parse_i64(Some(start), flag)?, parse_i64(Some(end), flag)?))
}

fn current_time_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn parse_hex16(raw: &str) -> Result<[u8; 16], String> {
    if raw.len() != 32 {
        return Err("--artifact-id requires exactly 32 hex characters".into());
    }
    let mut out = [0u8; 16];
    for (index, chunk) in raw.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(chunk[0])
            .ok_or_else(|| "--artifact-id contains non-hex characters".to_string())?;
        let low = hex_nibble(chunk[1])
            .ok_or_else(|| "--artifact-id contains non-hex characters".to_string())?;
        out[index] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
        "public" | "developer" | "proof" | "coded" | "ai" | "forensic"
    )
}

fn explain_policy_for_cli(mode: &str) -> ExplainDisclosurePolicy {
    match mode {
        "developer" => ExplainDisclosurePolicy::Developer,
        "proof" | "coded" | "ai" => ExplainDisclosurePolicy::Proof,
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

fn run_vec(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "missing vec subcommand\n\n{}",
            usage(HelpTopic::Vec)
        ));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "build" => run_vec_build(args),
        "-h" | "--help" => {
            print_usage(HelpTopic::Vec);
            Ok(())
        }
        other => Err(format!(
            "unknown vec subcommand '{other}'\n\n{}",
            usage(HelpTopic::Vec)
        )),
    }
}

fn run_vec_build(args: Vec<String>) -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut dimension_count: Option<u32> = None;
    let mut file_codes = Vec::new();
    let mut deterministic = false;
    let mut payload_path: Option<PathBuf> = None;
    let mut artifact_id = [0u8; 16];
    let mut created_at_us: Option<i64> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            "--dimension" => {
                dimension_count = Some(parse_positive_u32(iter.next().as_deref(), "--dimension")?);
            }
            "--file-code" => {
                let value = parse_u32_arg(iter.next().as_deref(), "--file-code")?;
                file_codes.push(value);
            }
            "--deterministic" => deterministic = true,
            "--payload" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--payload requires a path".to_string())?;
                payload_path = Some(PathBuf::from(value));
            }
            "--artifact-id" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--artifact-id requires 32 hex characters".to_string())?;
                artifact_id = parse_hex16(&value)?;
            }
            "--created-at-us" => {
                created_at_us = Some(parse_i64(iter.next().as_deref(), "--created-at-us")?);
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Vec);
                return Ok(());
            }
            other => return Err(format!("unknown vec build argument '{other}'")),
        }
    }

    if deterministic && payload_path.is_some() {
        return Err("--deterministic and --payload are mutually exclusive".into());
    }
    if !deterministic && payload_path.is_none() {
        return Err("vec build requires --deterministic or --payload <f32le.bin>".into());
    }
    let out = out.ok_or_else(|| "vec build requires --out <vectors.covev>".to_string())?;
    let dimension_count =
        dimension_count.ok_or_else(|| "vec build requires --dimension <n>".to_string())?;
    if file_codes.is_empty() {
        return Err("vec build requires at least one --file-code <u32>".into());
    }

    let vector_payload = if let Some(path) = payload_path {
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?
    } else {
        deterministic_vector_payload(&file_codes, dimension_count)?
    };
    let created_at_us = created_at_us.unwrap_or_else(current_time_us);
    let bytes = write_covev_filecode_vectors(&CoveVecFileCodeVectorBuild {
        artifact_id,
        created_at_us,
        dimension_count,
        file_codes: file_codes.clone(),
        vector_payload,
    })
    .map_err(|error| format!("cannot build {}: {error}", out.display()))?;
    fs::write(&out, &bytes).map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    let parsed = CoveAiFile::parse(&bytes)
        .map_err(|error| format!("built {} but validation failed: {error}", out.display()))?;
    println!(
        "Wrote {}: {} FileCode vectors, dimension {}, payload_access={:?}",
        out.display(),
        parsed.descriptor_tables.filecode_vector_bindings.len(),
        parsed
            .descriptor_tables
            .vector_spaces
            .first()
            .map(|space| space.dimension_count)
            .unwrap_or(dimension_count),
        parsed.payload_access
    );
    Ok(())
}

fn run_train(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "missing train subcommand\n\n{}",
            usage(HelpTopic::Train)
        ));
    }
    let subcommand = args.remove(0);
    match subcommand.as_str() {
        "export" => run_train_export(args),
        "-h" | "--help" => {
            print_usage(HelpTopic::Train);
            Ok(())
        }
        other => Err(format!(
            "unknown train subcommand '{other}'\n\n{}",
            usage(HelpTopic::Train)
        )),
    }
}

fn run_train_export(args: Vec<String>) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut format = "json".to_string();
    let mut profile_filter: Option<u32> = None;
    let mut split_filter: Option<u32> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                format = iter
                    .next()
                    .ok_or_else(|| "--format requires json or jsonl".to_string())?;
                if !matches!(format.as_str(), "json" | "jsonl") {
                    return Err("--format must be json or jsonl".into());
                }
            }
            "--out" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out requires a path".to_string())?;
                out = Some(PathBuf::from(value));
            }
            "--profile" => {
                profile_filter = Some(parse_u32_arg(iter.next().as_deref(), "--profile")?);
            }
            "--split" => {
                split_filter = Some(parse_u32_arg(iter.next().as_deref(), "--split")?);
            }
            "-h" | "--help" => {
                print_usage(HelpTopic::Train);
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown train export argument '{value}'"));
            }
            value => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("train export accepts exactly one input sidecar".into());
                }
            }
        }
    }

    let input = input
        .ok_or_else(|| "train export requires <training.coveai|training.covev>".to_string())?;
    let bytes =
        fs::read(&input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    if bytes.len() < 4
        || (bytes[bytes.len() - 4..] != MAGIC_COVEAI && bytes[bytes.len() - 4..] != MAGIC_COVEV)
    {
        return Err(format!(
            "{} is not a COVE-AI companion artifact (.coveai/.covev)",
            input.display()
        ));
    }
    let sidecar = CoveAiFile::parse(&bytes)
        .map_err(|error| format!("{}: invalid COVE-AI sidecar: {error}", input.display()))?;

    let text = if format == "jsonl" {
        training_export_jsonl(&sidecar, profile_filter, split_filter)?
    } else {
        serde_json::to_string_pretty(&training_export_json(
            &input,
            &sidecar,
            profile_filter,
            split_filter,
        ))
        .unwrap()
    };
    if let Some(out) = out {
        fs::write(&out, text)
            .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    } else if format == "jsonl" {
        print!("{text}");
    } else {
        println!("{text}");
    }
    Ok(())
}

fn training_export_json(
    input: &Path,
    sidecar: &CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
) -> serde_json::Value {
    let samples = filtered_training_samples(sidecar, profile_filter, split_filter)
        .into_iter()
        .map(training_sample_json)
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    if !matches!(
        sidecar.payload_access,
        cove_core::artifact::coveai::AiPayloadAccessState::StructurallyAllowed
    ) {
        diagnostics.push(serde_json::json!({
            "code": "COVE_AI_PAYLOAD_POLICY_BLOCKED",
            "message": "direct AI payload access is blocked until privacy summaries and policy scopes validate",
            "payload_access": format!("{:?}", sidecar.payload_access),
        }));
    }
    serde_json::json!({
        "path": input.display().to_string(),
        "artifact": match sidecar.artifact_kind {
            CoveAiArtifactKind::CoveAiBundle => "coveai",
            CoveAiArtifactKind::CoveVec => "covev",
        },
        "artifact_id": hex_bytes(&sidecar.header.artifact_id),
        "payload_access": format!("{:?}", sidecar.payload_access),
        "filters": {
            "training_profile_id": profile_filter,
            "split_id": split_filter,
        },
        "counts": {
            "training_profiles": sidecar.descriptor_tables.training_profiles.len(),
            "dataset_splits": sidecar.descriptor_tables.dataset_splits.len(),
            "dedup_groups": sidecar.descriptor_tables.dedup_groups.len(),
            "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.len(),
            "training_labels": sidecar.descriptor_tables.training_labels.len(),
            "samples_total": sidecar.descriptor_tables.training_samples.len(),
            "samples_exported": samples.len(),
        },
        "training_profiles": sidecar.descriptor_tables.training_profiles.iter().map(|profile| serde_json::json!({
            "training_profile_id": profile.training_profile_id,
            "profile_name_ref": profile.profile_name_ref,
            "task_family": profile.task_family,
            "modality_mask": profile.modality_mask,
            "source_snapshot_ref": profile.source_snapshot_ref,
            "map_profile_ref": profile.map_profile_ref,
            "chunk_profile_ref": profile.chunk_profile_ref,
            "tokenizer_profile_ref": profile.tokenizer_profile_ref,
            "vector_space_ref": profile.vector_space_ref,
            "multimodal_sequence_profile_ref": profile.multimodal_sequence_profile_ref,
            "split_policy_ref": profile.split_policy_ref,
            "sampling_policy_ref": profile.sampling_policy_ref,
            "dedup_policy_ref": profile.dedup_policy_ref,
            "quality_policy_ref": profile.quality_policy_ref,
            "license_policy_ref": profile.license_policy_ref,
            "redaction_policy_ref": profile.redaction_policy_ref,
            "default_generator_provenance_ref": profile.default_generator_provenance_ref,
            "reproducibility_class": profile.reproducibility_class,
            "flags": profile.flags,
        })).collect::<Vec<_>>(),
        "dataset_splits": sidecar.descriptor_tables.dataset_splits.iter().map(|split| serde_json::json!({
            "split_id": split.split_id,
            "split_name_ref": split.split_name_ref,
            "split_method": split.split_method,
            "source_snapshot_ref": split.source_snapshot_ref,
            "filter_policy_ref": split.filter_policy_ref,
            "seed": split.seed,
            "hash_function_ref": split.hash_function_ref,
            "stratification_path_ref": split.stratification_path_ref,
            "grouping_ref": split.grouping_ref,
            "ordering_policy_ref": split.ordering_policy_ref,
            "dedup_policy_ref": split.dedup_policy_ref,
            "sample_count": split.sample_count,
            "first_sample_ref": split.first_sample_ref,
            "flags": split.flags,
        })).collect::<Vec<_>>(),
        "dedup_groups": sidecar.descriptor_tables.dedup_groups.iter().map(|group| serde_json::json!({
            "dedup_group_id": group.dedup_group_id,
            "dedup_policy_ref": group.dedup_policy_ref,
            "canonical_member_sample_id": group.canonical_member_sample_id,
            "similarity_kind": group.similarity_kind,
            "dedup_authority": group.dedup_authority,
            "confidence_ppm": group.confidence_ppm,
            "first_member_ref": group.first_member_ref,
            "member_count": group.member_count,
            "flags": group.flags,
        })).collect::<Vec<_>>(),
        "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.iter().map(|plan| serde_json::json!({
            "epoch_plan_id": plan.epoch_plan_id,
            "training_profile_id": plan.training_profile_id,
            "split_ref": plan.split_ref,
            "seed": plan.seed,
            "permutation_kind": plan.permutation_kind,
            "rng_algorithm_ref": plan.rng_algorithm_ref,
            "permutation_function_ref": plan.permutation_function_ref,
            "shard_count": plan.shard_count,
            "first_shard_ref": plan.first_shard_ref,
            "shard_ref_count": plan.shard_ref_count,
            "flags": plan.flags,
        })).collect::<Vec<_>>(),
        "training_labels": sidecar.descriptor_tables.training_labels.iter().map(|label| serde_json::json!({
            "label_id": label.label_id,
            "label_kind": label.label_kind,
            "label_authority": label.label_authority,
            "label_payload_ref": label.label_payload_ref,
            "generator_provenance_ref": label.generator_provenance_ref,
            "human_review_ref": label.human_review_ref,
            "confidence_ppm": label.confidence_ppm,
            "evidence_ref": label.evidence_ref,
            "policy_ref": label.policy_ref,
            "flags": label.flags,
        })).collect::<Vec<_>>(),
        "samples": samples,
        "policy_withheld_diagnostics": diagnostics,
    })
}

fn training_export_jsonl(
    sidecar: &CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
) -> Result<String, String> {
    let mut out = String::new();
    for sample in filtered_training_samples(sidecar, profile_filter, split_filter) {
        out.push_str(&serde_json::to_string(&training_sample_json(sample)).unwrap());
        out.push('\n');
    }
    Ok(out)
}

fn filtered_training_samples<'a>(
    sidecar: &'a CoveAiFile,
    profile_filter: Option<u32>,
    split_filter: Option<u32>,
) -> Vec<&'a cove_core::artifact::coveai::TrainingSampleEntryV1> {
    sidecar
        .descriptor_tables
        .training_samples
        .iter()
        .filter(|sample| {
            profile_filter.is_none_or(|profile| sample.training_profile_id == profile)
                && split_filter.is_none_or(|split| sample.split_ref == split)
        })
        .collect()
}

fn training_sample_json(
    sample: &cove_core::artifact::coveai::TrainingSampleEntryV1,
) -> serde_json::Value {
    serde_json::json!({
        "sample_id": sample.sample_id,
        "training_profile_id": sample.training_profile_id,
        "example_kind": sample.example_kind,
        "split_ref": sample.split_ref,
        "source_ref": sample.source_ref,
        "evidence_ref": sample.evidence_ref,
        "input_ref": sample.input_ref,
        "target_ref": sample.target_ref,
        "label_ref": sample.label_ref,
        "metadata_ref": sample.metadata_ref,
        "token_sequence_pack_ref": sample.token_sequence_pack_ref,
        "multimodal_sequence_pack_ref": sample.multimodal_sequence_pack_ref,
        "vector_ref": sample.vector_ref,
        "quality_score_ppm": sample.quality_score_ppm,
        "sample_weight_ppm": sample.sample_weight_ppm,
        "dedup_group_ref": sample.dedup_group_ref,
        "license_ref": sample.license_ref,
        "policy_ref": sample.policy_ref,
        "teacher_model_ref": sample.teacher_model_ref,
        "generator_provenance_ref": sample.generator_provenance_ref,
        "judge_generator_provenance_ref": sample.judge_generator_provenance_ref,
        "label_generator_provenance_ref": sample.label_generator_provenance_ref,
        "flags": sample.flags,
    })
}

fn deterministic_vector_payload(
    file_codes: &[u32],
    dimension_count: u32,
) -> Result<Vec<u8>, String> {
    let value_count = file_codes
        .len()
        .checked_mul(
            usize::try_from(dimension_count)
                .map_err(|_| "--dimension is too large for this platform".to_string())?,
        )
        .ok_or_else(|| "deterministic vector payload size overflows usize".to_string())?;
    let mut payload = Vec::with_capacity(
        value_count
            .checked_mul(4)
            .ok_or_else(|| "deterministic vector payload size overflows usize".to_string())?,
    );
    for file_code in file_codes {
        for dimension in 0..dimension_count {
            let seed = u64::from(*file_code)
                .wrapping_mul(1_000_003)
                .wrapping_add(u64::from(dimension).wrapping_mul(97))
                .wrapping_add(17);
            let value = (seed % 10_000) as f32 / 10_000.0;
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(payload)
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
        ExportFormat::Arrow if arrow_export_uses_coveql_query(&args) => {
            run_arrow_query_export(args)
        }
        ExportFormat::Arrow => cove_datafusion::arrow_export_cli::run(args),
    }
}

fn arrow_export_uses_coveql_query(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--query" | "--query-file"))
}

fn run_arrow_query_export(args: Vec<String>) -> Result<(), String> {
    let command = parse_arrow_query_export_args(args)?;
    let input_bytes = fs::read(&command.input)
        .map_err(|error| format!("cannot read {}: {error}", command.input.display()))?;
    let query = match (&command.query, &command.query_file) {
        (Some(query), None) => query.clone(),
        (None, Some(path)) => read_query_file(path)?,
        _ => return Err("cove export arrow --query accepts exactly one query source".into()),
    };
    let query = prepare_query_text(&query, command.take, None)?;
    let mut execute_options = ExecuteArtifactOptions::default();
    execute_options.resolve_options.output_mode = Some(CoveQlOutputMode::ArrowRecordBatch {
        zero_copy_requested: false,
    });
    apply_graph_budget(&mut execute_options, command.graph_budget);
    if command.enable_graph_traversal {
        execute_options.resolve_options.graph_traversal_contract =
            Some(cli_graph_traversal_contract(&execute_options));
    }

    let delta_manifest =
        cove_datafusion::delta_snapshot::delta_chain_required(&input_bytes).unwrap_or(false);
    let mut delta_report = None;
    let mut delta_execution = None;
    let executed = if delta_manifest {
        let dataset = command
            .dataset
            .as_deref()
            .ok_or_else(|| "delta manifest CoveQL export requires --dataset <dir>".to_string())?;
        let snapshot = cove_datafusion::delta_snapshot::load_validated_delta_snapshot(
            &command.input,
            dataset,
            command.delta_request,
        )?;
        let plan_json = cove_datafusion::delta_snapshot::delta_snapshot_plan_json(
            Some(&command.input),
            &snapshot.plan,
            &snapshot.extension,
        );
        if command.delta_plan_json {
            eprintln!("{}", serde_json::to_string_pretty(&plan_json).unwrap());
        } else if command.delta_plan {
            print_query_delta_plan_text(&command.input, &snapshot.plan);
        }
        if command.perf_report {
            eprintln!(
                "delta_chain_depth={} selected_delta_count={} skipped_delta_count={}",
                snapshot.plan.metrics.delta_chain_depth,
                snapshot.plan.metrics.selected_delta_count,
                snapshot.plan.metrics.skipped_delta_count
            );
        }
        delta_report = Some(plan_json);
        match cove_datafusion::delta_snapshot::direct_object_surface_support(&snapshot) {
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::Supported => {
                let surface =
                    cove_datafusion::delta_snapshot::read_validated_delta_object_surface(
                        &snapshot,
                    )?;
                delta_execution = Some("direct_object_surface");
                if command.perf_report {
                    eprintln!("delta_execution=direct_object_surface");
                }
                execute_delta_object_surface_query(
                    &snapshot.base.bytes,
                    &surface,
                    &query,
                    &execute_options,
                )
                .map_err(|error| format_execution_error(error, false))?
            }
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                reason,
            } => {
                return Err(format!(
                    "non-materializing CoveQL export requires a direct COVE-O object surface, but this delta snapshot requires materialized planner metadata: {reason}"
                ));
            }
        }
    } else {
        if command.delta_request != CovmDeltaPruneRequest::default()
            || command.delta_plan
            || command.delta_plan_json
        {
            return Err("delta snapshot options require a COVM delta manifest input".into());
        }
        execute_query_from_artifact(&input_bytes, &query, execute_options)
            .map_err(|error| format_artifact_query_error(error, false))?
    };

    let rows = executed.row_counts.output_rows;
    let output_fingerprint = executed.output_fingerprint.clone();
    let batches = match executed.result {
        CoveQlExecutionResult::ArrowRecordBatches(batches) => batches,
        _ => return Err("CoveQL export did not produce Arrow record batches".into()),
    };
    let schema = batches
        .first()
        .map(|batch| batch.schema())
        .ok_or_else(|| "CoveQL export produced no Arrow batches".to_string())?;
    let output_bytes = match command.format {
        ArrowQueryExportOutputFormat::Ipc => {
            cove_datafusion::arrow_export_cli::write_ipc(&schema, &batches)?
        }
        ArrowQueryExportOutputFormat::Json => {
            cove_datafusion::arrow_export_cli::write_json(&batches)?
        }
    };
    cove_core::durable::durable_replace(&command.output, &output_bytes).map_err(|error| {
        format!(
            "cannot durably publish {}: {error}",
            command.output.display()
        )
    })?;

    if let Some(report) = command.report {
        let report_json = serde_json::json!({
            "version": 1,
            "source": command.input.display().to_string(),
            "output": command.output.display().to_string(),
            "format": match command.format {
                ArrowQueryExportOutputFormat::Ipc => "ipc",
                ArrowQueryExportOutputFormat::Json => "json",
            },
            "execution": "coveql_arrow_record_batches",
            "delta_execution": delta_execution,
            "batches": batches.len(),
            "rows": rows,
            "columns": schema.fields().len(),
            "output_fingerprint": output_fingerprint,
            "delta_snapshot": delta_report,
        });
        let text = serde_json::to_string_pretty(&report_json)
            .map_err(|error| format!("cannot serialize export report: {error}"))?;
        match report {
            ArrowQueryExportReportTarget::Stdout => println!("{text}"),
            ArrowQueryExportReportTarget::Path(path) => fs::write(&path, text)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?,
        }
    }
    Ok(())
}

fn parse_arrow_query_export_args(args: Vec<String>) -> Result<ArrowQueryExportCommand, String> {
    let mut query = None;
    let mut query_file = None;
    let mut format = ArrowQueryExportOutputFormat::Ipc;
    let mut report = None;
    let mut dataset = None;
    let mut delta_request = CovmDeltaPruneRequest::default();
    let mut delta_plan = false;
    let mut delta_plan_json = false;
    let mut perf_report = false;
    let mut take = None;
    let mut graph_budget = GraphBudgetOverrides::default();
    let mut enable_graph_traversal = false;
    let mut positional = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--query" => {
                if query.is_some() || query_file.is_some() {
                    return Err("cove export arrow accepts only one --query or --query-file".into());
                }
                query = Some(
                    iter.next()
                        .ok_or_else(|| "--query requires CoveQL text".to_string())?,
                );
            }
            "--query-file" => {
                if query.is_some() || query_file.is_some() {
                    return Err("cove export arrow accepts only one --query or --query-file".into());
                }
                query_file =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--query-file requires a path or '-'".to_string()
                    })?));
            }
            "--format" => {
                format = parse_arrow_query_export_format(
                    &iter
                        .next()
                        .ok_or_else(|| "--format requires ipc or json".to_string())?,
                )?;
            }
            "--report" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--report requires '-' or a file path".to_string())?;
                report = Some(if raw == "-" {
                    ArrowQueryExportReportTarget::Stdout
                } else {
                    ArrowQueryExportReportTarget::Path(PathBuf::from(raw))
                });
            }
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--as-of-csn" => {
                delta_request.as_of_csn = Some(parse_u64(iter.next().as_deref(), "--as-of-csn")?);
            }
            "--as-of-commit-us" => {
                delta_request.as_of_commit_timestamp_us =
                    Some(parse_i64(iter.next().as_deref(), "--as-of-commit-us")?);
            }
            "--source-publish-range" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--source-publish-range requires start:end".to_string())?;
                delta_request.source_publish_range_us =
                    Some(parse_i64_range(&raw, "--source-publish-range")?);
            }
            "--delta-plan" => delta_plan = true,
            "--delta-plan-json" => {
                delta_plan = true;
                delta_plan_json = true;
            }
            "--perf-report" => perf_report = true,
            "--take" => {
                take = Some(parse_positive_usize(iter.next().as_deref(), "--take")?);
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
            "-h" | "--help" => {
                return Err("usage: cove export arrow --query '<coveql>' [--format ipc|json] [--report -|path] [--dataset dir] [--as-of-csn n|--as-of-commit-us n] [--delta-plan|--delta-plan-json] <input.cove|manifest.covm> <output.arrow|output.json>".into());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown CoveQL export option '{other}'"));
            }
            positional_arg => positional.push(PathBuf::from(positional_arg)),
        }
    }
    if query.is_none() && query_file.is_none() {
        return Err("cove export arrow --query requires --query or --query-file".into());
    }
    if positional.len() != 2 {
        return Err("expected <input.cove|manifest.covm> and <output.arrow|output.json>".into());
    }
    Ok(ArrowQueryExportCommand {
        input: positional.remove(0),
        output: positional.remove(0),
        query,
        query_file,
        format,
        report,
        dataset,
        delta_request,
        delta_plan,
        delta_plan_json,
        perf_report,
        take,
        graph_budget,
        enable_graph_traversal,
    })
}

fn parse_arrow_query_export_format(raw: &str) -> Result<ArrowQueryExportOutputFormat, String> {
    match raw {
        "ipc" => Ok(ArrowQueryExportOutputFormat::Ipc),
        "json" => Ok(ArrowQueryExportOutputFormat::Json),
        _ => Err("--format must be ipc or json".into()),
    }
}

fn run_map(args: Vec<String>) -> Result<(), String> {
    if args.first().is_some_and(|arg| arg == "delta") {
        return run_map_delta(args.into_iter().skip(1).collect());
    }
    cove_map::run_cli(args)
}

const MAP_DELTA_BUILD_USAGE: &str = "usage: cove map delta build <manifest.covm> --dataset <dir> --out-dir <dir> [--as-of-csn n|--as-of-commit-us n] [--force] [--json] [--publish-covm] [--verify] [--projection-output cove-t|none] [--object-name <file.cove>]\n       cove map delta build --base <manifest.covm> --dataset <dir> --mapping <mapping.covemap> --out <delta.covedelta> [--source-publish-range start:end] [--force] [--json] <source...>";

fn run_map_delta(mut args: Vec<String>) -> Result<(), String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err(MAP_DELTA_BUILD_USAGE.into());
    }
    let command = args.remove(0);
    if command != "build" {
        return Err(format!(
            "unknown map delta command '{command}'; expected build"
        ));
    }
    run_map_delta_build(args)
}

fn run_map_delta_build(args: Vec<String>) -> Result<(), String> {
    let mut manifest = None;
    let mut base_manifest = None;
    let mut mapping = None;
    let mut dataset = None;
    let mut out_dir = None;
    let mut out = None;
    let mut force = false;
    let mut json = false;
    let mut publish_covm = false;
    let mut verify = false;
    let mut object_name = None;
    let mut projection_output = cove_map::MapBuildProjectionOutput::CoveT;
    let mut request = CovmDeltaPruneRequest::default();
    let mut positional = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--manifest" | "--snapshot" => {
                manifest = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| format!("{arg} requires a manifest path"))?,
                ));
            }
            "--base" => {
                base_manifest =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--base requires a manifest path".to_string()
                    })?));
            }
            "--mapping" | "--map" => {
                mapping = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| format!("{arg} requires a COVE-MAP path"))?,
                ));
            }
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--out-dir" => {
                out_dir =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out-dir requires a directory path".to_string()
                    })?));
            }
            "--out" => {
                out = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--out requires a delta path".to_string())?,
                ));
            }
            "--as-of-csn" => {
                request.as_of_csn = Some(parse_u64(iter.next().as_deref(), "--as-of-csn")?);
            }
            "--as-of-commit-us" => {
                request.as_of_commit_timestamp_us =
                    Some(parse_i64(iter.next().as_deref(), "--as-of-commit-us")?);
            }
            "--source-publish-range" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--source-publish-range requires start:end".to_string())?;
                request.source_publish_range_us =
                    Some(parse_i64_range(&raw, "--source-publish-range")?);
            }
            "--force" => force = true,
            "--json" => json = true,
            "--publish-covm" => publish_covm = true,
            "--verify" => verify = true,
            "--object-name" => {
                object_name = Some(
                    iter.next()
                        .ok_or_else(|| "--object-name requires a file name".to_string())?,
                );
            }
            "--projection-output" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--projection-output requires cove-t or none".to_string())?;
                projection_output = match raw.as_str() {
                    "cove-t" => cove_map::MapBuildProjectionOutput::CoveT,
                    "none" => cove_map::MapBuildProjectionOutput::None,
                    _ => return Err("--projection-output must be cove-t or none".into()),
                };
            }
            "-h" | "--help" => {
                return Err(MAP_DELTA_BUILD_USAGE.into());
            }
            _ if arg.starts_with("--") => {
                return Err(format!("unknown map delta build option '{arg}'"))
            }
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let semantic_mode = base_manifest.is_some() || mapping.is_some() || out.is_some();
    if semantic_mode {
        if manifest.is_some() {
            return Err("map semantic delta build uses --base, not --manifest/--snapshot".into());
        }
        if out_dir.is_some() {
            return Err(
                "map semantic delta build writes --out <delta.covedelta>, not --out-dir".into(),
            );
        }
        if publish_covm || verify || object_name.is_some() {
            return Err(
                "map semantic delta build does not support --publish-covm, --verify, or --object-name"
                    .into(),
            );
        }
        if request.as_of_csn.is_some() || request.as_of_commit_timestamp_us.is_some() {
            return Err(
                "map semantic delta build currently uses the latest validated parent snapshot"
                    .into(),
            );
        }
        let base_manifest = base_manifest.ok_or_else(|| {
            "map semantic delta build requires --base <manifest.covm>".to_string()
        })?;
        let dataset = dataset
            .ok_or_else(|| "map semantic delta build requires --dataset <dir>".to_string())?;
        let mapping = mapping
            .ok_or_else(|| "map semantic delta build requires --mapping <file>".to_string())?;
        let out = out.ok_or_else(|| {
            "map semantic delta build requires --out <delta.covedelta>".to_string()
        })?;
        if positional.is_empty() {
            return Err("map semantic delta build requires at least one source path".into());
        }
        return run_map_semantic_delta_build(
            base_manifest,
            dataset,
            mapping,
            positional,
            out,
            force,
            json,
            request.source_publish_range_us,
        );
    }
    if manifest.is_none() && positional.len() == 1 {
        manifest = Some(positional.remove(0));
    }
    if !positional.is_empty() {
        return Err("map delta build accepts only one manifest positional argument".into());
    }
    let manifest =
        manifest.ok_or_else(|| "map delta build requires <manifest.covm>".to_string())?;
    let dataset = dataset.ok_or_else(|| "map delta build requires --dataset <dir>".to_string())?;
    let out_dir = out_dir.ok_or_else(|| "map delta build requires --out-dir <dir>".to_string())?;
    let (_snapshot, materialized) =
        cove_datafusion::delta_snapshot::materialize_delta_snapshot(&manifest, &dataset, request)?;
    let result = cove_map::build_from_cove_o_bytes(
        &format!("{}#delta-snapshot", manifest.display()),
        materialized.bytes,
        cove_map::MapBuildOptions {
            out_dir: out_dir.clone(),
            force,
            object_name,
            projection_output,
            evidence_encoding: cove_map::MapEvidenceEncoding::Compact,
            section_compression: cove_map::MapBuildSectionCompression::Zstd,
            verify,
            publish_covm,
            reuse_cache: true,
        },
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result.manifest).unwrap()
        );
    } else {
        println!("COVE-MAP delta build: {}", out_dir.display());
        if let Some(object) = result
            .manifest
            .pointer("/artifacts/object/path")
            .and_then(serde_json::Value::as_str)
        {
            println!("Object: {object}");
        }
        if let Some(covm) = result
            .manifest
            .pointer("/artifacts/covm/path")
            .and_then(serde_json::Value::as_str)
        {
            println!("Manifest: {covm}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_map_semantic_delta_build(
    base_manifest: PathBuf,
    dataset: PathBuf,
    mapping: PathBuf,
    sources: Vec<PathBuf>,
    out: PathBuf,
    force: bool,
    json: bool,
    source_publish_range_us: Option<(i64, i64)>,
) -> Result<(), String> {
    let snapshot = cove_datafusion::delta_snapshot::load_validated_delta_snapshot(
        &base_manifest,
        &dataset,
        CovmDeltaPruneRequest::default(),
    )?;
    let parent_surface =
        cove_datafusion::delta_snapshot::read_validated_delta_object_surface(&snapshot)?;
    let parent_object_states =
        cove_core::profile::cove_o::reconstruct_object_states(&parent_surface, &Default::default())
            .map_err(|error| {
                format!("cannot reconstruct map semantic delta parent states: {error}")
            })?;
    let parent_ref = snapshot
        .extension
        .ordered_delta_artifact_refs
        .last()
        .unwrap_or(&snapshot.extension.base_artifact_ref)
        .clone();
    let chain_ordinal = u32::try_from(snapshot.extension.ordered_delta_artifact_refs.len() + 1)
        .map_err(|_| "map semantic delta chain ordinal overflows".to_string())?;
    let commit_time_start_us =
        current_time_us().max(snapshot.extension.created_at_us.saturating_add(1));
    let result = cove_map::build_semantic_delta_from_paths(
        &mapping,
        &sources,
        cove_map::MapSemanticDeltaBuildOptions {
            out: out.clone(),
            force,
            parent: cove_map::MapSemanticDeltaParent {
                dataset_id: snapshot.extension.dataset_id,
                parent_snapshot_id: snapshot.extension.result_snapshot_id,
                chain_ordinal,
                chain_depth: chain_ordinal,
                parent_ref,
            },
            parent_object_types: parent_surface.object_types,
            parent_object_states,
            parent_evidence_entries: parent_surface
                .evidence_index
                .map(|index| index.entries)
                .unwrap_or_default(),
            parent_projection_catalog: parent_surface.projection_catalog,
            csn_start: snapshot.extension.csn_max.saturating_add(1),
            commit_time_start_us,
            source_publish_range_us,
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result.report).unwrap());
    } else {
        println!("COVE-MAP semantic delta: {}", out.display());
        if let Some(snapshot_id) = result
            .report
            .pointer("/delta/snapshot_id")
            .and_then(serde_json::Value::as_str)
        {
            println!("  snapshot_id: {snapshot_id}");
        }
        println!("  bytes_written: {}", result.bytes_written);
    }
    Ok(())
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

fn run_examples(json: bool) -> Result<(), String> {
    let sample_dir = "examples/coveql";
    let showcase_dir = "examples/customer360";
    let showcase_examples = vec![
        (
            "Generate the Customer 360 data-science showcase",
            "cove showcase customer360 --profile quick --out examples/customer360 --force",
        ),
        (
            "Generate the COVE-O proof suite",
            "cove showcase proof-suite --scenario all --profile quick --out target/cove-proof-suite --force",
        ),
        (
            "Inspect canonical customer surfaces",
            "cove inspect --queries --performance examples/customer360/customers.cove",
        ),
        (
            "Query canonical customer rows",
            "cove query examples/customer360/customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'",
        ),
        (
            "Join customers to generated events",
            "cove query examples/customer360/customers.cove --external-table events=examples/customer360/events.jsonl 'table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, tier: c.tier, event_kind: e.event_kind, event_score: e.score).take(10)'",
        ),
    ];
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
            "showcase_dir": showcase_dir,
            "showcases": [{
                "name": "customer360",
                "profile": "quick",
                "commands": showcase_examples.iter().map(|(title, command)| {
                    serde_json::json!({
                        "title": title,
                        "command": command,
                    })
                }).collect::<Vec<_>>(),
            }],
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
    println!("Customer 360 showcase directory: {showcase_dir}");
    println!();
    println!("Data-science showcase:");
    for (title, command) in &showcase_examples {
        println!("{title}:");
        println!("  {command}");
    }
    println!();
    println!("Sample directory: {sample_dir}");
    println!();
    for (title, command) in examples {
        println!("{title}:");
        println!("  {command}");
    }
    println!();
    println!("Regenerate samples from the repository root with:");
    println!("  cargo run -p cove-cli --example generate_beginner_samples -- examples/coveql");
    Ok(())
}

fn run_showcase_customer360(
    out_dir: &Path,
    profile: Customer360Profile,
    force: bool,
    json: bool,
) -> Result<(), String> {
    let manifest = generate_customer360(&Customer360Options {
        out_dir: out_dir.to_path_buf(),
        profile,
        force,
    })?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("cannot serialize Customer 360 manifest: {error}"))?
        );
    } else {
        println!(
            "Generated Customer 360 showcase ({}) at {}",
            profile.as_str(),
            out_dir.display()
        );
        println!(
            "Manifest: {}",
            out_dir.join("customer360-manifest.json").display()
        );
        println!("Try next:");
        println!(
            "  cove inspect --queries --performance {}/customers.cove",
            out_dir.display()
        );
        println!("  cove query {}/customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'", out_dir.display());
        println!(
            "  python3 {}/notebooks/customer360_analysis.py --input-dir {}",
            out_dir.display(),
            out_dir.display()
        );
    }
    Ok(())
}

fn run_showcase_proof_suite(
    out_dir: &Path,
    profile: Customer360Profile,
    scenario: ProofSuiteScenario,
    force: bool,
    json: bool,
) -> Result<(), String> {
    let manifest = generate_proof_suite(&ProofSuiteOptions {
        out_dir: out_dir.to_path_buf(),
        profile,
        scenario,
        force,
    })?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("cannot serialize proof-suite manifest: {error}"))?
        );
    } else {
        println!(
            "Generated COVE-O proof suite ({}, scenario {}) at {}",
            profile.as_str(),
            scenario.as_str(),
            out_dir.display()
        );
        println!(
            "Manifest: {}",
            out_dir.join("proof-suite-manifest.json").display()
        );
        println!("Try next:");
        println!(
            "  cove map doctor --bundle-dir {}/customer360/map-build-bundle",
            out_dir.display()
        );
        println!(
            "  cove map doctor --bundle-dir {}/claims/map-build-bundle",
            out_dir.display()
        );
        println!(
            "  cove map doctor --bundle-dir {}/catalog/map-build-bundle",
            out_dir.display()
        );
    }
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

fn run_inspect(
    file: &Path,
    queries: bool,
    json: bool,
    performance: bool,
    ai: bool,
) -> Result<(), String> {
    let bytes =
        fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?;
    if ai {
        return run_ai_inspect(file, &bytes, json);
    }
    if delta::is_covedelta_bytes(&bytes) {
        if queries || performance {
            return Err(
                "COVEDELTA inspect does not support --queries or --performance; use `cove delta inspect`"
                    .into(),
            );
        }
        return delta::inspect_covedelta_for_beginner(file, json);
    }
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

fn run_ai_inspect(file: &Path, bytes: &[u8], json: bool) -> Result<(), String> {
    if bytes.len() >= 4
        && (bytes[bytes.len() - 4..] == MAGIC_COVEAI || bytes[bytes.len() - 4..] == MAGIC_COVEV)
    {
        let sidecar = CoveAiFile::parse(bytes)
            .map_err(|error| format!("{}: invalid COVE-AI sidecar: {error}", file.display()))?;
        if json {
            let value = serde_json::json!({
                "path": file.display().to_string(),
                "artifact": match sidecar.artifact_kind {
                    CoveAiArtifactKind::CoveAiBundle => "coveai",
                    CoveAiArtifactKind::CoveVec => "covev",
                },
                "version": {
                    "major": sidecar.header.version_major,
                    "minor": sidecar.header.version_minor,
                },
                "artifact_id": hex_bytes(&sidecar.header.artifact_id),
                "section_count": sidecar.sections.len(),
                "payload_access": format!("{:?}", sidecar.payload_access),
                "records": {
                    "source_bindings": sidecar.descriptor_tables.source_bindings.len(),
                    "privacy_summaries": sidecar.descriptor_tables.privacy_summaries.len(),
                    "payload_refs": sidecar.descriptor_tables.payload_refs.len(),
                    "payload_integrity": sidecar.descriptor_tables.payload_integrity.len(),
                    "chunk_profiles": sidecar.descriptor_tables.chunk_profiles.len(),
                    "text_chunks": sidecar.descriptor_tables.text_chunks.len(),
                    "tokenizer_profiles": sidecar.descriptor_tables.tokenizer_profiles.len(),
                    "vector_spaces": sidecar.descriptor_tables.vector_spaces.len(),
                    "vector_payload_blocks": sidecar.descriptor_tables.vector_payload_blocks.len(),
                    "vector_entries": sidecar.descriptor_tables.vector_entries.len(),
                    "filecode_vector_bindings": sidecar.descriptor_tables.filecode_vector_bindings.len(),
                    "vector_indexes": sidecar.descriptor_tables.vector_indexes.len(),
                    "token_blocks": sidecar.descriptor_tables.token_blocks.len(),
                    "tokenized_spans": sidecar.descriptor_tables.tokenized_spans.len(),
                    "token_sequence_packs": sidecar.descriptor_tables.token_sequence_packs.len(),
                    "training_profiles": sidecar.descriptor_tables.training_profiles.len(),
                    "training_samples": sidecar.descriptor_tables.training_samples.len(),
                    "dataset_splits": sidecar.descriptor_tables.dataset_splits.len(),
                    "dedup_groups": sidecar.descriptor_tables.dedup_groups.len(),
                    "training_epoch_plans": sidecar.descriptor_tables.training_epoch_plans.len(),
                    "training_labels": sidecar.descriptor_tables.training_labels.len(),
                    "preference_pairs": sidecar.descriptor_tables.preference_pairs.len(),
                    "generator_provenance": sidecar.descriptor_tables.generator_provenance.len(),
                    "model_actors": sidecar.descriptor_tables.model_actors.len(),
                    "generation_decoding_profiles": sidecar.descriptor_tables.generation_decoding_profiles.len(),
                    "human_reviews": sidecar.descriptor_tables.human_reviews.len(),
                    "tensor_layouts": sidecar.descriptor_tables.tensor_layouts.len(),
                    "device_transfer_hints": sidecar.descriptor_tables.device_transfer_hints.len(),
                    "assets": sidecar.descriptor_tables.assets.len(),
                    "multimodal_sequence_packs": sidecar.descriptor_tables.multimodal_sequence_packs.len(),
                    "multimodal_sequence_elements": sidecar.descriptor_tables.multimodal_sequence_elements.len(),
                },
                "sections": sidecar.sections.iter().map(|section| serde_json::json!({
                    "section_id": section.entry.section_id,
                    "section_kind": section.entry.section_kind,
                    "offset": section.entry.offset,
                    "length": section.entry.length,
                    "profile": section.entry.profile_kind,
                    "payload_encoding": section.entry.payload_encoding,
                    "records": section.record_headers.len(),
                })).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            return Ok(());
        }
        println!("AI Inspect: {}", file.display());
        println!(
            "Artifact: {}",
            match sidecar.artifact_kind {
                CoveAiArtifactKind::CoveAiBundle => ".coveai (CVA2)",
                CoveAiArtifactKind::CoveVec => ".covev (CVV2)",
            }
        );
        println!("Artifact ID: {}", hex_bytes(&sidecar.header.artifact_id));
        println!("Sections: {}", sidecar.sections.len());
        println!("Payload access: {:?}", sidecar.payload_access);
        println!("Records:");
        println!(
            "  source_bindings={} privacy_summaries={} payload_refs={} payload_integrity={}",
            sidecar.descriptor_tables.source_bindings.len(),
            sidecar.descriptor_tables.privacy_summaries.len(),
            sidecar.descriptor_tables.payload_refs.len(),
            sidecar.descriptor_tables.payload_integrity.len()
        );
        println!(
            "  chunk_profiles={} text_chunks={} tokenizer_profiles={} token_blocks={} tokenized_spans={} token_sequence_packs={} training_profiles={} training_samples={} dataset_splits={} dedup_groups={} training_epoch_plans={} training_labels={} preference_pairs={} generator_provenance={} model_actors={} generation_decoding_profiles={} human_reviews={} tensor_layouts={} device_transfer_hints={} assets={} multimodal_sequence_packs={} multimodal_sequence_elements={} vector_spaces={} vector_blocks={} vector_entries={} filecode_bindings={} vector_indexes={}",
            sidecar.descriptor_tables.chunk_profiles.len(),
            sidecar.descriptor_tables.text_chunks.len(),
            sidecar.descriptor_tables.tokenizer_profiles.len(),
            sidecar.descriptor_tables.token_blocks.len(),
            sidecar.descriptor_tables.tokenized_spans.len(),
            sidecar.descriptor_tables.token_sequence_packs.len(),
            sidecar.descriptor_tables.training_profiles.len(),
            sidecar.descriptor_tables.training_samples.len(),
            sidecar.descriptor_tables.dataset_splits.len(),
            sidecar.descriptor_tables.dedup_groups.len(),
            sidecar.descriptor_tables.training_epoch_plans.len(),
            sidecar.descriptor_tables.training_labels.len(),
            sidecar.descriptor_tables.preference_pairs.len(),
            sidecar.descriptor_tables.generator_provenance.len(),
            sidecar.descriptor_tables.model_actors.len(),
            sidecar.descriptor_tables.generation_decoding_profiles.len(),
            sidecar.descriptor_tables.human_reviews.len(),
            sidecar.descriptor_tables.tensor_layouts.len(),
            sidecar.descriptor_tables.device_transfer_hints.len(),
            sidecar.descriptor_tables.assets.len(),
            sidecar.descriptor_tables.multimodal_sequence_packs.len(),
            sidecar.descriptor_tables.multimodal_sequence_elements.len(),
            sidecar.descriptor_tables.vector_spaces.len(),
            sidecar.descriptor_tables.vector_payload_blocks.len(),
            sidecar.descriptor_tables.vector_entries.len(),
            sidecar.descriptor_tables.filecode_vector_bindings.len(),
            sidecar.descriptor_tables.vector_indexes.len()
        );
        if !sidecar.sections.is_empty() {
            println!("Sections:");
            for section in &sidecar.sections {
                println!(
                    "  - id={} kind={} offset={} len={} records={}",
                    section.entry.section_id,
                    section.entry.section_kind,
                    section.entry.offset,
                    section.entry.length,
                    section.record_headers.len()
                );
            }
        }
        return Ok(());
    }

    if bytes.len() >= 4 && bytes[bytes.len() - 4..] == MAGIC_COVEMAP {
        let map = CovemapFile::parse_validated(bytes)
            .map_err(|error| format!("{}: invalid COVE-MAP artifact: {error}", file.display()))?;
        let embedded = parse_covemap_embedded_sections(&map)?;
        let summary = map_ai_summary(&embedded);
        if json {
            let value = map_ai_summary_json(file, "covemap", &summary);
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            return Ok(());
        }
        print_map_ai_summary(file, "covemap", &summary);
        return Ok(());
    }

    let parsed = validate_bytes_with_options(bytes, ValidationOptions::default())
        .map_err(|error| format!("{}: invalid COVE file: {error}", file.display()))?;
    let ai_sections = parsed
        .validated
        .footer
        .sections
        .iter()
        .filter(|section| {
            SectionKind::from_u16(section.section_kind)
                .map(is_ai_section_kind_for_inspect)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let embedded_map_ai_sections = parse_cove_embedded_map_ai_sections(bytes, &parsed.validated)?;
    let summary = map_ai_summary(&embedded_map_ai_sections);
    if json {
        let value = serde_json::json!({
            "path": file.display().to_string(),
            "artifact": "cove",
            "embedded_ai_sections": ai_sections.iter().map(|section| serde_json::json!({
                "section_id": section.section_id,
                "section_kind": section.section_kind,
                "offset": section.offset,
                "length": section.length,
                "required_features": section.required_features,
                "optional_features": section.optional_features,
            })).collect::<Vec<_>>(),
            "map_ai": map_ai_summary_value(&summary),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }
    println!("AI Inspect: {}", file.display());
    println!("Artifact: .cove");
    println!("Embedded AI sections: {}", ai_sections.len());
    print_map_ai_summary_details(&summary);
    for section in ai_sections {
        println!(
            "  - id={} kind={:?} offset={} len={}",
            section.section_id,
            SectionKind::from_u16(section.section_kind).unwrap(),
            section.offset,
            section.length
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct MapAiInspectSummary {
    active_profiles: Vec<String>,
    inactive_profiles: Vec<String>,
    slot_policies: Vec<MapAiSlotInspect>,
    forbidden_slots: Vec<MapAiSlotInspect>,
    template_ids: Vec<String>,
    training_policy_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct MapAiSlotInspect {
    slot_policy_id: String,
    path: String,
    role: String,
    decision: String,
    granularity: String,
    sensitivity: String,
    source_id: Option<String>,
    source_column: Option<String>,
    object_type: Option<String>,
    property_id: Option<String>,
    association_type: Option<String>,
    template_id: Option<String>,
    chunk_profile_id: Option<String>,
    tokenizer_profile_id: Option<String>,
    training_policy_id: Option<String>,
}

fn parse_covemap_embedded_sections(map: &CovemapFile) -> Result<Vec<EmbeddedMapSection>, String> {
    let mut out = Vec::new();
    for section in &map.sections {
        let kind = u16::try_from(section.entry.section_id)
            .ok()
            .and_then(SectionKind::from_u16)
            .ok_or_else(|| format!("unknown COVE-MAP section {}", section.entry.section_id))?;
        if !is_map_ai_section_kind(kind) {
            continue;
        }
        out.push(
            parse_embedded_section(kind, &section.payload)
                .map_err(|error| format!("invalid {kind:?} payload: {error}"))?,
        );
    }
    Ok(out)
}

fn parse_cove_embedded_map_ai_sections(
    bytes: &[u8],
    validated: &ValidatedCoveFile,
) -> Result<Vec<EmbeddedMapSection>, String> {
    let mut out = Vec::new();
    for entry in &validated.footer.sections {
        let Some(kind) = SectionKind::from_u16(entry.section_kind) else {
            continue;
        };
        if !is_map_ai_section_kind(kind) {
            continue;
        }
        let payload = compression::section_payload(bytes, entry)
            .map_err(|error| format!("cannot decode embedded {kind:?}: {error}"))?;
        out.push(
            parse_embedded_section(kind, &payload)
                .map_err(|error| format!("invalid embedded {kind:?}: {error}"))?,
        );
    }
    Ok(out)
}

fn map_ai_summary(sections: &[EmbeddedMapSection]) -> MapAiInspectSummary {
    let mut summary = MapAiInspectSummary::default();
    for section in sections {
        match section {
            EmbeddedMapSection::AiProfileCatalog(catalog) => {
                for profile in &catalog.profiles {
                    if profile.active {
                        summary.active_profiles.push(profile.profile_id.clone());
                    } else {
                        summary.inactive_profiles.push(profile.profile_id.clone());
                    }
                }
                for slot in &catalog.slot_policies {
                    let inspect = MapAiSlotInspect {
                        slot_policy_id: slot.slot_policy_id.clone(),
                        path: slot.path.clone(),
                        role: slot.role.clone(),
                        decision: slot.decision.clone(),
                        granularity: slot.granularity.clone(),
                        sensitivity: slot.sensitivity.clone(),
                        source_id: slot.source_id.clone(),
                        source_column: slot.source_column.clone(),
                        object_type: slot.object_type.clone(),
                        property_id: slot.property_id.clone(),
                        association_type: slot.association_type.clone(),
                        template_id: slot.template_id.clone(),
                        chunk_profile_id: slot.chunk_profile_id.clone(),
                        tokenizer_profile_id: slot.tokenizer_profile_id.clone(),
                        training_policy_id: slot.training_policy_id.clone(),
                    };
                    if inspect.decision == "Forbidden" || inspect.sensitivity == "Forbidden" {
                        summary.forbidden_slots.push(inspect.clone());
                    }
                    summary.slot_policies.push(inspect);
                }
            }
            EmbeddedMapSection::AiTemplateCatalog(catalog) => {
                summary.template_ids.extend(
                    catalog
                        .templates
                        .iter()
                        .map(|template| template.template_id.clone()),
                );
            }
            EmbeddedMapSection::AiTrainingPolicyCatalog(catalog) => {
                summary.training_policy_ids.extend(
                    catalog
                        .training_policies
                        .iter()
                        .map(|policy| policy.training_policy_id.clone()),
                );
            }
            _ => {}
        }
    }
    summary.active_profiles.sort();
    summary.inactive_profiles.sort();
    summary.template_ids.sort();
    summary.training_policy_ids.sort();
    summary
}

fn map_ai_summary_json(
    file: &Path,
    artifact: &str,
    summary: &MapAiInspectSummary,
) -> serde_json::Value {
    serde_json::json!({
        "path": file.display().to_string(),
        "artifact": artifact,
        "map_ai": map_ai_summary_value(summary),
    })
}

fn map_ai_summary_value(summary: &MapAiInspectSummary) -> serde_json::Value {
    serde_json::json!({
        "active_profiles": summary.active_profiles,
        "inactive_profiles": summary.inactive_profiles,
        "slot_policy_count": summary.slot_policies.len(),
        "template_count": summary.template_ids.len(),
        "training_policy_count": summary.training_policy_ids.len(),
        "forbidden_slot_count": summary.forbidden_slots.len(),
        "templates": summary.template_ids,
        "training_policies": summary.training_policy_ids,
        "slot_policies": summary.slot_policies.iter().map(map_ai_slot_json).collect::<Vec<_>>(),
        "forbidden_slots": summary.forbidden_slots.iter().map(map_ai_slot_json).collect::<Vec<_>>(),
    })
}

fn map_ai_slot_json(slot: &MapAiSlotInspect) -> serde_json::Value {
    serde_json::json!({
        "slot_policy_id": slot.slot_policy_id,
        "path": slot.path,
        "role": slot.role,
        "decision": slot.decision,
        "granularity": slot.granularity,
        "sensitivity": slot.sensitivity,
        "source_id": slot.source_id,
        "source_column": slot.source_column,
        "object_type": slot.object_type,
        "property_id": slot.property_id,
        "association_type": slot.association_type,
        "template_id": slot.template_id,
        "chunk_profile_id": slot.chunk_profile_id,
        "tokenizer_profile_id": slot.tokenizer_profile_id,
        "training_policy_id": slot.training_policy_id,
    })
}

fn print_map_ai_summary(file: &Path, artifact: &str, summary: &MapAiInspectSummary) {
    println!("AI Inspect: {}", file.display());
    println!("Artifact: .{artifact}");
    print_map_ai_summary_details(summary);
}

fn print_map_ai_summary_details(summary: &MapAiInspectSummary) {
    println!("COVE-MAP-AI:");
    println!("  active_profiles: {}", summary.active_profiles.len());
    for profile in &summary.active_profiles {
        println!("    - {profile}");
    }
    println!("  slot_policies: {}", summary.slot_policies.len());
    println!("  templates: {}", summary.template_ids.len());
    println!("  training_policies: {}", summary.training_policy_ids.len());
    println!("  forbidden_slots: {}", summary.forbidden_slots.len());
    for slot in &summary.forbidden_slots {
        println!(
            "    - {} decision={} sensitivity={}",
            slot.path, slot.decision, slot.sensitivity
        );
    }
    for slot in &summary.slot_policies {
        println!(
            "    slot {} path={} role={} decision={} sensitivity={}",
            slot.slot_policy_id, slot.path, slot.role, slot.decision, slot.sensitivity
        );
    }
}

fn is_map_ai_section_kind(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapAiProfileCatalog
            | SectionKind::MapAiTemplateCatalog
            | SectionKind::MapAiTrainingPolicyCatalog
    )
}

fn is_ai_section_kind_for_inspect(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::MapAiProfileCatalog
            | SectionKind::MapAiTemplateCatalog
            | SectionKind::MapAiTrainingPolicyCatalog
            | SectionKind::AiCompanionArtifactRef
            | SectionKind::AiSourceBinding
            | SectionKind::AiChunkProfile
            | SectionKind::AiTextChunkIndex
            | SectionKind::AiTokenizerProfile
            | SectionKind::AiTokenBlock
            | SectionKind::AiTokenizedSpan
            | SectionKind::AiTokenSequencePack
            | SectionKind::AiVectorSpace
            | SectionKind::AiVectorBinding
            | SectionKind::AiVectorPayloadBlock
            | SectionKind::AiVectorComposition
            | SectionKind::AiVectorIndex
            | SectionKind::AiTensorLayout
            | SectionKind::AiAssetManifest
            | SectionKind::AiMultimodalSequence
            | SectionKind::AiTrainingProfile
            | SectionKind::AiTrainingSampleIndex
            | SectionKind::AiTrainingSplitDedupEpoch
            | SectionKind::AiLabelPreference
            | SectionKind::AiGeneratorProvenance
            | SectionKind::AiReferenceTables
            | SectionKind::AiPayloadIntegrity
            | SectionKind::AiPrivacySummary
            | SectionKind::AiSectionFeatureBinding
            | SectionKind::AiVectorDirectory
            | SectionKind::AiPayloadBytes
    )
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
    let mut bytes = match file {
        Some(file) => {
            fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?
        }
        None => external_only_context_bytes(),
    };
    let delta_manifest = file.is_some()
        && cove_datafusion::delta_snapshot::delta_chain_required(&bytes).unwrap_or(false);
    let mut delta_plan = None;
    let mut delta_snapshot = None;
    let mut delta_direct_surface = None;
    if delta_manifest {
        let manifest = file.expect("delta_manifest implies file");
        let dataset = options
            .dataset
            .as_deref()
            .ok_or_else(|| "delta manifest query requires --dataset <dir>".to_string())?;
        if options.physical_sidecars.has_any() {
            return Err(
                "explicit physical sidecars are not supported for materialized delta snapshots; build snapshot-bound sidecars or omit the sidecar flags"
                    .into(),
            );
        }
        let snapshot = cove_datafusion::delta_snapshot::load_validated_delta_snapshot(
            manifest,
            dataset,
            options.delta_request,
        )?;
        if options.strict_performance
            && snapshot
                .plan
                .recommendations
                .iter()
                .any(|item| {
                    *item
                        == cove_core::artifact::covm::CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth
                })
        {
            return Err(
                "strict performance requested, but the delta chain exceeds the hard read-amplification policy"
                    .into(),
            );
        }
        if options.delta_plan_json {
            eprintln!(
                "{}",
                serde_json::to_string_pretty(
                    &cove_datafusion::delta_snapshot::delta_snapshot_plan_json(
                        Some(manifest),
                        &snapshot.plan,
                        &snapshot.extension,
                    )
                )
                .unwrap()
            );
        } else if options.delta_plan {
            print_query_delta_plan_text(manifest, &snapshot.plan);
        }
        delta_plan = Some(snapshot.plan.clone());
        match cove_datafusion::delta_snapshot::direct_object_surface_support(&snapshot) {
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::Supported => {
                delta_direct_surface = Some(
                    cove_datafusion::delta_snapshot::read_validated_delta_object_surface(
                        &snapshot,
                    )?,
                );
                bytes = snapshot.base.bytes.clone();
            }
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                ..
            } => {
                let materialized =
                    cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(
                        &snapshot,
                    )?;
                bytes = materialized.bytes;
            }
        }
        delta_snapshot = Some(snapshot);
    } else if options.delta_request != CovmDeltaPruneRequest::default()
        || options.delta_plan
        || options.delta_plan_json
    {
        return Err("delta snapshot options require a COVM delta manifest input".into());
    }
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
    let acceleration_bundle = if !options.no_auto_sidecars && !delta_manifest {
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
    execute_options.manifest_members = if delta_manifest {
        explicit_manifest_members_for(&options)?
    } else {
        manifest_members_for(file, &bytes, &options)?
    };
    let query = match &options.query_file {
        Some(query_file) => read_query_file(query_file)?,
        None => query.to_string(),
    };
    let query = prepare_query_text(&query, options.take, options.explain.as_deref())?;
    let use_direct_delta_surface = delta_direct_surface.is_some()
        && matches!(
            execute_options.execution_engine,
            ArtifactExecutionEngine::Materialized
        );
    let executed = if use_direct_delta_surface {
        let surface = delta_direct_surface
            .as_ref()
            .expect("checked direct delta surface");
        match execute_delta_object_surface_query(&bytes, surface, &query, &execute_options) {
            Ok(executed) => {
                if options.perf_report {
                    print_query_perf_report(acceleration_bundle.as_ref(), None);
                    if let Some(plan) = &delta_plan {
                        eprintln!(
                            "delta_chain_depth={} selected_delta_count={} skipped_delta_count={}",
                            plan.metrics.delta_chain_depth,
                            plan.metrics.selected_delta_count,
                            plan.metrics.skipped_delta_count
                        );
                        eprintln!("delta_execution=direct_object_surface");
                    }
                }
                executed
            }
            Err(direct_error) => {
                let direct_error = direct_error.to_string();
                let snapshot = delta_snapshot
                    .as_ref()
                    .expect("delta snapshot is loaded when direct surface exists");
                let materialized =
                    cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(
                        snapshot,
                    )?;
                bytes = materialized.bytes;
                execute_query_with_cli_fallback(
                    &bytes,
                    &query,
                    execute_options.clone(),
                    &options,
                    acceleration_bundle.as_ref(),
                    delta_plan.as_ref(),
                    Some(&direct_error),
                )?
            }
        }
    } else {
        if delta_direct_surface.is_some() {
            let snapshot = delta_snapshot
                .as_ref()
                .expect("delta snapshot is loaded when direct surface exists");
            let materialized =
                cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(snapshot)?;
            bytes = materialized.bytes;
        }
        execute_query_with_cli_fallback(
            &bytes,
            &query,
            execute_options.clone(),
            &options,
            acceleration_bundle.as_ref(),
            delta_plan.as_ref(),
            None,
        )?
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

fn execute_delta_object_surface_query(
    planning_bytes: &[u8],
    surface: &CoveObjectSurface,
    query: &str,
    options: &ExecuteArtifactOptions,
) -> Result<ExecutedQuery, coveql::BuildExecutionError> {
    parse_resolve_plan_and_execute_query_on_object_surface(
        planning_bytes,
        surface,
        query,
        options.parse_options.clone(),
        options.resolve_options.clone(),
        options.plan_options.clone(),
        options.execution_options.clone(),
        options.validation_options.clone(),
    )
}

fn execute_query_with_cli_fallback(
    bytes: &[u8],
    query: &str,
    execute_options: ExecuteArtifactOptions,
    options: &QueryCommandOptions,
    acceleration_bundle: Option<&CoveAccelerationBundle>,
    delta_plan: Option<&cove_datafusion::delta_snapshot::DeltaSnapshotPlan>,
    materialized_fallback_reason: Option<&str>,
) -> Result<ExecutedQuery, String> {
    match execute_query_from_artifact(bytes, query, execute_options.clone()) {
        Ok(executed) => {
            if options.perf_report {
                print_query_perf_report(acceleration_bundle, materialized_fallback_reason);
                if let Some(plan) = delta_plan {
                    eprintln!(
                        "delta_chain_depth={} selected_delta_count={} skipped_delta_count={}",
                        plan.metrics.delta_chain_depth,
                        plan.metrics.selected_delta_count,
                        plan.metrics.skipped_delta_count
                    );
                }
            }
            Ok(executed)
        }
        Err(error) if options.engine == QueryEngine::Auto && !options.strict_performance => {
            let mut fallback_options = execute_options;
            fallback_options.execution_engine = ArtifactExecutionEngine::Materialized;
            match execute_query_from_artifact(bytes, query, fallback_options) {
                Ok(executed) => {
                    if options.perf_report {
                        let formatted_error =
                            format_artifact_query_error(error, options.json_diagnostics);
                        let fallback_reason =
                            materialized_fallback_reason.unwrap_or(&formatted_error);
                        print_query_perf_report(acceleration_bundle, Some(fallback_reason));
                    }
                    Ok(executed)
                }
                Err(fallback_error) => Err(format_artifact_query_error(
                    fallback_error,
                    options.json_diagnostics,
                )),
            }
        }
        Err(error) => Err(format_artifact_query_error(error, options.json_diagnostics)),
    }
}

fn external_only_context_bytes() -> Vec<u8> {
    ScanProfileCoveWriter::new(TableCatalog {
        flags: 0,
        tables: Vec::new(),
    })
    .write()
    .expect("empty COVE-T context file is valid")
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
        cove_ai_artifact_bytes: read_optional_bytes(&paths.cove_ai_artifact)?,
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

fn explicit_manifest_members_for(
    options: &QueryCommandOptions,
) -> Result<Vec<QueryArtifactMember>, String> {
    options
        .members
        .iter()
        .map(|(source, path)| {
            Ok(QueryArtifactMember {
                source: source.clone(),
                bytes: fs::read(path)
                    .map_err(|error| format!("cannot read member {}: {error}", path.display()))?,
            })
        })
        .collect()
}

fn print_query_delta_plan_text(
    manifest: &Path,
    plan: &cove_datafusion::delta_snapshot::DeltaSnapshotPlan,
) {
    eprintln!("Delta snapshot plan: {}", manifest.display());
    eprintln!("  selected: {:?}", plan.decision.selected_chain_ordinals);
    if plan.decision.skipped.is_empty() {
        eprintln!("  skipped: none");
    } else {
        eprintln!("  skipped:");
        for skip in &plan.decision.skipped {
            eprintln!(
                "    - {} ({})",
                skip.chain_ordinal,
                cove_datafusion::delta_snapshot::prune_reason_name(skip.reason)
            );
        }
    }
    eprintln!("  chain_depth: {}", plan.metrics.delta_chain_depth);
    eprintln!(
        "  selected_delta_count: {}",
        plan.metrics.selected_delta_count
    );
    eprintln!(
        "  skipped_delta_count: {}",
        plan.metrics.skipped_delta_count
    );
    eprintln!(
        "  object_store_request_count: {}",
        plan.metrics.object_store_request_count
    );
    eprintln!("  bytes_returned: {}", plan.metrics.bytes_returned);
    if plan.recommendations.is_empty() {
        eprintln!("  recommendations: none");
    } else {
        eprintln!("  recommendations:");
        for item in &plan.recommendations {
            eprintln!(
                "    - {}",
                cove_datafusion::delta_snapshot::recommendation_name(*item)
            );
        }
    }
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
