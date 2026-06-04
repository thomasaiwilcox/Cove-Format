use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    time::Instant,
};

use cove_core::{
    constants::{
        CompressionCodec, CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind,
        FEATURE_OBJECT_PROFILE,
    },
    profile::{
        cove_map::{MapEvidenceEntry, MapEvidenceIndex},
        cove_o::{
            ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        },
    },
    reader::ValidationOptions,
    writer::{MinimalCoveWriter, SectionPayload},
};
use cove_oql::{
    parse_resolve_and_plan_query, parse_resolve_plan_build_physical_and_execute_query,
    AssociationOptimizationReport, CoveOqlOutputMode, EvidenceOptimizationReport, ExecutionOptions,
    KernelExecutionMode, KernelExecutionOptions, MaterializedAssociationRow, OutputGrain,
    ParseOptions, PhysicalPlanOptions, PlanOptions, ResolveOptions, SecurityContext,
};
use serde_json::json;

fn main() {
    benchmark_object_kernel_execution_report();
    benchmark_association_report();
    benchmark_evidence_report();
}

fn benchmark_object_kernel_execution_report() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
    benchmark_object_kernel_case("direct_projection", bytes, "Thing.select(active)");
    benchmark_object_kernel_case(
        "typed_predicate",
        bytes,
        "Thing.where(active == true).select(active)",
    );
}

fn benchmark_object_kernel_case(label: &str, bytes: &[u8], query: &str) {
    let started = Instant::now();
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            redact_exact_counters: false,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .expect("object kernel benchmark query should execute");
    let elapsed_us = started.elapsed().as_micros();
    let report = &kernel.kernel_report;
    let counters = &report.counters;
    black_box(&kernel);
    println!(
        "kernel_execution object_kernel case={} rows_scanned={} rows_after_bitmap={} rows_pruned_by_bitmap={} rows_after_selection_vector={} rows_pruned_by_selection_vector={} output_rows={} elapsed_us={} rows_per_us={:.3} coded_predicate_rows={} coded_predicate_pct={:.3} typed_predicate_rows={} residual_rows_checked={} residual_rate_per_1k_rows={:.3} dictionary_lookups={} bytes_touched={} scratch_high_water_bytes={} final_materialization_rows={} compared_with_materialized={} materialized_fingerprint_present={} kernel_fingerprint_present={} fallback_reason={:?} authority_state={:?}",
        label,
        counters.rows_scanned,
        counters.rows_after_bitmap,
        report.metrics.rows_pruned_by_bitmap,
        counters.rows_after_selection_vector,
        report.metrics.rows_pruned_by_selection_vector,
        counters.output_rows,
        elapsed_us,
        per_us(counters.rows_scanned, elapsed_us),
        counters.coded_predicate_rows,
        percent(counters.coded_predicate_rows, counters.rows_scanned),
        counters.typed_predicate_rows,
        counters.residual_rows_checked,
        per_1k(counters.residual_rows_checked, counters.rows_scanned),
        counters.dictionary_lookups_at_materialization,
        counters.bytes_touched_estimate,
        counters.scratch_high_water_bytes,
        report.metrics.final_materialization_rows,
        report.compared_with_materialized,
        report.materialized_fingerprint.is_some(),
        report.kernel_fingerprint.is_some(),
        report.fallback_reason,
        report.optimization_authority.state
    );
}

