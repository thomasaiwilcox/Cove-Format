use std::path::PathBuf;

use serde_json::json;

use crate::{
    alias_import::{import_aliases_from_paths, AliasImportOptions},
    review::{
        export_reviewed_decisions, import_reviewed_decisions_from_paths, ReviewImportOptions,
    },
};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Validate {
        map: PathBuf,
    },
    Preview {
        map: PathBuf,
    },
    PlanKeys {
        map: PathBuf,
        sources: Vec<PathBuf>,
    },
    Candidates {
        map: PathBuf,
        sources: Vec<PathBuf>,
        output: Option<PathBuf>,
    },
    Review {
        candidates: PathBuf,
        output: Option<PathBuf>,
    },
    ReviewExport {
        map: PathBuf,
        output: Option<PathBuf>,
    },
    ReviewImport {
        map: PathBuf,
        review: PathBuf,
        output: PathBuf,
        replace: bool,
    },
    AliasesImport {
        map: PathBuf,
        aliases: PathBuf,
        catalog_id: String,
        resolver_id: String,
        output: PathBuf,
    },
    ReplayVerify {
        map: PathBuf,
        report: PathBuf,
    },
    Convert {
        map: PathBuf,
        sources: Vec<PathBuf>,
        output: Option<PathBuf>,
        format: OutputFormat,
    },
    Explain {
        map: PathBuf,
        id: String,
    },
    Diff {
        left: PathBuf,
        right: PathBuf,
    },
    Project {
        map: PathBuf,
        sources: Vec<PathBuf>,
        output: Option<PathBuf>,
        format: ProjectionFormat,
        projection_id: Option<String>,
    },
    ProjectCoveO {
        object: PathBuf,
        mapping: Option<PathBuf>,
        output: Option<PathBuf>,
        format: ProjectionFormat,
        projection_id: Option<String>,
    },
    Build {
        map: PathBuf,
        sources: Vec<PathBuf>,
        out_dir: PathBuf,
        force: bool,
        json: bool,
        object_name: Option<String>,
        projection_output: MapBuildProjectionOutput,
        evidence_encoding: MapEvidenceEncoding,
        section_compression: MapBuildSectionCompression,
        verify: bool,
        publish_covm: bool,
    },
    Publish {
        bundle_dir: PathBuf,
        output: PathBuf,
        force: bool,
        json: bool,
    },
    Doctor {
        bundle_dir: Option<PathBuf>,
        map: Option<PathBuf>,
        sources: Vec<PathBuf>,
        json: bool,
        strict: bool,
    },
    Suggest {
        sources: Vec<PathBuf>,
        output: Option<PathBuf>,
        json: bool,
    },
    Parity {
        map: PathBuf,
        sources: Vec<PathBuf>,
        options: ParityOptions,
        json: bool,
    },
    ParityCoveO {
        object: PathBuf,
        options: ParityOptions,
        json: bool,
    },
    Test {
        fixture: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Json,
    CoveO,
    Arrow,
    CoveT,
    Sql,
}

