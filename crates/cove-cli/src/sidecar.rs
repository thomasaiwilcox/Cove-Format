use std::{fs, path::PathBuf};

use cove_core::{
    artifact::covm::CovmDeltaPruneRequest,
    constants::DigestAlgorithm,
    durable,
    utility::{
        build_covm_artifact, build_covm_artifact_from_bytes, build_covx_artifact,
        build_covx_artifact_from_bytes, CovmInputArtifact,
    },
};

pub(crate) fn run_sidecar(mut args: Vec<String>) -> Result<(), String> {
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

fn inspect_index_sidecar(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
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

fn inspect_coverage_sidecar(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
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

fn inspect_layout_sidecar(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
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

fn inspect_cache_sidecar(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
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

fn inspect_runtime_sidecar(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
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
    if args.iter().any(|arg| arg == "--snapshot") {
        return build_covx_or_covm_snapshot_sidecar(args, covx);
    }
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

fn build_covx_or_covm_snapshot_sidecar(args: Vec<String>, covx: bool) -> Result<(), String> {
    let mut snapshot = None;
    let mut dataset = None;
    let mut output = None;
    let mut request = CovmDeltaPruneRequest::default();
    let mut positional = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--snapshot" => {
                snapshot = Some(PathBuf::from(next_value(&mut iter, "--snapshot")?));
            }
            "--dataset" => {
                dataset = Some(PathBuf::from(next_value(&mut iter, "--dataset")?));
            }
            "--out" | "--output" => {
                output = Some(PathBuf::from(next_value(&mut iter, "--out")?));
            }
            "--as-of-csn" => {
                request.as_of_csn = Some(parse_u64(
                    &next_value(&mut iter, "--as-of-csn")?,
                    "--as-of-csn",
                )?);
            }
            "--as-of-commit-us" => {
                request.as_of_commit_timestamp_us = Some(parse_i64(
                    &next_value(&mut iter, "--as-of-commit-us")?,
                    "--as-of-commit-us",
                )?);
            }
            "-h" | "--help" => {
                return Err(if covx {
                    "usage: cove sidecar build covx --snapshot <manifest.covm> --dataset <dir> --out <output.covx> [--as-of-csn n|--as-of-commit-us n]".into()
                } else {
                    "usage: cove sidecar build covm --snapshot <manifest.covm> --dataset <dir> --out <output.covm> [--as-of-csn n|--as-of-commit-us n]".into()
                })
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option: {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if output.is_none() && positional.len() == 1 {
        output = Some(positional.remove(0));
    }
    if !positional.is_empty() {
        return Err("snapshot sidecar build accepts only one output path".into());
    }
    let snapshot = snapshot.ok_or_else(|| "--snapshot <manifest.covm> is required".to_string())?;
    let dataset = dataset.ok_or_else(|| "--dataset <dir> is required".to_string())?;
    let output = output.ok_or_else(|| "--out <path> is required".to_string())?;
    let (_snapshot, materialized) =
        cove_datafusion::delta_snapshot::materialize_delta_snapshot(&snapshot, &dataset, request)?;
    let uri = snapshot_artifact_uri(&snapshot, request);
    let input = CovmInputArtifact {
        uri,
        bytes: &materialized.bytes,
    };
    let (bytes, report) = if covx {
        build_covx_artifact_from_bytes(&output, &[input]).map_err(|error| error.to_string())?
    } else {
        build_covm_artifact_from_bytes(&output, &[input]).map_err(|error| error.to_string())?
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
    use cove_index::build::{
        build_covi_from_cove_bytes, build_covi_from_cove_o_bytes, CoviBuildOptions,
        CoviObjectPropertyBuildOptions,
    };

    if args.iter().any(|arg| arg == "--snapshot") {
        return build_covi_snapshot_sidecar(args);
    }

    if args.len() == 1 {
        let output = PathBuf::from(&args[0]);
        let artifact = cove_index::CoviArtifactV2::new_empty([0u8; 16], [0u8; 16]);
        let bytes = artifact
            .serialize_empty()
            .map_err(|error| format!("failed to build empty COVE-I artifact: {error}"))?;
        durable::durable_replace(&output, &bytes)
            .map_err(|error| format!("cannot durably publish {}: {error}", output.display()))?;
        println!("wrote empty COVE-I artifact to {}", output.display());
        return Ok(());
    }

    let mut positionals = Vec::new();
    let mut options = CoviBuildOptions::default();
    let mut object_options = CoviObjectPropertyBuildOptions::default();
    let mut object_properties = false;
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
            "--object-properties" => object_properties = true,
            "--index-only-counts" => {
                options.include_index_only_counts = true;
                object_options.include_index_only_counts = true;
            }
            "--index-only-exists" => {
                options.include_index_only_exists = true;
                object_options.include_index_only_exists = true;
            }
            "--index-only-min-max" => {
                options.include_index_only_min_max = true;
                object_options.include_index_only_min_max = true;
            }
            "--index-only-distinct-count" => {
                options.include_index_only_distinct_count = true;
                object_options.include_index_only_distinct_count = true;
            }
            "--index-only-sum-avg" => {
                options.include_index_only_sum_avg = true;
                object_options.include_index_only_sum_avg = true;
            }
            "-h" | "--help" => {
                println!("usage: cove sidecar build covi <input.cove> <output.covi> [--table-id <id>] [--column-id <id> ... | --all-columns | --object-properties] [--index-only-counts] [--index-only-exists] [--index-only-min-max] [--index-only-distinct-count] [--index-only-sum-avg]");
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
    if object_properties
        && (options.all_columns || options.table_id.is_some() || !options.column_ids.is_empty())
    {
        return Err("--object-properties cannot be combined with table or column selection".into());
    }
    let input_path = positionals.remove(0);
    let output_path = PathBuf::from(positionals.remove(0));
    let input = fs::read(&input_path).map_err(|error| format!("{input_path}: {error}"))?;
    let bytes = if object_properties {
        build_covi_from_cove_o_bytes(&input, &object_options)
    } else {
        build_covi_from_cove_bytes(&input, &options)
    }
    .map_err(|error| format!("{input_path}: {error}"))?;
    durable::durable_replace(&output_path, &bytes)
        .map_err(|error| format!("cannot durably publish {}: {error}", output_path.display()))?;
    println!("wrote COVE-I artifact to {}", output_path.display());
    Ok(())
}

fn build_covi_snapshot_sidecar(args: Vec<String>) -> Result<(), String> {
    use cove_index::build::{
        build_covi_from_cove_bytes_with_delta_chain_binding,
        build_covi_from_cove_o_bytes_with_delta_chain_binding, CoviBuildOptions,
        CoviDeltaChainBinding, CoviObjectPropertyBuildOptions,
    };

    let mut snapshot = None;
    let mut dataset = None;
    let mut output = None;
    let mut request = CovmDeltaPruneRequest::default();
    let mut options = CoviBuildOptions::default();
    let mut object_options = CoviObjectPropertyBuildOptions::default();
    let mut object_properties = false;
    let mut positional = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--snapshot" => {
                snapshot = Some(PathBuf::from(next_value(&mut iter, "--snapshot")?));
            }
            "--dataset" => {
                dataset = Some(PathBuf::from(next_value(&mut iter, "--dataset")?));
            }
            "--out" | "--output" => {
                output = Some(PathBuf::from(next_value(&mut iter, "--out")?));
            }
            "--as-of-csn" => {
                request.as_of_csn = Some(parse_u64(
                    &next_value(&mut iter, "--as-of-csn")?,
                    "--as-of-csn",
                )?);
            }
            "--as-of-commit-us" => {
                request.as_of_commit_timestamp_us = Some(parse_i64(
                    &next_value(&mut iter, "--as-of-commit-us")?,
                    "--as-of-commit-us",
                )?);
            }
            "--table-id" => {
                let value = next_value(&mut iter, "--table-id")?;
                options.table_id = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --table-id value: {value}"))?,
                );
            }
            "--column-id" => {
                let value = next_value(&mut iter, "--column-id")?;
                options.column_ids.push(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --column-id value: {value}"))?,
                );
            }
            "--all-columns" => options.all_columns = true,
            "--object-properties" => object_properties = true,
            "--index-only-counts" => {
                options.include_index_only_counts = true;
                object_options.include_index_only_counts = true;
            }
            "--index-only-exists" => {
                options.include_index_only_exists = true;
                object_options.include_index_only_exists = true;
            }
            "--index-only-min-max" => {
                options.include_index_only_min_max = true;
                object_options.include_index_only_min_max = true;
            }
            "--index-only-distinct-count" => {
                options.include_index_only_distinct_count = true;
                object_options.include_index_only_distinct_count = true;
            }
            "--index-only-sum-avg" => {
                options.include_index_only_sum_avg = true;
                object_options.include_index_only_sum_avg = true;
            }
            "-h" | "--help" => {
                println!("usage: cove sidecar build covi --snapshot <manifest.covm> --dataset <dir> --out <output.covi> [--as-of-csn n|--as-of-commit-us n] [--table-id <id>] [--column-id <id> ... | --all-columns | --object-properties] [--index-only-counts] [--index-only-exists] [--index-only-min-max] [--index-only-distinct-count] [--index-only-sum-avg]");
                return Ok(());
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option: {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if output.is_none() && positional.len() == 1 {
        output = Some(positional.remove(0));
    }
    if !positional.is_empty() {
        return Err("snapshot COVE-I build accepts only one output path".into());
    }
    if options.all_columns && !options.column_ids.is_empty() {
        return Err("--all-columns cannot be combined with --column-id".into());
    }
    if object_properties
        && (options.all_columns || options.table_id.is_some() || !options.column_ids.is_empty())
    {
        return Err("--object-properties cannot be combined with table or column selection".into());
    }
    let snapshot = snapshot.ok_or_else(|| "--snapshot <manifest.covm> is required".to_string())?;
    let dataset = dataset.ok_or_else(|| "--dataset <dir> is required".to_string())?;
    let output = output.ok_or_else(|| "--out <output.covi> is required".to_string())?;
    let (validated_snapshot, materialized) =
        cove_datafusion::delta_snapshot::materialize_delta_snapshot(&snapshot, &dataset, request)?;
    let algorithm = DigestAlgorithm::from_u16(validated_snapshot.extension.chain_digest_algorithm)
        .filter(|algorithm| *algorithm != DigestAlgorithm::None)
        .ok_or_else(|| "delta chain manifest has no usable chain digest".to_string())?;
    let delta_chain_binding = CoviDeltaChainBinding {
        algorithm,
        digest: validated_snapshot.extension.chain_digest.clone(),
    };
    let bytes = if object_properties {
        build_covi_from_cove_o_bytes_with_delta_chain_binding(
            &materialized.bytes,
            &object_options,
            Some(&delta_chain_binding),
        )
    } else {
        build_covi_from_cove_bytes_with_delta_chain_binding(
            &materialized.bytes,
            &options,
            Some(&delta_chain_binding),
        )
    }
    .map_err(|error| format!("{}: {error}", snapshot.display()))?;
    durable::durable_replace(&output, &bytes)
        .map_err(|error| format!("cannot durably publish {}: {error}", output.display()))?;
    println!("wrote COVE-I artifact to {}", output.display());
    Ok(())
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

fn snapshot_artifact_uri(snapshot: &std::path::Path, request: CovmDeltaPruneRequest) -> String {
    let stem = snapshot
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("snapshot");
    if let Some(csn) = request.as_of_csn {
        format!("{stem}.as_of_csn_{csn}.cove")
    } else if let Some(timestamp_us) = request.as_of_commit_timestamp_us {
        format!("{stem}.as_of_commit_us_{timestamp_us}.cove")
    } else {
        format!("{stem}.snapshot.cove")
    }
}
