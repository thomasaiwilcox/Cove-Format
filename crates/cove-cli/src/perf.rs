use std::path::PathBuf;

use cove_datafusion::explain::{
    explain_pruning, parse_filter_dsl, parse_projection_dsl, parse_topn_dsl, plan_cost,
    ExplainOptions,
};

pub(crate) fn run_explain_pruning(args: Vec<String>) -> Result<(), String> {
    let Some((input, options)) = parse_explain_args(args)? else {
        print_explain_pruning_usage();
        return Ok(());
    };
    let report = explain_pruning(&input, options).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&report.to_json_value())
        .map_err(|error| format!("cannot serialize report: {error}"))?;
    println!("{json}");
    Ok(())
}

pub(crate) fn run_plan_cost(args: Vec<String>) -> Result<(), String> {
    let Some((input, options, execute)) = parse_plan_cost_args(args)? else {
        print_plan_cost_usage();
        return Ok(());
    };
    let report = plan_cost(&input, options, execute).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&report.to_json_value())
        .map_err(|error| format!("cannot serialize report: {error}"))?;
    println!("{json}");
    Ok(())
}

fn parse_explain_args(args: Vec<String>) -> Result<Option<(PathBuf, ExplainOptions)>, String> {
    parse_common_args(args, false).map(|parsed| {
        parsed.map(
            |ParsedPerfArgs {
                 input,
                 options,
                 execute: _,
             }| (input, options),
        )
    })
}

fn parse_plan_cost_args(
    args: Vec<String>,
) -> Result<Option<(PathBuf, ExplainOptions, bool)>, String> {
    parse_common_args(args, true).map(|parsed| {
        parsed.map(
            |ParsedPerfArgs {
                 input,
                 options,
                 execute,
             }| (input, options, execute),
        )
    })
}

struct ParsedPerfArgs {
    input: PathBuf,
    options: ExplainOptions,
    execute: bool,
}

fn parse_common_args(
    args: Vec<String>,
    allow_execute: bool,
) -> Result<Option<ParsedPerfArgs>, String> {
    let mut options = ExplainOptions::default();
    let mut execute = false;
    let mut input = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--execute" if allow_execute => execute = true,
            "--columns" | "--projection" => {
                let raw = next_value(&mut iter, &arg)?;
                options.projection = Some(parse_projection_dsl(&raw));
            }
            "--filter" => {
                let raw = next_value(&mut iter, "--filter")?;
                options
                    .filters
                    .push(parse_filter_dsl(&raw).map_err(|error| error.to_string())?);
            }
            "--top-n" => {
                let raw = next_value(&mut iter, "--top-n")?;
                options.top_n = Some(parse_topn_dsl(&raw).map_err(|error| error.to_string())?);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => {
                if input.replace(PathBuf::from(arg)).is_some() {
                    return Err("expected a single <input.cove>".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "expected <input.cove>".to_string())?;
    Ok(Some(ParsedPerfArgs {
        input,
        options,
        execute,
    }))
}

fn next_value(iter: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn print_explain_pruning_usage() {
    eprintln!(
        "usage: cove perf explain-pruning [--columns a,b] [--filter column=<name|index>,op=<eq|lt|lte|gt|gte|is-null|is-not-null>,value=<literal>] [--top-n column=<name|index>,fetch=<n>,desc=<bool>] <input.cove>"
    );
}

fn print_plan_cost_usage() {
    eprintln!(
        "usage: cove perf plan-cost [--execute] [--columns a,b] [--filter column=<name|index>,op=<eq|lt|lte|gt|gte|is-null|is-not-null>,value=<literal>] [--top-n column=<name|index>,fetch=<n>,desc=<bool>] <input.cove>"
    );
}
