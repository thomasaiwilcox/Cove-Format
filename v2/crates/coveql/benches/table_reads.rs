use std::{hint::black_box, sync::Arc, time::Instant};

use arrow_array::RecordBatch;
use cove_core::{
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile,
        SectionKind, FEATURE_OBJECT_PROFILE, FEATURE_SEMANTIC_MAP,
    },
    page::ColumnPageIndexEntryV1,
    page_payload::ColumnPagePayloadV1,
    profile::cove_o::{
        ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1, RecordKind, TemporalRowEntryV1,
        TemporalSegmentHeaderV1, TemporalSegmentIndex, TemporalSegmentIndexEntryV1,
        OBJECT_TYPE_FLAG_ENTITY_OBJECT, TEMPORAL_ROW_ENTRY_LEN, TEMPORAL_SEGMENT_HEADER_LEN,
    },
    reader::ValidationOptions,
    segment::{TableColumnDirectoryEntryV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN},
    writer::{MinimalCoveWriter, SectionPayload},
};
use cove_map::{
    projected_record_batches_from_cove_o_bytes, ProjectionBatchOptions, ProjectionFilter,
    ProjectionFilterLiteral, ProjectionFilterOp,
};
use coveql::{
    execute_physical_planned_query, execute_planned_query, parse_resolve_and_plan_query,
    parse_resolve_plan_and_build_physical_plan, parse_resolve_plan_and_execute_query,
    register_datafusion_coveql_provider_for_plan, CoveQlExecutionResult, CoveQlOutputMode,
    ExecutionOptions, KernelExecutionMode, KernelExecutionOptions, ParseOptions,
    PhysicalPlanOptions, PlanOptions, ResolveOptions,
};
use datafusion::execution::context::{SessionConfig, SessionContext};
use datafusion::physical_plan::collect as collect_physical_plan;
use serde_json::json;
use tokio::runtime::Runtime;

const ROW_COUNT: usize = 65_536;
const ITERATIONS: usize = 25;
const PROVIDER_SCAN_QUERY: &str = "table(thing_projection).select(active, enabled)";

fn main() {
    let runtime = Runtime::new().expect("tokio runtime");
    for divisor in [2usize, 4, 16] {
        run_filtered_suite(
            &runtime,
            &format!("selectivity_1_of_{divisor}_select_active"),
            divisor,
            "active",
        );
    }
    run_filtered_suite(
        &runtime,
        "selectivity_1_of_4_select_enabled_filter_active",
        4,
        "enabled",
    );
    run_filterless_suite(&runtime);
    run_computed_residual_suite(&runtime);
}

