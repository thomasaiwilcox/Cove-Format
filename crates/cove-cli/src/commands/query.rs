fn run_query(file: Option<&Path>, query: &str, options: QueryCommandOptions) -> Result<(), String> {
    let mut bytes = match file {
        Some(file) => {
            fs::read(file).map_err(|error| format!("cannot read {}: {error}", file.display()))?
        }
        None => external_only_context_bytes()?,
    };
    let delta_manifest = file.is_some()
        && cove_datafusion::delta_snapshot::delta_chain_required(&bytes).unwrap_or(false);
    let mut delta_plan = None;
    let mut delta_snapshot = None;
    let mut delta_direct_surface = None;
    if delta_manifest {
        let Some(manifest) = file else {
            return Err("internal error: delta manifest selected without input file".into());
        };
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
        )
        .map_err(|error| error.to_string())?;
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
            let plan_json = cove_datafusion::delta_snapshot::delta_snapshot_plan_json(
                Some(manifest),
                &snapshot.plan,
                &snapshot.extension,
            );
            eprint_json_pretty(&plan_json)?;
        } else if options.delta_plan {
            print_query_delta_plan_text(manifest, &snapshot.plan);
        }
        delta_plan = Some(snapshot.plan.clone());
        match cove_datafusion::delta_snapshot::direct_object_surface_support(&snapshot) {
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::Supported => {
                delta_direct_surface = Some(
                    cove_datafusion::delta_snapshot::read_validated_delta_object_surface(
                        &snapshot,
                    )
                    .map_err(|error| error.to_string())?,
                );
                bytes = snapshot.base.bytes.clone();
            }
            cove_datafusion::delta_snapshot::DirectDeltaObjectSurfaceSupport::RequiresMaterializedPlannerMetadata {
                ..
            } => {
                let materialized =
                    cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(
                        &snapshot,
                    )
                    .map_err(|error| error.to_string())?;
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
    let mut physical_sidecars = options.physical_sidecars.clone();
    if !options.no_auto_sidecars && physical_sidecars.cove_ai_artifact.is_none() {
        if let Some(input) = file {
            physical_sidecars.cove_ai_artifact = discover_query_ai_sidecar(
                input,
                options.dataset.as_deref(),
                query_selects_ai_operation(query, options.explain.as_deref()),
            )?;
        }
    }
    configure_execution_engine(&mut execute_options, &options, &physical_sidecars)?;
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
            && (bundle.has_usable_sidecars() || physical_sidecars.has_any())
        {
            execute_options = apply_acceleration_bundle(bundle, execute_options);
        }
        if options.strict_performance
            && options.engine != QueryEngine::Materialized
            && !bundle.has_usable_sidecars()
            && !physical_sidecars.has_any()
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
        let Some(surface) = delta_direct_surface.as_ref() else {
            return Err("internal error: direct delta surface selected but unavailable".into());
        };
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
                let Some(snapshot) = delta_snapshot.as_ref() else {
                    return Err(format!(
                        "direct delta execution failed ({direct_error}) and validated delta snapshot is unavailable"
                    ));
                };
                let materialized =
                    cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(snapshot)
                        .map_err(|error| error.to_string())?;
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
            let Some(snapshot) = delta_snapshot.as_ref() else {
                return Err(
                    "internal error: direct delta surface selected without validated snapshot"
                        .into(),
                );
            };
            let materialized =
                cove_datafusion::delta_snapshot::materialize_validated_delta_snapshot(snapshot)
                    .map_err(|error| error.to_string())?;
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

fn discover_query_ai_sidecar(
    input: &Path,
    dataset: Option<&Path>,
    strict_stale: bool,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = discover_covm_referenced_ai_sidecar(input, dataset, strict_stale)? {
        return Ok(Some(path));
    }
    let mut candidates = ai_sidecar_candidates(input);
    if let Some(dataset) = dataset {
        let file_name = input.file_name().and_then(|name| name.to_str());
        if let Some(file_name) = file_name {
            candidates.extend(ai_sidecar_candidates(&dataset.join(file_name)));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) || !candidate.is_file() {
            continue;
        }
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if CoveAiFile::parse(&bytes).is_ok() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn discover_covm_referenced_ai_sidecar(
    input: &Path,
    dataset: Option<&Path>,
    strict_stale: bool,
) -> Result<Option<PathBuf>, String> {
    let manifest_bytes = match fs::read(input) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let manifest = match CovmFile::parse(&manifest_bytes)
        .or_else(|_| CovmFile::parse_delta_aware(&manifest_bytes))
    {
        Ok(manifest) => manifest,
        Err(_) => return Ok(None),
    };
    let extension = match CovmAiSidecarExtensionV1::find_in_covm_bytes(&manifest_bytes) {
        Ok(Some(extension)) => extension,
        Ok(None) => return Ok(None),
        Err(error) if strict_stale => {
            return Err(format!(
                "COVM AI sidecar reference extension is invalid: {error}"
            ));
        }
        Err(_) => return Ok(None),
    };
    if let Err(error) = extension.validate_against_manifest(&manifest) {
        if strict_stale {
            return Err(format!(
                "COVM AI sidecar reference does not match manifest members: {error}"
            ));
        }
        return Ok(None);
    }
    let mut last_error = None;
    for reference in &extension.refs {
        let path = resolve_covm_ai_sidecar_path(input, dataset, &reference.uri);
        let sidecar_bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                last_error = Some(format!("cannot read {}: {error}", path.display()));
                continue;
            }
        };
        match reference.validate_sidecar_bytes(&sidecar_bytes) {
            Ok(_) => return Ok(Some(path)),
            Err(error) => {
                last_error = Some(format!("{} is stale or invalid: {error}", path.display()));
            }
        }
    }
    if strict_stale && !extension.refs.is_empty() {
        return Err(format!(
            "no digest-valid COVM AI sidecar reference is available{}",
            last_error
                .as_deref()
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        ));
    }
    Ok(None)
}

fn resolve_covm_ai_sidecar_path(input: &Path, dataset: Option<&Path>, uri: &str) -> PathBuf {
    let raw = PathBuf::from(uri);
    if raw.is_absolute() {
        return raw;
    }
    if let Some(dataset) = dataset {
        let candidate = dataset.join(&raw);
        if candidate.is_file() {
            return candidate;
        }
    }
    input
        .parent()
        .map(|parent| parent.join(&raw))
        .unwrap_or(raw)
}

fn ai_sidecar_candidates(input: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(input.with_extension("covev"));
    candidates.push(input.with_extension("coveai"));
    if let (Some(parent), Some(stem)) = (input.parent(), input.file_stem().and_then(|s| s.to_str()))
    {
        candidates.push(parent.join(format!("{stem}-ai.covev")));
        candidates.push(parent.join(format!("{stem}-ai.coveai")));
        candidates.push(parent.join(format!("{stem}.ai.covev")));
        candidates.push(parent.join(format!("{stem}.ai.coveai")));
    }
    candidates
}

fn query_selects_ai_operation(query: &str, explain: Option<&str>) -> bool {
    if matches!(explain, Some("ai")) {
        return true;
    }
    const AI_METHODS: &[&str] = &[
        ".embedding(",
        ".similar(",
        ".chunks(",
        ".tokens(",
        ".context(",
        ".asPromptContext(",
        ".trainingSamples(",
        ".split(",
        ".pack(",
        ".multimodal(",
        ".hybrid(",
        ".rerank(",
        ".generatorAudit(",
    ];
    AI_METHODS.iter().any(|method| query.contains(method))
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

fn external_only_context_bytes() -> Result<Vec<u8>, String> {
    ScanProfileCoveWriter::new(TableCatalog {
        flags: 0,
        tables: Vec::new(),
    })
    .write()
    .map_err(|error| format!("cannot build empty COVE-T context file: {error}"))
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
    physical_sidecars: &QueryPhysicalSidecarPaths,
) -> Result<(), String> {
    let physical_requested = matches!(
        options.engine,
        QueryEngine::Physical | QueryEngine::Compare | QueryEngine::Kernel
    ) || physical_sidecars.has_any()
        || options.allow_index_only
        || options.allow_zero_copy;
    if !physical_requested {
        return Ok(());
    }
    let physical_options = PhysicalPlanOptions {
        allow_index_only_answers: options.allow_index_only,
        allow_zero_copy_output: options.allow_zero_copy,
        sidecars: physical_sidecars_from_paths(physical_sidecars)?,
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
    let explain = explain
        .map(str::parse::<ExplainMode>)
        .transpose()
        .map_err(|error| error.to_string())?;
    cove::prepare_query_text(query, PreparedQueryTextOptions { take, explain }).map_err(|error| {
        match error {
            QueryTextError::NonPositiveTake => "--take requires a positive integer".into(),
        }
    })
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