pub fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let Some(command) = parse_args(args)? else {
        print_usage();
        return Ok(());
    };
    match command {
        Command::Validate { map } => {
            parse_map(&map)?;
            println!("{}", json!({"ok": true, "path": map.display().to_string()}));
        }
        Command::Preview { map } => {
            let file = parse_map(&map)?;
            print_json(&preview(&file));
        }
        Command::PlanKeys { map, sources } => {
            let file = parse_map(&map)?;
            let inputs = read_source_inputs(&sources)?;
            validate_source_inputs(&file, &inputs.states)?;
            print_json(&plan_keys(&file, &inputs.rows));
        }
        Command::Candidates {
            map,
            sources,
            output,
        } => {
            let file = parse_map(&map)?;
            let inputs = read_source_inputs(&sources)?;
            validate_source_inputs(&file, &inputs.states)?;
            let candidates = candidate_matches(&file, &inputs.rows)?;
            write_or_print(output, &candidates)?;
        }
        Command::Review { candidates, output } => {
            let bytes = std::fs::read(&candidates)
                .map_err(|err| format!("cannot read {}: {err}", candidates.display()))?;
            let candidates_json: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|err| format!("invalid candidate review input JSON: {err}"))?;
            let worklist = review_worklist_from_candidate_matches(&candidates_json)?;
            write_or_print(output, &worklist)?;
        }
        Command::ReviewExport { map, output } => {
            let file = parse_map(&map)?;
            let review = export_reviewed_decisions(&file)?;
            write_or_print(output, &review)?;
        }
        Command::ReviewImport {
            map,
            review,
            output,
            replace,
        } => {
            let report = import_reviewed_decisions_from_paths(
                &map,
                &review,
                &output,
                &ReviewImportOptions { replace },
            )?;
            print_json(&report);
        }
        Command::AliasesImport {
            map,
            aliases,
            catalog_id,
            resolver_id,
            output,
        } => {
            let report = import_aliases_from_paths(
                &map,
                &aliases,
                &output,
                &AliasImportOptions {
                    catalog_id,
                    resolver_id,
                },
            )?;
            print_json(&report);
        }
        Command::ReplayVerify { map, report } => {
            let file = parse_map(&map)?;
            let bytes = std::fs::read(&report)
                .map_err(|err| format!("cannot read {}: {err}", report.display()))?;
            let report_json: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|err| format!("invalid replay report JSON: {err}"))?;
            print_json(&verify_replay_report(&file, &report_json)?);
        }
        Command::Convert {
            map,
            sources,
            output,
            format,
        } => {
            let file = parse_map(&map)?;
            let inputs = read_source_inputs(&sources)?;
            validate_source_inputs(&file, &inputs.states)?;
            match format {
                OutputFormat::Json => {
                    let materialized =
                        materialize_with_source_states(&file, &inputs.rows, &inputs.states)?;
                    write_or_print(output, &materialized.conversion_report)?;
                }
                OutputFormat::CoveO => {
                    let output = output.ok_or_else(|| {
                        "convert --format cove-o requires --output <path>".to_string()
                    })?;
                    let bytes =
                        build_cove_o_with_source_states(&file, &inputs.rows, &inputs.states)?;
                    durable::durable_replace(&output, &bytes).map_err(|err| {
                        format!("cannot durably publish {}: {err}", output.display())
                    })?;
                }
                OutputFormat::Arrow | OutputFormat::CoveT | OutputFormat::Sql => {
                    return Err("convert supports --format json or cove-o only".into())
                }
            }
        }
        Command::Explain { map, id } => {
            let file = parse_map(&map)?;
            print_json(&explain(&file, &id)?);
        }
        Command::Diff { left, right } => {
            let left = parse_map(&left)?;
            let right = parse_map(&right)?;
            print_json(&diff_maps(&left, &right));
        }
        Command::Project {
            map,
            sources,
            output,
            format,
            projection_id,
        } => {
            let file = parse_map(&map)?;
            let inputs = read_source_inputs(&sources)?;
            validate_source_inputs(&file, &inputs.states)?;
            let projected = project_rows_with_source_states_output(
                &file,
                &inputs.rows,
                &inputs.states,
                format,
                projection_id.as_deref(),
            )?;
            write_projection_output(output, format, &projected)?;
        }
        Command::ProjectCoveO {
            object,
            mapping,
            output,
            format,
            projection_id,
        } => {
            let projected = project_cove_o_path_output(
                &object,
                mapping.as_deref(),
                format,
                projection_id.as_deref(),
            )?;
            write_projection_output(output, format, &projected)?;
        }
        Command::Build {
            map,
            sources,
            out_dir,
            force,
            json,
            object_name,
            projection_output,
            evidence_encoding,
            section_compression,
            verify,
            publish_covm,
        } => {
            let result = build_from_paths(
                &map,
                &sources,
                MapBuildOptions {
                    out_dir,
                    force,
                    object_name,
                    projection_output,
                    evidence_encoding,
                    section_compression,
                    verify,
                    publish_covm,
                    reuse_cache: true,
                },
            )?;
            if json {
                print_json(&result.manifest);
            } else {
                print_build_summary(&result.manifest);
            }
        }
        Command::Publish {
            bundle_dir,
            output,
            force,
            json,
        } => {
            let report = publish_covm_from_bundle(&bundle_dir, &output, force)?;
            if json {
                print_json(&report);
            } else {
                println!("COVE-MAP publish: wrote {}", output.display());
            }
        }
        Command::Doctor {
            bundle_dir,
            map,
            sources,
            json,
            strict,
        } => {
            let report = match (bundle_dir, map) {
                (Some(bundle_dir), None) => verify_bundle_dir(&bundle_dir)?,
                (None, Some(map)) => verify_from_paths(&map, &sources)?,
                (Some(_), Some(_)) => {
                    return Err(
                        "doctor accepts either --bundle-dir or <mapping.covemap> <source...>"
                            .into(),
                    )
                }
                (None, None) => {
                    return Err(
                        "doctor requires --bundle-dir or <mapping.covemap> <source...>".into(),
                    )
                }
            };
            if json {
                print_json(&report);
            } else {
                print_doctor_summary(&report);
            }
            if report_has_failures(&report, strict) {
                return Err("map doctor found validation failures".into());
            }
        }
        Command::Suggest {
            sources,
            output,
            json: _,
        } => {
            let suggestions = suggest_from_paths(&sources)?;
            write_or_print(output, &suggestions)?;
        }
        Command::Parity {
            map,
            sources,
            options,
            json,
        } => {
            let report = parity_from_paths(&map, &sources, &options)?;
            if json {
                print_json(&report);
            } else {
                print_parity_summary(&report);
            }
            if parity_has_failures(&report) {
                return Err("map parity found differences".into());
            }
        }
        Command::ParityCoveO {
            object,
            options,
            json,
        } => {
            let report = parity_from_cove_o_path(&object, &options)?;
            if json {
                print_json(&report);
            } else {
                print_parity_summary(&report);
            }
            if parity_has_failures(&report) {
                return Err("map parity found differences".into());
            }
        }
        Command::Test { fixture } => run_fixture_path(&fixture)?,
    }
    Ok(())
}