fn run_filtered_suite(
    runtime: &Runtime,
    label: &str,
    selectivity_divisor: usize,
    selected_column: &str,
) {
    let bytes = projection_backed_bool_object_file(ROW_COUNT, selectivity_divisor);
    let expected_rows = ROW_COUNT / selectivity_divisor;
    let direct_filter_query =
        format!("table(thing_projection).where(active == true).select({selected_column})");
    let datafusion_sql =
        format!("select {selected_column} from thing_projection_coveql where active = true");
    println!(
        "table_reads fixture={} rows={} true_rows={} bytes={} iterations={}",
        label,
        ROW_COUNT,
        expected_rows,
        bytes.len(),
        ITERATIONS
    );

    let direct_plan = plan_arrow_query(&bytes, &direct_filter_query);
    let physical_plan = parse_resolve_plan_and_build_physical_plan(
        &bytes,
        &direct_filter_query,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        validation_options(),
    )
    .expect("direct CoveQL physical table-read plan");

    bench_case(
        &format!("{label}:covemap_raw_projection_readback_filter"),
        || {
            let batches = projected_record_batches_from_cove_o_bytes(
                &bytes,
                None,
                "thing_projection",
                &ProjectionBatchOptions {
                    max_rows: None,
                    output_columns: Some(vec![selected_column.into()]),
                    pushed_filters: vec![ProjectionFilter::Compare {
                        column: "active".into(),
                        op: ProjectionFilterOp::Eq,
                        literal: ProjectionFilterLiteral::Boolean(true),
                    }],
                    batch_size: execution_options().batch_size,
                },
            )
            .expect("raw COVE-MAP projection readback");
            let rows = record_batch_rows(&batches);
            assert_eq!(rows, expected_rows);
            rows
        },
    );

    bench_case(&format!("{label}:coveql_direct_preplanned_filter"), || {
        let executed = execute_planned_query(&bytes, direct_plan.clone(), execution_options())
            .expect("execute direct CoveQL plan");
        let rows = arrow_rows(&executed.result);
        assert_eq!(rows, expected_rows);
        rows
    });

    bench_case(&format!("{label}:coveql_physical_prebuilt_filter"), || {
        let executed = execute_physical_planned_query(
            &bytes,
            physical_plan.clone(),
            execution_options(),
            KernelExecutionOptions {
                mode: KernelExecutionMode::ForceKernel,
                ..KernelExecutionOptions::default()
            },
        )
        .expect("execute direct CoveQL physical plan");
        let rows = arrow_rows(&executed.executed.result);
        assert_eq!(rows, expected_rows);
        rows
    });

    bench_case(&format!("{label}:coveql_parse_plan_execute_filter"), || {
        let executed = parse_resolve_plan_and_execute_query(
            &bytes,
            &direct_filter_query,
            ParseOptions::default(),
            ResolveOptions {
                output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                    zero_copy_requested: false,
                }),
                ..ResolveOptions::default()
            },
            PlanOptions::default(),
            execution_options(),
            validation_options(),
        )
        .expect("parse, plan, and execute direct CoveQL query");
        let rows = arrow_rows(&executed.result);
        assert_eq!(rows, expected_rows);
        rows
    });

    let ctx = bench_session_context();
    register_provider(runtime, &ctx, &bytes, label);
    let prepared = runtime.block_on(async {
        let dataframe = ctx.sql(&datafusion_sql).await.expect("DataFusion SQL");
        let started = Instant::now();
        let plan = dataframe
            .create_physical_plan()
            .await
            .expect("DataFusion physical plan");
        println!(
            "table_reads setup={} setup=datafusion_create_physical_plan setup_us={}",
            label,
            started.elapsed().as_micros()
        );
        plan
    });
    let task_ctx = ctx.task_ctx();

    bench_case(&format!("{label}:datafusion_sql_each_iteration"), || {
        runtime.block_on(async {
            let dataframe = ctx.sql(&datafusion_sql).await.expect("DataFusion SQL");
            let batches = dataframe.collect().await.expect("DataFusion collect");
            let rows = record_batch_rows(&batches);
            assert_eq!(rows, expected_rows);
            rows
        })
    });

    bench_case(
        &format!("{label}:datafusion_prebuilt_physical_plan"),
        || {
            runtime.block_on(async {
                let batches = collect_physical_plan(Arc::clone(&prepared), Arc::clone(&task_ctx))
                    .await
                    .expect("DataFusion prebuilt collect");
                let rows = record_batch_rows(&batches);
                assert_eq!(rows, expected_rows);
                rows
            })
        },
    );
}

fn run_filterless_suite(runtime: &Runtime) {
    let label = "filterless_scan_baseline";
    let bytes = projection_backed_bool_object_file(ROW_COUNT, 4);
    let direct_scan_query = "table(thing_projection).select(active)";
    let direct_plan = plan_arrow_query(&bytes, direct_scan_query);
    println!(
        "table_reads fixture={} rows={} bytes={} iterations={}",
        label,
        ROW_COUNT,
        bytes.len(),
        ITERATIONS
    );

    bench_case(&format!("{label}:covemap_raw_projection_readback"), || {
        let batches = projected_record_batches_from_cove_o_bytes(
            &bytes,
            None,
            "thing_projection",
            &ProjectionBatchOptions {
                output_columns: Some(vec!["active".into()]),
                batch_size: execution_options().batch_size,
                ..ProjectionBatchOptions::default()
            },
        )
        .expect("raw COVE-MAP projection readback");
        let rows = record_batch_rows(&batches);
        assert_eq!(rows, ROW_COUNT);
        rows
    });

    bench_case(&format!("{label}:coveql_direct_preplanned"), || {
        let executed = execute_planned_query(&bytes, direct_plan.clone(), execution_options())
            .expect("execute direct CoveQL scan");
        let rows = arrow_rows(&executed.result);
        assert_eq!(rows, ROW_COUNT);
        rows
    });

    let ctx = bench_session_context();
    register_provider(runtime, &ctx, &bytes, label);
    bench_case(&format!("{label}:datafusion_sql_each_iteration"), || {
        runtime.block_on(async {
            let dataframe = ctx
                .sql("select active from thing_projection_coveql")
                .await
                .expect("DataFusion SQL");
            let batches = dataframe.collect().await.expect("DataFusion collect");
            let rows = record_batch_rows(&batches);
            assert_eq!(rows, ROW_COUNT);
            rows
        })
    });
}

