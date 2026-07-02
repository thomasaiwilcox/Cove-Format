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
    ShowcaseAiTraining {
        out_dir: PathBuf,
        profile: Customer360Profile,
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
    Vector {
        args: Vec<String>,
    },
    Ai {
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
        "vec" => Ok(parse_passthrough_or_help(args, HelpTopic::Vec, |args| {
            Command::Vector { args }
        })),
        "ai" => Ok(parse_passthrough_or_help(args, HelpTopic::Ai, |args| {
            Command::Ai { args }
        })),
        "train" => Ok(parse_passthrough_or_help(args, HelpTopic::Train, |args| {
            Command::Train { args }
        })),
        "dump" => Ok(Command::Dump { args }),
        "map" => Ok(parse_passthrough_or_help(args, HelpTopic::Map, |args| {
            Command::Map { args }
        })),
        "export" => parse_export(args),
        "perf" => parse_perf(args),
        "sidecar" => Ok(parse_passthrough_or_help(
            args,
            HelpTopic::Sidecar,
            |args| Command::Sidecar { args },
        )),
        "delta" => Ok(parse_passthrough_or_help(args, HelpTopic::Delta, |args| {
            Command::Delta { args }
        })),
        "digest" => parse_digest(args),
        "profile" => Ok(Command::Profile { args }),
        "canonicalise" | "canonicalize" => Ok(Command::Canonicalise { args }),
        other => Err(format!(
            "unknown command '{other}'\n\n{}",
            usage(HelpTopic::Global)
        )),
    }
}

fn parse_passthrough_or_help(
    args: Vec<String>,
    help_topic: HelpTopic,
    command: impl FnOnce(Vec<String>) -> Command,
) -> Command {
    if args.first().is_some_and(|arg| is_help_flag(arg)) {
        Command::Help(help_topic)
    } else {
        command(args)
    }
}

fn is_help_flag(arg: &str) -> bool {
    arg == "-h" || arg == "--help"
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
    if name == "ai-training" {
        return parse_showcase_ai_training(args);
    }
    Err(format!(
        "unknown showcase '{name}'; expected customer360, proof-suite, or ai-training\n\n{}",
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
                profile = Customer360Profile::parse(&value).map_err(|error| error.to_string())?;
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
                profile = Customer360Profile::parse(&value).map_err(|error| error.to_string())?;
            }
            "--scenario" => {
                let value = iter.next().ok_or_else(|| {
                    "--scenario requires customer360, claims, catalog, or all".to_string()
                })?;
                scenario = ProofSuiteScenario::parse(&value).map_err(|error| error.to_string())?;
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

fn parse_showcase_ai_training(args: Vec<String>) -> Result<Command, String> {
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
                profile = Customer360Profile::parse(&value).map_err(|error| error.to_string())?;
            }
            "--force" => force = true,
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help(HelpTopic::Showcase)),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown ai-training showcase option '{arg}'"))
            }
            _ => return Err("showcase ai-training does not accept positional arguments".into()),
        }
    }
    let out_dir = out_dir.ok_or_else(|| {
        format!(
            "--out is required for showcase ai-training\n\n{}",
            usage(HelpTopic::Showcase)
        )
    })?;
    Ok(Command::ShowcaseAiTraining {
        out_dir,
        profile,
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
    value.parse::<ExplainMode>().is_ok()
}

fn explain_policy_for_cli(mode: &str) -> ExplainDisclosurePolicy {
    explain_policy_for_mode(mode.parse::<ExplainMode>().unwrap_or_default())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn pass_through_commands_route_first_arg_help_to_topic() {
        for (command, topic) in [
            ("vec", HelpTopic::Vec),
            ("ai", HelpTopic::Ai),
            ("train", HelpTopic::Train),
            ("map", HelpTopic::Map),
            ("sidecar", HelpTopic::Sidecar),
            ("delta", HelpTopic::Delta),
        ] {
            assert_eq!(
                parse_args(args(&[command, "--help"])).unwrap(),
                Command::Help(topic)
            );
            assert_eq!(
                parse_args(args(&[command, "-h"])).unwrap(),
                Command::Help(topic)
            );
        }
    }

    #[test]
    fn pass_through_commands_preserve_forwarded_arguments() {
        assert_eq!(
            parse_args(args(&["vec", "build", "--metric", "cosine"])).unwrap(),
            Command::Vector {
                args: args(&["build", "--metric", "cosine"])
            }
        );
        assert_eq!(
            parse_args(args(&["map", "project", "--format", "json"])).unwrap(),
            Command::Map {
                args: args(&["project", "--format", "json"])
            }
        );
    }

    #[test]
    fn canonicalise_aliases_preserve_forwarded_arguments() {
        let forwarded = args(&["digest", "--json"]);
        assert_eq!(
            parse_args(args(&["canonicalise", "digest", "--json"])).unwrap(),
            Command::Canonicalise {
                args: forwarded.clone()
            }
        );
        assert_eq!(
            parse_args(args(&["canonicalize", "digest", "--json"])).unwrap(),
            Command::Canonicalise { args: forwarded }
        );
    }

    #[test]
    fn run_cli_reports_typed_usage_error_with_stable_text() {
        let error = run_cli(args(&["unknown-command"])).unwrap_err();
        assert!(matches!(error, CliError::Usage(_)));
        let message = error.to_string();
        assert!(message.starts_with("unknown command 'unknown-command'\n\nUsage:"));
        assert!(message.contains("cove examples [--json]"));
    }
}