pub(crate) fn parse_args(
    args: impl IntoIterator<Item = String>,
) -> Result<Option<Command>, String> {
    let mut args = args.into_iter();
    let Some(subcommand) = args.next() else {
        return Ok(None);
    };
    if subcommand == "-h" || subcommand == "--help" {
        return Ok(None);
    }
    let command = match subcommand.as_str() {
        "validate" => Command::Validate {
            map: one_path(&mut args, "validate <mapping.covemap>")?,
        },
        "preview" => Command::Preview {
            map: one_path(&mut args, "preview <mapping.covemap>")?,
        },
        "plan-keys" => {
            let map = one_path(&mut args, "plan-keys <mapping.covemap> <source...>")?;
            Command::PlanKeys {
                map,
                sources: args.map(PathBuf::from).collect(),
            }
        }
        "candidates" => {
            let (output, positional) = parse_output_and_positionals(args)?;
            let mut positional = positional.into_iter();
            let map = positional
                .next()
                .ok_or_else(|| "candidates requires <mapping.covemap>".to_string())?;
            let sources = positional.collect::<Vec<_>>();
            if sources.is_empty() {
                return Err("candidates requires at least one source path".into());
            }
            Command::Candidates {
                map,
                sources,
                output,
            }
        }
        "review" => parse_review_args(args)?,
        "aliases" => parse_aliases_args(args)?,
        "replay" => parse_replay_args(args)?,
        "convert" => {
            let (output, format, positional) = parse_output_format_and_positionals(args)?;
            let mut positional = positional.into_iter();
            let map = positional
                .next()
                .ok_or_else(|| "convert requires <mapping.covemap>".to_string())?;
            Command::Convert {
                map,
                sources: positional.collect(),
                output,
                format,
            }
        }
        "explain" => {
            let map = one_path(&mut args, "explain <mapping.covemap> <goid|assertion-id>")?;
            let id = args
                .next()
                .ok_or_else(|| "explain requires an id".to_string())?;
            Command::Explain { map, id }
        }
        "diff" => Command::Diff {
            left: one_path(&mut args, "diff <left.covemap> <right.covemap>")?,
            right: one_path(&mut args, "diff <left.covemap> <right.covemap>")?,
        },
        "project" => {
            let (output, format, projection_id, positional) =
                parse_output_format_projection_and_positionals(args)?;
            let mut positional = positional.into_iter();
            let map = positional
                .next()
                .ok_or_else(|| "project requires <mapping.covemap>".to_string())?;
            Command::Project {
                map,
                sources: positional.collect(),
                output,
                format: project_format(format)?,
                projection_id,
            }
        }
        "project-cove-o" => {
            let (object, mapping, output, format, projection_id) = parse_project_cove_o_args(args)?;
            Command::ProjectCoveO {
                object,
                mapping,
                output,
                format,
                projection_id,
            }
        }
        "build" => {
            let (
                out_dir,
                force,
                json,
                object_name,
                projection_output,
                evidence_encoding,
                section_compression,
                verify,
                publish_covm,
                positional,
            ) = parse_build_args(args)?;
            let mut positional = positional.into_iter();
            let map = positional
                .next()
                .ok_or_else(|| "build requires <mapping.covemap>".to_string())?;
            let sources = positional.collect::<Vec<_>>();
            if sources.is_empty() {
                return Err("build requires at least one source path".into());
            }
            Command::Build {
                map,
                sources,
                out_dir,
                force,
                json,
                object_name,
                projection_output,
                evidence_encoding,
                section_compression,
                verify,
                publish_covm,
            }
        }
        "publish" => {
            let (bundle_dir, output, force, json) = parse_publish_args(args)?;
            Command::Publish {
                bundle_dir,
                output,
                force,
                json,
            }
        }
        "doctor" => {
            let (bundle_dir, json, strict, positional) = parse_doctor_args(args)?;
            let mut positional = positional.into_iter();
            let map = positional.next();
            let sources = positional.collect::<Vec<_>>();
            if bundle_dir.is_none() && map.is_some() && sources.is_empty() {
                return Err("doctor with a mapping requires at least one source path".into());
            }
            Command::Doctor {
                bundle_dir,
                map,
                sources,
                json,
                strict,
            }
        }
        "suggest" => {
            let (json, output, sources) = parse_suggest_args(args)?;
            if sources.is_empty() {
                return Err("suggest requires at least one source path".into());
            }
            Command::Suggest {
                sources,
                output,
                json,
            }
        }
        "parity" => {
            let (json, options, positional) = parse_parity_args(args)?;
            let mut positional = positional.into_iter();
            let map = positional
                .next()
                .ok_or_else(|| "parity requires <mapping.covemap>".to_string())?;
            let sources = positional.collect::<Vec<_>>();
            if sources.is_empty() {
                return Err("parity requires at least one source path".into());
            }
            Command::Parity {
                map,
                sources,
                options,
                json,
            }
        }
        "parity-cove-o" => {
            let (json, options, positional) = parse_parity_args(args)?;
            if positional.len() != 1 {
                return Err("parity-cove-o requires exactly one <object.cove>".into());
            }
            Command::ParityCoveO {
                object: positional[0].clone(),
                options,
                json,
            }
        }
        "test" => Command::Test {
            fixture: one_path(&mut args, "test <fixture.json>")?,
        },
        _ => return Err(format!("unknown subcommand {subcommand}")),
    };
    Ok(Some(command))
}

