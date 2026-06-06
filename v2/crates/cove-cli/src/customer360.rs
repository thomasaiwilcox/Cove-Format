use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, SectionKind, FEATURE_SEMANTIC_MAP,
    },
    durable,
    table::{ColumnEntry, TableCatalog, TableEntry},
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
};
use cove_map::{ProjectionBatchOptions, ProjectionFormat};
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

    let manifest = customer360_manifest(profile, customers, events);
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
    let mut event_ids = Vec::new();
    let mut customer_ids = Vec::new();
    let mut event_kinds = Vec::new();
    let mut scores = Vec::new();
    for i in 0..rows {
        event_ids.extend_from_slice(&((i + 1) as u64).to_le_bytes());
        push_varbytes(&mut customer_ids, &customer_id(i % customers));
        push_varbytes(
            &mut event_kinds,
            ["login", "ticket", "invoice", "upgrade", "downgrade"][i % 5],
        );
        scores.extend_from_slice(&(((i * 17 + 11) % 100) as u64).to_le_bytes());
    }
    let mut segment = ScanSegment::new(1, 0, 0, rows as u32, 4);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(rows as u32, event_ids)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
    segment.set_column_pages(
        2,
        vec![ScanPageSpec::new(rows as u32, customer_ids)
            .with_encoding_root(CoveEncodingKind::VarBytes as u32)],
    );
    segment.set_column_pages(
        3,
        vec![ScanPageSpec::new(rows as u32, event_kinds)
            .with_encoding_root(CoveEncodingKind::VarBytes as u32)],
    );
    segment.set_column_pages(
        4,
        vec![ScanPageSpec::new(rows as u32, scores)
            .with_encoding_root(CoveEncodingKind::NumCode as u32)],
    );
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
    json!({
        "rule_id": rule_id,
        "source_id": source_id,
        "identity_rule_id": "customer_by_id",
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

fn customer360_manifest(profile: Customer360Profile, customers: usize, events: usize) -> Value {
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
            "note": "The runnable COVE-O archive is materialized from reconciled canonical rows so CoveQL readback is deterministic; the CRM/support/billing files and customer360.covemap document the messy multi-source mapping surface."
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
            }
        },
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
            }
        ],
        "benchmark_cases": [
            "customer360_projection_scan",
            "customer360_selective_filter",
            "customer360_event_filter",
            "customer360_object_store_compare"
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
multi-source mapping contract for the messy source artifacts.

Try from this directory:

```bash
cove doctor customers.cove
cove inspect --queries --performance customers.cove
cove query customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'
cove query customers.cove 'table(customer_evidence).groupBy(source_id).select(source_id, rows: count(*))'
cove query customers.cove --external-table events=events.jsonl 'table(customers) as c.join(table(events) as e, on: c.customer_id == e.customer_id).select(customer_id: c.customer_id, tier: c.tier, event_kind: e.event_kind, event_score: e.score).take(10)'
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