fn run_computed_residual_suite(runtime: &Runtime) {
    let label = "computed_select_residual_filter";
    let bytes = projection_backed_bool_object_file(ROW_COUNT, 4);
    let ctx = bench_session_context();
    let provider_plan = parse_resolve_and_plan_query(
        &bytes,
        "table(thing_projection).select(value: coalesce(active, false))",
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .expect("CoveQL computed provider plan");
    let started = Instant::now();
    let report = register_datafusion_coveql_provider_for_plan(
        &ctx,
        "computed_projection_coveql",
        Arc::new(bytes),
        &provider_plan,
        execution_options(),
    )
    .expect("register computed CoveQL DataFusion provider");
    println!(
        "table_reads setup={} setup=datafusion_provider_register_once setup_us={} schema_probe_rows={} schema_probe_batches={} scan_projection_pushdown_supported={} scan_filter_pushdown_supported={}",
        label,
        started.elapsed().as_micros(),
        report.row_count,
        report.batch_count,
        report.scan_projection_pushdown_supported,
        report.scan_filter_pushdown_supported
    );
    bench_case(&format!("{label}:datafusion_residual_sql_filter"), || {
        runtime.block_on(async {
            let dataframe = ctx
                .sql("select value from computed_projection_coveql where value = true")
                .await
                .expect("DataFusion SQL");
            let batches = dataframe.collect().await.expect("DataFusion collect");
            let rows = record_batch_rows(&batches);
            assert_eq!(rows, ROW_COUNT / 4);
            rows
        })
    });
}

fn plan_arrow_query(bytes: &[u8], query: &str) -> coveql::PlannedQuery {
    parse_resolve_and_plan_query(
        bytes,
        query,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::ArrowRecordBatch {
                zero_copy_requested: false,
            }),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .expect("CoveQL Arrow plan")
}

fn register_provider(runtime: &Runtime, ctx: &SessionContext, bytes: &[u8], label: &str) {
    let provider_plan = parse_resolve_and_plan_query(
        bytes,
        PROVIDER_SCAN_QUERY,
        ParseOptions::default(),
        ResolveOptions {
            output_mode: Some(CoveQlOutputMode::DataFusionTableProvider),
            ..ResolveOptions::default()
        },
        PlanOptions::default(),
        validation_options(),
    )
    .expect("CoveQL DataFusion provider plan");
    let started = Instant::now();
    let report = register_datafusion_coveql_provider_for_plan(
        ctx,
        "thing_projection_coveql",
        Arc::new(bytes.to_vec()),
        &provider_plan,
        execution_options(),
    )
    .expect("register CoveQL DataFusion provider");
    println!(
        "table_reads setup={} setup=datafusion_provider_register_once setup_us={} schema_probe_rows={} schema_probe_batches={} scan_projection_pushdown_supported={} scan_filter_pushdown_supported={}",
        label,
        started.elapsed().as_micros(),
        report.row_count,
        report.batch_count,
        report.scan_projection_pushdown_supported,
        report.scan_filter_pushdown_supported
    );
    let rows = runtime.block_on(async {
        let dataframe = ctx
            .sql("select active from thing_projection_coveql where active = true")
            .await
            .expect("DataFusion SQL");
        let batches = dataframe.collect().await.expect("DataFusion collect");
        record_batch_rows(&batches)
    });
    black_box(rows);
}

fn bench_case(mut label: &str, mut run: impl FnMut() -> usize) {
    let warmup_rows = run();
    black_box(warmup_rows);
    let started = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        rows = black_box(run());
    }
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "table_reads case={} iterations={} rows_per_iter={} total_us={} avg_us={:.3} rows_per_us={:.3}",
        label,
        ITERATIONS,
        rows,
        elapsed_us,
        elapsed_us as f64 / ITERATIONS as f64,
        rows as f64 / (elapsed_us.max(1) as f64 / ITERATIONS as f64)
    );
    label = black_box(label);
    black_box(label);
}