fn one_path(args: &mut impl Iterator<Item = String>, usage: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("usage: cove map {usage}"))
}

fn parse_output_and_positionals(
    args: impl Iterator<Item = String>,
) -> Result<(Option<PathBuf>, Vec<PathBuf>), String> {
    let mut output = None;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "--output" | "-o" => {
                output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("{arg} requires a path"))?,
                );
            }
            _ if arg.starts_with("--out=") => {
                output = Some(PathBuf::from(arg.trim_start_matches("--out=")));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    Ok((output, positional))
}

fn parse_review_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let collected = args.collect::<Vec<_>>();
    if collected.first().is_some_and(|arg| arg == "export") {
        return parse_review_export_args(collected.into_iter().skip(1));
    }
    if collected.first().is_some_and(|arg| arg == "import") {
        return parse_review_import_args(collected.into_iter().skip(1));
    }

    let (output, positional) = parse_output_and_positionals(collected.into_iter())?;
    if positional.len() != 1 {
        return Err("review requires <candidate-matches.json>".into());
    }
    Ok(Command::Review {
        candidates: positional[0].clone(),
        output,
    })
}

fn parse_review_export_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let (output, positional) = parse_output_and_positionals(args)?;
    if positional.len() != 1 {
        return Err("review export requires <mapping.covemap>".into());
    }
    Ok(Command::ReviewExport {
        map: positional[0].clone(),
        output,
    })
}

