use super::*;

#[test]
fn parses_validate_command() {
    assert_eq!(
        parse_args(["validate".to_string(), "mapping.covemap".to_string()])
            .unwrap()
            .unwrap(),
        Command::Validate {
            map: PathBuf::from("mapping.covemap")
        }
    );
}

#[test]
fn parses_convert_cove_o_format() {
    let command = parse_args([
        "convert".to_string(),
        "--format".to_string(),
        "cove-o".to_string(),
        "-o".to_string(),
        "out.cove".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        command,
        Command::Convert {
            map: PathBuf::from("mapping.covemap"),
            sources: vec![PathBuf::from("source.jsonl")],
            output: Some(PathBuf::from("out.cove")),
            format: OutputFormat::CoveO,
        }
    );
}

#[test]
fn parses_project_cove_o_command() {
    let command = parse_args([
        "project-cove-o".to_string(),
        "--mapping".to_string(),
        "mapping.covemap".to_string(),
        "-o".to_string(),
        "projection.json".to_string(),
        "object.cove".to_string(),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        command,
        Command::ProjectCoveO {
            object: PathBuf::from("object.cove"),
            mapping: Some(PathBuf::from("mapping.covemap")),
            output: Some(PathBuf::from("projection.json")),
            format: ProjectionFormat::Json,
            projection_id: None,
        }
    );
}

#[test]
fn parses_build_command_defaults() {
    let command = parse_args([
        "build".to_string(),
        "--out-dir".to_string(),
        "bundle".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        command,
        Command::Build {
            map: PathBuf::from("mapping.covemap"),
            sources: vec![PathBuf::from("source.jsonl")],
            out_dir: PathBuf::from("bundle"),
            force: false,
            json: false,
            object_name: None,
            projection_output: MapBuildProjectionOutput::CoveT,
            evidence_encoding: MapEvidenceEncoding::Compact,
            section_compression: MapBuildSectionCompression::Zstd,
            verify: false,
            publish_covm: false,
        }
    );
}

#[test]
fn parses_build_command_options() {
    let command = parse_args([
        "build".to_string(),
        "--out-dir".to_string(),
        "bundle".to_string(),
        "--force".to_string(),
        "--json".to_string(),
        "--verify".to_string(),
        "--publish-covm".to_string(),
        "--object-name".to_string(),
        "people.cove".to_string(),
        "--projection-output".to_string(),
        "none".to_string(),
        "--evidence-encoding".to_string(),
        "expanded".to_string(),
        "--section-compression".to_string(),
        "none".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        command,
        Command::Build {
            map: PathBuf::from("mapping.covemap"),
            sources: vec![PathBuf::from("source.jsonl")],
            out_dir: PathBuf::from("bundle"),
            force: true,
            json: true,
            object_name: Some("people.cove".into()),
            projection_output: MapBuildProjectionOutput::None,
            evidence_encoding: MapEvidenceEncoding::Expanded,
            section_compression: MapBuildSectionCompression::None,
            verify: true,
            publish_covm: true,
        }
    );
    let err = parse_args([
        "build".to_string(),
        "--out-dir".to_string(),
        "bundle".to_string(),
        "--evidence-encoding".to_string(),
        "json".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap_err();
    assert!(err.contains("--evidence-encoding"));
    let err = parse_args([
        "build".to_string(),
        "--out-dir".to_string(),
        "bundle".to_string(),
        "--section-compression".to_string(),
        "brotli".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap_err();
    assert!(err.contains("--section-compression"));
    assert_eq!(
        parse_args([
            "publish".to_string(),
            "--bundle-dir".to_string(),
            "bundle".to_string(),
            "--out".to_string(),
            "dataset.covm".to_string(),
            "--force".to_string(),
            "--json".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Publish {
            bundle_dir: PathBuf::from("bundle"),
            output: PathBuf::from("dataset.covm"),
            force: true,
            json: true,
        }
    );
    assert_eq!(
        parse_args([
            "candidates".to_string(),
            "--out".to_string(),
            "candidates.json".to_string(),
            "mapping.covemap".to_string(),
            "suppliers.csv".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Candidates {
            map: PathBuf::from("mapping.covemap"),
            sources: vec![PathBuf::from("suppliers.csv")],
            output: Some(PathBuf::from("candidates.json")),
        }
    );
    assert_eq!(
        parse_args([
            "review".to_string(),
            "--out".to_string(),
            "reviewed.json".to_string(),
            "candidates.json".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Review {
            candidates: PathBuf::from("candidates.json"),
            output: Some(PathBuf::from("reviewed.json")),
        }
    );
    assert_eq!(
        parse_args([
            "review".to_string(),
            "import".to_string(),
            "mapping.covemap".to_string(),
            "reviewed.json".to_string(),
            "--out".to_string(),
            "mapping-reviewed.covemap".to_string(),
            "--replace".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::ReviewImport {
            map: PathBuf::from("mapping.covemap"),
            review: PathBuf::from("reviewed.json"),
            output: PathBuf::from("mapping-reviewed.covemap"),
            replace: true,
        }
    );
    assert_eq!(
        parse_args([
            "review".to_string(),
            "export".to_string(),
            "mapping-reviewed.covemap".to_string(),
            "--out".to_string(),
            "reviewed-export.json".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::ReviewExport {
            map: PathBuf::from("mapping-reviewed.covemap"),
            output: Some(PathBuf::from("reviewed-export.json")),
        }
    );
    assert_eq!(
        parse_args([
            "aliases".to_string(),
            "import".to_string(),
            "mapping.covemap".to_string(),
            "aliases.csv".to_string(),
            "--catalog-id".to_string(),
            "company_aliases".to_string(),
            "--resolver-id".to_string(),
            "uk_company_name_resolver".to_string(),
            "--out".to_string(),
            "mapping-with-aliases.covemap".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::AliasesImport {
            map: PathBuf::from("mapping.covemap"),
            aliases: PathBuf::from("aliases.csv"),
            catalog_id: "company_aliases".into(),
            resolver_id: "uk_company_name_resolver".into(),
            output: PathBuf::from("mapping-with-aliases.covemap"),
        }
    );
    assert_eq!(
        parse_args([
            "replay".to_string(),
            "verify".to_string(),
            "mapping.covemap".to_string(),
            "conversion-report.json".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::ReplayVerify {
            map: PathBuf::from("mapping.covemap"),
            report: PathBuf::from("conversion-report.json"),
        }
    );
}

#[test]
fn run_cli_reports_typed_usage_error_with_stable_text() {
    let error = run_cli(["unknown".to_string()]).unwrap_err();
    assert!(matches!(error, MapCliError::Usage(_)));
    assert_eq!(error.to_string(), "unknown subcommand unknown");
}

#[test]
fn parses_doctor_suggest_and_parity_commands() {
    assert_eq!(
        parse_args([
            "doctor".to_string(),
            "--json".to_string(),
            "--strict".to_string(),
            "--bundle-dir".to_string(),
            "bundle".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Doctor {
            bundle_dir: Some(PathBuf::from("bundle")),
            map: None,
            sources: Vec::new(),
            json: true,
            strict: true,
        }
    );
    assert_eq!(
        parse_args([
            "suggest".to_string(),
            "--json".to_string(),
            "--out".to_string(),
            "suggestions.json".to_string(),
            "people.csv".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Suggest {
            sources: vec![PathBuf::from("people.csv")],
            output: Some(PathBuf::from("suggestions.json")),
            json: true,
        }
    );
    assert_eq!(
        parse_args([
            "parity".to_string(),
            "--json".to_string(),
            "--projection-id".to_string(),
            "people.v1".to_string(),
            "--expected".to_string(),
            "expected.csv".to_string(),
            "--key".to_string(),
            "id,name".to_string(),
            "mapping.covemap".to_string(),
            "people.csv".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::Parity {
            map: PathBuf::from("mapping.covemap"),
            sources: vec![PathBuf::from("people.csv")],
            options: ParityOptions {
                projection_id: "people.v1".into(),
                expected: PathBuf::from("expected.csv"),
                expected_query: None,
                key: vec!["id".into(), "name".into()],
            },
            json: true,
        }
    );
    assert_eq!(
        parse_args([
            "parity-cove-o".to_string(),
            "--projection-id".to_string(),
            "people.v1".to_string(),
            "--expected".to_string(),
            "expected.csv".to_string(),
            "--expected-query".to_string(),
            r#"where(status == "open")"#.to_string(),
            "object.cove".to_string(),
        ])
        .unwrap()
        .unwrap(),
        Command::ParityCoveO {
            object: PathBuf::from("object.cove"),
            options: ParityOptions {
                projection_id: "people.v1".into(),
                expected: PathBuf::from("expected.csv"),
                expected_query: Some(r#"where(status == "open")"#.into()),
                key: Vec::new(),
            },
            json: false,
        }
    );
    assert!(parse_args([
        "doctor".to_string(),
        "--bundle-dir".to_string(),
        "bundle".to_string(),
        "mapping.covemap".to_string(),
    ])
    .unwrap_err()
    .contains("either --bundle-dir"));
    assert!(parse_args([
        "parity-cove-o".to_string(),
        "--expected".to_string(),
        "expected.csv".to_string(),
        "object.cove".to_string(),
    ])
    .unwrap_err()
    .contains("--projection-id"));
}

#[test]
fn rejects_build_without_out_dir_or_bad_projection_output() {
    assert!(parse_args([
        "build".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap_err()
    .contains("--out-dir"));
    assert!(parse_args([
        "build".to_string(),
        "--out-dir".to_string(),
        "bundle".to_string(),
        "--projection-output".to_string(),
        "parquet".to_string(),
        "mapping.covemap".to_string(),
        "source.jsonl".to_string(),
    ])
    .unwrap_err()
    .contains("cove-t or none"));
}
