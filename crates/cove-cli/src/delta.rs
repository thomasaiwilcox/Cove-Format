use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use cove_core::{
    artifact::{
        covedelta::{
            CoveDeltaFile, CoveDeltaFooterV1, CoveDeltaHeaderV1, CoveDeltaPostscriptV1,
            CoveDeltaSection, CoveDeltaSectionDirectoryEntryV1, CoveDeltaSectionKind,
            DeltaParentRefV1, DELTA_FEATURE_ASSOCIATION_TOMBSTONES,
            DELTA_FEATURE_CHECKPOINT_BASELINES, DELTA_FEATURE_CONTINUATION_ANCHORS,
            DELTA_FEATURE_COVERAGE_PATCH, DELTA_FEATURE_EXACT_TOMBSTONE_SET,
            DELTA_FEATURE_EXACT_TOUCHED_SET, DELTA_FEATURE_HISTORICAL_COMMIT_INSERT,
            DELTA_FEATURE_INDEX_HINTS, DELTA_FEATURE_INLINE_DICTIONARY,
            DELTA_FEATURE_MAP_EVIDENCE_PATCH, DELTA_FEATURE_OBJECT_TOMBSTONES,
            DELTA_FEATURE_PARENT_DICTIONARY_ALIASES, DELTA_FEATURE_PROJECTION_PATCH,
            DELTA_FEATURE_PROPERTY_TOMBSTONES, DELTA_FEATURE_SPARSE_PATCH_ROWS,
            DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT, DELTA_PARENT_REF_LINEAGE_PARENT,
            DELTA_REF_NONE,
        },
        covm::{
            validate_selected_delta_chain_with_base, CovmDeltaArtifactRefV1,
            CovmDeltaChainExtensionV1, CovmDeltaChainSummaryV1, CovmDeltaPruneReason,
            CovmDeltaPruneRequest, CovmDeltaReadAmplificationRecommendation, CovmFile,
            CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1, DeltaChainSummaryEntryV1,
            COVM_DELTA_ARTIFACT_REF_LEN, COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN,
            COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1, COVM_DELTA_CHAIN_SUMMARY_KIND_NONE,
            COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED, COVM_POSTSCRIPT_LEN,
            COVM_POSTSCRIPT_TAIL_SIZE, DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE,
            DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_EXACT, DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT,
            DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT,
        },
    },
    checksum,
    constants::{DigestAlgorithm, MAGIC_COVEDELTA},
    digest::compute_digest,
    durable,
    profile::cove_o::{
        read_object_surface_from_base_and_delta_files_with_parent_identity,
        read_object_surface_from_bytes_with_options, reconstruct_object_states,
        CoveObjectDeltaParentIdentity, CoveObjectReadOptions, CoveObjectReconstructionOptions,
        CoveObjectState, ObjectTypeEntryV1,
    },
    reader::{validate_bytes_with_options, ValidationOptions},
    utility::{build_covm_artifact_from_bytes, hex_encode, CovmInputArtifact},
};
use serde_json::{json, Value};

const USAGE: &str = r#"Usage:
  cove delta inspect <delta.covedelta> [--json]
  cove delta validate <delta.covedelta> [--object-delta] [--json]
  cove delta dump <delta.covedelta> (--section <id|kind> | --parent-refs | --summary) [--max-bytes n]
  cove delta chain inspect <manifest.covm> [--extension <file>] [--json]
  cove delta chain validate <manifest.covm> --dataset <dir> [--extension <file>] [--summary <file>] [--json]
  cove delta chain plan <manifest.covm> --dataset <dir> [--extension <file>] [--summary <file>] [--as-of-csn n] [--as-of-commit-us n] [--source-publish-range start:end] [--json]
  cove delta chain graph <manifest.covm> --dataset <dir> [--extension <file>] [--format text|json|dot]
  cove delta chain extend --manifest <manifest.covm> --delta <delta.covedelta> --out <manifest.covm> [--summary-out <file>] [--force] [--json]
  cove delta reconstruct <manifest.covm> --dataset <dir> --out <snapshot.cove> [--json]
  cove delta compact <manifest.covm> --dataset <dir> --out <snapshot.cove> [--publish-covm <manifest.covm>] [--json]
  cove delta checkpoint <manifest.covm> --dataset <dir> --out <checkpoint.covedelta> [--summary-out <file>] [--json]
  cove delta publish --base <base.cove> --delta <delta.covedelta>... --out <manifest.covm> [--summary <file>|--summary-out <file>] [--json]
  cove delta publish-atomic --delta <delta.covedelta> --manifest <manifest.covm> [--json]"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeltaCommand {
    Inspect {
        path: PathBuf,
        json: bool,
    },
    Validate {
        path: PathBuf,
        object_delta: bool,
        json: bool,
    },
    Dump {
        path: PathBuf,
        selector: DumpSelector,
        max_bytes: usize,
    },
    Chain(ChainCommand),
    Reconstruct(ReconstructCommand),
    Compact(CompactCommand),
    Checkpoint(CheckpointCommand),
    Publish(PublishCommand),
    PublishAtomic {
        delta: PathBuf,
        manifest: PathBuf,
        json: bool,
    },
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChainCommand {
    Inspect(ChainInspectCommand),
    Validate(ChainValidateCommand),
    Plan(ChainPlanCommand),
    Graph(ChainGraphCommand),
    Extend(ChainExtendCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainInspectCommand {
    manifest: PathBuf,
    extension: Option<PathBuf>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainValidateCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    extension: Option<PathBuf>,
    summary: Option<PathBuf>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainPlanCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    extension: Option<PathBuf>,
    summary: Option<PathBuf>,
    request: CovmDeltaPruneRequest,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainGraphCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    extension: Option<PathBuf>,
    format: GraphFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainExtendCommand {
    manifest: PathBuf,
    delta: PathBuf,
    out: PathBuf,
    summary_out: Option<PathBuf>,
    force: bool,
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphFormat {
    Text,
    Json,
    Dot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishCommand {
    base: PathBuf,
    deltas: Vec<PathBuf>,
    out: PathBuf,
    summary: Option<PathBuf>,
    summary_out: Option<PathBuf>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconstructCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    out: PathBuf,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    out: PathBuf,
    publish_covm: Option<PathBuf>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckpointCommand {
    manifest: PathBuf,
    dataset: PathBuf,
    out: PathBuf,
    summary_out: Option<PathBuf>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DumpSelector {
    ParentRefs,
    Section(String),
    Summary,
}

#[derive(Debug, Clone)]
struct ManifestDeltaContext {
    manifest: CovmFile,
    extension: Option<CovmDeltaChainExtensionV1>,
    inline_summary_bytes: Option<Vec<u8>>,
    extension_source: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtifactBytes {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PublishedDelta {
    path: PathBuf,
    file: CoveDeltaFile,
    reference: CovmDeltaArtifactRefV1,
}

#[derive(Debug, Clone)]
struct ValidatedDeltaSnapshot {
    extension: CovmDeltaChainExtensionV1,
    base: ArtifactBytes,
    deltas: Vec<ArtifactBytes>,
    parsed_deltas: Vec<CoveDeltaFile>,
    summary_validated: bool,
}

#[derive(Debug, Clone)]
struct MaterializedSnapshot {
    bytes: Vec<u8>,
    state_count: Option<usize>,
    passthrough_base: bool,
}

#[derive(Debug, Clone)]
struct ReconstructedObjectSnapshot {
    object_types: Vec<ObjectTypeEntryV1>,
    states: Vec<CoveObjectState>,
}

pub(crate) fn usage() -> &'static str {
    USAGE
}

pub(crate) fn run_delta(args: Vec<String>) -> Result<(), String> {
    match parse_delta(args)? {
        DeltaCommand::Help => {
            println!("{USAGE}");
            Ok(())
        }
        DeltaCommand::Inspect { path, json } => run_inspect(&path, json),
        DeltaCommand::Validate {
            path,
            object_delta,
            json,
        } => run_validate(&path, object_delta, json),
        DeltaCommand::Dump {
            path,
            selector,
            max_bytes,
        } => run_dump(&path, selector, max_bytes),
        DeltaCommand::Chain(command) => run_chain(command),
        DeltaCommand::Reconstruct(command) => run_reconstruct(command),
        DeltaCommand::Compact(command) => run_compact(command),
        DeltaCommand::Checkpoint(command) => run_checkpoint(command),
        DeltaCommand::Publish(command) => run_publish(command),
        DeltaCommand::PublishAtomic {
            delta,
            manifest,
            json,
        } => run_publish_atomic(&delta, &manifest, json),
    }
}

pub(crate) fn is_covedelta_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[bytes.len() - 4..] == MAGIC_COVEDELTA
}

pub(crate) fn inspect_covedelta_for_beginner(path: &Path, json_out: bool) -> Result<(), String> {
    run_inspect(path, json_out)
}

fn parse_delta(mut args: Vec<String>) -> Result<DeltaCommand, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Ok(DeltaCommand::Help);
    }
    let command = args.remove(0);
    match command.as_str() {
        "inspect" => parse_inspect(args),
        "validate" => parse_validate(args),
        "dump" => parse_dump(args),
        "chain" => parse_chain(args).map(DeltaCommand::Chain),
        "reconstruct" => parse_reconstruct(args).map(DeltaCommand::Reconstruct),
        "compact" => parse_compact(args).map(DeltaCommand::Compact),
        "checkpoint" => parse_checkpoint(args).map(DeltaCommand::Checkpoint),
        "publish" => parse_publish(args).map(DeltaCommand::Publish),
        "publish-atomic" => parse_publish_atomic(args),
        other => Err(format!("unknown delta command '{other}'\n\n{USAGE}")),
    }
}

fn parse_inspect(args: Vec<String>) -> Result<DeltaCommand, String> {
    let mut json = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(DeltaCommand::Help),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta inspect option '{arg}'"))
            }
            value => {
                if path.replace(PathBuf::from(value)).is_some() {
                    return Err("delta inspect accepts one .covedelta path".into());
                }
            }
        }
    }
    Ok(DeltaCommand::Inspect {
        path: path
            .ok_or_else(|| "usage: cove delta inspect <delta.covedelta> [--json]".to_string())?,
        json,
    })
}

fn parse_validate(args: Vec<String>) -> Result<DeltaCommand, String> {
    let mut json = false;
    let mut object_delta = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--object-delta" => object_delta = true,
            "-h" | "--help" => return Ok(DeltaCommand::Help),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta validate option '{arg}'"))
            }
            value => {
                if path.replace(PathBuf::from(value)).is_some() {
                    return Err("delta validate accepts one .covedelta path".into());
                }
            }
        }
    }
    Ok(DeltaCommand::Validate {
        path: path.ok_or_else(|| {
            "usage: cove delta validate <delta.covedelta> [--object-delta] [--json]".to_string()
        })?,
        object_delta,
        json,
    })
}

fn parse_dump(mut args: Vec<String>) -> Result<DeltaCommand, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Ok(DeltaCommand::Help);
    }
    let path = PathBuf::from(args.remove(0));
    let mut selector = None;
    let mut max_bytes = 256usize;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--parent-refs" => set_dump_selector(&mut selector, DumpSelector::ParentRefs)?,
            "--summary" => set_dump_selector(&mut selector, DumpSelector::Summary)?,
            "--section" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--section requires a section id or kind".to_string())?;
                set_dump_selector(&mut selector, DumpSelector::Section(value))?;
            }
            "--max-bytes" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--max-bytes requires a positive integer".to_string())?;
                max_bytes = parse_positive_usize(&value, "--max-bytes")?;
            }
            "-h" | "--help" => return Ok(DeltaCommand::Help),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta dump option '{arg}'"))
            }
            _ => return Err("delta dump accepts one .covedelta path".into()),
        }
    }
    Ok(DeltaCommand::Dump {
        path,
        selector: selector.ok_or_else(|| {
            "delta dump requires --section <id|kind>, --parent-refs, or --summary".to_string()
        })?,
        max_bytes,
    })
}