fn parse_review_import_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut output = None;
    let mut replace = false;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "--output" | "-o" => {
                output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("{arg} requires a path"))?,
                );
            }
            "--replace" => replace = true,
            _ if arg.starts_with("--out=") => {
                output = Some(PathBuf::from(arg.trim_start_matches("--out=")));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if positional.len() != 2 {
        return Err(
            "review import requires <mapping.covemap> <reviewed.json> --out <mapping.covemap>"
                .into(),
        );
    }
    Ok(Command::ReviewImport {
        map: positional[0].clone(),
        review: positional[1].clone(),
        output: output
            .ok_or_else(|| "review import requires --out <mapping.covemap>".to_string())?,
        replace,
    })
}

fn parse_aliases_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let subcommand = args
        .next()
        .ok_or_else(|| "aliases requires a subcommand: import".to_string())?;
    if subcommand != "import" {
        return Err("aliases supports only: import".into());
    }

    let mut output = None;
    let mut catalog_id = None;
    let mut resolver_id = None;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "--output" | "-o" => {
                output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("{arg} requires a path"))?,
                );
            }
            "--catalog-id" => {
                catalog_id = Some(
                    args.next()
                        .ok_or_else(|| "--catalog-id requires an id".to_string())?,
                );
            }
            "--resolver-id" => {
                resolver_id = Some(
                    args.next()
                        .ok_or_else(|| "--resolver-id requires an id".to_string())?,
                );
            }
            _ if arg.starts_with("--out=") => {
                output = Some(PathBuf::from(arg.trim_start_matches("--out=")));
            }
            _ if arg.starts_with("--catalog-id=") => {
                catalog_id = Some(arg.trim_start_matches("--catalog-id=").to_string());
            }
            _ if arg.starts_with("--resolver-id=") => {
                resolver_id = Some(arg.trim_start_matches("--resolver-id=").to_string());
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }

    if positional.len() != 2 {
        return Err(
            "aliases import requires <mapping.covemap> <aliases.csv> --catalog-id <id> --resolver-id <id> --out <mapping.covemap>"
                .into(),
        );
    }
    Ok(Command::AliasesImport {
        map: positional[0].clone(),
        aliases: positional[1].clone(),
        catalog_id: catalog_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "aliases import requires --catalog-id <id>".to_string())?,
        resolver_id: resolver_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "aliases import requires --resolver-id <id>".to_string())?,
        output: output
            .ok_or_else(|| "aliases import requires --out <mapping.covemap>".to_string())?,
    })
}

fn parse_replay_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let subcommand = args
        .next()
        .ok_or_else(|| "replay requires a subcommand: verify".to_string())?;
    if subcommand != "verify" {
        return Err("replay supports only: verify".into());
    }
    let positional = args.map(PathBuf::from).collect::<Vec<_>>();
    if positional.len() != 2 {
        return Err("replay verify requires <mapping.covemap> <conversion-report.json>".into());
    }
    Ok(Command::ReplayVerify {
        map: positional[0].clone(),
        report: positional[1].clone(),
    })
}

fn parse_output_format_and_positionals(
    args: impl Iterator<Item = String>,
) -> Result<(Option<PathBuf>, OutputFormat, Vec<PathBuf>), String> {
    let mut output = None;
    let mut format = OutputFormat::Json;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if arg == "--output" || arg == "-o" {
            output = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{arg} requires a path"))?,
            );
        } else if arg == "--format" {
            let raw = args
                .next()
                .ok_or_else(|| "--format requires json or cove-o".to_string())?;
            format = match raw.as_str() {
                "json" => OutputFormat::Json,
                "cove-o" => OutputFormat::CoveO,
                "arrow" => OutputFormat::Arrow,
                "cove-t" => OutputFormat::CoveT,
                "sql" => OutputFormat::Sql,
                _ => return Err("--format must be one of: json, cove-o, arrow, cove-t, sql".into()),
            };
        } else if arg.starts_with('-') {
            return Err(format!("unknown option {arg}"));
        } else {
            positional.push(PathBuf::from(arg));
        }
    }
    Ok((output, format, positional))
}

#[allow(clippy::type_complexity)]
fn parse_build_args(
    args: impl Iterator<Item = String>,
) -> Result<
    (
        PathBuf,
        bool,
        bool,
        Option<String>,
        MapBuildProjectionOutput,
        MapEvidenceEncoding,
        MapBuildSectionCompression,
        bool,
        bool,
        Vec<PathBuf>,
    ),
    String,
