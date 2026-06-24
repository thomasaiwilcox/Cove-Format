use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use arrow_array::{ArrayRef, BooleanArray, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, SectionKind, DEFAULT_MORSEL_ROW_COUNT,
        FEATURE_SEMANTIC_MAP,
    },
    durable,
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use cove_map::{
    build_from_paths, projected_output_from_cove_o_path, projected_rows_from_cove_o_path,
    MapBuildOptions, ProjectionBatchOptions, ProjectionFormat,
};
use parquet::{arrow::ArrowWriter, file::properties::WriterProperties};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Customer360Profile {
    Quick,
    Standard,
    Publication,
}

impl Customer360Profile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "publication" => Ok(Self::Publication),
            other => Err(format!(
                "unknown customer360 profile '{other}'; expected quick, standard, or publication"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Publication => "publication",
        }
    }

    fn customer_count(self) -> usize {
        match self {
            Self::Quick => 12,
            Self::Standard => 4_096,
            Self::Publication => 65_536,
        }
    }

    fn event_count(self) -> usize {
        self.customer_count().saturating_mul(3)
    }
}

#[derive(Debug, Clone)]
pub struct Customer360Options {
    pub out_dir: PathBuf,
    pub profile: Customer360Profile,
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofSuiteScenario {
    Customer360,
    Claims,
    Catalog,
    All,
}

impl ProofSuiteScenario {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "customer360" => Ok(Self::Customer360),
            "claims" => Ok(Self::Claims),
            "catalog" => Ok(Self::Catalog),
            "all" => Ok(Self::All),
            other => Err(format!(
                "unknown proof-suite scenario '{other}'; expected customer360, claims, catalog, or all"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Customer360 => "customer360",
            Self::Claims => "claims",
            Self::Catalog => "catalog",
            Self::All => "all",
        }
    }