fn set_dump_selector(slot: &mut Option<DumpSelector>, value: DumpSelector) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err("delta dump accepts only one selector".into());
    }
    Ok(())
}

fn parse_chain(mut args: Vec<String>) -> Result<ChainCommand, String> {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        return Err(format!(
            "usage: cove delta chain <inspect|validate|plan|graph|extend> ...\n\n{USAGE}"
        ));
    }
    let command = args.remove(0);
    match command.as_str() {
        "inspect" => parse_chain_inspect(args).map(ChainCommand::Inspect),
        "validate" => parse_chain_validate(args).map(ChainCommand::Validate),
        "plan" => parse_chain_plan(args).map(ChainCommand::Plan),
        "graph" => parse_chain_graph(args).map(ChainCommand::Graph),
        "extend" => parse_chain_extend(args).map(ChainCommand::Extend),
        other => Err(format!("unknown delta chain command '{other}'")),
    }
}

fn parse_chain_inspect(args: Vec<String>) -> Result<ChainInspectCommand, String> {
    let mut manifest = None;
    let mut extension = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--extension" => {
                extension =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--extension requires a file path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta chain inspect option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta chain inspect accepts one manifest path".into());
                }
            }
        }
    }
    Ok(ChainInspectCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta chain inspect <manifest.covm> [--json]".to_string()
        })?,
        extension,
        json,
    })
}

fn parse_chain_validate(args: Vec<String>) -> Result<ChainValidateCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut extension = None;
    let mut summary = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--extension" => {
                extension =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--extension requires a file path".to_string()
                    })?));
            }
            "--summary" => {
                summary =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary requires a file path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta chain validate option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta chain validate accepts one manifest path".into());
                }
            }
        }
    }
    Ok(ChainValidateCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta chain validate <manifest.covm> --dataset <dir>".to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        extension,
        summary,
        json,
    })
}

fn parse_chain_plan(args: Vec<String>) -> Result<ChainPlanCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut extension = None;
    let mut summary = None;
    let mut json = false;
    let mut request = CovmDeltaPruneRequest::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--extension" => {
                extension =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--extension requires a file path".to_string()
                    })?));
            }
            "--summary" => {
                summary =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary requires a file path".to_string()
                    })?));
            }
            "--as-of-csn" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--as-of-csn requires an integer".to_string())?;
                request.as_of_csn = Some(parse_u64(&value, "--as-of-csn")?);
            }
            "--as-of-commit-us" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--as-of-commit-us requires an integer".to_string())?;
                request.as_of_commit_timestamp_us = Some(parse_i64(&value, "--as-of-commit-us")?);
            }
            "--source-publish-range" => {
                let value = iter.next().ok_or_else(|| {
                    "--source-publish-range requires start:end microsecond values".to_string()
                })?;
                request.source_publish_range_us = Some(parse_i64_range(&value)?);
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta chain plan option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta chain plan accepts one manifest path".into());
                }
            }
        }
    }
    Ok(ChainPlanCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta chain plan <manifest.covm> --dataset <dir>".to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        extension,
        summary,
        request,
        json,
    })
}

fn parse_chain_graph(args: Vec<String>) -> Result<ChainGraphCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut extension = None;
    let mut format = GraphFormat::Text;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--extension" => {
                extension =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--extension requires a file path".to_string()
                    })?));
            }
            "--format" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--format requires text, json, or dot".to_string())?;
                format = match value.as_str() {
                    "text" => GraphFormat::Text,
                    "json" => GraphFormat::Json,
                    "dot" => GraphFormat::Dot,
                    _ => return Err("--format requires text, json, or dot".into()),
                };
            }
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta chain graph option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta chain graph accepts one manifest path".into());
                }
            }
        }
    }
    Ok(ChainGraphCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta chain graph <manifest.covm> --dataset <dir>".to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        extension,
        format,
    })
}

fn parse_chain_extend(args: Vec<String>) -> Result<ChainExtendCommand, String> {
    let mut manifest = None;
    let mut delta = None;
    let mut out = None;
    let mut summary_out = None;
    let mut force = false;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--manifest" => {
                manifest =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--manifest requires a file path".to_string()
                    })?));
            }
            "--delta" => {
                delta =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--delta requires a .covedelta path".to_string()
                    })?));
            }
            "--out" => {
                out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a manifest path".to_string()
                    })?));
            }
            "--summary-out" => {
                summary_out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary-out requires a file path".to_string()
                    })?));
            }
            "--force" => force = true,
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta chain extend option '{arg}'"))
            }
            _ => return Err("delta chain extend accepts only flags".into()),
        }
    }
    Ok(ChainExtendCommand {
        manifest: manifest.ok_or_else(|| "--manifest is required".to_string())?,
        delta: delta.ok_or_else(|| "--delta is required".to_string())?,
        out: out.ok_or_else(|| "--out is required".to_string())?,
        summary_out,
        force,
        json,
    })
}

fn parse_reconstruct(args: Vec<String>) -> Result<ReconstructCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut out = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--out" => {
                out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a snapshot path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta reconstruct option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta reconstruct accepts one manifest path".into());
                }
            }
        }
    }
    Ok(ReconstructCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta reconstruct <manifest.covm> --dataset <dir> --out <snapshot.cove>"
                .to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        out: out.ok_or_else(|| "--out is required".to_string())?,
        json,
    })
}

fn parse_compact(args: Vec<String>) -> Result<CompactCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut out = None;
    let mut publish_covm = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--out" => {
                out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a snapshot path".to_string()
                    })?));
            }
            "--publish-covm" => {
                publish_covm =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--publish-covm requires a manifest path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta compact option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta compact accepts one manifest path".into());
                }
            }
        }
    }
    Ok(CompactCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta compact <manifest.covm> --dataset <dir> --out <snapshot.cove>"
                .to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        out: out.ok_or_else(|| "--out is required".to_string())?,
        publish_covm,
        json,
    })
}

fn parse_checkpoint(args: Vec<String>) -> Result<CheckpointCommand, String> {
    let mut manifest = None;
    let mut dataset = None;
    let mut out = None;
    let mut summary_out = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset" => {
                dataset =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--dataset requires a directory path".to_string()
                    })?));
            }
            "--out" => {
                out = Some(PathBuf::from(iter.next().ok_or_else(|| {
                    "--out requires a checkpoint delta path".to_string()
                })?));
            }
            "--summary-out" => {
                summary_out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary-out requires a file path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta checkpoint option '{arg}'"))
            }
            value => {
                if manifest.replace(PathBuf::from(value)).is_some() {
                    return Err("delta checkpoint accepts one manifest path".into());
                }
            }
        }
    }
    Ok(CheckpointCommand {
        manifest: manifest.ok_or_else(|| {
            "usage: cove delta checkpoint <manifest.covm> --dataset <dir> --out <checkpoint.covedelta>"
                .to_string()
        })?,
        dataset: dataset.ok_or_else(|| "--dataset is required".to_string())?,
        out: out.ok_or_else(|| "--out is required".to_string())?,
        summary_out,
        json,
    })
}

fn parse_publish(args: Vec<String>) -> Result<PublishCommand, String> {
    let mut base = None;
    let mut deltas = Vec::new();
    let mut out = None;
    let mut summary = None;
    let mut summary_out = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--base" => {
                base = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--base requires a .cove path".to_string())?,
                ));
            }
            "--delta" => {
                deltas
                    .push(PathBuf::from(iter.next().ok_or_else(|| {
                        "--delta requires a .covedelta path".to_string()
                    })?));
            }
            "--out" => {
                out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--out requires a manifest path".to_string()
                    })?));
            }
            "--summary" => {
                summary =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary requires a file path".to_string()
                    })?));
            }
            "--summary-out" => {
                summary_out =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--summary-out requires a file path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta publish option '{arg}'"))
            }
            _ => return Err("delta publish accepts only flags".into()),
        }
    }
    if summary.is_some() && summary_out.is_some() {
        return Err("use either --summary or --summary-out, not both".into());
    }
    if deltas.is_empty() {
        return Err("delta publish requires at least one --delta".into());
    }
    Ok(PublishCommand {
        base: base.ok_or_else(|| "--base is required".to_string())?,
        deltas,
        out: out.ok_or_else(|| "--out is required".to_string())?,
        summary,
        summary_out,
        json,
    })
}

fn parse_publish_atomic(args: Vec<String>) -> Result<DeltaCommand, String> {
    let mut delta = None;
    let mut manifest = None;
    let mut json = false;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--delta" => {
                delta = Some(PathBuf::from(
                    iter.next()
                        .ok_or_else(|| "--delta requires a file path".to_string())?,
                ));
            }
            "--manifest" => {
                manifest =
                    Some(PathBuf::from(iter.next().ok_or_else(|| {
                        "--manifest requires a file path".to_string()
                    })?));
            }
            "--json" => json = true,
            "-h" | "--help" => return Ok(DeltaCommand::Help),
            arg if arg.starts_with("--") => {
                return Err(format!("unknown delta publish-atomic option '{arg}'"))
            }
            _ => return Err("delta publish-atomic accepts only flags".into()),
        }
    }
    Ok(DeltaCommand::PublishAtomic {
        delta: delta.ok_or_else(|| "--delta is required".to_string())?,
        manifest: manifest.ok_or_else(|| "--manifest is required".to_string())?,
        json,
    })
}

fn run_inspect(path: &Path, json_out: bool) -> Result<(), String> {
    let bytes = read_file(path)?;
    let file =
        CoveDeltaFile::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let object_validation = file.validate_object_delta();
    let value = covedelta_json(path, &file, object_validation.as_ref().ok());
    if json_out {
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }

    println!("COVEDELTA: {}", path.display());
    println!("  artifact_id: {}", hex16(&file.header.delta_artifact_id));
    println!("  dataset_id: {}", hex16(&file.header.dataset_id));
    println!("  snapshot_id: {}", hex16(&file.header.snapshot_id));
    println!(
        "  parent_snapshot_id: {}",
        hex16(&file.header.parent_snapshot_id)
    );
    println!("  chain_ordinal: {}", file.header.chain_ordinal);
    println!("  chain_depth: {}", file.header.chain_depth);
    println!(
        "  csn_range: {}..{}",
        file.header.csn_min, file.header.csn_max
    );
    println!(
        "  commit_time_range_us: {}..{}",
        file.header.commit_time_range_start_us, file.header.commit_time_range_end_us
    );
    if file.header.flags & DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT != 0 {
        println!(
            "  source_publish_range_us: {}..{}",
            file.header.source_publish_range_start_us, file.header.source_publish_range_end_us
        );
    }
    println!(
        "  required_delta_features: {}",
        format_feature_bits(file.header.required_delta_features)
    );
    println!(
        "  optional_delta_features: {}",
        format_feature_bits(file.header.optional_delta_features)
    );
    println!("  parent_refs: {}", file.parent_refs.len());
    println!("  sections: {}", file.sections.len());
    for section in &file.sections {
        println!(
            "    - id={} kind={} offset={} length={} items={}",
            section.entry.section_id,
            section_kind_name(section.entry.section_kind),
            section.entry.offset,
            section.entry.length,
            section.entry.item_count
        );
    }
    match object_validation {
        Ok(validation) => {
            println!("  object_delta: valid");
            println!(
                "  temporal_segments: {}",
                validation.temporal_segments.len()
            );
            println!(
                "  sparse_patch_rows: {}",
                validation.sparse_patch_records.len()
            );
            println!(
                "  touched_ranges: {}",
                validation.touched_object_ranges.len()
            );
            println!(
                "  tombstone_ranges: {}",
                validation.tombstone_object_ranges.len()
            );
        }
        Err(error) => {
            println!("  object_delta: not valid ({error})");
        }
    }
    Ok(())
}