> {
    let mut out_dir = None;
    let mut force = false;
    let mut json = false;
    let mut object_name = None;
    let mut projection_output = MapBuildProjectionOutput::CoveT;
    let mut evidence_encoding = MapEvidenceEncoding::Compact;
    let mut section_compression = MapBuildSectionCompression::Zstd;
    let mut verify = false;
    let mut publish_covm = false;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out-dir" => {
                out_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| "--out-dir requires a path".to_string())?,
                );
            }
            "--force" => force = true,
            "--json" => json = true,
            "--verify" => verify = true,
            "--publish-covm" => publish_covm = true,
            "--object-name" => {
                object_name = Some(
                    args.next()
                        .ok_or_else(|| "--object-name requires a file name".to_string())?,
                );
            }
            "--projection-output" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--projection-output requires cove-t or none".to_string())?;
                projection_output = match raw.as_str() {
                    "cove-t" => MapBuildProjectionOutput::CoveT,
                    "none" => MapBuildProjectionOutput::None,
                    _ => return Err("--projection-output must be cove-t or none".into()),
                };
            }
            "--evidence-encoding" => {
                let raw = args.next().ok_or_else(|| {
                    "--evidence-encoding requires compact, expanded, or both".to_string()
                })?;
                evidence_encoding = match raw.as_str() {
                    "compact" => MapEvidenceEncoding::Compact,
                    "expanded" => MapEvidenceEncoding::Expanded,
                    "both" => MapEvidenceEncoding::Both,
                    _ => {
                        return Err("--evidence-encoding must be compact, expanded, or both".into())
                    }
                };
            }
            "--section-compression" => {
                let raw = args
                    .next()
                    .ok_or_else(|| "--section-compression requires zstd or none".to_string())?;
                section_compression = match raw.as_str() {
                    "zstd" => MapBuildSectionCompression::Zstd,
                    "none" => MapBuildSectionCompression::None,
                    _ => return Err("--section-compression must be zstd or none".into()),
                };
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let out_dir = out_dir.ok_or_else(|| "build requires --out-dir <dir>".to_string())?;
    Ok((
        out_dir,
        force,
        json,
        object_name,
        projection_output,
        evidence_encoding,
        section_compression,
        verify,
        publish_covm,
        positional,
    ))
}

fn parse_publish_args(
    args: impl Iterator<Item = String>,
) -> Result<(PathBuf, PathBuf, bool, bool), String> {
    let mut bundle_dir = None;
    let mut output = None;
    let mut force = false;
    let mut json = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bundle-dir" => {
                bundle_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| "--bundle-dir requires a path".to_string())?,
                );
            }
            "--out" | "-o" => {
                output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("{arg} requires a path"))?,
                );
            }
            "--force" => force = true,
            "--json" => json = true,
            _ if arg.starts_with("--bundle-dir=") => {
                bundle_dir = Some(PathBuf::from(arg.trim_start_matches("--bundle-dir=")));
            }
            _ if arg.starts_with("--out=") => {
                output = Some(PathBuf::from(arg.trim_start_matches("--out=")));
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => return Err("publish accepts --bundle-dir <dir> --out <dataset.covm>".into()),
        }
    }
    let bundle_dir = bundle_dir.ok_or_else(|| "publish requires --bundle-dir <dir>".to_string())?;
    let output = output.ok_or_else(|| "publish requires --out <dataset.covm>".to_string())?;
    Ok((bundle_dir, output, force, json))
}

#[allow(clippy::type_complexity)]
fn parse_doctor_args(
    args: impl Iterator<Item = String>,
) -> Result<(Option<PathBuf>, bool, bool, Vec<PathBuf>), String> {
    let mut bundle_dir = None;
    let mut json = false;
    let mut strict = false;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bundle-dir" => {
                bundle_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| "--bundle-dir requires a path".to_string())?,
                );
            }
            "--json" => json = true,
            "--strict" => strict = true,
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    if bundle_dir.is_some() && !positional.is_empty() {
        return Err("doctor accepts either --bundle-dir or positional mapping inputs".into());
    }
    Ok((bundle_dir, json, strict, positional))
}

fn parse_suggest_args(
    args: impl Iterator<Item = String>,
) -> Result<(bool, Option<PathBuf>, Vec<PathBuf>), String> {
    let mut json = false;
    let mut output = None;
    let mut sources = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--out" | "-o" => {
                output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("{arg} requires a path"))?,
                );
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => sources.push(PathBuf::from(arg)),
        }
    }
    Ok((json, output, sources))
}

