fn run_map(args: Vec<String>) -> Result<(), String> {
    if args.first().is_some_and(|arg| arg == "delta") {
        return run_map_delta(args.into_iter().skip(1).collect());
    }
    cove_map::run_cli(args).map_err(|error| error.to_string())
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
        cove_datafusion::delta_snapshot::materialize_delta_snapshot(&manifest, &dataset, request)
            .map_err(|error| error.to_string())?;
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
    )
    .map_err(|error| error.to_string())?;
    if json {
        print_json_pretty(&result.manifest)?;
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
    )
    .map_err(|error| error.to_string())?;
    let parent_surface =
        cove_datafusion::delta_snapshot::read_validated_delta_object_surface(&snapshot)
            .map_err(|error| error.to_string())?;
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
    )
    .map_err(|error| error.to_string())?;
    if json {
        print_json_pretty(&result.report)?;
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