fn run_validate(path: &Path, object_delta: bool, json_out: bool) -> Result<(), String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            if json_out {
                print_validation_json(path, false, None, &format!("io: {error}"));
                return Err("validation failed".into());
            }
            return Err(format!("cannot read {}: {error}", path.display()));
        }
    };
    let parsed = CoveDeltaFile::parse(&bytes).and_then(|file| {
        if object_delta {
            file.validate_object_delta().map(|_| file)
        } else {
            Ok(file)
        }
    });
    match parsed {
        Ok(file) => {
            if json_out {
                let value = json!({
                    "path": path.display().to_string(),
                    "ok": true,
                    "artifact": "covedelta",
                    "object_delta": object_delta,
                    "version_major": file.header.version_major,
                    "version_minor": file.header.version_minor,
                    "section_count": file.sections.len(),
                    "parent_ref_count": file.parent_refs.len(),
                });
                println!("{}", serde_json::to_string_pretty(&value).unwrap());
            } else {
                let mode = if object_delta {
                    "object-delta"
                } else {
                    "structural"
                };
                println!("{}: OK [{mode}]", path.display());
            }
            Ok(())
        }
        Err(error) => {
            if json_out {
                print_validation_json(path, false, error.spec_code(), &error.to_string());
            } else {
                eprintln!("{}: INVALID", path.display());
                eprintln!("  [ERR] {error}");
            }
            Err("validation failed".into())
        }
    }
}