#[allow(clippy::type_complexity)]
fn parse_parity_args(
    args: impl Iterator<Item = String>,
) -> Result<(bool, ParityOptions, Vec<PathBuf>), String> {
    let mut json = false;
    let mut projection_id = None;
    let mut expected = None;
    let mut expected_query = None;
    let mut key = Vec::new();
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--projection-id" => {
                projection_id = Some(
                    args.next()
                        .ok_or_else(|| "--projection-id requires an id".to_string())?,
                );
            }
            "--expected" => {
                expected = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| "--expected requires a path".to_string())?,
                );
            }
            "--expected-query" => {
                expected_query =
                    Some(args.next().ok_or_else(|| {
                        "--expected-query requires a CoveQL expression".to_string()
                    })?);
            }
            "--key" => {
                key = args
                    .next()
                    .ok_or_else(|| "--key requires comma-separated columns".to_string())?
                    .split(',')
                    .map(str::trim)
                    .filter(|column| !column.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    Ok((
        json,
        ParityOptions {
            projection_id: projection_id
                .ok_or_else(|| "parity requires --projection-id <id>".to_string())?,
            expected: expected.ok_or_else(|| "parity requires --expected <table>".to_string())?,
            expected_query,
            key,
        },
        positional,
    ))
}

#[allow(clippy::type_complexity)]
fn parse_project_cove_o_args(
    args: impl Iterator<Item = String>,
) -> Result<
    (
        PathBuf,
        Option<PathBuf>,
        Option<PathBuf>,
        ProjectionFormat,
        Option<String>,
    ),
    String,
> {
    let mut output = None;
    let mut mapping = None;
    let mut format = ProjectionFormat::Json;
    let mut projection_id = None;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if arg == "--output" || arg == "-o" {
            output = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{arg} requires a path"))?,
            );
        } else if arg == "--mapping" {
            mapping = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--mapping requires a path".to_string())?,
            );
        } else if arg == "--format" {
            let raw = args.next().ok_or_else(|| {
                "--format requires json, cove-o, arrow, cove-t, or sql".to_string()
            })?;
            format = match raw.as_str() {
                "json" => ProjectionFormat::Json,
                "arrow" => ProjectionFormat::Arrow,
                "cove-t" => ProjectionFormat::CoveT,
                "sql" => ProjectionFormat::Sql,
                "cove-o" => ProjectionFormat::CoveO,
                _ => return Err("--format must be one of: json, cove-o, arrow, cove-t, sql".into()),
            };
        } else if arg == "--projection-id" {
            projection_id = Some(
                args.next()
                    .ok_or_else(|| "--projection-id requires an id".to_string())?,
            );
        } else if arg.starts_with('-') {
            return Err(format!("unknown option {arg}"));
        } else {
            positional.push(PathBuf::from(arg));
        }
    }
    if positional.len() != 1 {
        return Err("project-cove-o requires exactly one <object.cove>".into());
    }
    Ok((positional.remove(0), mapping, output, format, projection_id))
}

#[allow(clippy::type_complexity)]
fn parse_output_format_projection_and_positionals(
    args: impl Iterator<Item = String>,
) -> Result<(Option<PathBuf>, OutputFormat, Option<String>, Vec<PathBuf>), String> {
    let mut output = None;
    let mut format = OutputFormat::Json;
    let mut projection_id = None;
    let mut positional = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if arg == "--output" || arg == "-o" {
            output = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{arg} requires a path"))?,
            );
        } else if arg == "--format" {
            let raw = args.next().ok_or_else(|| {
                "--format requires json, cove-o, arrow, cove-t, or sql".to_string()
            })?;
            format = match raw.as_str() {
                "json" => OutputFormat::Json,
                "arrow" => OutputFormat::Arrow,
                "cove-t" => OutputFormat::CoveT,
                "sql" => OutputFormat::Sql,
                "cove-o" => OutputFormat::CoveO,
                _ => return Err("--format must be one of: json, cove-o, arrow, cove-t, sql".into()),
            };
        } else if arg == "--projection-id" {
            projection_id = Some(
                args.next()
                    .ok_or_else(|| "--projection-id requires an id".to_string())?,
            );
        } else if arg.starts_with('-') {
            return Err(format!("unknown option {arg}"));
        } else {
            positional.push(PathBuf::from(arg));
        }
    }
    Ok((output, format, projection_id, positional))
}