fn arrow_rows(result: &CoveQlExecutionResult) -> usize {
    let CoveQlExecutionResult::ArrowRecordBatches(batches) = result else {
        panic!("benchmark expected ArrowRecordBatch output");
    };
    record_batch_rows(batches)
}

fn record_batch_rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

fn bench_session_context() -> SessionContext {
    SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1))
}

fn validation_options() -> ValidationOptions {
    ValidationOptions {
        semantic: true,
        ..ValidationOptions::default()
    }
}

fn execution_options() -> ExecutionOptions {
    let mut options = ExecutionOptions::default();
    options.resource_budget.maximum_rows_without_explicit_take = ROW_COUNT;
    options.resource_budget.maximum_decode_bytes = 64 * 1024 * 1024;
    options
}

fn projection_backed_bool_object_file(
    row_count: usize,
    active_selectivity_divisor: usize,
) -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Thing".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![
                PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
                PropertyEntryV1 {
                    property_id: 2,
                    property_name: "enabled".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                },
            ],
        }],
    };
    let active_values = (0..row_count)
        .map(|index| index % active_selectivity_divisor == 0)
        .collect::<Vec<_>>();
    let enabled_values = (0..row_count)
        .map(|index| index % 2 == 0)
        .collect::<Vec<_>>();
    let rows = active_values
        .iter()
        .enumerate()
        .map(|(index, _)| TemporalRowEntryV1 {
            timestamp_us: 10 + index as i64,
            csn: 1 + index as u64,
            branch_key: 0,
            goid: id_bytes(index as u128),
            record_id: id_bytes(index as u128 + 1_000_000),
            record_kind: RecordKind::Baseline,
            prev_ref: None,
        })
        .collect::<Vec<_>>();
    let segment = temporal_segment_with_bool_properties(&rows, &active_values, &enabled_values);
    let index = TemporalSegmentIndex {
        flags: 0,
        entries: vec![temporal_segment_entry_for_rows(
            7,
            &rows,
            segment.len() as u64,
        )],
    };
    let projection = json!({
        "schema_id": "org.coveformat.covemap.v2",
        "section_id": SectionKind::MapProjectionCatalog as u16,
        "mapping_id": "thing-map",
        "mapping_version": "2026.05",
        "projections": [{
            "projection_id": "thing_projection",
            "output_table": "thing_projection",
            "row_grain": "one_row_per_object",
            "anchor": {"object_type": "Thing"},
            "temporal_mode": "latest_committed",
            "multi_value_policy": "reject",
            "columns": [
                {
                    "name": "active",
                    "value": "property.active",
                    "logical_type": "bool"
                },
                {
                    "name": "enabled",
                    "value": "property.enabled",
                    "logical_type": "bool"
                }
            ],
            "output_modes": ["json", "arrow"]
        }]
    });

    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().expect("catalog serialization"),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentIndex as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: index.serialize().expect("segment index serialization"),
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::TemporalSegmentData as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: 1,
        row_count: rows.len() as u64,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: segment,
    });
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::MapProjectionCatalog as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&projection).expect("projection catalog JSON"),
    });
    writer.write().expect("benchmark COVE-O file")
}

fn temporal_segment_entry_for_rows(
    segment_id: u32,
    rows: &[TemporalRowEntryV1],
    length: u64,
) -> TemporalSegmentIndexEntryV1 {
    TemporalSegmentIndexEntryV1 {
        segment_id,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        delta_count: 0,
        snapshot_count: 0,
        baseline_count: rows.len() as u32,
        tombstone_count: 0,
        min_goid: rows.iter().map(|row| row.goid).min().unwrap_or([0; 16]),
        max_goid: rows.iter().map(|row| row.goid).max().unwrap_or([0; 16]),
        offset: 0,
        length,
        checksum: 0,
    }
}