fn benchmark_association_report() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .expect("association benchmark query should plan");
    let rows = (0..250_000usize)
        .map(|index| MaterializedAssociationRow {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::AssociationState,
            change: None,
            object_type_id: 7,
            association_type: Some("CustomerPlacedOrder".into()),
            branch_key: 0,
            goid: format!("edge-{index}"),
            record_id: format!("record-{index}"),
            source_goid: Some(format!("source-{}", index % 50_000)),
            target_goid: Some(format!("target-{}", index % 125_000)),
            timestamp_us: index as i64,
            csn: index as u64,
            record_kind: "baseline".into(),
            tombstone_status: "live".into(),
            properties: BTreeMap::new(),
            property_ids: BTreeMap::new(),
            redacted_properties: BTreeSet::new(),
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    let report = AssociationOptimizationReport::for_plan(&planned, &rows);
    let elapsed_us = started.elapsed().as_micros();
    let fast_path_candidates = report.semi_join_candidates
        + report.anti_join_candidates
        + report.count_fast_path_candidates
        + report.distinct_target_fast_path_candidates
        + report.validity_interval_fast_path_candidates;
    black_box(&report);
    println!(
        "kernel_execution association_report edges={} elapsed_us={} edges_per_us={:.3} fast_path_candidates={} candidate_rate_per_1k_edges={:.3} semi_join_candidates={} anti_join_candidates={} count_fast_path_candidates={} distinct_target_fast_path_candidates={} validity_interval_fast_path_candidates={} endpoint_plan_count={} fallback_count={}",
        report.edge_count,
        elapsed_us,
        per_us(report.edge_count, elapsed_us),
        fast_path_candidates,
        per_1k(fast_path_candidates, report.edge_count),
        report.semi_join_candidates,
        report.anti_join_candidates,
        report.count_fast_path_candidates,
        report.distinct_target_fast_path_candidates,
        report.validity_interval_fast_path_candidates,
        report.endpoint_plans.len(),
        report.fallback_reasons.len()
    );
}

fn benchmark_evidence_report() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_file(),
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .expect("evidence benchmark query should plan");
    let index = MapEvidenceIndex {
        mapping_id: "bench-map".into(),
        mapping_version: "2026.06".into(),
        entries: (0..250_000usize)
            .map(|row| {
                let mut operation_metadata = BTreeMap::new();
                operation_metadata.insert("operation_target".into(), json!("object"));
                MapEvidenceEntry {
                    source_id: format!("source-{}", row % 512),
                    source_row_identity: format!("source-row-{row}"),
                    rule_id: "rule-object".into(),
                    assertion_id: format!("assertion-{row}"),
                    output_object_id: format!("object-{}", row % 50_000),
                    observed_schema_fingerprint: None,
                    observed_snapshot_digest: None,
                    operation_metadata,
                }
            })
            .collect(),
    };

    let started = Instant::now();
    let report = EvidenceOptimizationReport::for_plan(&planned, Some(&index));
    let elapsed_us = started.elapsed().as_micros();
    let indexed_entries = report
        .index_reports
        .iter()
        .map(|grain| grain.indexed_entries)
        .sum::<usize>();
    let fallback_entries = report
        .index_reports
        .iter()
        .map(|grain| grain.fallback_entries)
        .sum::<usize>();
    let fast_path_candidates =
        report.existence_fast_path_candidates + report.count_fast_path_candidates;
    black_box(&report);
    println!(
        "kernel_execution evidence_report entries={} elapsed_us={} entries_per_us={:.3} fast_path_candidates={} candidate_rate_per_1k_entries={:.3} existence_candidates={} count_candidates={} existence_exact={} count_exact={} grain_index_count={} indexed_entries={} fallback_entries={} fallback_rate_per_1k_entries={:.3} hidden_entry_filtering_applied={} target_index_kind_count={} fallback_count={}",
        report.evidence_entry_count,
        elapsed_us,
        per_us(report.evidence_entry_count, elapsed_us),
        fast_path_candidates,
        per_1k(fast_path_candidates, report.evidence_entry_count),
        report.existence_fast_path_candidates,
        report.count_fast_path_candidates,
        report.existence_fast_path_exact,
        report.count_fast_path_exact,
        report.index_reports.len(),
        indexed_entries,
        fallback_entries,
        per_1k(fallback_entries, report.evidence_entry_count),
        report.hidden_entry_filtering_applied,
        report.target_index_kinds.len(),
        report.fallback_reasons.len()
    );
}

fn per_us(count: usize, elapsed_us: u128) -> f64 {
    count as f64 / elapsed_us.max(1) as f64
}

fn per_1k(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 1_000.0 / total as f64
    }
}

fn percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 100.0 / total as f64
    }
}

fn json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveOqlOutputMode::JsonRows),
        ..ResolveOptions::default()
    }
}

fn protected_json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveOqlOutputMode::JsonRows),
        security: SecurityContext {
            metadata_disclosure_policy: cove_oql::MetadataDisclosurePolicy::AllowProtected,
            ..SecurityContext::default()
        },
        ..ResolveOptions::default()
    }
}

fn validation_options() -> ValidationOptions {
    ValidationOptions {
        semantic: true,
        ..ValidationOptions::default()
    }
}

fn minimal_object_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    write_catalog(catalog)
}

fn minimal_object_with_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    write_catalog(catalog)
}

fn write_catalog(catalog: ObjectTypeCatalog) -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: catalog.types.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().expect("catalog serialization"),
    });
    writer.write().expect("minimal COVE file")
}