fn project_format(format: OutputFormat) -> Result<ProjectionFormat, String> {
    match format {
        OutputFormat::Json => Ok(ProjectionFormat::Json),
        OutputFormat::CoveO => Ok(ProjectionFormat::CoveO),
        OutputFormat::Arrow => Ok(ProjectionFormat::Arrow),
        OutputFormat::CoveT => Ok(ProjectionFormat::CoveT),
        OutputFormat::Sql => Ok(ProjectionFormat::Sql),
    }
}

fn write_projection_output(
    output: Option<PathBuf>,
    format: ProjectionFormat,
    bytes: &[u8],
) -> Result<(), String> {
    match output {
        Some(path) => durable::durable_replace(&path, bytes)
            .map(|_| ())
            .map_err(|err| format!("cannot durably publish {}: {err}", path.display())),
        None if matches!(format, ProjectionFormat::Json | ProjectionFormat::Sql) => {
            println!(
                "{}",
                std::str::from_utf8(bytes)
                    .map_err(|err| format!("projection JSON is not UTF-8: {err}"))?
            );
            Ok(())
        }
        None => Err(format!(
            "project --format {} requires --output <path>",
            format.as_str()
        )),
    }
}

fn print_build_summary(manifest: &serde_json::Value) {
    println!("COVE-MAP build complete");
    if let Some(object) = manifest
        .pointer("/artifacts/object/path")
        .and_then(serde_json::Value::as_str)
    {
        println!("  object: {object}");
    }
    if let Some(projections) = manifest
        .pointer("/artifacts/projections")
        .and_then(serde_json::Value::as_array)
    {
        println!("  projections: {}", projections.len());
        for projection in projections {
            if let (Some(id), Some(path)) = (
                projection
                    .get("projection_id")
                    .and_then(serde_json::Value::as_str),
                projection.get("path").and_then(serde_json::Value::as_str),
            ) {
                println!("    {id}: {path}");
            }
        }
    }
    if let Some(indexes) = manifest
        .pointer("/artifacts/indexes")
        .and_then(serde_json::Value::as_array)
    {
        println!("  indexes: {}", indexes.len());
        for index in indexes {
            if let (Some(id), Some(path)) = (
                index.get("index_id").and_then(serde_json::Value::as_str),
                index.get("path").and_then(serde_json::Value::as_str),
            ) {
                println!("    {id}: {path}");
            }
        }
    }
    if let Some(report) = manifest
        .pointer("/artifacts/report/path")
        .and_then(serde_json::Value::as_str)
    {
        println!("  report: {report}");
    }
    if let Some(path) = manifest
        .pointer("/artifacts/manifest/path")
        .and_then(serde_json::Value::as_str)
    {
        println!("  manifest: {path}");
    }
}

fn print_doctor_summary(report: &serde_json::Value) {
    println!(
        "COVE-MAP doctor: {}",
        report
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
    );
    if let Some(checks) = report.get("checks").and_then(serde_json::Value::as_array) {
        println!("  checks: {}", checks.len());
        for check in checks {
            let name = check
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("check");
            let ok = check
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            println!("    {name}: {}", if ok { "ok" } else { "failed" });
        }
    }
    if let Some(warnings) = report.get("warnings").and_then(serde_json::Value::as_array) {
        println!("  warnings: {}", warnings.len());
        for warning in warnings {
            if let Some(code) = warning.get("code").and_then(serde_json::Value::as_str) {
                println!("    {code}");
            }
        }
    }
    if let Some(errors) = report.get("errors").and_then(serde_json::Value::as_array) {
        println!("  errors: {}", errors.len());
        for error in errors {
            if let Some(code) = error.get("code").and_then(serde_json::Value::as_str) {
                println!("    {code}");
            }
        }
    }
}

fn print_parity_summary(report: &serde_json::Value) {
    println!(
        "COVE-MAP parity: {}",
        report
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
    );
    if let Some(diff) = report.get("diff") {
        println!(
            "  missing: {}",
            diff.get("missing_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  extra: {}",
            diff.get("extra_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  changed: {}",
            diff.get("changed_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
        println!(
            "  duplicate keys: {}",
            diff.get("duplicate_key_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        );
    }
}