fn print_validation_json(path: &Path, ok: bool, code: Option<&str>, error: &str) {
    let mut value = json!({
        "path": path.display().to_string(),
        "ok": ok,
        "artifact": "covedelta",
        "error": error,
    });
    if let Some(code) = code {
        value["error_code"] = json!(code);
    }
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

fn run_dump(path: &Path, selector: DumpSelector, max_bytes: usize) -> Result<(), String> {
    let bytes = read_file(path)?;
    let file =
        CoveDeltaFile::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    match selector {
        DumpSelector::ParentRefs => {
            println!("parent_refs={}", file.parent_refs.len());
            for parent in &file.parent_refs {
                println!(
                    "  parent_ref={} kind={} lineage={} artifact_id={} snapshot_id={} file_len={} footer_crc32c={:#010x} digest_algorithm={} digest_len={} digest_ref={} uri_ref={}",
                    parent.parent_ref,
                    parent.parent_kind,
                    parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0,
                    hex16(&parent.artifact_id),
                    hex16(&parent.snapshot_id),
                    parent.file_len,
                    parent.footer_crc32c,
                    parent.digest_algorithm,
                    parent.digest_len,
                    parent.digest_ref,
                    parent.uri_ref
                );
            }
        }
        DumpSelector::Section(raw) => {
            let section = find_delta_section(&file, &raw)?;
            let shown = section.payload.len().min(max_bytes);
            println!(
                "section_id={} kind={} len={} showing={} bytes",
                section.entry.section_id,
                section_kind_name(section.entry.section_kind),
                section.payload.len(),
                shown
            );
            print_hex(&section.payload[..shown]);
        }
        DumpSelector::Summary => {
            let validation = file.validate_object_delta().map_err(|error| {
                format!(
                    "{}: object-delta summary unavailable: {error}",
                    path.display()
                )
            })?;
            let value = json!({
                "temporal_segments": validation.temporal_segments.len(),
                "sparse_patch_rows": validation.sparse_patch_records.len(),
                "checkpoint_row_count": validation.checkpoint_row_count,
                "touched_object_ranges": validation.touched_object_ranges.len(),
                "tombstone_object_ranges": validation.tombstone_object_ranges.len(),
                "dictionary_overlay_entries": validation.dictionary_overlay_entries.len(),
                "index_hints": validation.index_hints.len(),
                "coverage_patches": validation.coverage_patches.len(),
                "catalog_patches": validation.catalog_patches.len(),
                "evidence_patches": validation.evidence_patches.len(),
                "projection_patches": validation.projection_patches.len(),
            });
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
        }
    }
    Ok(())
}

fn run_chain(command: ChainCommand) -> Result<(), String> {
    match command {
        ChainCommand::Inspect(command) => run_chain_inspect(command),
        ChainCommand::Validate(command) => run_chain_validate(command),
        ChainCommand::Plan(command) => run_chain_plan(command),
        ChainCommand::Graph(command) => run_chain_graph(command),
        ChainCommand::Extend(command) => run_chain_extend(command),
    }
}

fn run_chain_inspect(command: ChainInspectCommand) -> Result<(), String> {
    let context = load_manifest_delta_context(&command.manifest, command.extension.as_deref())?;
    let delta_required =
        context.manifest.postscript.flags & COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED != 0;
    if command.json {
        let mut value = json!({
            "manifest": command.manifest.display().to_string(),
            "delta_chain_required": delta_required,
            "file_count": context.manifest.files.len(),
            "extension_source": context.extension_source,
        });
        if let Some(extension) = &context.extension {
            value["delta_chain"] = extension_json(extension);
            value["inline_summary_bytes"] =
                json!(context.inline_summary_bytes.as_ref().map_or(0, Vec::len));
        } else {
            value["delta_chain"] = Value::Null;
        }
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }

    println!("COVM delta chain: {}", command.manifest.display());
    println!("  delta_chain_required: {delta_required}");
    println!("  file_entries: {}", context.manifest.files.len());
    match &context.extension {
        Some(extension) => {
            println!(
                "  extension_source: {}",
                context.extension_source.as_deref().unwrap_or("manifest")
            );
            print_extension_text(extension);
            println!(
                "  inline_summary_bytes: {}",
                context.inline_summary_bytes.as_ref().map_or(0, Vec::len)
            );
        }
        None => println!("  delta_chain: not declared"),
    }
    Ok(())
}

fn run_chain_validate(command: ChainValidateCommand) -> Result<(), String> {
    let context = load_manifest_delta_context(&command.manifest, command.extension.as_deref())?;
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?;
    let (base, deltas) = load_selected_artifacts(&context, extension, &command.dataset)?;
    let summary_bytes = command
        .summary
        .as_deref()
        .map(read_file)
        .transpose()?
        .or_else(|| context.inline_summary_bytes.clone());
    let summary = summary_bytes
        .as_deref()
        .map(CovmDeltaChainSummaryV1::parse)
        .transpose()
        .map_err(|error| format!("invalid delta chain summary: {error}"))?;
    let delta_slices = deltas
        .iter()
        .map(|artifact| artifact.bytes.as_slice())
        .collect::<Vec<_>>();
    let parsed = validate_selected_delta_chain_with_base(
        extension,
        summary.as_ref(),
        Some(base.bytes.as_slice()),
        &delta_slices,
    )
    .map_err(|error| format!("selected delta chain is invalid: {error}"))?;
    if command.json {
        let value = json!({
            "manifest": command.manifest.display().to_string(),
            "ok": true,
            "base": base.path.display().to_string(),
            "delta_count": parsed.len(),
            "summary": summary.is_some(),
            "result_snapshot_id": hex16(&extension.result_snapshot_id),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Delta chain OK: {}", command.manifest.display());
        println!("  base: {}", base.path.display());
        println!("  deltas: {}", parsed.len());
        println!(
            "  result_snapshot_id: {}",
            hex16(&extension.result_snapshot_id)
        );
        println!("  summary_validated: {}", summary.is_some());
    }
    Ok(())
}

fn run_chain_plan(command: ChainPlanCommand) -> Result<(), String> {
    let context = load_manifest_delta_context(&command.manifest, command.extension.as_deref())?;
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?;
    let summary_bytes = command
        .summary
        .as_deref()
        .map(read_file)
        .transpose()?
        .or_else(|| context.inline_summary_bytes.clone())
        .ok_or_else(|| {
            "delta chain plan requires an inline summary or --summary <file>".to_string()
        })?;
    let summary = CovmDeltaChainSummaryV1::parse(&summary_bytes)
        .map_err(|error| format!("invalid delta chain summary: {error}"))?;
    summary
        .validate_against_delta_chain_extension(extension)
        .map_err(|error| format!("delta chain summary is stale: {error}"))?;
    let decision = summary
        .prune_delta_chain(command.request)
        .map_err(|error| format!("cannot plan delta chain: {error}"))?;
    let mut metrics = summary.read_amplification_metrics(&decision);
    metrics.base_file_bytes = extension.base_artifact_ref.file_len;
    metrics.total_delta_bytes = extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| reference.file_len)
        .sum();
    metrics.bytes_returned = metrics.base_file_bytes.saturating_add(selected_delta_bytes(
        extension,
        &decision.selected_chain_ordinals,
    ));
    let recommendations = metrics.recommendations(Default::default());
    if command.json {
        let value = json!({
            "manifest": command.manifest.display().to_string(),
            "selected_chain_ordinals": decision.selected_chain_ordinals,
            "skipped": decision.skipped.iter().map(|skip| json!({
                "chain_ordinal": skip.chain_ordinal,
                "reason": prune_reason_name(skip.reason),
            })).collect::<Vec<_>>(),
            "metrics": read_amplification_json(metrics),
            "recommendations": recommendations.iter().map(|item| recommendation_name(*item)).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return Ok(());
    }
    println!("Delta chain plan: {}", command.manifest.display());
    println!("  selected: {:?}", decision.selected_chain_ordinals);
    if decision.skipped.is_empty() {
        println!("  skipped: none");
    } else {
        println!("  skipped:");
        for skip in &decision.skipped {
            println!(
                "    - {} ({})",
                skip.chain_ordinal,
                prune_reason_name(skip.reason)
            );
        }
    }
    println!("  chain_depth: {}", metrics.delta_chain_depth);
    println!("  selected_delta_count: {}", metrics.selected_delta_count);
    println!("  skipped_delta_count: {}", metrics.skipped_delta_count);
    println!(
        "  object_store_request_count: {}",
        metrics.object_store_request_count
    );
    println!("  chain_summary_bytes: {}", metrics.chain_summary_bytes);
    println!("  base_file_bytes: {}", metrics.base_file_bytes);
    println!("  total_delta_bytes: {}", metrics.total_delta_bytes);
    if recommendations.is_empty() {
        println!("  recommendations: none");
    } else {
        println!("  recommendations:");
        for item in recommendations {
            println!("    - {}", recommendation_name(item));
        }
    }
    Ok(())
}

fn run_chain_graph(command: ChainGraphCommand) -> Result<(), String> {
    let context = load_manifest_delta_context(&command.manifest, command.extension.as_deref())?;
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?;
    let names = artifact_uri_map(&context.manifest);
    match command.format {
        GraphFormat::Text => {
            println!(
                "{} ({})",
                artifact_label(&names, &extension.base_artifact_ref),
                hex16(&extension.base_artifact_ref.snapshot_id)
            );
            for reference in &extension.ordered_delta_artifact_refs {
                println!(
                    "  -> {} ({})",
                    artifact_label(&names, reference),
                    hex16(&reference.snapshot_id)
                );
            }
        }
        GraphFormat::Json => {
            let value = json!({
                "base": artifact_graph_json(&names, &extension.base_artifact_ref),
                "deltas": extension.ordered_delta_artifact_refs.iter().map(|reference| {
                    artifact_graph_json(&names, reference)
                }).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
        }
        GraphFormat::Dot => {
            println!("digraph cove_delta_chain {{");
            println!("  rankdir=LR;");
            let base = dot_node_id(&extension.base_artifact_ref.artifact_id);
            println!(
                "  {base} [label=\"{}\\n{}\"];",
                dot_escape(&artifact_label(&names, &extension.base_artifact_ref)),
                hex16(&extension.base_artifact_ref.snapshot_id)
            );
            let mut previous = base;
            for reference in &extension.ordered_delta_artifact_refs {
                let node = dot_node_id(&reference.artifact_id);
                println!(
                    "  {node} [label=\"{}\\n{}\"];",
                    dot_escape(&artifact_label(&names, reference)),
                    hex16(&reference.snapshot_id)
                );
                println!("  {previous} -> {node};");
                previous = node;
            }
            println!("}}");
        }
    }
    let _ = command.dataset;
    Ok(())
}

fn run_chain_extend(command: ChainExtendCommand) -> Result<(), String> {
    let context = load_manifest_delta_context(&command.manifest, None)?;
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?;
    let names = artifact_uri_map(&context.manifest);
    let base_path = names
        .get(&extension.base_artifact_ref.artifact_id)
        .map(PathBuf::from)
        .ok_or_else(|| "manifest does not include the selected base artifact URI".to_string())?;
    let mut delta_paths = extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| {
            names
                .get(&reference.artifact_id)
                .map(PathBuf::from)
                .ok_or_else(|| {
                    format!(
                        "manifest does not include delta artifact URI for {}",
                        hex16(&reference.artifact_id)
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    delta_paths.push(command.delta);
    let publish = PublishCommand {
        base: base_path,
        deltas: delta_paths,
        out: command.out,
        summary: None,
        summary_out: command.summary_out,
        json: command.json,
    };
    if publish.out.exists() && !command.force {
        return Err(format!(
            "{} already exists; use --force to overwrite",
            publish.out.display()
        ));
    }
    run_publish(publish)
}

fn run_reconstruct(command: ReconstructCommand) -> Result<(), String> {
    let snapshot = load_validated_delta_snapshot(&command.manifest, &command.dataset)?;
    let materialized = materialize_validated_delta_snapshot(&snapshot)?;
    durable::durable_replace(&command.out, &materialized.bytes)
        .map_err(|error| format!("cannot write {}: {error}", command.out.display()))?;

    if command.json {
        let value = json!({
            "ok": true,
            "output": command.out.display().to_string(),
            "manifest": command.manifest.display().to_string(),
            "base": snapshot.base.path.display().to_string(),
            "delta_count": snapshot.deltas.len(),
            "summary_validated": snapshot.summary_validated,
            "result_snapshot_id": hex16(&snapshot.extension.result_snapshot_id),
            "bytes_written": materialized.bytes.len(),
            "state_count": materialized.state_count,
            "passthrough_base": materialized.passthrough_base,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Reconstructed delta snapshot: {}", command.out.display());
        println!("  manifest: {}", command.manifest.display());
        println!("  base: {}", snapshot.base.path.display());
        println!("  deltas: {}", snapshot.deltas.len());
        println!(
            "  result_snapshot_id: {}",
            hex16(&snapshot.extension.result_snapshot_id)
        );
        println!("  bytes_written: {}", materialized.bytes.len());
        if let Some(state_count) = materialized.state_count {
            println!("  object_states: {state_count}");
        }
        println!("  passthrough_base: {}", materialized.passthrough_base);
    }
    Ok(())
}

fn run_compact(command: CompactCommand) -> Result<(), String> {
    let snapshot = load_validated_delta_snapshot(&command.manifest, &command.dataset)?;
    let materialized = materialize_validated_delta_snapshot(&snapshot)?;
    durable::durable_replace(&command.out, &materialized.bytes)
        .map_err(|error| format!("cannot write {}: {error}", command.out.display()))?;

    let mut published_covm = None;
    if let Some(path) = &command.publish_covm {
        let artifact = CovmInputArtifact {
            uri: command.out.display().to_string(),
            bytes: &materialized.bytes,
        };
        let (manifest_bytes, _report) = build_covm_artifact_from_bytes(path, &[artifact])
            .map_err(|error| format!("cannot build compacted COVM {}: {error}", path.display()))?;
        durable::durable_replace(path, &manifest_bytes)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        published_covm = Some(path.display().to_string());
    }

    let before_delta_bytes: u64 = snapshot
        .extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| reference.file_len)
        .sum();
    let before_chain_bytes = snapshot
        .extension
        .base_artifact_ref
        .file_len
        .saturating_add(before_delta_bytes);
    if command.json {
        let value = json!({
            "ok": true,
            "output": command.out.display().to_string(),
            "published_covm": published_covm,
            "manifest": command.manifest.display().to_string(),
            "base": snapshot.base.path.display().to_string(),
            "delta_count_before": snapshot.deltas.len(),
            "delta_chain_depth_before": snapshot.extension.ordered_delta_artifact_refs.len(),
            "base_file_bytes_before": snapshot.extension.base_artifact_ref.file_len,
            "delta_bytes_before": before_delta_bytes,
            "chain_bytes_before": before_chain_bytes,
            "compacted_bytes": materialized.bytes.len(),
            "state_count": materialized.state_count,
            "passthrough_base": materialized.passthrough_base,
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Compacted delta snapshot: {}", command.out.display());
        println!("  manifest: {}", command.manifest.display());
        println!("  deltas_before: {}", snapshot.deltas.len());
        println!(
            "  base_file_bytes_before: {}",
            snapshot.extension.base_artifact_ref.file_len
        );
        println!("  delta_bytes_before: {before_delta_bytes}");
        println!("  chain_bytes_before: {before_chain_bytes}");
        println!("  compacted_bytes: {}", materialized.bytes.len());
        if let Some(state_count) = materialized.state_count {
            println!("  object_states: {state_count}");
        }
        println!("  passthrough_base: {}", materialized.passthrough_base);
        if let Some(path) = published_covm {
            println!("  published_covm: {path}");
        }
    }
    Ok(())
}

fn run_checkpoint(command: CheckpointCommand) -> Result<(), String> {
    let snapshot = load_validated_delta_snapshot(&command.manifest, &command.dataset)?;
    let object_snapshot = reconstruct_validated_object_snapshot(&snapshot)?;
    let (checkpoint_bytes, checkpoint_file) =
        build_checkpoint_delta(&snapshot, &object_snapshot, &command.out)?;
    durable::durable_replace(&command.out, &checkpoint_bytes)
        .map_err(|error| format!("cannot write {}: {error}", command.out.display()))?;

    let checkpoint_ref = delta_ref_from_file(&checkpoint_file, &checkpoint_bytes)?;
    let mut summary_out = None;
    if let Some(path) = &command.summary_out {
        let summary_bytes = build_checkpoint_summary(
            &snapshot,
            &checkpoint_file,
            checkpoint_ref.clone(),
            &command.out,
        )?;
        durable::durable_replace(path, &summary_bytes)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        summary_out = Some(path.display().to_string());
    }

    if command.json {
        let value = json!({
            "ok": true,
            "output": command.out.display().to_string(),
            "summary_out": summary_out,
            "manifest": command.manifest.display().to_string(),
            "base": snapshot.base.path.display().to_string(),
            "source_delta_count": snapshot.deltas.len(),
            "state_count": object_snapshot.states.len(),
            "checkpoint_artifact_id": hex16(&checkpoint_file.header.delta_artifact_id),
            "parent_snapshot_id": hex16(&checkpoint_file.header.parent_snapshot_id),
            "snapshot_id": hex16(&checkpoint_file.header.snapshot_id),
            "chain_ordinal": checkpoint_file.header.chain_ordinal,
            "bytes_written": checkpoint_bytes.len(),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Created checkpoint delta: {}", command.out.display());
        println!("  manifest: {}", command.manifest.display());
        println!("  source_deltas: {}", snapshot.deltas.len());
        println!("  object_states: {}", object_snapshot.states.len());
        println!(
            "  checkpoint_artifact_id: {}",
            hex16(&checkpoint_file.header.delta_artifact_id)
        );
        println!(
            "  parent_snapshot_id: {}",
            hex16(&checkpoint_file.header.parent_snapshot_id)
        );
        println!(
            "  snapshot_id: {}",
            hex16(&checkpoint_file.header.snapshot_id)
        );
        println!("  chain_ordinal: {}", checkpoint_file.header.chain_ordinal);
        println!("  bytes_written: {}", checkpoint_bytes.len());
        if let Some(path) = summary_out {
            println!("  summary_out: {path}");
        }
    }
    Ok(())
}

fn run_publish(command: PublishCommand) -> Result<(), String> {
    let base_bytes = read_file(&command.base)?;
    let mut base_identity = base_identity(&command.base, &base_bytes)?;
    let mut deltas = Vec::new();
    for path in &command.deltas {
        let bytes = read_file(path)?;
        let file = CoveDeltaFile::parse(&bytes)
            .map_err(|error| format!("{}: invalid delta artifact: {error}", path.display()))?;
        let reference = delta_ref_from_file(&file, &bytes)?;
        deltas.push(PublishedDelta {
            path: path.clone(),
            file,
            reference,
        });
    }
    let base_snapshot_id = validate_publish_chain(&base_identity, &deltas)?;
    base_identity.reference.snapshot_id = base_snapshot_id;
    let dataset_id = deltas[0].file.header.dataset_id;
    let result_snapshot_id = deltas
        .last()
        .map(|delta| delta.file.header.snapshot_id)
        .unwrap_or(base_snapshot_id);
    let mut extension = CovmDeltaChainExtensionV1::new(
        dataset_id,
        base_snapshot_id,
        result_snapshot_id,
        base_identity.reference.clone(),
        deltas.iter().map(|delta| delta.reference.clone()).collect(),
    );
    extension.csn_min = deltas
        .iter()
        .map(|delta| delta.file.header.csn_min)
        .min()
        .unwrap_or(0);
    extension.csn_max = deltas
        .iter()
        .map(|delta| delta.file.header.csn_max)
        .max()
        .unwrap_or(0);
    extension.created_at_us = now_us();
    extension.required_delta_features = deltas.iter().fold(0u64, |bits, delta| {
        bits | delta.file.header.required_delta_features
    });
    extension.optional_delta_features = deltas.iter().fold(0u64, |bits, delta| {
        bits | delta.file.header.optional_delta_features
    });

    let extension = CovmDeltaChainExtensionV1::parse(
        &extension
            .serialize()
            .map_err(|error| format!("cannot serialize delta chain extension: {error}"))?,
    )
    .map_err(|error| format!("cannot validate delta chain extension: {error}"))?;

    let summary_bytes = match (&command.summary, &command.summary_out) {
        (Some(summary), None) => {
            let bytes = read_file(summary)?;
            let summary = CovmDeltaChainSummaryV1::parse(&bytes)
                .map_err(|error| format!("invalid --summary {}: {error}", summary.display()))?;
            summary
                .validate_against_delta_chain_extension(&extension)
                .map_err(|error| format!("--summary is stale for this chain: {error}"))?;
            Some(bytes)
        }
        (None, Some(summary_out)) => {
            let bytes = build_summary(&extension, &deltas)?;
            durable::durable_replace(summary_out, &bytes)
                .map_err(|error| format!("cannot write {}: {error}", summary_out.display()))?;
            Some(bytes)
        }
        (None, None) => Some(build_summary(&extension, &deltas)?),
        (Some(_), Some(_)) => unreachable!("parse rejects this case"),
    };

    let extension = extension_with_summary_binding(extension, summary_bytes.as_deref())?;
    let manifest_bytes = build_delta_covm_bytes(
        &base_identity,
        &deltas,
        &extension,
        summary_bytes.as_deref(),
    )?;
    durable::durable_replace(&command.out, &manifest_bytes)
        .map_err(|error| format!("cannot write {}: {error}", command.out.display()))?;

    if command.json {
        let value = json!({
            "ok": true,
            "output": command.out.display().to_string(),
            "base": command.base.display().to_string(),
            "deltas": command.deltas.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "delta_count": deltas.len(),
            "result_snapshot_id": hex16(&extension.result_snapshot_id),
            "chain_digest": hex_encode(&extension.chain_digest),
            "inline_summary": summary_bytes.is_some(),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Published delta manifest: {}", command.out.display());
        println!("  base: {}", command.base.display());
        println!("  deltas: {}", deltas.len());
        println!(
            "  result_snapshot_id: {}",
            hex16(&extension.result_snapshot_id)
        );
        println!("  chain_digest: {}", hex_encode(&extension.chain_digest));
        println!("  inline_summary: {}", summary_bytes.is_some());
    }
    Ok(())
}

fn run_publish_atomic(delta: &Path, manifest: &Path, json_out: bool) -> Result<(), String> {
    let delta_bytes = read_file(delta)?;
    let manifest_bytes = read_file(manifest)?;
    durable::durable_publish_delta_then_manifest(delta, &delta_bytes, manifest, &manifest_bytes)
        .map_err(|error| format!("atomic delta/manifest publish failed: {error}"))?;
    if json_out {
        let value = json!({
            "ok": true,
            "delta": delta.display().to_string(),
            "manifest": manifest.display().to_string(),
            "publish_order": ["delta", "manifest"],
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    } else {
        println!("Published delta then manifest:");
        println!("  delta: {}", delta.display());
        println!("  manifest: {}", manifest.display());
    }
    Ok(())
}

fn load_validated_delta_snapshot(
    manifest: &Path,
    dataset: &Path,
) -> Result<ValidatedDeltaSnapshot, String> {
    let context = load_manifest_delta_context(manifest, None)?;
    let extension = context
        .extension
        .as_ref()
        .ok_or_else(|| "manifest does not contain a delta-chain extension".to_string())?
        .clone();
    let (base, deltas) = load_selected_artifacts(&context, &extension, dataset)?;
    let summary = context
        .inline_summary_bytes
        .as_deref()
        .map(CovmDeltaChainSummaryV1::parse)
        .transpose()
        .map_err(|error| format!("invalid delta chain summary: {error}"))?;
    let delta_slices = deltas
        .iter()
        .map(|artifact| artifact.bytes.as_slice())
        .collect::<Vec<_>>();
    let parsed_deltas = validate_selected_delta_chain_with_base(
        &extension,
        summary.as_ref(),
        Some(base.bytes.as_slice()),
        &delta_slices,
    )
    .map_err(|error| format!("selected delta chain is invalid: {error}"))?;
    Ok(ValidatedDeltaSnapshot {
        extension,
        base,
        deltas,
        parsed_deltas,
        summary_validated: summary.is_some(),
    })
}

fn materialize_validated_delta_snapshot(
    snapshot: &ValidatedDeltaSnapshot,
) -> Result<MaterializedSnapshot, String> {
    if snapshot
        .parsed_deltas
        .iter()
        .all(|delta| delta.sections.is_empty())
    {
        return Ok(MaterializedSnapshot {
            bytes: snapshot.base.bytes.clone(),
            state_count: None,
            passthrough_base: true,
        });
    }

    let ReconstructedObjectSnapshot {
        object_types,
        states,
    } = reconstruct_validated_object_snapshot(snapshot)?;
    let state_count = states.len();
    let bytes = cove_map::compact_cove_o_from_object_states(object_types, &states)
        .map_err(|error| format!("cannot write reconstructed COVE-O snapshot: {error}"))?;
    Ok(MaterializedSnapshot {
        bytes,
        state_count: Some(state_count),
        passthrough_base: false,
    })
}

fn reconstruct_validated_object_snapshot(
    snapshot: &ValidatedDeltaSnapshot,
) -> Result<ReconstructedObjectSnapshot, String> {
    let read_options = CoveObjectReadOptions::default();
    let surface = if snapshot
        .parsed_deltas
        .iter()
        .all(|delta| delta.sections.is_empty())
    {
        read_object_surface_from_bytes_with_options(&snapshot.base.bytes, &read_options)
            .map_err(|error| format!("cannot read selected COVE-O base snapshot: {error}"))?
    } else {
        read_object_surface_from_base_and_delta_files_with_parent_identity(
            &snapshot.base.bytes,
            Some(&cove_object_delta_parent_identity(
                &snapshot.extension.base_artifact_ref,
            )),
            &snapshot.parsed_deltas,
            &read_options,
        )
        .map_err(|error| format!("cannot read selected COVE-O delta snapshot: {error}"))?
    };
    let states = reconstruct_object_states(&surface, &CoveObjectReconstructionOptions::default())
        .map_err(|error| {
        format!("cannot reconstruct selected COVE-O object states: {error}")
    })?;
    Ok(ReconstructedObjectSnapshot {
        object_types: surface.object_types,
        states,
    })
}

fn build_checkpoint_delta(
    snapshot: &ValidatedDeltaSnapshot,
    object_snapshot: &ReconstructedObjectSnapshot,
    out: &Path,
) -> Result<(Vec<u8>, CoveDeltaFile), String> {
    if object_snapshot.states.is_empty() {
        return Err(
            "checkpoint delta requires at least one live reconstructed object state".into(),
        );
    }
    let now = now_us();
    let state_count = object_snapshot.states.len() as u64;
    let checkpoint_csn = snapshot
        .extension
        .csn_max
        .max(
            object_snapshot
                .states
                .iter()
                .map(|state| state.csn)
                .max()
                .unwrap_or(0),
        )
        .saturating_add(1);
    let checkpoint_timestamp_us = snapshot
        .extension
        .created_at_us
        .max(
            object_snapshot
                .states
                .iter()
                .map(|state| state.timestamp_us)
                .max()
                .unwrap_or(0),
        )
        .saturating_add(1);
    let mut checkpoint_states = object_snapshot.states.clone();
    for state in &mut checkpoint_states {
        state.csn = checkpoint_csn;
        state.timestamp_us = checkpoint_timestamp_us;
    }
    let delta_artifact_id = derive_checkpoint_id(
        b"checkpoint-artifact",
        &snapshot.extension.result_snapshot_id,
        out,
        now,
        state_count,
    )?;
    let snapshot_id = derive_checkpoint_id(
        b"checkpoint-snapshot",
        &snapshot.extension.result_snapshot_id,
        out,
        now,
        state_count,
    )?;
    let mut header = CoveDeltaHeaderV1::new(
        delta_artifact_id,
        snapshot.extension.dataset_id,
        snapshot_id,
        snapshot.extension.result_snapshot_id,
    );
    header.chain_ordinal = u32::try_from(snapshot.extension.ordered_delta_artifact_refs.len() + 1)
        .map_err(|_| "checkpoint chain ordinal overflows".to_string())?;
    header.chain_depth = header.chain_ordinal;
    header.csn_min = checkpoint_csn;
    header.csn_max = checkpoint_csn;
    header.commit_time_range_start_us = checkpoint_timestamp_us;
    header.commit_time_range_end_us = checkpoint_timestamp_us;
    header.created_at_us = now;
    header.required_delta_features = DELTA_FEATURE_CHECKPOINT_BASELINES;

    let parent = checkpoint_lineage_parent_ref(snapshot);
    let temporal_sections = cove_map::checkpoint_temporal_sections_from_object_states(
        &object_snapshot.object_types,
        &checkpoint_states,
    )
    .map_err(|error| format!("cannot encode checkpoint temporal sections: {error}"))?;
    let sections = temporal_sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| {
            let section_id =
                u32::try_from(index + 1).map_err(|_| "too many checkpoint sections".to_string())?;
            Ok(CoveDeltaSection {
                entry: CoveDeltaSectionDirectoryEntryV1 {
                    section_id,
                    section_kind: CoveDeltaSectionKind::TemporalSegmentData as u16,
                    flags: 0,
                    offset: 0,
                    length: 0,
                    uncompressed_length: 0,
                    item_count: 1,
                    compression: 0,
                    encryption: 0,
                    alignment_log2: 0,
                    reserved0: 0,
                    required_delta_features: DELTA_FEATURE_CHECKPOINT_BASELINES,
                    optional_delta_features: 0,
                    crc32c: 0,
                    checksum: 0,
                },
                payload: section.payload,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let file = CoveDeltaFile {
        header,
        parent_refs: vec![parent],
        sections,
        footer: CoveDeltaFooterV1 {
            header_offset: 0,
            header_length: 0,
            section_directory_offset: 0,
            section_directory_length: 0,
            section_count: 0,
            parent_ref_count: 0,
            footer_crc32c: 0,
            checksum: 0,
        },
        postscript: CoveDeltaPostscriptV1 {
            required_delta_features: DELTA_FEATURE_CHECKPOINT_BASELINES,
            optional_delta_features: 0,
            file_len: 0,
            footer_offset: 0,
            footer_length: 0,
            checksum: 0,
        },
    };
    let bytes = file
        .serialize()
        .map_err(|error| format!("cannot serialize checkpoint delta: {error}"))?;
    let parsed = CoveDeltaFile::parse(&bytes)
        .map_err(|error| format!("serialized checkpoint delta is invalid: {error}"))?;
    parsed
        .validate_object_delta()
        .map_err(|error| format!("serialized checkpoint object delta is invalid: {error}"))?;
    Ok((bytes, parsed))
}

fn derive_checkpoint_id(
    label: &[u8],
    parent_snapshot_id: &[u8; 16],
    out: &Path,
    now_us: i64,
    state_count: u64,
) -> Result<[u8; 16], String> {
    let mut seed = Vec::new();
    seed.extend_from_slice(label);
    seed.extend_from_slice(parent_snapshot_id);
    seed.extend_from_slice(out.display().to_string().as_bytes());
    seed.extend_from_slice(&now_us.to_le_bytes());
    seed.extend_from_slice(&state_count.to_le_bytes());
    let digest = compute_digest(DigestAlgorithm::Sha256, &seed)
        .map_err(|error| format!("cannot derive checkpoint id: {error}"))?;
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    Ok(id)
}

fn checkpoint_lineage_parent_ref(snapshot: &ValidatedDeltaSnapshot) -> DeltaParentRefV1 {
    let parent = snapshot
        .extension
        .ordered_delta_artifact_refs
        .last()
        .unwrap_or(&snapshot.extension.base_artifact_ref);
    DeltaParentRefV1 {
        parent_ref: 0,
        parent_kind: 0,
        flags: DELTA_PARENT_REF_LINEAGE_PARENT,
        artifact_id: parent.artifact_id,
        snapshot_id: parent.snapshot_id,
        file_len: parent.file_len,
        footer_crc32c: parent.footer_crc32c,
        digest_algorithm: parent.digest_algorithm,
        digest_len: parent.digest_len,
        digest_ref: 0,
        uri_ref: parent.uri_ref,
        schema_fingerprint_ref: 0,
        object_catalog_fingerprint_ref: 0,
        semantic_map_fingerprint_ref: 0,
        projection_fingerprint_ref: 0,
        checksum: 0,
    }
}

fn build_checkpoint_summary(
    snapshot: &ValidatedDeltaSnapshot,
    checkpoint_file: &CoveDeltaFile,
    checkpoint_ref: CovmDeltaArtifactRefV1,
    checkpoint_path: &Path,
) -> Result<Vec<u8>, String> {
    let mut refs = snapshot.extension.ordered_delta_artifact_refs.clone();
    refs.push(checkpoint_ref.clone());
    let mut extension = CovmDeltaChainExtensionV1::new(
        snapshot.extension.dataset_id,
        snapshot.extension.base_snapshot_id,
        checkpoint_file.header.snapshot_id,
        snapshot.extension.base_artifact_ref.clone(),
        refs,
    );
    extension.csn_min = snapshot
        .extension
        .csn_min
        .min(checkpoint_file.header.csn_min);
    extension.csn_max = snapshot
        .extension
        .csn_max
        .max(checkpoint_file.header.csn_max);
    extension.created_at_us = now_us();
    extension.required_delta_features =
        snapshot.extension.required_delta_features | checkpoint_file.header.required_delta_features;
    extension.optional_delta_features =
        snapshot.extension.optional_delta_features | checkpoint_file.header.optional_delta_features;
    let extension = CovmDeltaChainExtensionV1::parse(
        &extension
            .serialize()
            .map_err(|error| format!("cannot serialize checkpoint chain extension: {error}"))?,
    )
    .map_err(|error| format!("cannot validate checkpoint chain extension: {error}"))?;

    let mut deltas = snapshot
        .parsed_deltas
        .iter()
        .zip(snapshot.deltas.iter())
        .zip(snapshot.extension.ordered_delta_artifact_refs.iter())
        .map(|((file, artifact), reference)| PublishedDelta {
            path: artifact.path.clone(),
            file: file.clone(),
            reference: reference.clone(),
        })
        .collect::<Vec<_>>();
    deltas.push(PublishedDelta {
        path: checkpoint_path.to_path_buf(),
        file: checkpoint_file.clone(),
        reference: checkpoint_ref,
    });
    build_summary(&extension, &deltas)
}

fn load_manifest_delta_context(
    manifest_path: &Path,
    extension_override: Option<&Path>,
) -> Result<ManifestDeltaContext, String> {
    let manifest_bytes = read_file(manifest_path)?;
    let manifest = CovmFile::parse_delta_aware(&manifest_bytes).map_err(|error| {
        format!(
            "{} is not a valid COVM manifest: {error}",
            manifest_path.display()
        )
    })?;
    let (extension, inline_summary_bytes, extension_source) = if let Some(path) = extension_override
    {
        let bytes = read_file(path)?;
        let extension = CovmDeltaChainExtensionV1::parse(&bytes).map_err(|error| {
            format!(
                "{} is not a valid delta-chain extension: {error}",
                path.display()
            )
        })?;
        (Some(extension), None, Some(path.display().to_string()))
    } else {
        let region = manifest_extension_region(&manifest_bytes, &manifest)?;
        if region.is_empty() {
            (None, None, None)
        } else {
            let extension_len = covm_delta_extension_encoded_len(region)?;
            if extension_len > region.len() {
                return Err(
                    "COVM delta-chain extension extends past manifest extension region".into(),
                );
            }
            let extension = CovmDeltaChainExtensionV1::parse(&region[..extension_len])
                .map_err(|error| format!("manifest delta-chain extension is invalid: {error}"))?;
            let summary = if extension_len < region.len() {
                Some(region[extension_len..].to_vec())
            } else {
                None
            };
            (Some(extension), summary, Some("manifest".into()))
        }
    };
    Ok(ManifestDeltaContext {
        manifest,
        extension,
        inline_summary_bytes,
        extension_source,
    })
}

fn manifest_extension_region<'a>(
    manifest_bytes: &'a [u8],
    manifest: &CovmFile,
) -> Result<&'a [u8], String> {
    let entries_start = usize::try_from(manifest.postscript.entries_offset)
        .map_err(|_| "COVM entries offset out of range".to_string())?;
    let entries_len = usize::try_from(manifest.postscript.entries_len)
        .map_err(|_| "COVM entries length out of range".to_string())?;
    let entries_end = entries_start
        .checked_add(entries_len)
        .ok_or_else(|| "COVM entries range overflows".to_string())?;
    let postscript_total = COVM_POSTSCRIPT_LEN as usize + COVM_POSTSCRIPT_TAIL_SIZE;
    if manifest_bytes.len() < postscript_total {
        return Err("COVM file too short for postscript".into());
    }
    let postscript_start = manifest_bytes.len() - postscript_total;
    if entries_end > postscript_start {
        return Err("COVM entries overlap postscript".into());
    }
    Ok(&manifest_bytes[entries_end..postscript_start])
}

fn covm_delta_extension_encoded_len(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN {
        return Err("COVM delta-chain extension header is truncated".into());
    }
    let ordered_delta_count = read_u32_at(bytes, 184)? as usize;
    let chain_digest_len = read_u16_at(bytes, 206)? as usize;
    let chain_summary_digest_len = read_u16_at(bytes, 240)? as usize;
    COVM_DELTA_CHAIN_EXTENSION_HEADER_LEN
        .checked_add(
            ordered_delta_count
                .checked_mul(COVM_DELTA_ARTIFACT_REF_LEN)
                .ok_or_else(|| "COVM delta artifact ref length overflows".to_string())?,
        )
        .and_then(|len| len.checked_add(chain_digest_len))
        .and_then(|len| len.checked_add(chain_summary_digest_len))
        .ok_or_else(|| "COVM delta-chain extension length overflows".to_string())
}

fn read_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset + 2;
    if end > bytes.len() {
        return Err("COVM delta-chain extension header is truncated".into());
    }
    Ok(u16::from_le_bytes(bytes[offset..end].try_into().unwrap()))
}

fn read_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset + 4;
    if end > bytes.len() {
        return Err("COVM delta-chain extension header is truncated".into());
    }
    Ok(u32::from_le_bytes(bytes[offset..end].try_into().unwrap()))
}

fn load_selected_artifacts(
    context: &ManifestDeltaContext,
    extension: &CovmDeltaChainExtensionV1,
    dataset: &Path,
) -> Result<(ArtifactBytes, Vec<ArtifactBytes>), String> {
    let paths = artifact_uri_map(&context.manifest);
    let base = read_artifact_for_ref(dataset, &paths, &extension.base_artifact_ref, "base")?;
    let deltas = extension
        .ordered_delta_artifact_refs
        .iter()
        .map(|reference| read_artifact_for_ref(dataset, &paths, reference, "delta"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((base, deltas))
}

fn artifact_uri_map(manifest: &CovmFile) -> BTreeMap<[u8; 16], String> {
    manifest
        .files
        .iter()
        .map(|entry| (entry.file_id, entry.uri.clone()))
        .collect()
}

fn read_artifact_for_ref(
    dataset: &Path,
    paths: &BTreeMap<[u8; 16], String>,
    reference: &CovmDeltaArtifactRefV1,
    label: &str,
) -> Result<ArtifactBytes, String> {
    let uri = paths.get(&reference.artifact_id).ok_or_else(|| {
        format!(
            "manifest does not contain {label} artifact URI for {}",
            hex16(&reference.artifact_id)
        )
    })?;
    let path = dataset.join(uri);
    let bytes = read_file(&path)?;
    Ok(ArtifactBytes { path, bytes })
}

fn base_identity(path: &Path, bytes: &[u8]) -> Result<ArtifactBytesWithRef, String> {
    let validation = validate_bytes_with_options(
        bytes,
        ValidationOptions {
            semantic: true,
            verify_digests: false,
            ..ValidationOptions::default()
        },
    )
    .map_err(|error| format!("{} is not a valid base COVE file: {error}", path.display()))?;
    let digest = compute_digest(DigestAlgorithm::Sha256, bytes)
        .map_err(|error| format!("cannot digest {}: {error}", path.display()))?;
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    let file_id = validation.validated.header.file_id;
    let reference = CovmDeltaArtifactRefV1 {
        chain_ordinal: 0,
        flags: 0,
        artifact_id: file_id,
        snapshot_id: file_id,
        parent_snapshot_id: [0; 16],
        file_len: bytes.len() as u64,
        footer_crc32c: validation.validated.postscript.footer.crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: 0,
        checksum: 0,
    };
    Ok(ArtifactBytesWithRef {
        path: path.to_path_buf(),
        reference,
    })
}

fn cove_object_delta_parent_identity(
    reference: &CovmDeltaArtifactRefV1,
) -> CoveObjectDeltaParentIdentity {
    CoveObjectDeltaParentIdentity {
        artifact_id: reference.artifact_id,
        snapshot_id: reference.snapshot_id,
        file_len: reference.file_len,
        footer_crc32c: reference.footer_crc32c,
    }
}

#[derive(Debug, Clone)]
struct ArtifactBytesWithRef {
    path: PathBuf,
    reference: CovmDeltaArtifactRefV1,
}

fn delta_ref_from_file(
    file: &CoveDeltaFile,
    bytes: &[u8],
) -> Result<CovmDeltaArtifactRefV1, String> {
    let digest = compute_digest(DigestAlgorithm::Sha256, bytes)
        .map_err(|error| format!("cannot digest delta artifact: {error}"))?;
    let mut digest_array = [0u8; 32];
    digest_array.copy_from_slice(&digest);
    Ok(CovmDeltaArtifactRefV1 {
        chain_ordinal: file.header.chain_ordinal,
        flags: 0,
        artifact_id: file.header.delta_artifact_id,
        snapshot_id: file.header.snapshot_id,
        parent_snapshot_id: file.header.parent_snapshot_id,
        file_len: bytes.len() as u64,
        footer_crc32c: file.footer.footer_crc32c,
        digest_algorithm: DigestAlgorithm::Sha256 as u16,
        digest_len: 32,
        digest: digest_array,
        uri_ref: file.header.chain_ordinal,
        checksum: 0,
    })
}

fn validate_publish_chain(
    base: &ArtifactBytesWithRef,
    deltas: &[PublishedDelta],
) -> Result<[u8; 16], String> {
    let first = deltas
        .first()
        .ok_or_else(|| "delta publish requires at least one delta".to_string())?;
    let mut expected_parent_snapshot = first.file.header.parent_snapshot_id;
    let mut base_snapshot_id = None;
    let dataset_id = first.file.header.dataset_id;
    for (index, delta) in deltas.iter().enumerate() {
        let lineage = delta
            .file
            .parent_refs
            .iter()
            .find(|parent| parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0)
            .ok_or_else(|| format!("{} lacks a lineage parent reference", delta.path.display()))?;
        if index == 0 {
            let snapshot_id = validate_base_lineage_parent_matches_ref(lineage, &base.reference)?;
            if snapshot_id != delta.file.header.parent_snapshot_id {
                return Err(format!(
                    "{} lineage parent snapshot ID does not match delta header parent snapshot ID",
                    delta.path.display()
                ));
            }
            base_snapshot_id = Some(snapshot_id);
        } else {
            validate_lineage_parent_matches_ref(
                lineage,
                &deltas[index - 1].reference,
                "previous delta",
            )?;
        }

        let expected_ordinal =
            u32::try_from(index + 1).map_err(|_| "too many delta artifacts".to_string())?;
        if delta.file.header.chain_ordinal != expected_ordinal {
            return Err(format!(
                "{} has chain ordinal {}, expected {expected_ordinal}",
                delta.path.display(),
                delta.file.header.chain_ordinal
            ));
        }
        if delta.file.header.dataset_id != dataset_id {
            return Err(format!(
                "{} belongs to a different dataset",
                delta.path.display()
            ));
        }
        if delta.file.header.parent_snapshot_id != expected_parent_snapshot {
            return Err(format!(
                "{} does not extend the selected parent snapshot",
                delta.path.display()
            ));
        }
        expected_parent_snapshot = delta.file.header.snapshot_id;
    }
    base_snapshot_id.ok_or_else(|| "delta publish requires at least one delta".to_string())
}

fn validate_base_lineage_parent_matches_ref(
    parent: &DeltaParentRefV1,
    expected: &CovmDeltaArtifactRefV1,
) -> Result<[u8; 16], String> {
    if parent.digest_ref == DELTA_REF_NONE {
        return Err("base lineage parent must declare a digest reference".into());
    }
    if parent.file_len != expected.file_len {
        return Err("base file length does not match lineage parent".into());
    }
    if parent.footer_crc32c != expected.footer_crc32c {
        return Err("base footer CRC does not match lineage parent".into());
    }
    if parent.artifact_id != expected.artifact_id {
        return Err("base artifact ID does not match lineage parent".into());
    }
    if parent.digest_algorithm != expected.digest_algorithm
        || parent.digest_len != expected.digest_len
    {
        return Err("base digest metadata does not match lineage parent".into());
    }
    if parent.uri_ref != expected.uri_ref {
        return Err("base URI ref does not match lineage parent".into());
    }
    Ok(parent.snapshot_id)
}

fn validate_lineage_parent_matches_ref(
    parent: &DeltaParentRefV1,
    expected: &CovmDeltaArtifactRefV1,
    label: &str,
) -> Result<(), String> {
    if parent.digest_ref == DELTA_REF_NONE {
        return Err(format!(
            "{label} lineage parent must declare a digest reference"
        ));
    }
    if parent.file_len != expected.file_len {
        return Err(format!("{label} file length does not match lineage parent"));
    }
    if parent.footer_crc32c != expected.footer_crc32c {
        return Err(format!("{label} footer CRC does not match lineage parent"));
    }
    if parent.artifact_id != expected.artifact_id {
        return Err(format!("{label} artifact ID does not match lineage parent"));
    }
    if parent.snapshot_id != expected.snapshot_id {
        return Err(format!("{label} snapshot ID does not match lineage parent"));
    }
    if parent.digest_algorithm != expected.digest_algorithm
        || parent.digest_len != expected.digest_len
    {
        return Err(format!(
            "{label} digest metadata does not match lineage parent"
        ));
    }
    if parent.uri_ref != expected.uri_ref {
        return Err(format!("{label} URI ref does not match lineage parent"));
    }
    Ok(())
}

fn build_summary(
    extension: &CovmDeltaChainExtensionV1,
    deltas: &[PublishedDelta],
) -> Result<Vec<u8>, String> {
    let entries = deltas
        .iter()
        .map(|delta| {
            let mut time_flags = DELTA_SUMMARY_TIME_COMMIT_RANGE_PRESENT;
            let mut exactness_flags = 0u32;
            if delta.file.header.flags & DELTA_FLAG_SOURCE_PUBLISH_RANGE_PRESENT != 0 {
                time_flags |= DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT;
                exactness_flags |= DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_CONSERVATIVE
                    | DELTA_SUMMARY_EXACTNESS_SOURCE_PUBLISH_EXACT;
            }
            DeltaChainSummaryEntryV1 {
                chain_ordinal: delta.file.header.chain_ordinal,
                delta_artifact_ref: delta.reference.clone(),
                delta_artifact_id: delta.file.header.delta_artifact_id,
                required_delta_features: delta.file.header.required_delta_features,
                optional_delta_features: delta.file.header.optional_delta_features,
                csn_min: delta.file.header.csn_min,
                csn_max: delta.file.header.csn_max,
                commit_time_start_us: delta.file.header.commit_time_range_start_us,
                commit_time_end_us: delta.file.header.commit_time_range_end_us,
                artifact_created_at_us: delta.file.header.created_at_us,
                first_published_at_us: now_us(),
                selected_snapshot_published_at_us: now_us(),
                time_field_presence_flags: time_flags,
                time_summary_exactness_flags: exactness_flags,
                source_publish_range_start_us: if time_flags
                    & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT
                    != 0
                {
                    delta.file.header.source_publish_range_start_us
                } else {
                    0
                },
                source_publish_range_end_us: if time_flags
                    & DELTA_SUMMARY_TIME_SOURCE_PUBLISH_RANGE_PRESENT
                    != 0
                {
                    delta.file.header.source_publish_range_end_us
                } else {
                    0
                },
                scope_summary_ref: 0,
                branch_summary_ref: 0,
                object_type_summary_ref: 0,
                goid_range_summary_ref: 0,
                touched_summary_ref: 0,
                tombstone_summary_ref: 0,
                property_summary_ref: 0,
                temporal_role_summary_ref: 0,
                delta_header_range_offset: 0,
                delta_header_range_length: 0,
                hot_summary_range_offset: 0,
                hot_summary_range_length: 0,
                checksum: 0,
            }
        })
        .collect::<Vec<_>>();
    CovmDeltaChainSummaryV1::new(
        extension.dataset_id,
        extension.result_snapshot_id,
        extension.chain_digest_algorithm,
        extension.chain_digest.clone(),
        entries,
    )
    .serialize()
    .map_err(|error| format!("cannot build delta chain summary: {error}"))
}

fn extension_with_summary_binding(
    mut extension: CovmDeltaChainExtensionV1,
    summary_bytes: Option<&[u8]>,
) -> Result<CovmDeltaChainExtensionV1, String> {
    if let Some(summary_bytes) = summary_bytes {
        let digest = compute_digest(DigestAlgorithm::Sha256, summary_bytes)
            .map_err(|error| format!("cannot digest delta chain summary: {error}"))?;
        extension.chain_summary_kind = COVM_DELTA_CHAIN_SUMMARY_KIND_CDS1;
        extension.chain_summary_ref = 1;
        extension.chain_summary_offset = 0;
        extension.chain_summary_length = summary_bytes.len() as u64;
        extension.chain_summary_crc32c = checksum::crc32c(summary_bytes);
        extension.chain_summary_digest_algorithm = DigestAlgorithm::Sha256 as u16;
        extension.chain_summary_digest = digest;
    } else {
        extension.chain_summary_kind = COVM_DELTA_CHAIN_SUMMARY_KIND_NONE;
        extension.chain_summary_ref = 0;
        extension.chain_summary_offset = 0;
        extension.chain_summary_length = 0;
        extension.chain_summary_crc32c = 0;
        extension.chain_summary_digest_algorithm = DigestAlgorithm::None as u16;
        extension.chain_summary_digest.clear();
    }
    let bytes = extension
        .serialize()
        .map_err(|error| format!("cannot serialize delta chain extension: {error}"))?;
    CovmDeltaChainExtensionV1::parse(&bytes)
        .map_err(|error| format!("cannot validate delta chain extension: {error}"))
}

fn build_delta_covm_bytes(
    base: &ArtifactBytesWithRef,
    deltas: &[PublishedDelta],
    extension: &CovmDeltaChainExtensionV1,
    summary_bytes: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut files = Vec::with_capacity(deltas.len() + 1);
    files.push(covm_entry_from_ref(&base.reference, &base.path, 0, 0));
    for delta in deltas {
        files.push(covm_entry_from_ref(&delta.reference, &delta.path, 0, 0));
    }
    let header = CovmHeaderV1::new(
        extension.dataset_id,
        1,
        u32::try_from(files.len()).map_err(|_| "too many COVM file entries".to_string())?,
        now_us(),
    );
    let header_bytes = header.serialize();
    let mut entries_bytes = Vec::new();
    for entry in &files {
        entries_bytes.extend_from_slice(
            &entry
                .serialize()
                .map_err(|error| format!("cannot serialize COVM entry: {error}"))?,
        );
    }
    let extension_bytes = extension
        .serialize()
        .map_err(|error| format!("cannot serialize delta chain extension: {error}"))?;
    let mut extension_region = extension_bytes;
    if let Some(summary_bytes) = summary_bytes {
        extension_region.extend_from_slice(summary_bytes);
    }
    let header_offset = 0u64;
    let header_len = header_bytes.len() as u64;
    let entries_offset = header_len;
    let entries_len = entries_bytes.len() as u64;
    let postscript_total = COVM_POSTSCRIPT_LEN as u64 + COVM_POSTSCRIPT_TAIL_SIZE as u64;
    let file_len = entries_offset
        .saturating_add(entries_len)
        .saturating_add(extension_region.len() as u64)
        .saturating_add(postscript_total);
    let postscript = CovmPostscriptV1 {
        header_offset,
        header_len,
        entries_offset,
        entries_len,
        file_len,
        flags: COVM_POSTSCRIPT_FLAG_DELTA_CHAIN_REQUIRED,
        checksum: 0,
    };
    let mut out = Vec::with_capacity(file_len as usize);
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&entries_bytes);
    out.extend_from_slice(&extension_region);
    out.extend_from_slice(&postscript.serialize_tail());
    Ok(out)
}

fn covm_entry_from_ref(
    reference: &CovmDeltaArtifactRefV1,
    path: &Path,
    row_count: u64,
    segment_count: u32,
) -> CovmFileEntryV1 {
    CovmFileEntryV1 {
        file_id: reference.artifact_id,
        uri: path.display().to_string(),
        file_len: reference.file_len,
        footer_crc32c: reference.footer_crc32c,
        digest_algorithm: reference.digest_algorithm,
        digest: reference.digest[..reference.digest_len as usize].to_vec(),
        row_count,
        segment_count,
        file_stats_ref: 0,
        file_exact_set_ref: 0,
        flags: 0,
    }
}

fn covedelta_json(
    path: &Path,
    file: &CoveDeltaFile,
    object_validation: Option<&cove_core::artifact::covedelta::CoveDeltaObjectValidation>,
) -> Value {
    json!({
        "path": path.display().to_string(),
        "artifact": "covedelta",
        "version_major": file.header.version_major,
        "version_minor": file.header.version_minor,
        "delta_artifact_id": hex16(&file.header.delta_artifact_id),
        "dataset_id": hex16(&file.header.dataset_id),
        "snapshot_id": hex16(&file.header.snapshot_id),
        "parent_snapshot_id": hex16(&file.header.parent_snapshot_id),
        "chain_ordinal": file.header.chain_ordinal,
        "chain_depth": file.header.chain_depth,
        "csn_min": file.header.csn_min,
        "csn_max": file.header.csn_max,
        "commit_time_range_start_us": file.header.commit_time_range_start_us,
        "commit_time_range_end_us": file.header.commit_time_range_end_us,
        "source_publish_range_start_us": file.header.source_publish_range_start_us,
        "source_publish_range_end_us": file.header.source_publish_range_end_us,
        "required_delta_features": feature_bits_json(file.header.required_delta_features),
        "optional_delta_features": feature_bits_json(file.header.optional_delta_features),
        "parent_refs": file.parent_refs.iter().map(parent_ref_json).collect::<Vec<_>>(),
        "sections": file.sections.iter().map(|section| json!({
            "section_id": section.entry.section_id,
            "section_kind": section.entry.section_kind,
            "section_kind_name": section_kind_name(section.entry.section_kind),
            "offset": section.entry.offset,
            "length": section.entry.length,
            "item_count": section.entry.item_count,
            "required_delta_features": feature_bits_json(section.entry.required_delta_features),
            "optional_delta_features": feature_bits_json(section.entry.optional_delta_features),
        })).collect::<Vec<_>>(),
        "object_delta": object_validation.map(|validation| json!({
            "valid": true,
            "temporal_segments": validation.temporal_segments.len(),
            "sparse_patch_rows": validation.sparse_patch_records.len(),
            "checkpoint_row_count": validation.checkpoint_row_count,
            "touched_object_ranges": validation.touched_object_ranges.len(),
            "tombstone_object_ranges": validation.tombstone_object_ranges.len(),
            "dictionary_overlay_entries": validation.dictionary_overlay_entries.len(),
            "catalog_patches": validation.catalog_patches.len(),
            "evidence_patches": validation.evidence_patches.len(),
            "projection_patches": validation.projection_patches.len(),
            "index_hints": validation.index_hints.len(),
            "coverage_patches": validation.coverage_patches.len(),
        })),
    })
}

fn parent_ref_json(parent: &cove_core::artifact::covedelta::DeltaParentRefV1) -> Value {
    json!({
        "parent_ref": parent.parent_ref,
        "parent_kind": parent.parent_kind,
        "lineage_parent": parent.flags & DELTA_PARENT_REF_LINEAGE_PARENT != 0,
        "artifact_id": hex16(&parent.artifact_id),
        "snapshot_id": hex16(&parent.snapshot_id),
        "file_len": parent.file_len,
        "footer_crc32c": parent.footer_crc32c,
        "digest_algorithm": parent.digest_algorithm,
        "digest_len": parent.digest_len,
        "digest_ref": parent.digest_ref,
        "uri_ref": parent.uri_ref,
    })
}

fn extension_json(extension: &CovmDeltaChainExtensionV1) -> Value {
    json!({
        "profile_id": extension.delta_chain_profile_id,
        "profile_version_major": extension.delta_chain_profile_version_major,
        "profile_version_minor": extension.delta_chain_profile_version_minor,
        "required_delta_features": feature_bits_json(extension.required_delta_features),
        "optional_delta_features": feature_bits_json(extension.optional_delta_features),
        "dataset_id": hex16(&extension.dataset_id),
        "base_snapshot_id": hex16(&extension.base_snapshot_id),
        "result_snapshot_id": hex16(&extension.result_snapshot_id),
        "base_artifact_ref": artifact_ref_json(&extension.base_artifact_ref),
        "ordered_delta_artifact_refs": extension.ordered_delta_artifact_refs.iter().map(artifact_ref_json).collect::<Vec<_>>(),
        "chain_digest_algorithm": extension.chain_digest_algorithm,
        "chain_digest": hex_encode(&extension.chain_digest),
        "chain_summary_kind": extension.chain_summary_kind,
        "chain_summary_length": extension.chain_summary_length,
        "chain_summary_crc32c": extension.chain_summary_crc32c,
        "chain_summary_digest_algorithm": extension.chain_summary_digest_algorithm,
        "chain_summary_digest": hex_encode(&extension.chain_summary_digest),
        "effective_schema_fingerprint_ref": extension.effective_schema_fingerprint_ref,
        "effective_object_catalog_fingerprint_ref": extension.effective_object_catalog_fingerprint_ref,
        "effective_projection_fingerprint_ref": extension.effective_projection_fingerprint_ref,
        "effective_semantic_map_fingerprint_ref": extension.effective_semantic_map_fingerprint_ref,
        "effective_visibility_fingerprint_ref": extension.effective_visibility_fingerprint_ref,
        "effective_redaction_fingerprint_ref": extension.effective_redaction_fingerprint_ref,
        "csn_min": extension.csn_min,
        "csn_max": extension.csn_max,
        "created_at_us": extension.created_at_us,
    })
}

fn artifact_ref_json(reference: &CovmDeltaArtifactRefV1) -> Value {
    json!({
        "chain_ordinal": reference.chain_ordinal,
        "artifact_id": hex16(&reference.artifact_id),
        "snapshot_id": hex16(&reference.snapshot_id),
        "parent_snapshot_id": hex16(&reference.parent_snapshot_id),
        "file_len": reference.file_len,
        "footer_crc32c": reference.footer_crc32c,
        "digest_algorithm": reference.digest_algorithm,
        "digest_len": reference.digest_len,
        "digest": hex_encode(&reference.digest[..reference.digest_len as usize]),
        "uri_ref": reference.uri_ref,
    })
}

fn print_extension_text(extension: &CovmDeltaChainExtensionV1) {
    println!("  dataset_id: {}", hex16(&extension.dataset_id));
    println!("  base_snapshot_id: {}", hex16(&extension.base_snapshot_id));
    println!(
        "  result_snapshot_id: {}",
        hex16(&extension.result_snapshot_id)
    );
    println!(
        "  ordered_delta_count: {}",
        extension.ordered_delta_artifact_refs.len()
    );
    println!("  chain_digest: {}", hex_encode(&extension.chain_digest));
    println!("  chain_summary_kind: {}", extension.chain_summary_kind);
    println!("  chain_summary_length: {}", extension.chain_summary_length);
    println!("  csn_range: {}..{}", extension.csn_min, extension.csn_max);
    println!(
        "  required_delta_features: {}",
        format_feature_bits(extension.required_delta_features)
    );
    println!(
        "  optional_delta_features: {}",
        format_feature_bits(extension.optional_delta_features)
    );
    println!(
        "  base_artifact: {}",
        hex16(&extension.base_artifact_ref.artifact_id)
    );
    for reference in &extension.ordered_delta_artifact_refs {
        println!(
            "  delta[{}]: artifact={} snapshot={} parent={} bytes={}",
            reference.chain_ordinal,
            hex16(&reference.artifact_id),
            hex16(&reference.snapshot_id),
            hex16(&reference.parent_snapshot_id),
            reference.file_len
        );
    }
}

fn read_amplification_json(
    metrics: cove_core::artifact::covm::CovmDeltaReadAmplificationMetrics,
) -> Value {
    json!({
        "delta_chain_depth": metrics.delta_chain_depth,
        "chain_summary_bytes": metrics.chain_summary_bytes,
        "chain_summary_range_requests": metrics.chain_summary_range_requests,
        "selected_delta_count": metrics.selected_delta_count,
        "skipped_delta_count": metrics.skipped_delta_count,
        "delta_artifacts_opened": metrics.delta_artifacts_opened,
        "delta_artifacts_skipped_before_open": metrics.delta_artifacts_skipped_before_open,
        "base_ranges_requested": metrics.base_ranges_requested,
        "delta_ranges_requested": metrics.delta_ranges_requested,
        "object_store_request_count": metrics.object_store_request_count,
        "bytes_returned": metrics.bytes_returned,
        "base_file_bytes": metrics.base_file_bytes,
        "total_delta_bytes": metrics.total_delta_bytes,
        "source_publish_range_prunes": metrics.source_publish_range_prunes,
        "commit_time_range_prunes": metrics.commit_time_range_prunes,
    })
}

fn selected_delta_bytes(extension: &CovmDeltaChainExtensionV1, selected: &[u32]) -> u64 {
    extension
        .ordered_delta_artifact_refs
        .iter()
        .filter(|reference| selected.contains(&reference.chain_ordinal))
        .map(|reference| reference.file_len)
        .sum()
}

fn artifact_graph_json(
    names: &BTreeMap<[u8; 16], String>,
    reference: &CovmDeltaArtifactRefV1,
) -> Value {
    json!({
        "uri": artifact_label(names, reference),
        "chain_ordinal": reference.chain_ordinal,
        "artifact_id": hex16(&reference.artifact_id),
        "snapshot_id": hex16(&reference.snapshot_id),
        "parent_snapshot_id": hex16(&reference.parent_snapshot_id),
    })
}

fn artifact_label(
    names: &BTreeMap<[u8; 16], String>,
    reference: &CovmDeltaArtifactRefV1,
) -> String {
    names
        .get(&reference.artifact_id)
        .cloned()
        .unwrap_or_else(|| hex16(&reference.artifact_id))
}

fn feature_bits_json(bits: u64) -> Value {
    json!({
        "raw": bits,
        "hex": format!("0x{bits:016x}"),
        "names": delta_feature_names(bits),
    })
}

fn format_feature_bits(bits: u64) -> String {
    let names = delta_feature_names(bits);
    if names.is_empty() {
        format!("0x{bits:016x}")
    } else {
        format!("0x{bits:016x} ({})", names.join(","))
    }
}

fn delta_feature_names(bits: u64) -> Vec<&'static str> {
    let features = [
        (DELTA_FEATURE_SPARSE_PATCH_ROWS, "sparse_patch_rows"),
        (DELTA_FEATURE_OBJECT_TOMBSTONES, "object_tombstones"),
        (DELTA_FEATURE_PROPERTY_TOMBSTONES, "property_tombstones"),
        (
            DELTA_FEATURE_ASSOCIATION_TOMBSTONES,
            "association_tombstones",
        ),
        (DELTA_FEATURE_CONTINUATION_ANCHORS, "continuation_anchors"),
        (DELTA_FEATURE_INLINE_DICTIONARY, "inline_dictionary"),
        (
            DELTA_FEATURE_PARENT_DICTIONARY_ALIASES,
            "parent_dictionary_aliases",
        ),
        (DELTA_FEATURE_EXACT_TOUCHED_SET, "exact_touched_set"),
        (DELTA_FEATURE_EXACT_TOMBSTONE_SET, "exact_tombstone_set"),
        (DELTA_FEATURE_CHECKPOINT_BASELINES, "checkpoint_baselines"),
        (DELTA_FEATURE_COVERAGE_PATCH, "coverage_patch"),
        (DELTA_FEATURE_INDEX_HINTS, "index_hints"),
        (DELTA_FEATURE_MAP_EVIDENCE_PATCH, "map_evidence_patch"),
        (DELTA_FEATURE_PROJECTION_PATCH, "projection_patch"),
        (
            DELTA_FEATURE_HISTORICAL_COMMIT_INSERT,
            "historical_commit_insert",
        ),
    ];
    features
        .iter()
        .filter_map(|(bit, name)| if bits & *bit != 0 { Some(*name) } else { None })
        .collect()
}

fn section_kind_name(raw: u16) -> &'static str {
    match CoveDeltaSectionKind::from_u16(raw) {
        Some(CoveDeltaSectionKind::ParentRefs) => "parent_refs",
        Some(CoveDeltaSectionKind::CatalogPatch) => "catalog_patch",
        Some(CoveDeltaSectionKind::DictionaryOverlay) => "dictionary_overlay",
        Some(CoveDeltaSectionKind::TemporalSegmentIndex) => "temporal_segment_index",
        Some(CoveDeltaSectionKind::TemporalSegmentData) => "temporal_segment_data",
        Some(CoveDeltaSectionKind::ContinuationAnchors) => "continuation_anchors",
        Some(CoveDeltaSectionKind::TouchedObjectSet) => "touched_object_set",
        Some(CoveDeltaSectionKind::TombstoneSet) => "tombstone_set",
        Some(CoveDeltaSectionKind::PropertyOps) => "property_ops",
        Some(CoveDeltaSectionKind::EvidencePatch) => "evidence_patch",
        Some(CoveDeltaSectionKind::ProjectionPatch) => "projection_patch",
        Some(CoveDeltaSectionKind::CoveragePatch) => "coverage_patch",
        Some(CoveDeltaSectionKind::IndexHints) => "index_hints",
        Some(CoveDeltaSectionKind::LayoutHints) => "layout_hints",
        Some(CoveDeltaSectionKind::TrustContinuation) => "trust_continuation",
        Some(CoveDeltaSectionKind::StringTable) => "string_table",
        Some(CoveDeltaSectionKind::BranchIdentityTable) => "branch_identity_table",
        Some(CoveDeltaSectionKind::ScopeTable) => "scope_table",
        Some(CoveDeltaSectionKind::TemporalRoleSummaryTable) => "temporal_role_summary_table",
        Some(CoveDeltaSectionKind::TouchedSummaryTable) => "touched_summary_table",
        Some(CoveDeltaSectionKind::TombstoneSummaryTable) => "tombstone_summary_table",
        Some(CoveDeltaSectionKind::StateHashTable) => "state_hash_table",
        Some(CoveDeltaSectionKind::Extension) => "extension",
        None => "unknown",
    }
}

fn find_delta_section<'a>(
    file: &'a CoveDeltaFile,
    raw: &str,
) -> Result<&'a cove_core::artifact::covedelta::CoveDeltaSection, String> {
    if let Ok(id) = raw.parse::<u32>() {
        return file
            .sections
            .iter()
            .find(|section| section.entry.section_id == id)
            .ok_or_else(|| format!("section id {id} not found"));
    }
    file.sections
        .iter()
        .find(|section| section_kind_name(section.entry.section_kind) == raw)
        .ok_or_else(|| format!("section kind '{raw}' not found"))
}

fn prune_reason_name(reason: CovmDeltaPruneReason) -> &'static str {
    match reason {
        CovmDeltaPruneReason::AsOfCsnBeforeDelta => "as_of_csn_before_delta",
        CovmDeltaPruneReason::AsOfCommitBeforeDelta => "as_of_commit_before_delta",
        CovmDeltaPruneReason::SourcePublishRangeOutsideDelta => {
            "source_publish_range_outside_delta"
        }
    }
}

fn recommendation_name(recommendation: CovmDeltaReadAmplificationRecommendation) -> &'static str {
    match recommendation {
        CovmDeltaReadAmplificationRecommendation::WarnChainDepth => "WarnChainDepth",
        CovmDeltaReadAmplificationRecommendation::RequireOverrideChainDepth => {
            "RequireOverrideChainDepth"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendCheckpoint => "RecommendCheckpoint",
        CovmDeltaReadAmplificationRecommendation::RecommendCompaction => "RecommendCompaction",
        CovmDeltaReadAmplificationRecommendation::RecommendSnapshotLevelIndex => {
            "RecommendSnapshotLevelIndex"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendSummaryHoistingOrCompaction => {
            "RecommendSummaryHoistingOrCompaction"
        }
        CovmDeltaReadAmplificationRecommendation::RecommendPackingSmallDeltas => {
            "RecommendPackingSmallDeltas"
        }
    }
}

fn parse_positive_usize(value: &str, flag: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{flag} requires a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{flag} requires a positive integer"));
    }
    Ok(parsed)
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

fn parse_i64_range(value: &str) -> Result<(i64, i64), String> {
    let (start, end) = value
        .split_once(':')
        .ok_or_else(|| "--source-publish-range requires start:end".to_string())?;
    Ok((
        parse_i64(start, "--source-publish-range")?,
        parse_i64(end, "--source-publish-range")?,
    ))
}

fn hex16(bytes: &[u8; 16]) -> String {
    hex_encode(bytes)
}

fn print_hex(bytes: &[u8]) {
    for (idx, chunk) in bytes.chunks(16).enumerate() {
        print!("{:08x}  ", idx * 16);
        for byte in chunk {
            print!("{byte:02x} ");
        }
        println!();
    }
}

fn dot_node_id(bytes: &[u8; 16]) -> String {
    format!("n{}", hex16(bytes))
}

fn dot_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