fn temporal_segment_with_bool_properties(
    rows: &[TemporalRowEntryV1],
    active_values: &[bool],
    enabled_values: &[bool],
) -> Vec<u8> {
    assert_eq!(rows.len(), active_values.len());
    assert_eq!(rows.len(), enabled_values.len());
    let row_directory_offset = TEMPORAL_SEGMENT_HEADER_LEN as u64;
    let row_bytes = (rows.len() * TEMPORAL_ROW_ENTRY_LEN) as u64;
    let row_end = row_directory_offset + row_bytes;
    let column_directory_offset = row_end;
    let column_directory_length = 2 * TABLE_COLUMN_DIRECTORY_ENTRY_LEN as u64;
    let page_index_offset = column_directory_offset + column_directory_length;
    let page_index_entry_length = cove_core::page::COLUMN_PAGE_INDEX_ENTRY_LEN as u64;
    let page_index_length = 2 * page_index_entry_length;
    let data_offset = page_index_offset + page_index_length;
    let active_payload = bool_page_payload(rows.len(), active_values);
    let enabled_payload = bool_page_payload(rows.len(), enabled_values);
    let active_data_offset = data_offset;
    let enabled_data_offset = active_data_offset + active_payload.len() as u64;
    let header = TemporalSegmentHeaderV1 {
        segment_id: 7,
        object_type_id: 1,
        time_range_start_us: rows.first().map(|row| row.timestamp_us).unwrap_or(0),
        time_range_end_us: rows.last().map(|row| row.timestamp_us).unwrap_or(0),
        csn_min: rows.first().map(|row| row.csn).unwrap_or(0),
        csn_max: rows.last().map(|row| row.csn).unwrap_or(0),
        row_count: rows.len() as u32,
        morsel_count: u32::from(!rows.is_empty()),
        morsel_row_count: if rows.is_empty() {
            0
        } else {
            rows.len() as u32
        },
        column_count: 2,
        row_directory_offset,
        column_directory_offset,
        page_index_offset,
        data_offset,
        flags: 0,
        checksum: 0,
    };
    let directories = [
        bool_column_directory(
            1,
            page_index_offset,
            page_index_entry_length,
            active_data_offset,
            active_payload.len(),
        ),
        bool_column_directory(
            2,
            page_index_offset + page_index_entry_length,
            page_index_entry_length,
            enabled_data_offset,
            enabled_payload.len(),
        ),
    ];
    let pages = [
        bool_page_index(1, rows.len(), active_data_offset, &active_payload),
        bool_page_index(2, rows.len(), enabled_data_offset, &enabled_payload),
    ];

    let mut bytes = header.serialize().to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.serialize());
    }
    for directory in directories {
        bytes.extend_from_slice(&directory.serialize());
    }
    for page in pages {
        bytes.extend_from_slice(&page.serialize());
    }
    bytes.extend_from_slice(&active_payload);
    bytes.extend_from_slice(&enabled_payload);
    bytes
}

fn bool_page_payload(row_count: usize, values: &[bool]) -> Vec<u8> {
    let value_bytes = values.iter().map(|value| u8::from(*value)).collect();
    ColumnPagePayloadV1::build_single_node(
        row_count as u32,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        None,
        value_bytes,
    )
    .expect("boolean page payload")
}

fn bool_column_directory(
    column_id: u32,
    page_index_offset: u64,
    page_index_length: u64,
    data_offset: u64,
    payload_len: usize,
) -> TableColumnDirectoryEntryV1 {
    TableColumnDirectoryEntryV1 {
        column_id,
        logical_type: CoveLogicalType::Bool,
        physical_kind: CovePhysicalKind::Boolean,
        flags: 0,
        page_index_offset,
        page_index_length,
        data_offset,
        data_length: payload_len as u64,
        stats_ref: u32::MAX,
        domain_ref: u32::MAX,
        checksum: 0,
    }
}

fn bool_page_index(
    column_id: u32,
    row_count: usize,
    page_offset: u64,
    payload: &[u8],
) -> ColumnPageIndexEntryV1 {
    ColumnPageIndexEntryV1 {
        column_id,
        morsel_id: 0,
        row_count: row_count as u32,
        non_null_count: row_count as u32,
        null_count: 0,
        encoding_root: CoveEncodingKind::PlainFixed as u32,
        page_offset,
        page_length: payload.len() as u64,
        uncompressed_length: payload.len() as u64,
        stats_ref: u32::MAX,
        flags: 0,
        checksum: checksum::crc32c(&payload),
    }
}

fn id_bytes(value: u128) -> [u8; 16] {
    value.to_be_bytes()
}
