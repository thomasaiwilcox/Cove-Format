fn run_convert(format: ConvertFormat, args: Vec<String>) -> Result<(), String> {
    let result = match format {
        ConvertFormat::Parquet => cove_convert_parquet::commands::run_parquet(args),
        ConvertFormat::Arrow => cove_convert_parquet::commands::run_arrow(args),
        ConvertFormat::Orc => cove_convert_parquet::commands::run_orc(args),
        ConvertFormat::Csv => cove_convert_parquet::commands::run_csv(args),
        ConvertFormat::Report => cove_convert_parquet::commands::run_report(args),
    };
    result.map_err(|error| error.to_string())
}

fn run_validate(args: Vec<String>) -> Result<(), String> {
    if cove_validate::run_cli(args).map_err(|error| error.to_string())? {
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
    let mut index_kind = "exact".to_string();
    let mut metric = "dot".to_string();
    let mut quantization = "none".to_string();
    let mut index_parameters: Vec<(String, String)> = Vec::new();
    let mut deterministic_seed = 17u64;
    let mut integrity_report: Option<PathBuf> = None;

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
            "--index" => {
                index_kind = iter.next().ok_or_else(|| {
                    "--index requires exact|hnsw|ivf-flat|ivf-pq|diskann|vamana".to_string()
                })?;
                if !matches!(
                    index_kind.as_str(),
                    "exact"
                        | "exact-flat"
                        | "exact_flat"
                        | "hnsw"
                        | "ivf-flat"
                        | "ivf_pq"
                        | "ivf-pq"
                        | "diskann"
                        | "vamana"
                ) {
                    return Err(
                        "--index must be exact, hnsw, ivf-flat, ivf-pq, diskann, or vamana".into(),
                    );
                }
            }
            "--metric" => {
                metric = iter
                    .next()
                    .ok_or_else(|| "--metric requires cosine|dot|l2|l1".to_string())?;
                if !matches!(metric.as_str(), "cosine" | "dot" | "l2" | "l1") {
                    return Err("--metric must be cosine, dot, l2, or l1".into());
                }
            }
            "--quantization" => {
                quantization = iter
                    .next()
                    .ok_or_else(|| "--quantization requires none|int8|uint8|pq".to_string())?;
                if !matches!(quantization.as_str(), "none" | "int8" | "uint8" | "pq") {
                    return Err("--quantization must be none, int8, uint8, or pq".into());
                }
            }
            "--seed" => {
                let value = parse_u64(iter.next().as_deref(), "--seed")?;
                deterministic_seed = value;
                index_parameters.push(("seed".into(), value.to_string()));
            }
            "--integrity-report" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                integrity_report = Some(PathBuf::from(value.clone()));
                index_parameters.push(("integrity_report".into(), value));
            }
            "--ef" | "--ef-search" | "--ef-construction" | "--probes" | "--lists"
            | "--shard-count" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                index_parameters.push((arg.trim_start_matches('-').to_string(), value));
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
        deterministic_vector_payload(&file_codes, dimension_count, deterministic_seed)?
    };
    let created_at_us = created_at_us.unwrap_or_else(current_time_us);
    let bytes = write_covev_filecode_vectors_with_options(
        &CoveVecFileCodeVectorBuild {
            artifact_id,
            created_at_us,
            dimension_count,
            file_codes: file_codes.clone(),
            vector_payload,
        },
        CoveVecFileCodeVectorBuildOptions {
            index_kind: Some(cove_vec_index_kind(index_kind.as_str())?),
            metric: cove_vec_metric(metric.as_str())?,
            quantization_kind: cove_vec_quantization_kind(quantization.as_str())?,
        },
    )
    .map_err(|error| format!("cannot build {}: {error}", out.display()))?;
    fs::write(&out, &bytes).map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    let parsed = CoveAiFile::parse(&bytes)
        .map_err(|error| format!("built {} but validation failed: {error}", out.display()))?;
    if let Some(report_path) = integrity_report {
        let report = vec_build_integrity_report(
            &out,
            &bytes,
            &parsed,
            index_kind.as_str(),
            metric.as_str(),
            quantization.as_str(),
            &index_parameters,
        )?;
        fs::write(&report_path, report)
            .map_err(|error| format!("cannot write {}: {error}", report_path.display()))?;
    }
    println!(
        "Wrote {}: {} FileCode vectors, dimension {}, payload_access={:?}, index={}, metric={}, quantization={}, index_parameters={}",
        out.display(),
        parsed.descriptor_tables.filecode_vector_bindings.len(),
        parsed
            .descriptor_tables
            .vector_spaces
            .first()
            .map(|space| space.dimension_count)
            .unwrap_or(dimension_count),
        parsed.payload_access,
        index_kind,
        metric,
        quantization,
        index_parameters.len()
    );
    Ok(())
}


fn run_inspect_detailed(args: Vec<String>) -> Result<(), String> {
    if cove_inspect::run_cli(args).map_err(|error| error.to_string())? {
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
        ExportFormat::Arrow => arrow_export::run(args),
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
        )
        .map_err(|error| error.to_string())?;
        let plan_json = cove_datafusion::delta_snapshot::delta_snapshot_plan_json(
            Some(&command.input),
            &snapshot.plan,
            &snapshot.extension,
        );
        if command.delta_plan_json {
            eprint_json_pretty(&plan_json)?;
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
                    )
                    .map_err(|error| error.to_string())?;
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
        ArrowQueryExportOutputFormat::Ipc => arrow_export::write_ipc(&schema, &batches)?,
        ArrowQueryExportOutputFormat::Json => arrow_export::write_json(&batches)?,
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

fn run_perf(command: PerfCommand, args: Vec<String>) -> Result<(), String> {
    match command {
        PerfCommand::ExplainPruning => perf::run_explain_pruning(args),
        PerfCommand::PlanCost => perf::run_plan_cost(args),
    }
}

fn run_profile(args: Vec<String>) -> Result<(), String> {
    if cove_core::profile_cli::run(args).map_err(|error| error.to_string())? {
        Ok(())
    } else {
        Err("profile command failed".into())
    }
}

fn run_canonicalise(args: Vec<String>) -> Result<(), String> {
    if cove_core::canonicalise_cli::run(args).map_err(|error| error.to_string())? {
        Ok(())
    } else {
        Err("canonicalise command failed".into())
    }
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
            "Generate the COVE-AI training archive showcase",
            "cove showcase ai-training --profile quick --out target/cove-ai-training --force",
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
        print_json_pretty(&value)?;
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
        print_json_pretty(&report)?;
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