    fn selected(self) -> Vec<Self> {
        match self {
            Self::All => vec![Self::Customer360, Self::Claims, Self::Catalog],
            scenario => vec![scenario],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProofSuiteOptions {
    pub out_dir: PathBuf,
    pub profile: Customer360Profile,
    pub scenario: ProofSuiteScenario,
    pub force: bool,
}

pub fn generate_customer360(options: &Customer360Options) -> Result<Value, String> {
    prepare_output_dir(&options.out_dir, options.force)?;

    let profile = options.profile;
    let customers = profile.customer_count();
    let events = profile.event_count();

    let crm_path = options.out_dir.join("crm.csv");
    let support_path = options.out_dir.join("support.jsonl");
    let billing_path = options.out_dir.join("billing.parquet");
    let events_jsonl_path = options.out_dir.join("events.jsonl");
    let events_cove_path = options.out_dir.join("events.cove");
    let mapping_path = options.out_dir.join("customer360.covemap");
    let readback_source_path = options.out_dir.join("customers_360.jsonl");
    let readback_mapping_path = options.out_dir.join("customer360_readback.covemap");
    let mapped_path = options.out_dir.join("customers.cove");
    let customers_projection_path = options.out_dir.join("customers_projection.cove");
    let evidence_projection_path = options.out_dir.join("evidence_projection.cove");
    let customers_parquet_path = options.out_dir.join("customers_projection.parquet");
    let evidence_parquet_path = options.out_dir.join("evidence_projection.parquet");

    write_crm_csv(&crm_path, customers)?;
    write_support_jsonl(&support_path, customers)?;
    write_billing_parquet(&billing_path, customers)?;
    write_reconciled_jsonl(&readback_source_path, customers)?;
    write_events_jsonl(&events_jsonl_path, customers, events)?;
    durable::durable_replace(&events_cove_path, &events_cove_t(customers, events)?)
        .map_err(|err| format!("cannot write {}: {err}", events_cove_path.display()))?;

    let mapping = customer360_covemap();
    durable::durable_replace(
        &mapping_path,
        &mapping.serialize().map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", mapping_path.display()))?;

    let readback_mapping = customer360_readback_covemap();
    durable::durable_replace(
        &readback_mapping_path,
        &readback_mapping
            .serialize()
            .map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", readback_mapping_path.display()))?;

    let mapped_bytes = cove_map::cove_o_from_paths(
        &readback_mapping_path,
        std::slice::from_ref(&readback_source_path),
    )
    .map_err(|err| format!("cannot build Customer 360 COVE-O: {err}"))?;
    durable::durable_replace(&mapped_path, &mapped_bytes)
        .map_err(|err| format!("cannot write {}: {err}", mapped_path.display()))?;

    let customers_projection = cove_map::projected_output_from_cove_o_path(
        &mapped_path,
        None,
        ProjectionFormat::CoveT,
        Some("customer_360.v1"),
    )
    .map_err(|err| format!("cannot build customers COVE-T projection: {err}"))?;
    durable::durable_replace(&customers_projection_path, &customers_projection).map_err(|err| {
        format!(
            "cannot write {}: {err}",
            customers_projection_path.display()
        )
    })?;

    let evidence_projection = cove_map::projected_output_from_cove_o_path(
        &mapped_path,
        None,
        ProjectionFormat::CoveT,
        Some("customer_evidence.v1"),
    )
    .map_err(|err| format!("cannot build evidence COVE-T projection: {err}"))?;
    durable::durable_replace(&evidence_projection_path, &evidence_projection)
        .map_err(|err| format!("cannot write {}: {err}", evidence_projection_path.display()))?;

    write_projection_parquet(&mapped_bytes, "customer_360.v1", &customers_parquet_path)?;
    write_projection_parquet(
        &mapped_bytes,
        "customer_evidence.v1",
        &evidence_parquet_path,
    )?;

    let proof = write_customer360_proof_artifacts(
        &options.out_dir,
        profile,
        customers,
        &[crm_path.clone(), support_path.clone(), billing_path.clone()],
        &mapping_path,
    )?;

    let manifest = customer360_manifest(profile, customers, events, Some(proof));
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|err| err.to_string())?;
    manifest_bytes.push(b'\n');
    fs::write(
        options.out_dir.join("customer360-manifest.json"),
        manifest_bytes,
    )
    .map_err(|err| format!("cannot write customer360 manifest: {err}"))?;
    fs::write(options.out_dir.join("README.md"), customer360_readme())
        .map_err(|err| format!("cannot write customer360 README: {err}"))?;
    let notebooks_dir = options.out_dir.join("notebooks");
    fs::create_dir_all(&notebooks_dir)
        .map_err(|err| format!("cannot create {}: {err}", notebooks_dir.display()))?;
    fs::write(
        notebooks_dir.join("customer360_analysis.py"),
        customer360_notebook_script(),
    )
    .map_err(|err| format!("cannot write Customer 360 notebook script: {err}"))?;

    Ok(manifest)
}

pub fn generate_proof_suite(options: &ProofSuiteOptions) -> Result<Value, String> {
    prepare_output_dir(&options.out_dir, options.force)?;
    let mut scenarios = Vec::new();
    for scenario in options.scenario.selected() {
        let scenario_dir = options.out_dir.join(scenario.as_str());
        let manifest = match scenario {
            ProofSuiteScenario::Customer360 => generate_customer360(&Customer360Options {
                out_dir: scenario_dir,
                profile: options.profile,
                force: true,
            })?,
            ProofSuiteScenario::Claims => generate_claims_proof_scenario(
                &scenario_dir,
                options.profile,
                scenario_row_count(options.profile),
            )?,
            ProofSuiteScenario::Catalog => generate_catalog_proof_scenario(
                &scenario_dir,
                options.profile,
                scenario_row_count(options.profile),
            )?,
            ProofSuiteScenario::All => unreachable!("selected() expands all scenarios"),
        };
        scenarios.push(manifest);
    }
    let manifest = json!({
        "format": "cove-proof-suite-manifest-v1",
        "profile": options.profile.as_str(),
        "requested_scenario": options.scenario.as_str(),
        "scenarios": scenarios,
        "recommended_commands": [
            "cove map doctor --bundle-dir <scenario>/map-build-bundle",
            "cove query <scenario>/map-build-bundle/<object.cove> 'table(<projection>).take(10)'",
            "cargo run -p cove-bench -- check"
        ],
        "caveat": "The proof suite is deterministic generated evidence, not a claim of universal superiority over table formats."
    });
    write_json_pretty(
        &options.out_dir.join("proof-suite-manifest.json"),
        &manifest,
    )?;
    fs::write(options.out_dir.join("README.md"), proof_suite_readme())
        .map_err(|err| format!("cannot write proof-suite README: {err}"))?;
    Ok(manifest)
}

fn prepare_output_dir(out_dir: &Path, force: bool) -> Result<(), String> {
    if out_dir.exists() {
        let has_entries = fs::read_dir(out_dir)
            .map_err(|err| format!("cannot inspect {}: {err}", out_dir.display()))?
            .next()
            .transpose()
            .map_err(|err| format!("cannot inspect {}: {err}", out_dir.display()))?
            .is_some();
        if has_entries {
            if !force {
                return Err(format!(
                    "{} already exists and is not empty; pass --force to replace it",
                    out_dir.display()
                ));
            }
            fs::remove_dir_all(out_dir)
                .map_err(|err| format!("cannot replace {}: {err}", out_dir.display()))?;
        }
    }
    fs::create_dir_all(out_dir).map_err(|err| format!("cannot create {}: {err}", out_dir.display()))
}

fn write_crm_csv(path: &Path, rows: usize) -> Result<(), String> {
    let mut csv = String::from("id,full_name,region,tier\n");
    for i in 0..rows {
        let id = customer_id(i);
        let name = if i % 17 == 0 {
            String::new()
        } else {
            format!("Customer {i:06}")
        };
        let region = ["north", "south", "east", "west", "emea", "apac"][i % 6];
        let tier = ["bronze", "silver", "gold", "platinum"][i % 4];
        csv.push_str(&format!("{id},{name},{region},{tier}\n"));
    }
    fs::write(path, csv).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_support_jsonl(path: &Path, rows: usize) -> Result<(), String> {
    let mut out = String::new();
    for i in 0..rows {
        let value = json!({
            "id": customer_id(i),
            "active": i % 9 != 0,
            "score": ((i * 37) % 100) as i64,
            "status": if i % 13 == 0 { "dormant" } else if i % 5 == 0 { "watch" } else { "active" },
        });
        out.push_str(&serde_json::to_string(&value).map_err(|err| err.to_string())?);
        out.push('\n');
    }
    fs::write(path, out).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_billing_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let ids = (0..rows).map(customer_id).collect::<Vec<_>>();
    let plans = (0..rows)
        .map(|i| match i % 4 {
            0 => Some("free"),
            1 => Some("team"),
            2 => Some("business"),
            _ => Some("enterprise"),
        })
        .collect::<Vec<_>>();
    let mrr = (0..rows)
        .map(|i| {
            if i % 19 == 0 {
                None
            } else {
                Some(((i * 29) % 2_500) as i64)
            }
        })
        .collect::<Vec<_>>();
    let tiers = (0..rows)
        .map(|i| Some(["bronze", "silver", "gold", "platinum"][(i + 1) % 4]))
        .collect::<Vec<_>>();
    let billing_status = (0..rows)
        .map(|i| {
            if i % 11 == 0 {
                Some("past_due")
            } else {
                Some("current")
            }
        })
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("plan", Arc::new(StringArray::from(plans)) as ArrayRef),
        ("mrr", Arc::new(Int64Array::from(mrr)) as ArrayRef),
        ("tier", Arc::new(StringArray::from(tiers)) as ArrayRef),
        (
            "billing_status",
            Arc::new(StringArray::from(billing_status)) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_reconciled_jsonl(path: &Path, rows: usize) -> Result<(), String> {
    let mut out = String::new();
    for i in 0..rows {
        let region = ["north", "south", "east", "west", "emea", "apac"][i % 6];
        let tier = ["bronze", "silver", "gold", "platinum"][(i + 1) % 4];
        let status = if i % 13 == 0 {
            "dormant"
        } else if i % 5 == 0 {
            "watch"
        } else {
            "active"
        };
        let plan = ["free", "team", "business", "enterprise"][i % 4];
        let billing_status = if i % 11 == 0 { "past_due" } else { "current" };
        let value = json!({
            "id": customer_id(i),
            "customer_id": customer_id(i),
            "full_name": if i % 17 == 0 { Value::Null } else { Value::String(format!("Customer {i:06}")) },
            "region": region,
            "tier": tier,
            "active": i % 9 != 0,
            "score": ((i * 37) % 100) as i64,
            "status": status,
            "plan": plan,
            "mrr": if i % 19 == 0 { Value::Null } else { Value::Number(((i * 29) % 2_500).into()) },
            "billing_status": billing_status,
        });
        out.push_str(&serde_json::to_string(&value).map_err(|err| err.to_string())?);
        out.push('\n');
    }
    fs::write(path, out).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_events_jsonl(path: &Path, customers: usize, events: usize) -> Result<(), String> {
    let mut out = String::new();
    for i in 0..events {
        let event_kind = ["login", "ticket", "invoice", "upgrade", "downgrade"][i % 5];
        let value = json!({
            "event_id": i as i64 + 1,
            "customer_id": customer_id(i % customers),
            "event_kind": event_kind,
            "score": ((i * 17 + 11) % 100) as i64,
        });
        out.push_str(&serde_json::to_string(&value).map_err(|err| err.to_string())?);
        out.push('\n');
    }
    fs::write(path, out).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn events_cove_t(customers: usize, rows: usize) -> Result<Vec<u8>, String> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "customer360".into(),
            name: "events".into(),
            row_count: rows as u64,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "event_id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                ),
                column(
                    2,
                    "customer_id",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                ),
                column(
                    3,
                    "event_kind",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                ),
                column(
                    4,
                    "score",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                ),
            ],
        }],
    };
    let mut event_id_pages = Vec::new();
    let mut customer_id_pages = Vec::new();
    let mut event_kind_pages = Vec::new();
    let mut score_pages = Vec::new();
    let mut first_row = 0usize;
    while first_row < rows {
        let row_count = rows
            .saturating_sub(first_row)
            .min(DEFAULT_MORSEL_ROW_COUNT as usize);
        let mut event_ids = Vec::with_capacity(row_count.saturating_mul(8));
        let mut customer_ids = Vec::new();
        let mut event_kinds = Vec::new();
        let mut scores = Vec::with_capacity(row_count.saturating_mul(8));
        for i in first_row..first_row + row_count {
            event_ids.extend_from_slice(&((i + 1) as u64).to_le_bytes());
            push_varbytes(&mut customer_ids, &customer_id(i % customers));
            push_varbytes(
                &mut event_kinds,
                ["login", "ticket", "invoice", "upgrade", "downgrade"][i % 5],
            );
            scores.extend_from_slice(&(((i * 17 + 11) % 100) as u64).to_le_bytes());
        }
        let row_count_u32 = row_count as u32;
        event_id_pages.push(
            ScanPageSpec::new(row_count_u32, event_ids)
                .with_encoding_root(CoveEncodingKind::NumCode as u32),
        );
        customer_id_pages.push(
            ScanPageSpec::new(row_count_u32, customer_ids)
                .with_encoding_root(CoveEncodingKind::VarBytes as u32),
        );
        event_kind_pages.push(
            ScanPageSpec::new(row_count_u32, event_kinds)
                .with_encoding_root(CoveEncodingKind::VarBytes as u32),
        );
        score_pages.push(
            ScanPageSpec::new(row_count_u32, scores)
                .with_encoding_root(CoveEncodingKind::NumCode as u32),
        );
        first_row += row_count;
    }
    let mut segment = ScanSegment::new(1, 0, 0, rows as u32, 4);
    segment.set_column_pages(1, event_id_pages);
    segment.set_column_pages(2, customer_id_pages);
    segment.set_column_pages(3, event_kind_pages);
    segment.set_column_pages(4, score_pages);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().map_err(|err| err.to_string())
}

fn column(
    column_id: u32,
    name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
) -> ColumnEntry {
    ColumnEntry {
        column_id,
        name: name.into(),
        logical,
        physical,
        nullable: false,
        sort_order: 0,
        collation_id: 0,
        precision: 0,
        scale: 0,
        flags: 0,
    }
}

fn push_varbytes(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn write_projection_parquet(
    object_bytes: &[u8],
    projection_id: &str,
    path: &Path,
) -> Result<(), String> {
    let batches = cove_map::projected_record_batches_from_cove_o_bytes(
        object_bytes,
        None,
        projection_id,
        &ProjectionBatchOptions::default(),
    )
    .map_err(|err| format!("cannot build {projection_id} Arrow projection: {err}"))?;
    write_parquet_batches(path, &batches)
}

fn write_parquet_batches(path: &Path, batches: &[RecordBatch]) -> Result<(), String> {
    let Some(first) = batches.first() else {
        return Err(format!("cannot write {}: no batches", path.display()));
    };
    let file =
        fs::File::create(path).map_err(|err| format!("cannot create {}: {err}", path.display()))?;
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, first.schema(), Some(props))
        .map_err(|err| format!("cannot create Parquet writer {}: {err}", path.display()))?;
    for batch in batches {
        writer
            .write(batch)
            .map_err(|err| format!("cannot write Parquet batch {}: {err}", path.display()))?;
    }
    writer
        .close()
        .map_err(|err| format!("cannot close Parquet writer {}: {err}", path.display()))?;
    Ok(())
}

fn generate_claims_proof_scenario(
    root: &Path,
    profile: Customer360Profile,
    rows: usize,
) -> Result<Value, String> {
    fs::create_dir_all(root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let claims_csv = root.join("claims.csv");
    let events_jsonl = root.join("claim_events.jsonl");
    let providers_parquet = root.join("providers.parquet");
    let mapping_path = root.join("claims.covemap");
    write_claims_csv(&claims_csv, rows)?;
    write_claim_events_jsonl(&events_jsonl, rows)?;
    write_claim_providers_parquet(&providers_parquet, rows)?;
    durable::durable_replace(
        &mapping_path,
        &claims_covemap()
            .serialize()
            .map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", mapping_path.display()))?;

    let baseline_dir = root.join("proof-baselines");
    let source_parquet_dir = baseline_dir.join("source-parquet");
    fs::create_dir_all(&source_parquet_dir)
        .map_err(|err| format!("cannot create {}: {err}", source_parquet_dir.display()))?;
    let claims_parquet = source_parquet_dir.join("claims.parquet");
    let events_parquet = source_parquet_dir.join("claim_events.parquet");
    let providers_baseline_parquet = source_parquet_dir.join("providers.parquet");
    write_claims_summary_parquet(&claims_parquet, rows)?;
    write_claim_events_parquet(&events_parquet, rows)?;
    write_claim_providers_parquet(&providers_baseline_parquet, rows)?;

    let sources = vec![claims_csv, events_jsonl, providers_parquet];
    let out_dir = root.join("map-build-bundle");
    let build = run_proof_map_build(&mapping_path, &sources, &out_dir)?;
    let object_path = proof_object_path(&out_dir, &build.manifest)?;
    let object_bytes = fs::read(&object_path)
        .map_err(|err| format!("cannot read {}: {err}", object_path.display()))?;
    let denormalized_claims = baseline_dir.join("denormalized_claims.parquet");
    let denormalized_evidence = baseline_dir.join("denormalized_claim_evidence.parquet");
    write_projection_parquet(&object_bytes, "claims.v1", &denormalized_claims)?;
    write_projection_parquet(&object_bytes, "claim_evidence.v1", &denormalized_evidence)?;
    let parity_reports = write_projection_parity_reports(
        &root.join("parity"),
        &mapping_path,
        &sources,
        &object_path,
        &["claims.v1", "claim_evidence.v1"],
    )?;
    let doctor = build
        .report
        .get("verification")
        .cloned()
        .unwrap_or_else(|| json!({"status": "not_run"}));
    write_json_pretty(&root.join("doctor-report.json"), &doctor)?;
    let size_report = proof_size_report(ProofSizeReportInput {
        scenario: "claims",
        profile,
        sources: &sources,
        source_parquet: &[claims_parquet, events_parquet, providers_baseline_parquet],
        denormalized_parquet: &[denormalized_claims, denormalized_evidence],
        bundle_dir: &out_dir,
        manifest: &build.manifest,
        build_time_ns: build.elapsed_ns,
        doctor: &doctor,
        parity_reports: &parity_reports,
    })?;
    let manifest = json!({
        "format": "cove-proof-scenario-v1",
        "scenario": "claims",
        "profile": profile.as_str(),
        "row_counts": {"claims": rows, "claim_events": rows, "providers": rows},
        "artifacts": {
            "mapping": "claims.covemap",
            "map_build_bundle": "map-build-bundle",
            "doctor_report": "doctor-report.json",
            "parity_dir": "parity",
            "size_comparison": "proof-size-comparison.json"
        },
        "doctor_status": doctor.get("status").cloned().unwrap_or(Value::Null),
        "parity_status": aggregate_parity_status(&parity_reports),
        "size": size_report,
    });
    write_json_pretty(&root.join("proof-size-comparison.json"), &manifest["size"])?;
    write_json_pretty(&root.join("claims-manifest.json"), &manifest)?;
    fs::write(root.join("README.md"), scenario_readme("claims", "claims"))
        .map_err(|err| format!("cannot write claims README: {err}"))?;
    Ok(manifest)
}

fn generate_catalog_proof_scenario(
    root: &Path,
    profile: Customer360Profile,
    rows: usize,
) -> Result<Value, String> {
    fs::create_dir_all(root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let products_csv = root.join("products.csv");
    let prices_jsonl = root.join("vendor_prices.jsonl");
    let attributes_parquet = root.join("attributes.parquet");
    let mapping_path = root.join("catalog.covemap");
    write_products_csv(&products_csv, rows)?;
    write_vendor_prices_jsonl(&prices_jsonl, rows)?;
    write_product_attributes_parquet(&attributes_parquet, rows)?;
    durable::durable_replace(
        &mapping_path,
        &catalog_covemap()
            .serialize()
            .map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("cannot write {}: {err}", mapping_path.display()))?;

    let baseline_dir = root.join("proof-baselines");
    let source_parquet_dir = baseline_dir.join("source-parquet");
    fs::create_dir_all(&source_parquet_dir)
        .map_err(|err| format!("cannot create {}: {err}", source_parquet_dir.display()))?;
    let products_parquet = source_parquet_dir.join("products.parquet");
    let prices_parquet = source_parquet_dir.join("vendor_prices.parquet");
    let attributes_baseline_parquet = source_parquet_dir.join("attributes.parquet");
    write_products_parquet(&products_parquet, rows)?;
    write_vendor_prices_parquet(&prices_parquet, rows)?;
    write_product_attributes_parquet(&attributes_baseline_parquet, rows)?;

    let sources = vec![products_csv, prices_jsonl, attributes_parquet];
    let out_dir = root.join("map-build-bundle");
    let build = run_proof_map_build(&mapping_path, &sources, &out_dir)?;
    let object_path = proof_object_path(&out_dir, &build.manifest)?;
    let object_bytes = fs::read(&object_path)
        .map_err(|err| format!("cannot read {}: {err}", object_path.display()))?;
    let denormalized_products = baseline_dir.join("denormalized_products.parquet");
    let denormalized_evidence = baseline_dir.join("denormalized_product_evidence.parquet");
    write_projection_parquet(&object_bytes, "products.v1", &denormalized_products)?;
    write_projection_parquet(&object_bytes, "product_evidence.v1", &denormalized_evidence)?;
    let parity_reports = write_projection_parity_reports(
        &root.join("parity"),
        &mapping_path,
        &sources,
        &object_path,
        &["products.v1", "product_evidence.v1"],
    )?;
    let doctor = build
        .report
        .get("verification")
        .cloned()
        .unwrap_or_else(|| json!({"status": "not_run"}));
    write_json_pretty(&root.join("doctor-report.json"), &doctor)?;
    let size_report = proof_size_report(ProofSizeReportInput {
        scenario: "catalog",
        profile,
        sources: &sources,
        source_parquet: &[
            products_parquet,
            prices_parquet,
            attributes_baseline_parquet,
        ],
        denormalized_parquet: &[denormalized_products, denormalized_evidence],
        bundle_dir: &out_dir,
        manifest: &build.manifest,
        build_time_ns: build.elapsed_ns,
        doctor: &doctor,
        parity_reports: &parity_reports,
    })?;
    let manifest = json!({
        "format": "cove-proof-scenario-v1",
        "scenario": "catalog",
        "profile": profile.as_str(),
        "row_counts": {"products": rows, "vendor_prices": rows, "attributes": rows},
        "artifacts": {
            "mapping": "catalog.covemap",
            "map_build_bundle": "map-build-bundle",
            "doctor_report": "doctor-report.json",
            "parity_dir": "parity",
            "size_comparison": "proof-size-comparison.json"
        },
        "doctor_status": doctor.get("status").cloned().unwrap_or(Value::Null),
        "parity_status": aggregate_parity_status(&parity_reports),
        "size": size_report,
    });
    write_json_pretty(&root.join("proof-size-comparison.json"), &manifest["size"])?;
    write_json_pretty(&root.join("catalog-manifest.json"), &manifest)?;
    fs::write(
        root.join("README.md"),
        scenario_readme("catalog", "products"),
    )
    .map_err(|err| format!("cannot write catalog README: {err}"))?;
    Ok(manifest)
}

fn scenario_row_count(profile: Customer360Profile) -> usize {
    match profile {
        Customer360Profile::Quick => 24,
        Customer360Profile::Standard => 8_192,
        Customer360Profile::Publication => 131_072,
    }
}

fn write_customer360_proof_artifacts(
    root: &Path,
    profile: Customer360Profile,
    customers: usize,
    sources: &[PathBuf],
    mapping_path: &Path,
) -> Result<Value, String> {
    let baseline_dir = root.join("proof-baselines");
    fs::create_dir_all(&baseline_dir)
        .map_err(|err| format!("cannot create {}: {err}", baseline_dir.display()))?;
    let source_parquet_dir = baseline_dir.join("source-parquet");
    fs::create_dir_all(&source_parquet_dir)
        .map_err(|err| format!("cannot create {}: {err}", source_parquet_dir.display()))?;
    let crm_parquet = source_parquet_dir.join("crm.parquet");
    let support_parquet = source_parquet_dir.join("support.parquet");
    let billing_parquet = source_parquet_dir.join("billing.parquet");
    write_crm_parquet(&crm_parquet, customers)?;
    write_support_parquet(&support_parquet, customers)?;
    write_billing_parquet(&billing_parquet, customers)?;

    let out_dir = root.join("map-build-bundle");
    let build = run_proof_map_build(mapping_path, sources, &out_dir)?;
    let object_path = proof_object_path(&out_dir, &build.manifest)?;
    let denormalized_customers = baseline_dir.join("denormalized_customers.parquet");
    let denormalized_evidence = baseline_dir.join("denormalized_customer_evidence.parquet");
    let object_bytes = fs::read(&object_path)
        .map_err(|err| format!("cannot read {}: {err}", object_path.display()))?;
    write_projection_parquet(&object_bytes, "customer_360.v1", &denormalized_customers)?;
    write_projection_parquet(
        &object_bytes,
        "customer_evidence.v1",
        &denormalized_evidence,
    )?;

    let parity_dir = root.join("parity");
    let parity_reports = write_projection_parity_reports(
        &parity_dir,
        mapping_path,
        sources,
        &object_path,
        &["customer_360.v1", "customer_evidence.v1"],
    )?;
    let doctor_path = root.join("doctor-report.json");
    let doctor = build
        .report
        .get("verification")
        .cloned()
        .unwrap_or_else(|| json!({"status": "not_run"}));
    write_json_pretty(&doctor_path, &doctor)?;
    let size_report = proof_size_report(ProofSizeReportInput {
        scenario: "customer360",
        profile,
        sources,
        source_parquet: &[crm_parquet, support_parquet, billing_parquet],
        denormalized_parquet: &[denormalized_customers, denormalized_evidence],
        bundle_dir: &out_dir,
        manifest: &build.manifest,
        build_time_ns: build.elapsed_ns,
        doctor: &doctor,
        parity_reports: &parity_reports,
    })?;
    let size_path = root.join("proof-size-comparison.json");
    write_json_pretty(&size_path, &size_report)?;
    Ok(json!({
        "format": "cove-proof-scenario-v1",
        "scenario": "customer360",
        "profile": profile.as_str(),
        "row_counts": {"customers": customers},
        "artifacts": {
            "map_build_bundle": "map-build-bundle",
            "doctor_report": "doctor-report.json",
            "parity_dir": "parity",
            "size_comparison": "proof-size-comparison.json",
            "source_parquet_baselines": "proof-baselines/source-parquet"
        },
        "doctor_status": doctor.get("status").cloned().unwrap_or(Value::Null),
        "parity_status": aggregate_parity_status(&parity_reports),
        "size": size_report,
    }))
}

struct TimedBuild {
    manifest: Value,
    report: Value,
    elapsed_ns: u128,
}

fn run_proof_map_build(
    mapping_path: &Path,
    sources: &[PathBuf],
    out_dir: &Path,
) -> Result<TimedBuild, String> {
    let mut options = MapBuildOptions::new(out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(mapping_path, sources, options)
        .map_err(|err| format!("proof-suite map build failed: {err}"))?;
    Ok(TimedBuild {
        manifest: result.manifest,
        report: result.report,
        elapsed_ns: start.elapsed().as_nanos(),
    })
}

fn proof_object_path(out_dir: &Path, manifest: &Value) -> Result<PathBuf, String> {
    let object_rel = manifest
        .pointer("/artifacts/object/path")
        .and_then(Value::as_str)
        .ok_or_else(|| "proof-suite map build manifest missing object path".to_string())?;
    Ok(out_dir.join(object_rel))
}

fn write_projection_parity_reports(
    parity_dir: &Path,
    _mapping_path: &Path,
    _sources: &[PathBuf],
    object_path: &Path,
    projection_ids: &[&str],
) -> Result<Vec<Value>, String> {
    fs::create_dir_all(parity_dir)
        .map_err(|err| format!("cannot create {}: {err}", parity_dir.display()))?;
    let actual = projected_rows_from_cove_o_path(object_path, None)?;
    let mut reports = Vec::new();
    for projection_id in projection_ids {
        let expected_bytes = projected_output_from_cove_o_path(
            object_path,
            None,
            ProjectionFormat::Json,
            Some(projection_id),
        )?;
        let expected: Value = serde_json::from_slice(&expected_bytes)
            .map_err(|err| format!("cannot parse {projection_id} projection JSON: {err}"))?;
        let report = projection_parity_report(projection_id, &expected, &actual);
        write_json_pretty(
            &parity_dir.join(format!("{}.json", projection_id.replace(['/', ':'], "_"))),
            &report,
        )?;
        reports.push(report);
    }
    Ok(reports)
}

fn projection_parity_report(projection_id: &str, expected: &Value, actual: &Value) -> Value {
    let expected_rows = projection_rows(expected, projection_id);
    let actual_rows = projection_rows(actual, projection_id);
    let expected_set = canonical_row_set(&expected_rows);
    let actual_set = canonical_row_set(&actual_rows);
    let missing = expected_set
        .difference(&actual_set)
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    let extra = actual_set
        .difference(&expected_set)
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    let ok = missing.is_empty()
        && extra.is_empty()
        && expected_rows.len() == actual_rows.len()
        && expected_set.len() == actual_set.len();
    json!({
        "format": "cove-proof-suite-parity-v1",
        "projection_id": projection_id,
        "status": if ok { "ok" } else { "mismatch" },
        "expected_row_count": expected_rows.len(),
        "actual_row_count": actual_rows.len(),
        "expected_key_count": expected_set.len(),
        "actual_key_count": actual_set.len(),
        "missing_count": expected_set.difference(&actual_set).count(),
        "extra_count": actual_set.difference(&expected_set).count(),
        "sample_missing_rows": missing,
        "sample_extra_rows": extra,
    })
}

fn projection_rows(value: &Value, projection_id: &str) -> Vec<Value> {
    value
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("projection_id").and_then(Value::as_str) == Some(projection_id))
        .cloned()
        .collect()
}

fn canonical_row_set(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .map(|row| serde_json::to_string(row).unwrap_or_else(|_| "null".to_string()))
        .collect()
}

fn aggregate_parity_status(reports: &[Value]) -> Value {
    if reports
        .iter()
        .all(|report| report.get("status").and_then(Value::as_str) == Some("ok"))
    {
        json!("ok")
    } else {
        json!("mismatch")
    }
}

struct ProofSizeReportInput<'a> {
    scenario: &'a str,
    profile: Customer360Profile,
    sources: &'a [PathBuf],
    source_parquet: &'a [PathBuf],
    denormalized_parquet: &'a [PathBuf],
    bundle_dir: &'a Path,
    manifest: &'a Value,
    build_time_ns: u128,
    doctor: &'a Value,
    parity_reports: &'a [Value],
}

fn proof_size_report(input: ProofSizeReportInput<'_>) -> Result<Value, String> {
    let source_bytes = paths_size(input.sources);
    let source_parquet_bundle_bytes = paths_size(input.source_parquet);
    let denormalized_parquet_bytes = paths_size(input.denormalized_parquet);
    let cove_o_bytes = input
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cove_t_bytes = input
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|items| sum_artifact_bytes(items))
        .unwrap_or(0);
    let covi_bytes = input
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|items| sum_artifact_bytes(items))
        .unwrap_or(0);
    let covm_bytes = input
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size_local(input.bundle_dir)?;
    Ok(json!({
        "format": "cove-proof-suite-size-comparison-v1",
        "scenario": input.scenario,
        "profile": input.profile.as_str(),
        "build_time_ns": input.build_time_ns,
        "source_bytes": source_bytes,
        "source_parquet_bundle_bytes": source_parquet_bundle_bytes,
        "normalized_parquet_bundle_bytes": source_parquet_bundle_bytes,
        "denormalized_parquet_bytes": denormalized_parquet_bytes,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_bytes": cove_t_bytes,
        "covi_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "object_count": input.manifest.pointer("/counts/object_count").cloned().unwrap_or(Value::Null),
        "property_value_count": input.manifest.pointer("/counts/property_value_count").cloned().unwrap_or(Value::Null),
        "evidence_entry_count": input.manifest.pointer("/counts/evidence_entry_count").cloned().unwrap_or(Value::Null),
        "duplication_ratio_vs_source": ratio(total_bundle_bytes, source_bytes),
        "cove_o_vs_source_ratio": ratio(cove_o_bytes, source_bytes),
        "cove_o_vs_source_parquet_ratio": ratio(cove_o_bytes, source_parquet_bundle_bytes),
        "bundle_vs_denormalized_parquet_ratio": ratio(total_bundle_bytes, denormalized_parquet_bytes),
        "doctor_status_ok": input.doctor.get("status").and_then(Value::as_str) == Some("ok"),
        "parity_status_ok": aggregate_parity_status(input.parity_reports) == json!("ok"),
    }))
}

fn sum_artifact_bytes(items: &[Value]) -> u64 {
    items
        .iter()
        .filter_map(|item| item.get("byte_size").and_then(Value::as_u64))
        .sum()
}

fn ratio(numerator: u64, denominator: u64) -> Value {
    if denominator == 0 {
        Value::Null
    } else {
        json!(numerator as f64 / denominator as f64)
    }
}

fn paths_size(paths: &[PathBuf]) -> u64 {
    paths
        .iter()
        .map(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .sum()
}

fn directory_size_local(path: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for entry in
        fs::read_dir(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?
    {
        let entry = entry.map_err(|err| format!("cannot read {} entry: {err}", path.display()))?;
        let metadata = entry
            .metadata()
            .map_err(|err| format!("cannot stat {}: {err}", entry.path().display()))?;
        if metadata.is_dir() {
            total = total
                .checked_add(directory_size_local(&entry.path())?)
                .ok_or_else(|| "directory size overflow".to_string())?;
        } else {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| "directory size overflow".to_string())?;
        }
    }
    Ok(total)
}

fn write_json_pretty(path: &Path, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_crm_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let ids = (0..rows).map(customer_id).collect::<Vec<_>>();
    let names = (0..rows)
        .map(|i| {
            if i % 17 == 0 {
                None
            } else {
                Some(format!("Customer {i:06}"))
            }
        })
        .collect::<Vec<_>>();
    let regions = (0..rows)
        .map(|i| Some(["north", "south", "east", "west", "emea", "apac"][i % 6]))
        .collect::<Vec<_>>();
    let tiers = (0..rows)
        .map(|i| Some(["bronze", "silver", "gold", "platinum"][i % 4]))
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("full_name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("region", Arc::new(StringArray::from(regions)) as ArrayRef),
        ("tier", Arc::new(StringArray::from(tiers)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_support_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let ids = (0..rows).map(customer_id).collect::<Vec<_>>();
    let active = (0..rows).map(|i| i % 9 != 0).collect::<Vec<_>>();
    let scores = (0..rows)
        .map(|i| ((i * 37) % 100) as i64)
        .collect::<Vec<_>>();
    let statuses = (0..rows)
        .map(|i| {
            if i % 13 == 0 {
                "dormant"
            } else if i % 5 == 0 {
                "watch"
            } else {
                "active"
            }
        })
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("active", Arc::new(BooleanArray::from(active)) as ArrayRef),
        ("score", Arc::new(Int64Array::from(scores)) as ArrayRef),
        ("status", Arc::new(StringArray::from(statuses)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_claims_csv(path: &Path, rows: usize) -> Result<(), String> {
    let mut csv = String::from("claim_id,policy_id,person_id,status,amount\n");
    for i in 0..rows {
        csv.push_str(&format!(
            "{},pol{:06},per{:06},{},{}\n",
            claim_id(i),
            i % (rows / 3).max(1),
            i % (rows / 2).max(1),
            ["open", "review", "approved", "closed"][i % 4],
            500 + ((i * 91) % 50_000)
        ));
    }
    fs::write(path, csv).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_claim_events_jsonl(path: &Path, rows: usize) -> Result<(), String> {
    let mut out = String::new();
    for i in 0..rows {
        let event_kind = ["opened", "documented", "reviewed", "paid", "appealed"][i % 5];
        let value = json!({
            "claim_id": claim_id(i),
            "event_kind": event_kind,
            "severity": ((i * 13) % 10) as i64,
            "document_id": format!("doc{i:06}"),
        });
        out.push_str(&serde_json::to_string(&value).map_err(|err| err.to_string())?);
        out.push('\n');
    }
    fs::write(path, out).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_claims_summary_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let claim_ids = (0..rows).map(claim_id).collect::<Vec<_>>();
    let policy_ids = (0..rows)
        .map(|i| format!("pol{:06}", i % (rows / 3).max(1)))
        .collect::<Vec<_>>();
    let person_ids = (0..rows)
        .map(|i| format!("per{:06}", i % (rows / 2).max(1)))
        .collect::<Vec<_>>();
    let statuses = (0..rows)
        .map(|i| ["open", "review", "approved", "closed"][i % 4])
        .collect::<Vec<_>>();
    let amounts = (0..rows)
        .map(|i| (500 + ((i * 91) % 50_000)) as i64)
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        (
            "claim_id",
            Arc::new(StringArray::from(claim_ids)) as ArrayRef,
        ),
        (
            "policy_id",
            Arc::new(StringArray::from(policy_ids)) as ArrayRef,
        ),
        (
            "person_id",
            Arc::new(StringArray::from(person_ids)) as ArrayRef,
        ),
        ("status", Arc::new(StringArray::from(statuses)) as ArrayRef),
        ("amount", Arc::new(Int64Array::from(amounts)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_claim_events_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let claim_ids = (0..rows).map(claim_id).collect::<Vec<_>>();
    let event_kinds = (0..rows)
        .map(|i| ["opened", "documented", "reviewed", "paid", "appealed"][i % 5])
        .collect::<Vec<_>>();
    let severities = (0..rows)
        .map(|i| ((i * 13) % 10) as i64)
        .collect::<Vec<_>>();
    let document_ids = (0..rows).map(|i| format!("doc{i:06}")).collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        (
            "claim_id",
            Arc::new(StringArray::from(claim_ids)) as ArrayRef,
        ),
        (
            "event_kind",
            Arc::new(StringArray::from(event_kinds)) as ArrayRef,
        ),
        (
            "severity",
            Arc::new(Int64Array::from(severities)) as ArrayRef,
        ),
        (
            "document_id",
            Arc::new(StringArray::from(document_ids)) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_claim_providers_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let claim_ids = (0..rows).map(claim_id).collect::<Vec<_>>();
    let provider_ids = (0..rows)
        .map(|i| format!("prv{:04}", i % 97))
        .collect::<Vec<_>>();
    let provider_names = (0..rows)
        .map(|i| format!("Provider {:04}", i % 97))
        .collect::<Vec<_>>();
    let regions = (0..rows)
        .map(|i| ["north", "south", "east", "west"][i % 4])
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        (
            "claim_id",
            Arc::new(StringArray::from(claim_ids)) as ArrayRef,
        ),
        (
            "provider_id",
            Arc::new(StringArray::from(provider_ids)) as ArrayRef,
        ),
        (
            "provider_name",
            Arc::new(StringArray::from(provider_names)) as ArrayRef,
        ),
        (
            "provider_region",
            Arc::new(StringArray::from(regions)) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_products_csv(path: &Path, rows: usize) -> Result<(), String> {
    let mut csv = String::from("sku,title,brand,category\n");
    for i in 0..rows {
        csv.push_str(&format!(
            "{},Product {:06},Brand {:03},{}\n",
            sku(i),
            i,
            i % 128,
            ["tools", "home", "apparel", "electronics", "grocery"][i % 5]
        ));
    }
    fs::write(path, csv).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_vendor_prices_jsonl(path: &Path, rows: usize) -> Result<(), String> {
    let mut out = String::new();
    for i in 0..rows {
        let value = json!({
            "sku": sku(i),
            "vendor_id": format!("vendor{:03}", i % 41),
            "price": ((i * 17) % 10_000) as i64,
            "currency": "USD",
            "availability": if i % 7 == 0 { "backorder" } else { "available" },
        });
        out.push_str(&serde_json::to_string(&value).map_err(|err| err.to_string())?);
        out.push('\n');
    }
    fs::write(path, out).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

fn write_products_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let skus = (0..rows).map(sku).collect::<Vec<_>>();
    let titles = (0..rows)
        .map(|i| format!("Product {i:06}"))
        .collect::<Vec<_>>();
    let brands = (0..rows)
        .map(|i| format!("Brand {:03}", i % 128))
        .collect::<Vec<_>>();
    let categories = (0..rows)
        .map(|i| ["tools", "home", "apparel", "electronics", "grocery"][i % 5])
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("sku", Arc::new(StringArray::from(skus)) as ArrayRef),
        ("title", Arc::new(StringArray::from(titles)) as ArrayRef),
        ("brand", Arc::new(StringArray::from(brands)) as ArrayRef),
        (
            "category",
            Arc::new(StringArray::from(categories)) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_vendor_prices_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let skus = (0..rows).map(sku).collect::<Vec<_>>();
    let vendor_ids = (0..rows)
        .map(|i| format!("vendor{:03}", i % 41))
        .collect::<Vec<_>>();
    let prices = (0..rows)
        .map(|i| ((i * 17) % 10_000) as i64)
        .collect::<Vec<_>>();
    let currencies = vec!["USD"; rows];
    let availability = (0..rows)
        .map(|i| if i % 7 == 0 { "backorder" } else { "available" })
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("sku", Arc::new(StringArray::from(skus)) as ArrayRef),
        (
            "vendor_id",
            Arc::new(StringArray::from(vendor_ids)) as ArrayRef,
        ),
        ("price", Arc::new(Int64Array::from(prices)) as ArrayRef),
        (
            "currency",
            Arc::new(StringArray::from(currencies)) as ArrayRef,
        ),
        (
            "availability",
            Arc::new(StringArray::from(availability)) as ArrayRef,
        ),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn write_product_attributes_parquet(path: &Path, rows: usize) -> Result<(), String> {
    let skus = (0..rows).map(sku).collect::<Vec<_>>();
    let colors = (0..rows)
        .map(|i| ["red", "blue", "green", "black", "white"][i % 5])
        .collect::<Vec<_>>();
    let sizes = (0..rows)
        .map(|i| ["xs", "s", "m", "l", "xl"][i % 5])
        .collect::<Vec<_>>();
    let ratings = (0..rows)
        .map(|i| ((i * 19) % 100) as i64)
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_from_iter(vec![
        ("sku", Arc::new(StringArray::from(skus)) as ArrayRef),
        ("color", Arc::new(StringArray::from(colors)) as ArrayRef),
        ("size", Arc::new(StringArray::from(sizes)) as ArrayRef),
        ("rating", Arc::new(Int64Array::from(ratings)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())?;
    write_parquet_batches(path, &[batch])
}

fn claim_id(index: usize) -> String {
    format!("clm{index:06}")
}

fn sku(index: usize) -> String {
    format!("sku{index:06}")
}

fn claims_covemap() -> CovemapFile {
    proof_covemap(ProofCovemapSpec {
        file_id_byte: 0xD1,
        mapping_id: "claims",
        mapping_version: "claims/v1",
        object_type: "Claim",
        identity_rule_id: "claim_by_id",
        join_column: "claim_id",
        sources: vec![
            source_decl("claims", "claim_by_id", 10),
            source_decl("claim_events", "claim_by_id", 20),
            source_decl("providers", "claim_by_id", 30),
        ],
        rules: vec![
            row_rule_for(
                "claim_by_id",
                "claim_summary_row",
                "claims",
                vec![
                    property_binding("claim_id", "claim_id", "utf8"),
                    property_binding("policy_id", "policy_id", "utf8"),
                    property_binding("person_id", "person_id", "utf8"),
                    property_binding("status", "status", "utf8"),
                    property_binding("amount", "amount", "int64"),
                ],
            ),
            row_rule_for(
                "claim_by_id",
                "claim_event_row",
                "claim_events",
                vec![
                    property_binding("claim_id", "claim_id", "utf8"),
                    property_binding("event_kind", "event_kind", "utf8"),
                    property_binding("severity", "severity", "int64"),
                    property_binding("document_id", "document_id", "utf8"),
                ],
            ),
            row_rule_for(
                "claim_by_id",
                "claim_provider_row",
                "providers",
                vec![
                    property_binding("claim_id", "claim_id", "utf8"),
                    property_binding("provider_id", "provider_id", "utf8"),
                    property_binding("provider_name", "provider_name", "utf8"),
                    property_binding("provider_region", "provider_region", "utf8"),
                ],
            ),
        ],
        projection_id: "claims.v1",
        output_table: "claims",
        projection_columns: vec![
            projection_column("goid", "object.goid", "uuid"),
            projection_column("claim_id", "claim_id", "utf8"),
            projection_column("policy_id", "policy_id", "utf8"),
            projection_column("person_id", "person_id", "utf8"),
            projection_column("status", "status", "utf8"),
            projection_column("amount", "amount", "int64"),
            projection_column("event_kind", "event_kind", "utf8"),
            projection_column("severity", "severity", "int64"),
            projection_column("document_id", "document_id", "utf8"),
            projection_column("provider_id", "provider_id", "utf8"),
            projection_column("provider_region", "provider_region", "utf8"),
        ],
        evidence_projection_id: "claim_evidence.v1",
        evidence_output_table: "claim_evidence",
    })
}

fn catalog_covemap() -> CovemapFile {
    proof_covemap(ProofCovemapSpec {
        file_id_byte: 0xD2,
        mapping_id: "catalog",
        mapping_version: "catalog/v1",
        object_type: "Product",
        identity_rule_id: "product_by_sku",
        join_column: "sku",
        sources: vec![
            source_decl("products", "product_by_sku", 10),
            source_decl("vendor_prices", "product_by_sku", 20),
            source_decl("attributes", "product_by_sku", 30),
        ],
        rules: vec![
            row_rule_for(
                "product_by_sku",
                "product_row",
                "products",
                vec![
                    property_binding("sku", "sku", "utf8"),
                    property_binding("title", "title", "utf8"),
                    property_binding("brand", "brand", "utf8"),
                    property_binding("category", "category", "utf8"),
                ],
            ),
            row_rule_for(
                "product_by_sku",
                "vendor_price_row",
                "vendor_prices",
                vec![
                    property_binding("sku", "sku", "utf8"),
                    property_binding("vendor_id", "vendor_id", "utf8"),
                    property_binding("price", "price", "int64"),
                    property_binding("currency", "currency", "utf8"),
                    property_binding("availability", "availability", "utf8"),
                ],
            ),
            row_rule_for(
                "product_by_sku",
                "attribute_row",
                "attributes",
                vec![
                    property_binding("sku", "sku", "utf8"),
                    property_binding("color", "color", "utf8"),
                    property_binding("size", "size", "utf8"),
                    property_binding("rating", "rating", "int64"),
                ],
            ),
        ],
        projection_id: "products.v1",
        output_table: "products",
        projection_columns: vec![
            projection_column("goid", "object.goid", "uuid"),
            projection_column("sku", "sku", "utf8"),
            projection_column("title", "title", "utf8"),
            projection_column("brand", "brand", "utf8"),
            projection_column("category", "category", "utf8"),
            projection_column("vendor_id", "vendor_id", "utf8"),
            projection_column("price", "price", "int64"),
            projection_column("currency", "currency", "utf8"),
            projection_column("availability", "availability", "utf8"),
            projection_column("color", "color", "utf8"),
            projection_column("size", "size", "utf8"),
            projection_column("rating", "rating", "int64"),
        ],
        evidence_projection_id: "product_evidence.v1",
        evidence_output_table: "product_evidence",
    })
}

struct ProofCovemapSpec<'a> {
    file_id_byte: u8,
    mapping_id: &'a str,
    mapping_version: &'a str,
    object_type: &'a str,
    identity_rule_id: &'a str,
    join_column: &'a str,
    sources: Vec<Value>,
    rules: Vec<Value>,
    projection_id: &'a str,
    output_table: &'a str,
    projection_columns: Vec<Value>,
    evidence_projection_id: &'a str,
    evidence_output_table: &'a str,
}

fn proof_covemap(spec: ProofCovemapSpec<'_>) -> CovemapFile {
    let ProofCovemapSpec {
        file_id_byte,
        mapping_id,
        mapping_version,
        object_type,
        identity_rule_id,
        join_column,
        sources,
        rules,
        projection_id,
        output_table,
        projection_columns,
        evidence_projection_id,
        evidence_output_table,
    } = spec;
    CovemapFile {
        header: CovemapHeaderV1::new([file_id_byte; 16], 0),
        mapping_version: mapping_version.into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "sources": sources,
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "identity_rules": [{
                        "rule_id": identity_rule_id,
                        "object_type": object_type,
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": join_column,
                            "source_column": join_column,
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "rules": rules,
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": mapping_id,
                    "mapping_version": mapping_version,
                    "projections": [
                        {
                            "projection_id": projection_id,
                            "output_table": output_table,
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": object_type},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": projection_columns,
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        },
                        {
                            "projection_id": evidence_projection_id,
                            "output_table": evidence_output_table,
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": object_type},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": [
                                {"name": "source_id", "value": "evidence.source_id", "logical_type": "utf8"},
                                {"name": "source_row_identity", "value": "evidence.source_row_identity", "logical_type": "utf8"},
                                {"name": "rule_id", "value": "evidence.rule_id", "logical_type": "utf8"},
                                {"name": "output_object_id", "value": "evidence.output_object_id", "logical_type": "uuid"}
                            ],
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        }
                    ]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn source_decl(source_id: &str, identity_rule_id: &str, source_priority: i64) -> Value {
    json!({
        "source_id": source_id,
        "row_identity_rules": [identity_rule_id],
        "source_priority": source_priority,
    })
}

fn projection_column(name: &str, value: &str, logical_type: &str) -> Value {
    json!({
        "name": name,
        "value": value,
        "logical_type": logical_type,
    })
}

fn customer360_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0xC3; 16], 0),
        mapping_version: "customer360/v1".into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "customer360",
                    "mapping_version": "customer360/v1",
                    "sources": [
                        {"source_id": "crm", "row_identity_rules": ["customer_by_id"], "source_priority": 10},
                        {"source_id": "support", "row_identity_rules": ["customer_by_id"], "source_priority": 20},
                        {"source_id": "billing", "row_identity_rules": ["customer_by_id"], "source_priority": 30}
                    ]
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "customer360",
                    "mapping_version": "customer360/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "customer360",
                    "mapping_version": "customer360/v1",
                    "identity_rules": [{
                        "rule_id": "customer_by_id",
                        "object_type": "Customer",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "customer_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer360",
                    "mapping_version": "customer360/v1",
                    "rules": [
                        row_rule("crm_customer_row", "crm", vec![
                            property_binding("customer_id", "id", "utf8"),
                            property_binding("full_name", "full_name", "utf8"),
                            property_binding("region", "region", "utf8"),
                            property_binding("tier", "tier", "utf8")
                        ]),
                        row_rule("support_customer_row", "support", vec![
                            property_binding("customer_id", "id", "utf8"),
                            property_binding("active", "active", "bool"),
                            property_binding("score", "score", "int64"),
                            property_binding("status", "status", "utf8")
                        ]),
                        row_rule("billing_customer_row", "billing", vec![
                            property_binding("customer_id", "id", "utf8"),
                            property_binding("plan", "plan", "utf8"),
                            property_binding("mrr", "mrr", "int64"),
                            property_binding("tier", "tier", "utf8"),
                            property_binding("billing_status", "billing_status", "utf8")
                        ])
                    ]
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "customer360",
                    "mapping_version": "customer360/v1",
                    "projections": [
                        {
                            "projection_id": "customer_360.v1",
                            "output_table": "customers",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Customer"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": [
                                {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                                {"name": "customer_id", "value": "customer_id", "logical_type": "utf8"},
                                {"name": "full_name", "value": "full_name", "logical_type": "utf8"},
                                {"name": "region", "value": "region", "logical_type": "utf8"},
                                {"name": "tier", "value": "tier", "logical_type": "utf8"},
                                {"name": "active", "value": "active", "logical_type": "bool"},
                                {"name": "score", "value": "score", "logical_type": "int64"},
                                {"name": "status", "value": "status", "logical_type": "utf8"},
                                {"name": "plan", "value": "plan", "logical_type": "utf8"},
                                {"name": "mrr", "value": "mrr", "logical_type": "int64"},
                                {"name": "billing_status", "value": "billing_status", "logical_type": "utf8"}
                            ],
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        },
                        {
                            "projection_id": "customer_evidence.v1",
                            "output_table": "customer_evidence",
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": "Customer"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": [
                                {"name": "source_id", "value": "evidence.source_id", "logical_type": "utf8"},
                                {"name": "source_row_identity", "value": "evidence.source_row_identity", "logical_type": "utf8"},
                                {"name": "rule_id", "value": "evidence.rule_id", "logical_type": "utf8"},
                                {"name": "output_object_id", "value": "evidence.output_object_id", "logical_type": "uuid"}
                            ],
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        }
                    ]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn customer360_readback_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0xC4; 16], 0),
        mapping_version: "customer360-readback/v1".into(),
        sections: vec![
            map_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "customer360-readback",
                    "mapping_version": "customer360-readback/v1",
                    "sources": [{
                        "source_id": "customers_360",
                        "row_identity_rules": ["customer_by_id"]
                    }]
                }),
            ),
            map_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "customer360-readback",
                    "mapping_version": "customer360-readback/v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            map_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "customer360-readback",
                    "mapping_version": "customer360-readback/v1",
                    "identity_rules": [{
                        "rule_id": "customer_by_id",
                        "object_type": "Customer",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "customer_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            ),
            map_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "customer360-readback",
                    "mapping_version": "customer360-readback/v1",
                    "rules": [
                        row_rule("customer_360_row", "customers_360", vec![
                            property_binding("customer_id", "customer_id", "utf8"),
                            property_binding("full_name", "full_name", "utf8"),
                            property_binding("region", "region", "utf8"),
                            property_binding("tier", "tier", "utf8"),
                            property_binding("active", "active", "bool"),
                            property_binding("score", "score", "int64"),
                            property_binding("status", "status", "utf8"),
                            property_binding("plan", "plan", "utf8"),
                            property_binding("mrr", "mrr", "int64"),
                            property_binding("billing_status", "billing_status", "utf8")
                        ])
                    ]
                }),
            ),
            map_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "customer360-readback",
                    "mapping_version": "customer360-readback/v1",
                    "projections": [
                        {
                            "projection_id": "customer_360.v1",
                            "output_table": "customers",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Customer"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": [
                                {"name": "goid", "value": "object.goid", "logical_type": "uuid"},
                                {"name": "customer_id", "value": "customer_id", "logical_type": "utf8"},
                                {"name": "full_name", "value": "full_name", "logical_type": "utf8"},
                                {"name": "region", "value": "region", "logical_type": "utf8"},
                                {"name": "tier", "value": "tier", "logical_type": "utf8"},
                                {"name": "active", "value": "active", "logical_type": "bool"},
                                {"name": "score", "value": "score", "logical_type": "int64"},
                                {"name": "status", "value": "status", "logical_type": "utf8"},
                                {"name": "plan", "value": "plan", "logical_type": "utf8"},
                                {"name": "mrr", "value": "mrr", "logical_type": "int64"},
                                {"name": "billing_status", "value": "billing_status", "logical_type": "utf8"}
                            ],
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        },
                        {
                            "projection_id": "customer_evidence.v1",
                            "output_table": "customer_evidence",
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": "Customer"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "columns": [
                                {"name": "source_id", "value": "evidence.source_id", "logical_type": "utf8"},
                                {"name": "source_row_identity", "value": "evidence.source_row_identity", "logical_type": "utf8"},
                                {"name": "rule_id", "value": "evidence.rule_id", "logical_type": "utf8"},
                                {"name": "output_object_id", "value": "evidence.output_object_id", "logical_type": "uuid"}
                            ],
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"]
                        }
                    ]
                }),
            ),
        ],
        postscript: CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    }
}

fn row_rule(rule_id: &str, source_id: &str, property_bindings: Vec<Value>) -> Value {
    row_rule_for("customer_by_id", rule_id, source_id, property_bindings)
}

fn row_rule_for(
    identity_rule_id: &str,
    rule_id: &str,
    source_id: &str,
    property_bindings: Vec<Value>,
) -> Value {
    json!({
        "rule_id": rule_id,
        "source_id": source_id,
        "identity_rule_id": identity_rule_id,
        "row_semantics_kind": "Object",
        "assertion_kinds": ["object", "property", "evidence"],
        "function_ids": ["identity"],
        "output_assertion_ids": [],
        "association_endpoints": [],
        "property_bindings": property_bindings
    })
}

fn property_binding(property: &str, source_column: &str, logical_type: &str) -> Value {
    json!({
        "assertion_id": property,
        "property_id": property,
        "property_name": property,
        "source_column": source_column,
        "logical_type": logical_type,
        "nullable": true,
        "missing_policy": "null",
        "conflict_policy": "source_priority_wins"
    })
}

fn map_section(kind: SectionKind, mut value: Value) -> CovemapSection {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".to_string(),
            Value::String("org.coveformat.covemap.v2".to_string()),
        );
        object.insert(
            "section_id".to_string(),
            Value::Number((kind as u16).into()),
        );
    }
    let payload = serde_json::to_vec_pretty(&value).unwrap();
    CovemapSection {
        entry: CovemapSectionEntryV1 {
            section_id: kind as u32,
            offset: 0,
            length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            compression: 0,
            payload_encoding: CovemapPayloadEncodingV2::CoveMapJsonV2 as u8,
            required: true,
            reserved: 0,
            checksum: 0,
        },
        payload,
    }
}

fn customer_id(index: usize) -> String {
    format!("c{index:06}")
}

fn customer360_manifest(
    profile: Customer360Profile,
    customers: usize,
    events: usize,
    proof: Option<Value>,
) -> Value {
    json!({
        "version": 1,
        "profile": profile.as_str(),
        "row_counts": {
            "crm": customers,
            "support": customers,
            "billing": customers,
            "events": events,
            "canonical_customers": customers,
            "expected_evidence_rows": customers
        },
        "pipeline": {
            "source_inputs": ["crm.csv", "support.jsonl", "billing.parquet"],
            "source_mapping_contract": "customer360.covemap",
            "canonical_readback_source": "customers_360.jsonl",
            "mapped_cove_o_materialization": "customers_360.jsonl + customer360_readback.covemap",
            "proof_map_build_materialization": "crm.csv + support.jsonl + billing.parquet + customer360.covemap",
            "note": "The runnable COVE-O archive is materialized from reconciled canonical rows so CoveQL readback is deterministic; map-build-bundle is generated directly from the messy CRM/support/billing sources to prove the adoption path end to end."
        },
        "artifacts": {
            "sources": {
                "crm": "crm.csv",
                "support": "support.jsonl",
                "billing": "billing.parquet",
                "reconciled": "customers_360.jsonl",
                "events_jsonl": "events.jsonl"
            },
            "mapping": "customer360.covemap",
            "readback_mapping": "customer360_readback.covemap",
            "mapped_cove_o": "customers.cove",
            "events_cove_t": "events.cove",
            "projected_cove_t": {
                "customers": "customers_projection.cove",
                "evidence": "evidence_projection.cove"
            },
            "parquet_baselines": {
                "customers": "customers_projection.parquet",
                "evidence": "evidence_projection.parquet"
            },
            "proof": proof.as_ref().and_then(|value| value.get("artifacts")).cloned().unwrap_or(Value::Null)
        },
        "proof": proof,
        "recommended_queries": [
            {
                "title": "Canonical customers",
                "command": "cove query customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'"
            },
            {
                "title": "Evidence by source",
                "command": "cove query customers.cove 'table(customer_evidence).groupBy(source_id).select(source_id, rows: count(*))'"
            },
            {
                "title": "High value customers",
                "command": "cove query --engine compare --perf-report customers.cove 'table(customers).where(score >= 80).select(customer_id, tier, score, status, mrr).take(10)'"
            },
            {
                "title": "Customer events through an external JSONL table",
                "command": "cove query customers.cove --external-table events=events.jsonl 'table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, tier: c.tier, event_kind: e.event_kind, event_score: e.score).take(10)'"
            },
            {
                "title": "Verify the true multi-source map-build bundle",
                "command": "cove map doctor --bundle-dir map-build-bundle"
            }
        ],
        "benchmark_cases": [
            "customer360_projection_scan",
            "customer360_selective_filter",
            "customer360_event_filter",
            "customer360_object_store_compare",
            "proof_suite_customer360"
        ]
    })
}

fn customer360_readme() -> &'static str {
    r#"# Customer 360 Data-Science Showcase

This directory is generated by:

```bash
cove showcase customer360 --profile quick --out examples/customer360 --force
```

It demonstrates COVE as a semantic archive: CRM, support, and billing inputs
are generated alongside a reconciled canonical readback source. The runnable
COVE-O archive is materialized from `customers_360.jsonl` with queryable
evidence and projected table readback; `customer360.covemap` records the
multi-source mapping contract for the messy source artifacts. The
`map-build-bundle/` directory is built directly from the messy sources with
`cove map build --verify --publish-covm`.

Try from this directory:

```bash
cove doctor customers.cove
cove inspect --queries --performance customers.cove
cove query customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'
cove query customers.cove 'table(customer_evidence).groupBy(source_id).select(source_id, rows: count(*))'
cove query customers.cove --external-table events=events.jsonl 'table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, tier: c.tier, event_kind: e.event_kind, event_score: e.score).take(10)'
cove map doctor --bundle-dir map-build-bundle
cove optimize events.cove
cove query --engine compare --perf-report events.cove 'table(events).where(score >= 80).select(event_id, customer_id, event_kind, score)'
python3 notebooks/customer360_analysis.py --input-dir .
```

For larger local data, write to `target/` instead of the checked-in example:

```bash
cove showcase customer360 --profile standard --out target/customer360-standard --force
```
"#
}

fn proof_suite_readme() -> &'static str {
    r#"# COVE-O Proof Suite

This directory is generated by:

```bash
cove showcase proof-suite --scenario all --profile quick --out target/cove-proof-suite --force
```

Each scenario contains source tables, a COVE-MAP file, a verified map-build
bundle, doctor report, projection parity reports, Parquet comparison baselines,
and a size-comparison JSON document.

Useful checks:

```bash
cove map doctor --bundle-dir customer360/map-build-bundle
cove map doctor --bundle-dir claims/map-build-bundle
cove map doctor --bundle-dir catalog/map-build-bundle
```
"#
}

fn scenario_readme(scenario: &str, projection_id: &str) -> String {
    format!(
        r#"# {scenario} Proof Scenario

This scenario is generated data for measuring COVE-O against overlapping source
tables and Parquet baselines. The map-build bundle is the semantic authority;
COVE-I and COVM files are optional validated companions.

Try from this directory:

```bash
cove map doctor --bundle-dir map-build-bundle
cove query map-build-bundle/{scenario}.cove 'table({projection_id}).take(10)'
```
"#
    )
}

fn customer360_notebook_script() -> &'static str {
    r#"#!/usr/bin/env python3
import argparse
import json
from collections import Counter
from pathlib import Path

def load_jsonl(path):
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows

def main():
    parser = argparse.ArgumentParser(description="Notebook-style Customer 360 analysis")
    parser.add_argument("--input-dir", default=".", help="Generated Customer 360 directory")
    args = parser.parse_args()
    root = Path(args.input_dir)
    manifest = json.loads((root / "customer360-manifest.json").read_text(encoding="utf-8"))
    support = load_jsonl(root / "support.jsonl")
    events = load_jsonl(root / "events.jsonl")

    print("Customer 360 profile:", manifest["profile"])
    print("Source rows:", manifest["row_counts"])
    print("Support status distribution:", dict(Counter(row["status"] for row in support)))
    print("Event kind distribution:", dict(Counter(row["event_kind"] for row in events)))

    try:
        import pandas as pd
        support_df = pd.DataFrame(support)
        events_df = pd.DataFrame(events)
        print("\nPandas status by active flag:")
        print(support_df.groupby(["status", "active"]).size().reset_index(name="rows").head(10))
        print("\nPandas event score summary:")
        print(events_df.groupby("event_kind")["score"].agg(["count", "mean"]).reset_index())
    except Exception as exc:
        print("\nPandas section skipped:", exc)

    try:
        import polars as pl
        support_pl = pl.DataFrame(support)
        print("\nPolars average score by status:")
        print(support_pl.group_by("status").agg(pl.col("score").mean().alias("avg_score")))
    except Exception as exc:
        print("\nPolars section skipped:", exc)

    print("\nRun these CLI queries for canonical rows and provenance:")
    for item in manifest["recommended_queries"]:
        print("-", item["command"])

if __name__ == "__main__":
    main()
"#
}

#[allow(dead_code)]
fn _schema_for_docs() -> Schema {
    Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("active", DataType::Boolean, true),
        Field::new("score", DataType::Int64, true),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customer360_events_cove_t_handles_multiple_morsels() {
        let rows = DEFAULT_MORSEL_ROW_COUNT as usize + 1;
        let bytes = events_cove_t(128, rows).unwrap();
        assert!(!bytes.is_empty());
    }
}
