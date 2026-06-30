use std::{
    fmt, fs,
    ops::Range,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use arrow_array::{
    builder::{Int32Builder, ListBuilder},
    ArrayRef as ArrowArrayRef, RecordBatch as ArrowRecordBatch,
};
use async_trait::async_trait;
use cove_arrow::parquet::{convert_arrow_record_batches, ParquetConversionOptions};
use cove_cache::{CoveCoverageCacheHeaderV2, CoverageCacheEntryV2, CoverageCacheV2};
#[cfg(all(feature = "covm", feature = "covx"))]
use cove_core::artifact::covx::{CovxFile, CovxHeaderV1, CovxPostscriptV1, CovxReferencedFileV1};
use cove_core::{
    artifact::covemap::{
        CovemapFile, CovemapHeaderV1, CovemapPayloadEncodingV2, CovemapPostscriptV1,
        CovemapSection, CovemapSectionEntryV1,
    },
    canonical::CanonicalValue,
    checksum,
    codec::{
        CodecExtensionDescriptorV2, CodecFallbackPolicyV2, CodecRequirementV2,
        CodecSpecificationStatusV2, LogicalPage, ABSENT_REF,
    },
    constants::{
        CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind,
        StorageClass, ValueTag, FEATURE_COLUMN_DOMAINS, FEATURE_ENGINE_PROFILE,
        FEATURE_EXTENDED_FEATURE_SET, FEATURE_REDACTIONS, FEATURE_REGISTERED_ENCODINGS,
        FEATURE_SEMANTIC_MAP, FEATURE_TABLE_PROFILE,
    },
    dictionary::{FileDictionary, FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
    domain::ColumnDomain,
    encoding::{
        bit_packed::BitPackedPayload,
        local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
    },
    feature_binding::{FeatureScopeV2, OperationKindV2},
    feature_scope::{
        cove_column_page_target_ref, ExtendedFeatureSetHeaderV2, ExtendedFeatureSetV2,
        ProfileCapabilityEntryV2, ProfileCapabilityMatrixHeaderV2, ProfileCapabilityMatrixV2,
    },
    footer::{CoveFooterHeaderV1, CoveSectionEntryV1, FOOTER_HEADER_SIZE, SECTION_ENTRY_SIZE},
    header::HEADER_SIZE,
    index::{
        aggregate::{AggregateEntry, AggregateSynopsis, SynopsisAccuracy, SynopsisKind},
        composite::{
            CompositeIndex, CompositeTransformKind, CompositeZoneIndexHeaderV1,
            COMPOSITE_ZONE_INDEX_HEADER_LEN,
        },
        inverted::{
            InvertedEntry, InvertedKeyKind, InvertedMorselIndex, InvertedMorselIndexHeaderV1,
        },
        lookup::{
            LookupEntry, LookupIndex, LookupIndexHeaderV1, LookupIndexKind, LookupKeyKind,
            LookupUniqueness,
        },
        topn::{TopNDirection, TopNSummary, TOPN_ZONE_SUMMARY_LEN},
    },
    page::{
        ColumnPageIndexEntryV1, PAGE_FLAG_ALL_NON_NULL, PAGE_FLAG_ALL_NULL,
        PAGE_FLAG_STATS_ONLY_CONSTANT,
    },
    page_payload::ColumnPagePayloadV1,
    postscript::{CovePostscriptV1, POSTSCRIPT_TOTAL_SIZE},
    profile::cove_e::{
        EngineMountPolicyV1, EngineProfileEntryV1, EngineProfileRegistry,
        ExecutionCodeCanonicality, ExecutionCodeComparisonScope, ExecutionCodeDescriptorV1,
        ExecutionCodeKind, ExecutionCodeLifetime, FileCodeMappingKind, MissingValuePolicy,
        NullCodePolicy, ReverseLookupPolicy, StaleMappingPolicy,
    },
    reader,
    redaction::{RedactionEntry, RedactionManifest},
    row_ref::RowRef,
    segment::TableSegmentPayloadV1,
    table::{ColumnEntry, TableCatalog, TableEntry},
    wire,
    writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment, SectionPayload},
    zone_stats::{
        StatKind, StatScalar, ZoneScope, ZoneStatFlags, ZoneStats, ZoneStatsEntry, ZoneStatsSection,
    },
    CoveError,
};
#[cfg(feature = "covm")]
use cove_core::{
    artifact::covm::{CovmFile, CovmFileEntryV1, CovmHeaderV1, CovmPostscriptV1},
    constants::DigestAlgorithm,
};
use cove_coverage::{
    coverage_set_payload_checksum, CoverageExactnessV2, CoverageGranularityV2, CoverageProofKindV2,
    CoverageProofRecordV2, CoverageProofStrengthV2, CoverageProviderDescriptorV2,
    CoverageSetEntryV2, CoverageSetHeaderV2, CoverageSetV2, PredicateAstNodeV2,
    PredicateAstOperandRefV2, PredicateAstPayloadHeaderV2, PredicateFormKindV2, PredicateLiteralV2,
    PredicateNormalFormV2, PredicateNullPolicyV2, PredicateOpV2, PredicateOperandKindV2,
};
#[cfg(feature = "covm")]
use cove_datafusion::register::{cove_table_from_covm_path, register_cove_covm};
#[cfg(feature = "covi")]
use cove_datafusion::{
    bootstrap::bootstrap_bytes_with_covi_artifacts,
    metadata_aggregate::exact_covi_unfiltered_min_max,
};
use cove_datafusion::{
    bootstrap::{
        bootstrap_bytes, bootstrap_bytes_with_options, bootstrap_local_file,
        bootstrap_local_file_async, bootstrap_range_reader_with_options, CoveMetadataCache,
    },
    dataset_state::embedded_coverage_snapshot_validity_ref,
    decode::{
        decode_local_dataset_scan_tasks, decode_scan, native_bool_group_count_scan,
        native_bool_i64_group_aggregate_scan, native_filecode_group_count_scan,
        native_filecode_i64_group_aggregate_scan, native_i64_i64_group_aggregate_scan,
    },
    expr_lowering::{lower_filter, LowerExpr, LowerLiteral, LowerOperator},
    overlay::{CoveOverlaySnapshot, OverlayFile, OverlayFileIdentity, RowRange, RowVisibility},
    planner::{
        plan_scan, CoveFilterUse, CovePredicate, FilterPlan, NullPredicateKind, NumericPredicateOp,
        PredicateLiteral,
    },
    range_reader::{coalesced_range_count, MemoryRangeReader, RangeCoalescingOptions},
    register::{
        cove_table_from_path, cove_table_from_path_async, register_cove_file,
        register_cove_file_async, register_cove_file_format, register_cove_file_with_options,
        register_cove_listing_table, register_cove_listing_table_with_options,
        register_cove_o_projection, register_cove_o_projections, register_cove_overlay_snapshot,
        CoveTableOptions, ExecutionCodePolicy, FilterResidualPolicy,
    },
    task_graph::build_task_graph,
};
#[cfg(feature = "covi")]
use cove_index::{
    execution::CoviAggregateKindV2, CoviAggregateAnswerBlockHeaderV2, CoviAggregateAnswerBlockV2,
    CoviAggregateAnswerV2, CoviArtifactV2, CoviComparatorKindV2, CoviEntryBlockHeaderV2,
    CoviEntryBlockV2, CoviIndexEntryV2, CoviIndexKindV2, CoviIndexRootV2, CoviIndexedTargetKindV2,
    CoviKeyBlockHeaderV2, CoviKeyBlockV2, CoviKeyEncodingKindV2, CoviPostingsBlockHeaderV2,
    CoviPostingsBlockV2, CoviReferencedFileV2, CoviSectionKindV2, CoviSectionPayloadV2,
    CoviSnapshotValidityV2, IndexCapabilityExactnessV2, IndexCapabilityV2, IndexOnlyCapabilityV2,
};
use cove_map::cove_o_from_paths;
use datafusion::object_store::{
    memory::InMemory, path::Path, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult,
};
use datafusion::{
    arrow::{
        array::{
            Array, BinaryArray, BinaryViewArray, DictionaryArray, Float32Array, Int32Array,
            ListArray, StringArray, StringViewArray,
        },
        datatypes::UInt32Type,
        util::pretty::pretty_format_batches,
    },
    assert_batches_eq,
    catalog::TableProvider,
    common::{stats::Precision, Column, ScalarValue},
    logical_expr::{
        expr::InList, Between, BinaryExpr, Expr, Like, Operator, TableProviderFilterPushDown,
    },
    physical_plan::{execution_plan::collect as collect_physical_plan, ExecutionPlan},
    prelude::SessionContext,
};
use futures::stream::BoxStream;
use parquet::arrow::ArrowWriter;
use serde_json::{json, Value};
use url::Url;

static NEXT_FILE_ID: AtomicU64 = AtomicU64::new(0);
const UNKNOWN_SCOPED_FEATURE: u64 = 1;

async fn collect_sql_with_cove_metric(
    ctx: &SessionContext,
    sql: &str,
    metric_name: &str,
) -> (Vec<datafusion::arrow::record_batch::RecordBatch>, usize) {
    let dataframe = ctx.sql(sql).await.unwrap();
    let plan = dataframe.create_physical_plan().await.unwrap();
    let batches = collect_physical_plan(Arc::clone(&plan), ctx.task_ctx())
        .await
        .unwrap();
    (batches, execution_plan_metric_sum(&plan, metric_name))
}

async fn collect_sql_with_cove_metrics(
    ctx: &SessionContext,
    sql: &str,
    metric_names: &[&str],
) -> (
    Vec<datafusion::arrow::record_batch::RecordBatch>,
    Vec<usize>,
) {
    let dataframe = ctx.sql(sql).await.unwrap();
    let plan = dataframe.create_physical_plan().await.unwrap();
    let batches = collect_physical_plan(Arc::clone(&plan), ctx.task_ctx())
        .await
        .unwrap();
    let metrics = metric_names
        .iter()
        .map(|metric_name| execution_plan_metric_sum(&plan, metric_name))
        .collect();
    (batches, metrics)
}

fn execution_plan_metric_sum(plan: &Arc<dyn ExecutionPlan>, metric_name: &str) -> usize {
    let own = plan
        .metrics()
        .and_then(|metrics| metrics.sum_by_name(metric_name))
        .map(|metric| metric.as_usize())
        .unwrap_or(0);
    own + plan
        .children()
        .into_iter()
        .map(|child| execution_plan_metric_sum(child, metric_name))
        .sum::<usize>()
}

fn assert_typed_i64_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=typed_numeric_i64"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("semantic_domain=cove.datafusion.native.i64"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap")
            || explain_text.contains("null_policy=validity-bitmap-nulls-never-match"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=none"),
        "{explain_text}"
    );
    assert!(explain_text.contains("fallback=none"), "{explain_text}");
}

fn assert_bool_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=boolean_dense"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("semantic_domain=cove.datafusion.native.bool"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=none"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("fallback=page-decode-boundary"),
        "{explain_text}"
    );
}

fn assert_bool_i64_group_aggregate_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=group_key:boolean_dense,value:typed_numeric_i64"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains(
            "semantic_domain=key:cove.datafusion.native.bool,value:cove.datafusion.native.i64"
        ),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=none"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("fallback=page-decode-boundary"),
        "{explain_text}"
    );
}

fn assert_i64_i64_group_aggregate_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=group_key:typed_numeric_i64,value:typed_numeric_i64"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains(
            "semantic_domain=key:cove.datafusion.native.i64,value:cove.datafusion.native.i64"
        ),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=none"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("fallback=page-decode-boundary"),
        "{explain_text}"
    );
}

fn assert_filecode_i64_group_aggregate_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=group_key:filecode_utf8,value:typed_numeric_i64"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains(
            "semantic_domain=key:file-local-dictionary-to-canonical-utf8,value:cove.datafusion.native.i64"
        ),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=group-label-output"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("fallback=page-decode-boundary"),
        "{explain_text}"
    );
}

fn assert_filecode_join_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=filecode_utf8_execution_code_u32"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("semantic_domain=file-local-dictionary-to-canonical-utf8"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("null_policy=validity-bitmap-nulls-never-match"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=join-key-canonicalization"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("fallback=page-decode-boundary"),
        "{explain_text}"
    );
}

fn assert_rowset_count_native_contract(explain_text: &str) {
    assert!(
        explain_text.contains("representation=rowset_count"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("semantic_domain=none"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("kernel=shared_cove_core"),
        "{explain_text}"
    );
    assert!(
        explain_text.contains("decode_boundary=none"),
        "{explain_text}"
    );
    assert!(explain_text.contains("fallback=none"), "{explain_text}");
}

#[path = "native_single_file/filter_pushdown.rs"]
mod filter_pushdown;
#[path = "native_single_file/metadata_overlay.rs"]
mod metadata_overlay;
#[path = "native_single_file/native_aggregates.rs"]
mod native_aggregates;
#[path = "native_single_file/native_projection.rs"]
mod native_projection;
#[path = "native_single_file/registration.rs"]
mod registration;

fn identity_for_state(state: &cove_datafusion::dataset_state::DatasetState) -> OverlayFileIdentity {
    OverlayFileIdentity {
        file_id: *state.file_id(),
        file_len: state.file_len(),
        footer_crc32c: state.footer_crc32c(),
        digest: None,
    }
}

fn aggregate_count_entry(
    table_id: u32,
    column_id: u32,
    row_count: u32,
    null_count: u32,
) -> AggregateEntry {
    AggregateEntry {
        table_id,
        segment_id: u32::MAX,
        morsel_id: u32::MAX,
        column_id,
        synopsis_kind: SynopsisKind::Count,
        key_kind: 0,
        accuracy: SynopsisAccuracy::Exact,
        flags: 0,
        row_count,
        null_count,
        payload_offset: 0,
        payload_length: 0,
        checksum: 0,
    }
}

fn dictionary_items_file_with_m4d_metadata() -> Vec<u8> {
    let catalog = dictionary_items_payload_catalog();
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    segment.set_column_pages(2, vec![varbytes_page(2, varbytes(&["first", "second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&sample_dictionary());
    writer.push_aggregate_synopsis(&AggregateSynopsis::from_entries(vec![
        aggregate_count_entry(7, 1, 2, 0),
    ]));
    writer.push_composite_zone_index(&composite_index(7, vec![1], vec![0, 0, 0]));
    writer.push_topn_summary(&topn_summary(7, 1, 0, 0, TopNDirection::Largest, 1));
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn nullable_events_file_with_count() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 11,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 4,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "maybe",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    true,
                ),
            ],
        }],
    };

    let mut mixed = ScanSegment::new(11, 0, 0, 2, 2);
    mixed.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[1, 2]))]);
    mixed.set_column_pages(2, vec![nullable_numcode_page(&[Some(10), None])]);

    let mut all_null = ScanSegment::new(11, 1, 2, 1, 2);
    all_null.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[3]))]);
    all_null.set_column_pages(2, vec![nullable_numcode_page(&[None])]);

    let mut all_non_null = ScanSegment::new(11, 2, 3, 1, 2);
    all_non_null.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[4]))]);
    all_non_null.set_column_pages(2, vec![nullable_numcode_page(&[Some(40)])]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_aggregate_synopsis(&AggregateSynopsis::from_entries(vec![
        aggregate_count_entry(11, 2, 4, 2),
    ]));
    writer.push_segment(mixed);
    writer.push_segment(all_null);
    writer.push_segment(all_non_null);
    writer.write().unwrap()
}

fn composite_pairs_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 17,
            namespace: "public".into(),
            name: "pairs".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "left",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "right",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    3,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(17, 0, 0, 2, 3);
    segment.morsel_row_count = 1;
    segment.set_column_pages(
        1,
        vec![
            filecode_page(1, filecodes(&[0])),
            filecode_page(1, filecodes(&[1])),
        ],
    );
    segment.set_column_pages(
        2,
        vec![
            filecode_page(1, filecodes(&[1])),
            filecode_page(1, filecodes(&[0])),
        ],
    );
    segment.set_column_pages(
        3,
        vec![
            varbytes_page(1, varbytes(&["hit"])),
            varbytes_page(1, varbytes(&["miss"])),
        ],
    );

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&sample_dictionary());
    writer.push_composite_zone_index(&composite_index(17, vec![1, 2], vec![0, 1, 0, 0]));
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn topn_events_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 19,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "id",
                CoveLogicalType::Int64,
                CovePhysicalKind::NumCode,
                false,
            )],
        }],
    };
    let mut low = ScanSegment::new(19, 0, 0, 1, 1);
    low.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[1]))]);
    let mut high = ScanSegment::new(19, 1, 1, 1, 1);
    high.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[9]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_topn_summary(&topn_summary(19, 1, 1, 0, TopNDirection::Largest, 9));
    writer.push_segment(low);
    writer.push_segment(high);
    writer.write().unwrap()
}

fn i64_key_file(values: &[i64]) -> Vec<u8> {
    let row_count = u32::try_from(values.len()).unwrap();
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 21,
            namespace: "public".into(),
            name: "keys".into(),
            row_count: u64::from(row_count),
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "id",
                CoveLogicalType::Int64,
                CovePhysicalKind::NumCode,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(21, 0, 0, row_count, 1);
    segment.set_column_pages(1, vec![numcode_page(row_count, numcode_i64(values))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn numeric_scores_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 22,
            namespace: "public".into(),
            name: "scores".into(),
            row_count: 5,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "score",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let mut first = ScanSegment::new(22, 0, 0, 3, 2);
    first.set_column_pages(1, vec![numcode_page(3, numcode_i64(&[1, 2, 1]))]);
    first.set_column_pages(2, vec![numcode_page(3, numcode_i64(&[10, 20, 30]))]);

    let mut second = ScanSegment::new(22, 1, 3, 2, 2);
    second.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[2, 1]))]);
    second.set_column_pages(2, vec![numcode_page(2, numcode_i64(&[40, 50]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(first);
    writer.push_segment(second);
    writer.write().unwrap()
}

fn nullable_i64_key_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 20,
            namespace: "public".into(),
            name: "keys".into(),
            row_count: 4,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "maybe",
                CoveLogicalType::Int64,
                CovePhysicalKind::NumCode,
                true,
            )],
        }],
    };
    let mut segment = ScanSegment::new(20, 0, 0, 4, 1);
    segment.set_column_pages(
        1,
        vec![nullable_numcode_page(&[Some(10), None, Some(40), None])],
    );

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn composite_index(
    table_id: u32,
    key_columns: Vec<u32>,
    tuple_entries: Vec<u32>,
) -> CompositeIndex {
    let mut entries = Vec::new();
    let tuple_width = key_columns.len() + 2;
    for tuple in tuple_entries.chunks_exact(tuple_width) {
        for code in &tuple[..key_columns.len()] {
            entries.extend_from_slice(&u64::from(*code).to_le_bytes());
        }
        entries.extend_from_slice(&tuple[key_columns.len()].to_le_bytes());
        entries.extend_from_slice(&tuple[key_columns.len() + 1].to_le_bytes());
    }
    CompositeIndex {
        header: CompositeZoneIndexHeaderV1 {
            table_id,
            key_column_count: key_columns.len() as u16,
            transform_kind: CompositeTransformKind::Tuple,
            flags: 0,
            zone_count: tuple_entries.len() as u32,
            key_columns_offset: COMPOSITE_ZONE_INDEX_HEADER_LEN as u64,
            entries_offset: 0,
            entries_length: 0,
            checksum: 0,
        },
        key_columns,
        entries,
    }
}

fn topn_summary(
    table_id: u32,
    column_id: u32,
    segment_id: u32,
    morsel_id: u32,
    direction: TopNDirection,
    value: u64,
) -> TopNSummary {
    let mut payload = Vec::new();
    payload.extend_from_slice(&value.to_le_bytes());
    payload.extend_from_slice(&1u64.to_le_bytes());
    TopNSummary {
        table_id,
        column_id,
        segment_id,
        morsel_id,
        direction,
        value_count: 1,
        flags: 0,
        payload_offset: TOPN_ZONE_SUMMARY_LEN as u64,
        payload_length: payload.len() as u64,
        checksum: 0,
        payload,
    }
}

fn primitive_events_file() -> Vec<u8> {
    primitive_events_writer().write().unwrap()
}

fn fixed_uuid_events_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 10,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "uid",
                    CoveLogicalType::Uuid,
                    CovePhysicalKind::FixedBytes,
                    false,
                ),
                column(
                    3,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(10, 0, 0, 3, 3);
    segment.set_column_pages(1, vec![numcode_page(3, numcode_i64(&[1, 2, 3]))]);
    segment.set_column_pages(
        2,
        vec![fixedbytes_page(
            3,
            [uuid_bytes(1), uuid_bytes(2), uuid_bytes(3)].concat(),
        )],
    );
    segment.set_column_pages(
        3,
        vec![varbytes_page(3, varbytes(&["alpha", "beta", "gamma"]))],
    );

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn uuid_bytes(last: u8) -> Vec<u8> {
    let mut out = vec![0u8; 16];
    out[15] = last;
    out
}

fn float_metrics_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "f32",
                    CoveLogicalType::Float32,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    3,
                    "f64",
                    CoveLogicalType::Float64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(1, 0, 0, 3, 3);
    segment.set_column_pages(1, vec![numcode_page(3, numcode_i64(&[1, 2, 3]))]);
    segment.set_column_pages(2, vec![numcode_page(3, numcode_f32(&[1.5, 2.25, -3.0]))]);
    segment.set_column_pages(3, vec![numcode_page(3, numcode_f64(&[1.5, 2.25, -3.0]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

#[cfg(feature = "covi")]
fn float_metric_sum_avg_covi_artifact(bytes: &[u8]) -> Vec<u8> {
    let state = bootstrap_bytes("float_metric_covi_identity", bytes.to_vec()).unwrap();
    let identity = state.file(0).unwrap().identity();
    let dataset_id = identity.file_id;
    let digest =
        cove_core::digest::compute_digest(cove_core::constants::DigestAlgorithm::Sha256, bytes)
            .unwrap();
    let mut snapshot_id = [0u8; 16];
    snapshot_id[0..4].copy_from_slice(&checksum::crc32c(bytes).to_le_bytes());
    snapshot_id[4..8].copy_from_slice(&identity.footer_crc32c.to_le_bytes());
    snapshot_id[8..16].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    let root = CoviIndexRootV2 {
        index_root_id: 0,
        indexed_target_kind: CoviIndexedTargetKindV2::TableColumn,
        index_kind: CoviIndexKindV2::AggregateOnly,
        coverage_granularity: 0,
        proof_strength: CoverageProofStrengthV2::ExactConservative as u8,
        exactness: 0,
        flags: 0,
        table_id: 1,
        column_id: 3,
        object_type_id: u32::MAX,
        property_id: u32::MAX,
        path_ref: u32::MAX,
        semantic_dimension_ref: u32::MAX,
        logical_type: CoveLogicalType::Float64 as u16,
        physical_kind: CovePhysicalKind::NumCode as u8,
        key_encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes as u8,
        comparator_kind: CoviComparatorKindV2::CanonicalOrdering as u16,
        collation_id: 0,
        null_semantics: 0,
        sort_order: 0,
        value_count: 3,
        distinct_count: 0,
        null_count: 0,
        min_key_ref: u32::MAX,
        max_key_ref: u32::MAX,
        key_block_section_id: 1,
        entry_block_section_id: 2,
        postings_block_section_id: 3,
        aggregate_block_section_id: 4,
        coverage_set_ref: u32::MAX,
        capability_ref: 0,
        snapshot_validity_ref: 0,
        checksum: 0,
    };
    let capability = IndexCapabilityV2 {
        capability_id: 0,
        index_root_id: 0,
        flags: 0,
        supports_eq: 0,
        supports_range: 0,
        supports_membership: 0,
        supports_prefix: 0,
        supports_contains: 0,
        supports_count: 1,
        supports_min: 0,
        supports_max: 0,
        supports_sum: 1,
        supports_distinct_count: 0,
        supports_join_coverage: 0,
        supports_index_only: 1,
        exactness: IndexCapabilityExactnessV2::Exact,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        null_semantics: 0,
        reserved: 0,
        snapshot_validity_ref: 0,
        coverage_provider_ref: u32::MAX,
        checksum: 0,
    };
    let index_only = [CoviAggregateKindV2::Sum, CoviAggregateKindV2::Avg]
        .into_iter()
        .map(|kind| IndexOnlyCapabilityV2 {
            capability_id: 0,
            aggregate_kind: kind as u16,
            predicate_supported: 0,
            exactness: IndexCapabilityExactnessV2::Exact,
            null_semantics: 0,
            flags: 0,
            snapshot_validity_ref: 0,
            required_visibility_overlay_ref: u32::MAX,
            checksum: 0,
        })
        .collect::<Vec<_>>();
    let key_block = CoviKeyBlockV2 {
        header: CoviKeyBlockHeaderV2 {
            magic: CoviKeyBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviKeyBlockHeaderV2::LEN as u16,
            reserved0: 0,
            key_block_id: 1,
            index_root_id: 0,
            key_count: 0,
            encoding_kind: CoviKeyEncodingKindV2::CanonicalValueBytes,
            comparator_kind: CoviComparatorKindV2::CanonicalOrdering,
            flags: 0,
            key_data_offset: CoviKeyBlockHeaderV2::LEN as u64,
            key_data_length: 0,
            checksum: 0,
        },
        key_data: Vec::new(),
    };
    let entry_block = CoviEntryBlockV2 {
        header: CoviEntryBlockHeaderV2 {
            magic: CoviEntryBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviEntryBlockHeaderV2::LEN as u16,
            entry_len: CoviIndexEntryV2::LEN as u16,
            entry_block_id: 2,
            index_root_id: 0,
            entry_count: 0,
            key_block_id: 1,
            postings_block_id: 3,
            aggregate_block_id: 4,
            entries_offset: CoviEntryBlockHeaderV2::LEN as u64,
            entries_length: 0,
            flags: 0,
            checksum: 0,
        },
        entries: Vec::new(),
    };
    let postings_block = CoviPostingsBlockV2 {
        header: CoviPostingsBlockHeaderV2 {
            magic: CoviPostingsBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviPostingsBlockHeaderV2::LEN as u16,
            postings_header_len: cove_index::CoviPostingsHeaderV2::LEN as u16,
            postings_block_id: 3,
            index_root_id: 0,
            postings_count: 0,
            row_ordinal_set_count: 0,
            postings_headers_offset: CoviPostingsBlockHeaderV2::LEN as u64,
            row_ordinal_headers_offset: 0,
            postings_payload_offset: 0,
            postings_payload_length: 0,
            flags: 0,
            checksum: 0,
        },
        postings: Vec::new(),
        row_ordinal_sets: Vec::new(),
        payload: Vec::new(),
    };
    let sum_payload = 0.75f64.to_bits().to_le_bytes();
    let mut aggregate_payload = Vec::new();
    let aggregate_answers = [CoviAggregateKindV2::Sum, CoviAggregateKindV2::Avg]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let value_ref = u32::try_from(aggregate_payload.len()).unwrap();
            aggregate_payload.extend_from_slice(&sum_payload);
            CoviAggregateAnswerV2 {
                aggregate_answer_ref: index as u32,
                index_root_id: 0,
                aggregate_kind: kind as u16,
                exactness: IndexCapabilityExactnessV2::Exact as u8,
                null_semantics: 0,
                flags: 0,
                row_count: 3,
                null_count: 0,
                non_null_count: 3,
                value_ref,
                predicate_form_ref: u32::MAX,
                snapshot_validity_ref: 0,
                checksum: 0,
            }
        })
        .collect::<Vec<_>>();
    let aggregate_block = CoviAggregateAnswerBlockV2 {
        header: CoviAggregateAnswerBlockHeaderV2 {
            magic: CoviAggregateAnswerBlockHeaderV2::MAGIC,
            version_major: 2,
            version_minor: 0,
            header_len: CoviAggregateAnswerBlockHeaderV2::LEN as u16,
            aggregate_answer_len: CoviAggregateAnswerV2::LEN as u16,
            aggregate_block_id: 4,
            index_root_id: 0,
            aggregate_answer_count: aggregate_answers.len() as u32,
            aggregate_answers_offset: CoviAggregateAnswerBlockHeaderV2::LEN as u64,
            aggregate_payload_offset: 0,
            aggregate_payload_length: aggregate_payload.len() as u64,
            flags: 0,
            checksum: 0,
        },
        answers: aggregate_answers,
        payload: aggregate_payload,
    };
    let index_only_payload = index_only
        .iter()
        .flat_map(|capability| capability.serialize().unwrap())
        .collect::<Vec<_>>();
    CoviArtifactV2::serialize_with_sections(
        dataset_id,
        snapshot_id,
        &[CoviReferencedFileV2 {
            file_ref: 0,
            flags: 0,
            file_id: identity.file_id,
            file_len: identity.file_len,
            footer_crc32c: identity.footer_crc32c,
            digest_algorithm: cove_core::constants::DigestAlgorithm::Sha256 as u16,
            digest_len: digest.len() as u16,
            digest_offset: 0,
            uri_ref: u32::MAX,
            schema_fingerprint_ref: u32::MAX,
            checksum: 0,
        }],
        &[CoviSnapshotValidityV2 {
            snapshot_validity_ref: 0,
            dataset_id,
            snapshot_id,
            schema_fingerprint_ref: u32::MAX,
            semantic_map_fingerprint_ref: u32::MAX,
            external_visibility_ref: u32::MAX,
            data_checksum_root_ref: u32::MAX,
            delta_chain_digest_algorithm: cove_core::constants::DigestAlgorithm::None as u16,
            delta_chain_digest_len: 0,
            delta_chain_digest_offset: 0,
            valid_from_us: 0,
            valid_until_us: i64::MAX,
            flags: 0,
            checksum: 0,
        }],
        &[root],
        &[capability],
        &[
            CoviSectionPayloadV2 {
                section_id: 1,
                section_kind: CoviSectionKindV2::KeyBlock,
                payload: key_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 2,
                section_kind: CoviSectionKindV2::EntryBlock,
                payload: entry_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 3,
                section_kind: CoviSectionKindV2::PostingsBlock,
                payload: postings_block.serialize().unwrap(),
                item_count: 0,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 4,
                section_kind: CoviSectionKindV2::AggregateAnswerBlock,
                payload: aggregate_block.serialize().unwrap(),
                item_count: 2,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 5,
                section_kind: CoviSectionKindV2::IndexOnlyCapabilities,
                payload: index_only_payload,
                item_count: 2,
                required_features: 0,
                optional_features: 0,
            },
            CoviSectionPayloadV2 {
                section_id: 6,
                section_kind: CoviSectionKindV2::StringTable,
                payload: digest,
                item_count: 1,
                required_features: 0,
                optional_features: 0,
            },
        ],
    )
    .unwrap()
}

fn stats_only_numeric_metrics_file(include_stats: bool) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "signed",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "unsigned",
                    CoveLogicalType::UInt64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(1, 0, 0, 3, 2);
    if include_stats {
        segment.set_column_pages(1, vec![stats_only_constant_page(3, 0)]);
        segment.set_column_pages(2, vec![stats_only_constant_page(3, 1)]);
    } else {
        segment.set_column_pages(1, vec![stats_only_all_null_page(3, 0)]);
        segment.set_column_pages(2, vec![stats_only_all_null_page(3, 1)]);
    }

    let mut writer = ScanProfileCoveWriter::new(catalog);
    if include_stats {
        writer
            .push_zone_stats(&ZoneStatsSection {
                entries: vec![
                    stats_only_entry(1, 0, 1, 3, StatKind::Int64, (-42i64).to_le_bytes().to_vec()),
                    stats_only_entry(1, 0, 2, 3, StatKind::UInt64, 42u64.to_le_bytes().to_vec()),
                ],
            })
            .unwrap();
    }
    writer.push_segment(segment);
    let bytes = writer.write().unwrap();
    if include_stats {
        bytes
    } else {
        rewrite_first_segment_pages(bytes, |page| {
            page.non_null_count = page.row_count;
            page.null_count = 0;
            page.flags = (page.flags & !PAGE_FLAG_ALL_NULL) | PAGE_FLAG_ALL_NON_NULL;
        })
    }
}

fn stats_only_float32_file(bits: u32) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "f32",
                CoveLogicalType::Float32,
                CovePhysicalKind::NumCode,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(1, 0, 0, 3, 1);
    segment.set_column_pages(1, vec![stats_only_constant_page(3, 0)]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer
        .push_zone_stats(&ZoneStatsSection {
            entries: vec![stats_only_entry(
                1,
                0,
                1,
                3,
                StatKind::FixedBytes,
                bits.to_le_bytes().to_vec(),
            )],
        })
        .unwrap();
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn stats_only_constant_page(row_count: u32, stats_ref: u32) -> ScanPageSpec {
    let mut page = ScanPageSpec::new(row_count, Vec::new())
        .with_counts(row_count, 0)
        .with_encoding_root(u32::MAX)
        .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NON_NULL);
    page.stats_ref = stats_ref;
    page
}

fn stats_only_all_null_page(row_count: u32, stats_ref: u32) -> ScanPageSpec {
    let mut page = ScanPageSpec::new(row_count, Vec::new())
        .with_counts(0, row_count)
        .with_encoding_root(u32::MAX)
        .with_flags(PAGE_FLAG_STATS_ONLY_CONSTANT | PAGE_FLAG_ALL_NULL);
    page.stats_ref = stats_ref;
    page
}

fn rewrite_first_segment_pages(
    mut bytes: Vec<u8>,
    mut mutate: impl FnMut(&mut ColumnPageIndexEntryV1),
) -> Vec<u8> {
    let mut postscript = CovePostscriptV1::parse_from_tail(&bytes).unwrap();
    let footer_start = postscript.footer.offset as usize;
    let footer_header = CoveFooterHeaderV1::parse(&bytes[footer_start..]).unwrap();
    let entries_start = footer_start + FOOTER_HEADER_SIZE;
    for index in 0..footer_header.section_count as usize {
        let entry_start = entries_start + index * SECTION_ENTRY_SIZE;
        let mut section_entry =
            CoveSectionEntryV1::parse(&bytes[entry_start..entry_start + SECTION_ENTRY_SIZE])
                .unwrap();
        if section_entry.section_kind != SectionKind::TableSegmentData as u16 {
            continue;
        }
        let segment_start = section_entry.offset as usize;
        let segment_end = segment_start + section_entry.length as usize;
        let segment = TableSegmentPayloadV1::parse(&bytes[segment_start..segment_end]).unwrap();
        for column in &segment.columns {
            let page_count = column.page_index_length as usize / 60;
            for page_index in 0..page_count {
                let page_start =
                    segment_start + column.page_index_offset as usize + page_index * 60;
                let mut page =
                    ColumnPageIndexEntryV1::parse(&bytes[page_start..page_start + 60]).unwrap();
                mutate(&mut page);
                bytes[page_start..page_start + 60].copy_from_slice(&page.serialize());
            }
        }
        section_entry.crc32c = checksum::crc32c(&bytes[segment_start..segment_end]);
        bytes[entry_start..entry_start + SECTION_ENTRY_SIZE]
            .copy_from_slice(&section_entry.serialize());

        let footer_end = footer_start + postscript.footer.length as usize;
        postscript.footer.crc32c = checksum::crc32c(&bytes[footer_start..footer_end]);
        let tail_start = bytes.len() - POSTSCRIPT_TOTAL_SIZE;
        bytes[tail_start..].copy_from_slice(&postscript.serialize_tail());
        return bytes;
    }
    panic!("generated COVE-T file did not contain TABLE_SEGMENT_DATA");
}

fn stats_only_entry(
    table_id: u32,
    segment_id: u32,
    column_id: u32,
    row_count: u32,
    kind: StatKind,
    bytes: Vec<u8>,
) -> ZoneStatsEntry {
    let scalar = StatScalar {
        kind,
        bytes,
        truncated: false,
    };
    ZoneStatsEntry {
        table_id,
        segment_id,
        morsel_id: 0,
        column_id,
        non_null_count: row_count,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: ZoneScope::Morsel,
            row_count: u64::from(row_count),
            null_count: 0,
            min: Some(scalar.clone()),
            max: Some(scalar),
            flags: ZoneStatFlags::HAS_MIN_MAX | ZoneStatFlags::CONSTANT,
        },
        min_domain_rank: 0,
        max_domain_rank: 0,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

fn primitive_events_file_with_name_gamma_coverage(bad_checksum: bool) -> Vec<u8> {
    let placeholder = primitive_events_file_with_name_gamma_coverage_snapshot(bad_checksum, 0);
    let placeholder_footer = reader::validate_bytes(&placeholder).unwrap().footer;
    primitive_events_file_with_name_gamma_coverage_snapshot(
        bad_checksum,
        embedded_coverage_snapshot_validity_ref(
            &placeholder_footer,
            &[0; 16],
            placeholder.len() as u64,
        ),
    )
}

fn primitive_events_file_with_name_gamma_coverage_snapshot(
    bad_checksum: bool,
    snapshot_validity_ref: u32,
) -> Vec<u8> {
    let mut writer = primitive_events_writer();
    for section in name_gamma_coverage_sections(bad_checksum, snapshot_validity_ref) {
        writer.push_extra_section(section);
    }
    writer.write().unwrap()
}

fn coverage_cache_bytes_for_state(
    state: &cove_datafusion::dataset_state::DatasetState,
    file_bytes: &[u8],
) -> Vec<u8> {
    let mut seed = Vec::new();
    seed.extend_from_slice(state.file_id());
    seed.extend_from_slice(&state.file_len().to_le_bytes());
    seed.extend_from_slice(&state.footer_crc32c().to_le_bytes());
    let file_digest = cove_core::digest::compute_digest(
        cove_core::constants::DigestAlgorithm::Sha256,
        file_bytes,
    )
    .unwrap();
    seed.extend_from_slice(&file_digest);
    let digest =
        cove_core::digest::compute_digest(cove_core::constants::DigestAlgorithm::Sha256, &seed)
            .unwrap();
    let mut snapshot_id = [0u8; 16];
    snapshot_id.copy_from_slice(&digest[..16]);
    let dataset_id = *state.file_id();
    CoverageCacheV2 {
        header: CoveCoverageCacheHeaderV2 {
            cache_format_namespace_ref: 1,
            cache_format_version_major: 2,
            cache_format_version_minor: 0,
            flags: 0,
            cache_id: [7u8; 16],
            dataset_id,
            snapshot_id,
            entry_count: 1,
            created_at_us: 0,
            producer_engine_ref: 0,
            reserved: [0; 32],
            checksum: 0,
        },
        entries: vec![CoverageCacheEntryV2 {
            entry_id: 1,
            dataset_id,
            snapshot_id,
            predicate_normal_form_ref: 1,
            interval_normal_form_ref: u32::MAX,
            coverage_set_ref: 1,
            coverage_granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            actual_coverage_size_bytes: 64,
            actual_read_cost_ns: 1,
            created_at_us: 0,
            valid_until_snapshot_ref: u32::MAX,
            producer_engine_ref: 0,
            checksum: 0,
        }],
    }
    .serialize()
    .unwrap()
}

fn name_gamma_coverage_sections(
    bad_checksum: bool,
    snapshot_validity_ref: u32,
) -> Vec<SectionPayload> {
    let predicate_form_ref = 1;
    let provider_id = 1;
    let coverage_set_id = 1;
    let predicate_form_section =
        predicate_normal_form_ast_section(predicate_form_ref, 1, name_eq_gamma_ast_payload());

    let provider = CoverageProviderDescriptorV2 {
        provider_id,
        provider_kind: CoverageProofKindV2::ValueToFragmentIndex as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        granularity: CoverageGranularityV2::Morsel,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        flags: 0,
        referenced_table_id: 1,
        referenced_column_id: 2,
        referenced_path_ref: u32::MAX,
        logical_type: CoveLogicalType::Utf8 as u16,
        collation_id: 0,
        null_semantics: 0,
        snapshot_validity_ref,
        predicate_form_ref,
        producer_ref: u32::MAX,
        checksum: 0,
    };
    let coverage_set = CoverageSetV2 {
        header: CoverageSetHeaderV2 {
            coverage_set_id,
            provider_id,
            granularity: CoverageGranularityV2::Morsel,
            proof_strength: CoverageProofStrengthV2::ExactConservative,
            exactness: CoverageExactnessV2::Exact,
            flags: 0,
            predicate_form_ref,
            snapshot_validity_ref,
            total_fragment_count: 2,
            covered_fragment_count: 0,
            required_fragment_count_estimate: 0,
            coverage_degree_ppm: 500_000,
            tightness_degree_ppm: 1_000_000,
            entries_offset: 0,
            entries_length: 0,
            checksum: 0,
        },
        entries: vec![CoverageSetEntryV2 {
            target_kind: CoverageGranularityV2::Morsel,
            flags: 0,
            file_ref: 0,
            table_id: 1,
            segment_id: 1,
            morsel_id: 0,
            page_ref: u32::MAX,
            object_type_id: u32::MAX,
            path_ref: u32::MAX,
            dimensional_bucket_ref: u32::MAX,
            row_start: 0,
            row_count: 0,
            row_ordinal_bitmap_ref: u32::MAX,
            byte_range_ref: u32::MAX,
            checksum: 0,
        }],
    };
    let coverage_set_bytes = coverage_set.serialize().unwrap();
    let mut coverage_set_checksum = coverage_set_payload_checksum(&coverage_set_bytes);
    if bad_checksum {
        coverage_set_checksum ^= 1;
    }
    let proof = CoverageProofRecordV2 {
        proof_id: 1,
        provider_id,
        coverage_set_id,
        predicate_form_ref,
        proof_kind: CoverageProofKindV2::ValueToFragmentIndex,
        proof_strength: CoverageProofStrengthV2::ExactConservative,
        exactness: CoverageExactnessV2::Exact,
        granularity: CoverageGranularityV2::Morsel,
        null_semantics: 0,
        flags: 0,
        snapshot_validity_ref,
        coverage_set_checksum,
        proof_payload_ref: u32::MAX,
        checksum: 0,
    };

    vec![
        coverage_section(
            SectionKind::CoverageProviderRegistry,
            1,
            provider.serialize().to_vec(),
        ),
        coverage_section(SectionKind::CoverageSet, 1, coverage_set_bytes),
        coverage_section(
            SectionKind::CoverageProofRecord,
            1,
            proof.serialize().unwrap().to_vec(),
        ),
        predicate_form_section,
    ]
}

fn predicate_normal_form_ast_section(
    predicate_form_id: u32,
    table_id: u32,
    payload: Vec<u8>,
) -> SectionPayload {
    let form = PredicateNormalFormV2 {
        predicate_form_id,
        form_kind: PredicateFormKindV2::PredicateAst,
        flags: 0,
        logical_context_ref: table_id,
        payload_offset: PredicateNormalFormV2::LEN as u64,
        payload_length: payload.len() as u64,
        checksum: 0,
    };
    let mut data = Vec::with_capacity(PredicateNormalFormV2::LEN + payload.len());
    data.extend_from_slice(&form.serialize().unwrap());
    data.extend_from_slice(&payload);
    coverage_section(SectionKind::PredicateNormalForm, 1, data)
}

fn name_eq_gamma_ast_payload() -> Vec<u8> {
    let canonical = CanonicalValue::Utf8("gamma").encode().unwrap();
    let node_offset = PredicateAstPayloadHeaderV2::LEN;
    let literal_offset = node_offset + PredicateAstNodeV2::LEN;
    let operand_ref_offset = literal_offset + PredicateLiteralV2::LEN;
    let canonical_offset = operand_ref_offset + 2 * PredicateAstOperandRefV2::LEN;

    let mut payload = Vec::new();
    payload.extend_from_slice(&predicate_ast_header(
        node_offset as u64,
        literal_offset as u64,
        operand_ref_offset as u64,
    ));
    payload.extend_from_slice(&predicate_ast_node());
    payload.extend_from_slice(&predicate_ast_literal(
        canonical_offset as u64,
        canonical.len() as u32,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        0,
        PredicateOperandKindV2::ColumnOrPath,
        2,
    ));
    payload.extend_from_slice(&predicate_ast_operand_ref(
        1,
        PredicateOperandKindV2::Literal,
        0,
    ));
    payload.extend_from_slice(&canonical);
    payload
}

fn predicate_ast_header(
    node_offset: u64,
    literal_offset: u64,
    operand_ref_offset: u64,
) -> [u8; PredicateAstPayloadHeaderV2::LEN] {
    let mut out = [0u8; PredicateAstPayloadHeaderV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..8].copy_from_slice(&1u32.to_le_bytes());
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..32].copy_from_slice(&node_offset.to_le_bytes());
    out[32..40].copy_from_slice(&literal_offset.to_le_bytes());
    out[56..64].copy_from_slice(&operand_ref_offset.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[68..72].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_node() -> [u8; PredicateAstNodeV2::LEN] {
    let mut out = [0u8; PredicateAstNodeV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(PredicateOpV2::Eq as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(CoveLogicalType::Bool as u16).to_le_bytes());
    out[12] = PredicateNullPolicyV2::SqlWhere as u8;
    out[14..16].copy_from_slice(&2u16.to_le_bytes());
    out[16..20].copy_from_slice(&0u32.to_le_bytes());
    out[20..24].copy_from_slice(&2u32.to_le_bytes());
    out[24..28].copy_from_slice(&0u32.to_le_bytes());
    out[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    out[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[36..40].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_literal(
    canonical_value_offset: u64,
    canonical_value_length: u32,
) -> [u8; PredicateLiteralV2::LEN] {
    let mut out = [0u8; PredicateLiteralV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&(ValueTag::Utf8 as u16).to_le_bytes());
    out[6..8].copy_from_slice(&(CoveLogicalType::Utf8 as u16).to_le_bytes());
    out[12..20].copy_from_slice(&canonical_value_offset.to_le_bytes());
    out[20..24].copy_from_slice(&canonical_value_length.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[24..28].copy_from_slice(&crc.to_le_bytes());
    out
}

fn predicate_ast_operand_ref(
    ordinal: u16,
    operand_kind: PredicateOperandKindV2,
    ref_id: u32,
) -> [u8; PredicateAstOperandRefV2::LEN] {
    let mut out = [0u8; PredicateAstOperandRefV2::LEN];
    out[0..4].copy_from_slice(&0u32.to_le_bytes());
    out[4..6].copy_from_slice(&ordinal.to_le_bytes());
    out[6] = operand_kind as u8;
    out[8..12].copy_from_slice(&ref_id.to_le_bytes());
    let crc = checksum::crc32c(&out);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    out
}

fn coverage_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::CoverageMetadata as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data,
    }
}

fn registered_names_file(include_descriptor: bool, include_fallback: bool) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 71,
            namespace: "public".into(),
            name: "names".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "name",
                CoveLogicalType::Utf8,
                CovePhysicalKind::VarBytes,
                false,
            )],
        }],
    };
    let values = ["alpha", "beta", "gamma"];
    let fallback = include_fallback.then(|| {
        ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::VarBytes,
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            None,
            varbytes(&values),
        )
        .unwrap()
    });
    let codec_id = if include_descriptor { 1 } else { 9001 };
    let registered_payload = ColumnPagePayloadV1::build_registered_single_node(
        3,
        3,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        codec_id,
        2,
        0,
        cfs2_payload(&values),
        fallback,
    )
    .unwrap();
    let mut segment = ScanSegment::new(71, 0, 0, 3, 1);
    segment.set_column_pages(
        1,
        vec![ScanPageSpec::new(3, registered_payload)
            .with_encoding_root(CoveEncodingKind::RegisteredEncoding as u32)],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    if include_descriptor {
        writer.push_extra_section(SectionPayload {
            section_kind: SectionKind::CodecExtensionRegistry as u16,
            profile: PrimaryProfile::CodecExtension as u8,
            flags: 0,
            item_count: 1,
            row_count: 0,
            compression: 0,
            alignment_log2: 0,
            required_features: 0,
            optional_features: FEATURE_REGISTERED_ENCODINGS,
            data: stable_fsst_descriptor().serialize().unwrap(),
        });
    }
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn primitive_events_writer() -> ScanProfileCoveWriter {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 1,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
                column(
                    3,
                    "active",
                    CoveLogicalType::Bool,
                    CovePhysicalKind::Boolean,
                    false,
                ),
            ],
        }],
    };
    let mut first = ScanSegment::new(1, 0, 0, 2, 3);
    first.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[1, 2]))]);
    first.set_column_pages(2, vec![varbytes_page(2, varbytes(&["alpha", "beta"]))]);
    first.set_column_pages(3, vec![bool_page(2, bools(&[true, false]))]);

    let mut second = ScanSegment::new(1, 1, 2, 1, 3);
    second.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[3]))]);
    second.set_column_pages(2, vec![varbytes_page(1, varbytes(&["gamma"]))]);
    second.set_column_pages(3, vec![bool_page(1, bools(&[true]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(first);
    writer.push_segment(second);
    writer
}

fn primitive_events_file_with_scoped_feature(entry: ProfileCapabilityEntryV2) -> Vec<u8> {
    let required_features = FEATURE_TABLE_PROFILE | FEATURE_EXTENDED_FEATURE_SET;
    let extended = ExtendedFeatureSetV2 {
        header: ExtendedFeatureSetHeaderV2 {
            word_count: 2,
            required_word_count: 2,
            optional_word_count: 1,
            flags: 0,
            checksum: 0,
        },
        required_feature_words: vec![required_features, UNKNOWN_SCOPED_FEATURE],
        optional_feature_words: vec![0],
    }
    .serialize()
    .unwrap();
    let matrix = ProfileCapabilityMatrixV2 {
        header: ProfileCapabilityMatrixHeaderV2 {
            magic: *b"PCM2",
            version_major: 2,
            header_len: ProfileCapabilityMatrixHeaderV2::LEN as u16,
            entry_len: ProfileCapabilityEntryV2::LEN as u16,
            reserved: 0,
            entry_count: 1,
            flags: 0,
            entries_offset: ProfileCapabilityMatrixHeaderV2::LEN as u64,
            entries_length: ProfileCapabilityEntryV2::LEN as u64,
            checksum: 0,
        },
        entries: vec![entry],
    }
    .serialize()
    .unwrap();

    let mut writer = primitive_events_writer();
    writer.push_extra_section(SectionPayload {
        section_kind: SectionKind::ExtendedFeatureSet as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count: 0,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_EXTENDED_FEATURE_SET,
        optional_features: 0,
        data: extended,
    });
    writer.push_extra_section(SectionPayload {
        section_kind: SectionKind::ProfileCapabilityMatrix as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count: 0,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: matrix,
    });
    let mut bytes = writer.write().unwrap();
    set_scoped_feature_header_ids(&mut bytes, 2, 3);
    bytes
}

fn scoped_feature_entry(
    scope: FeatureScopeV2,
    operation_kind: OperationKindV2,
    section_id: u32,
    target_local_ref: u64,
) -> ProfileCapabilityEntryV2 {
    ProfileCapabilityEntryV2 {
        profile: PrimaryProfile::TableScan as u8,
        scope,
        operation_kind,
        global_feature_word_index: 1,
        required_mask: UNKNOWN_SCOPED_FEATURE,
        optional_mask: 0,
        section_id,
        target_local_ref,
        flags: 0,
        reserved: 0,
        checksum: 0,
    }
}

fn set_scoped_feature_header_ids(
    bytes: &mut [u8],
    feature_set_section_id: u32,
    profile_capability_section_id: u32,
) {
    bytes[76..80].copy_from_slice(&feature_set_section_id.to_le_bytes());
    bytes[80..84].copy_from_slice(&profile_capability_section_id.to_le_bytes());
    bytes[156..160].fill(0);
    let header_crc = checksum::crc32c(&bytes[..HEADER_SIZE]);
    bytes[156..160].copy_from_slice(&header_crc.to_le_bytes());
}

fn binary_events_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 41,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "payload",
                CoveLogicalType::Binary,
                CovePhysicalKind::VarBytes,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(41, 0, 0, 2, 1);
    segment.set_column_pages(
        1,
        vec![varbytes_page(
            2,
            varbinary(&[b"short", b"long-binary-payload"]),
        )],
    );
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn nullable_events_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 11,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 4,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "maybe",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    true,
                ),
            ],
        }],
    };

    let mut mixed = ScanSegment::new(11, 0, 0, 2, 2);
    mixed.set_column_pages(1, vec![numcode_page(2, numcode_i64(&[1, 2]))]);
    mixed.set_column_pages(2, vec![nullable_numcode_page(&[Some(10), None])]);

    let mut all_null = ScanSegment::new(11, 1, 2, 1, 2);
    all_null.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[3]))]);
    all_null.set_column_pages(2, vec![nullable_numcode_page(&[None])]);

    let mut all_non_null = ScanSegment::new(11, 2, 3, 1, 2);
    all_non_null.set_column_pages(1, vec![numcode_page(1, numcode_i64(&[4]))]);
    all_non_null.set_column_pages(2, vec![nullable_numcode_page(&[Some(40)])]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_segment(mixed);
    writer.push_segment(all_null);
    writer.push_segment(all_non_null);
    writer.write().unwrap()
}

fn dictionary_items_file(dictionary: FileDictionary) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "name",
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, 2, 1);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    if has_redacted_entries(&dictionary) {
        writer.push_extra_section(redaction_manifest_section());
    }
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn filecode_key_file(dictionary: FileDictionary, codes: &[u32]) -> Vec<u8> {
    let row_count = u32::try_from(codes.len()).unwrap();
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: u64::from(row_count),
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "name",
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, row_count, 1);
    segment.set_column_pages(1, vec![filecode_page(row_count, filecodes(codes))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    if has_redacted_entries(&dictionary) {
        writer.push_extra_section(redaction_manifest_section());
    }
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn scored_dictionary_items_file(dictionary: FileDictionary, scores: [i64; 2]) -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "score",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    segment.set_column_pages(2, vec![numcode_page(2, numcode_i64(&scores))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn local_codebook_scored_dictionary_items_file() -> Vec<u8> {
    let dictionary = sample_dictionary();
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 23,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 4,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "score",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let local_codebook = LocalCodebookPayload {
        values: LocalCodebookValues::FileCode(vec![1, 0]),
        indexes: LocalIndexPayload::BitPacked(BitPackedPayload::pack(&[1, 0, 1, 0], 1).unwrap()),
    };
    let mut segment = ScanSegment::new(23, 0, 0, 4, 2);
    segment.set_column_pages(1, vec![local_codebook_page(4, local_codebook.encode())]);
    segment.set_column_pages(2, vec![numcode_page(4, numcode_i64(&[10, 20, 30, 40]))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn mixed_dictionary_items_file() -> Vec<u8> {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 3,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![
            inline_binary_entry(&[0xaa, 0xbb]),
            inline_utf8_entry("red"),
            inline_utf8_entry("blue"),
        ],
        payload: Vec::new(),
    };
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "blob",
                    CoveLogicalType::Binary,
                    CovePhysicalKind::FileCode,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[1, 2]))]);
    segment.set_column_pages(2, vec![filecode_page(2, filecodes(&[0, 0]))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn redacted_mixed_dictionary_items_file() -> Vec<u8> {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![redacted_binary_entry(), inline_utf8_entry("red")],
        payload: Vec::new(),
    };
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 1,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![column(
                1,
                "name",
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
                false,
            )],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, 1, 1);
    segment.set_column_pages(1, vec![filecode_page(1, filecodes(&[1]))]);
    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_extra_section(redaction_manifest_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn dictionary_items_file_with_domain_stats() -> Vec<u8> {
    let dictionary = sample_dictionary();
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column_with_collation(
                    1,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                    1,
                ),
                column(
                    2,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    };
    let mut first = ScanSegment::new(7, 0, 0, 1, 2);
    first.set_column_pages(1, vec![filecode_page(1, filecodes(&[0]))]);
    first.set_column_pages(2, vec![varbytes_page(1, varbytes(&["first"]))]);
    let mut second = ScanSegment::new(7, 1, 1, 1, 2);
    second.set_column_pages(1, vec![filecode_page(1, filecodes(&[1]))]);
    second.set_column_pages(2, vec![varbytes_page(1, varbytes(&["second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_extra_section(column_domain_section());
    writer.push_extra_section(filecode_zone_stats_section());
    writer.push_segment(first);
    writer.push_segment(second);
    writer.write().unwrap()
}

fn dictionary_items_file_with_lookup_index() -> Vec<u8> {
    let catalog = dictionary_items_payload_catalog();
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    segment.set_column_pages(2, vec![varbytes_page(2, varbytes(&["first", "second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&sample_dictionary());
    writer.push_extra_section(lookup_index_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn bool_filecode_items_file_with_lookup_index() -> Vec<u8> {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![
            inline_bool_entry(ValueTag::BoolFalse),
            inline_bool_entry(ValueTag::BoolTrue),
        ],
        payload: Vec::new(),
    };
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "active",
                    CoveLogicalType::Bool,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    segment.set_column_pages(2, vec![varbytes_page(2, varbytes(&["first", "second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_extra_section(lookup_index_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn dictionary_items_file_with_lookup_and_cove_e(
    dictionary: FileDictionary,
    supported_execution_code: bool,
) -> Vec<u8> {
    let catalog = dictionary_items_payload_catalog();
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.set_column_pages(1, vec![filecode_page(2, filecodes(&[0, 1]))]);
    segment.set_column_pages(2, vec![varbytes_page(2, varbytes(&["first", "second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&dictionary);
    writer.push_extra_section(lookup_index_section());
    for section in cove_e_sections(supported_execution_code) {
        writer.push_extra_section(section);
    }
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn dictionary_items_file_with_inverted_index() -> Vec<u8> {
    let catalog = dictionary_items_payload_catalog();
    let mut segment = ScanSegment::new(7, 0, 0, 2, 2);
    segment.morsel_row_count = 1;
    segment.set_column_pages(
        1,
        vec![
            filecode_page(1, filecodes(&[0])),
            filecode_page(1, filecodes(&[1])),
        ],
    );
    segment.set_column_pages(
        2,
        vec![
            varbytes_page(1, varbytes(&["first"])),
            varbytes_page(1, varbytes(&["second"])),
        ],
    );

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&sample_dictionary());
    writer.push_extra_section(inverted_index_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn dictionary_items_file_with_ambiguous_inverted_index() -> Vec<u8> {
    let catalog = dictionary_items_payload_catalog();
    let mut first = ScanSegment::new(7, 0, 0, 1, 2);
    first.morsel_row_count = 1;
    first.set_column_pages(1, vec![filecode_page(1, filecodes(&[0]))]);
    first.set_column_pages(2, vec![varbytes_page(1, varbytes(&["first"]))]);

    let mut second = ScanSegment::new(7, 1, 1, 1, 2);
    second.morsel_row_count = 1;
    second.set_column_pages(1, vec![filecode_page(1, filecodes(&[1]))]);
    second.set_column_pages(2, vec![varbytes_page(1, varbytes(&["second"]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_file_dictionary(&sample_dictionary());
    writer.push_extra_section(ambiguous_inverted_index_section());
    writer.push_segment(first);
    writer.push_segment(second);
    writer.write().unwrap()
}

fn numeric_lookup_events_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 8,
            namespace: "public".into(),
            name: "events".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(8, 0, 0, 3, 2);
    segment.set_column_pages(1, vec![numcode_page(3, numcode_i64(&[1, 2, 3]))]);
    segment.set_column_pages(
        2,
        vec![varbytes_page(3, varbytes(&["alpha", "beta", "gamma"]))],
    );

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_extra_section(numcode_lookup_index_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn float_zero_lookup_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 9,
            namespace: "public".into(),
            name: "metrics".into(),
            row_count: 3,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
                column(
                    2,
                    "f64",
                    CoveLogicalType::Float64,
                    CovePhysicalKind::NumCode,
                    false,
                ),
            ],
        }],
    };
    let mut segment = ScanSegment::new(9, 0, 0, 3, 2);
    segment.set_column_pages(1, vec![numcode_page(3, numcode_i64(&[1, 2, 3]))]);
    segment.set_column_pages(2, vec![numcode_page(3, numcode_f64(&[0.0, -0.0, 1.0]))]);

    let mut writer = ScanProfileCoveWriter::new(catalog);
    writer.push_extra_section(float_zero_lookup_index_section());
    writer.push_segment(segment);
    writer.write().unwrap()
}

fn float_zero_lookup_index_section() -> SectionPayload {
    let index = LookupIndex {
        header: LookupIndexHeaderV1 {
            table_id: 9,
            column_id: 2,
            key_kind: LookupKeyKind::NumCode,
            index_kind: LookupIndexKind::SparseSorted,
            uniqueness: LookupUniqueness::NonUnique,
            flags: 0,
            entry_count: 0,
            entries_offset: 0,
            entries_length: 0,
            rowref_offset: 0,
            rowref_length: 0,
            checksum: 0,
        },
        entries: vec![
            LookupEntry {
                key: 0.0f64.to_bits(),
                rows: vec![RowRef {
                    table_id: 9,
                    segment_id: 0,
                    morsel_id: 0,
                    row_in_morsel: 0,
                }],
            },
            LookupEntry {
                key: (-0.0f64).to_bits(),
                rows: vec![RowRef {
                    table_id: 9,
                    segment_id: 0,
                    morsel_id: 0,
                    row_in_morsel: 1,
                }],
            },
        ],
    };
    SectionPayload {
        section_kind: SectionKind::LookupIndex as u16,
        profile: PrimaryProfile::ArchiveAcceleration as u8,
        flags: 0,
        item_count: 1,
        row_count: 3,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: index.serialize().unwrap(),
    }
}

fn dictionary_items_payload_catalog() -> TableCatalog {
    TableCatalog {
        flags: 0,
        tables: vec![TableEntry {
            table_id: 7,
            namespace: "public".into(),
            name: "items".into(),
            row_count: 2,
            primary_sort_key_count: 0,
            clustering_key_count: 0,
            flags: 0,
            columns: vec![
                column(
                    1,
                    "name",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::FileCode,
                    false,
                ),
                column(
                    2,
                    "payload",
                    CoveLogicalType::Utf8,
                    CovePhysicalKind::VarBytes,
                    false,
                ),
            ],
        }],
    }
}

fn multiple_tables_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![
            TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "first".into(),
                row_count: 0,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                )],
            },
            TableEntry {
                table_id: 2,
                namespace: "public".into(),
                name: "second".into(),
                row_count: 0,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                )],
            },
        ],
    };
    ScanProfileCoveWriter::new(catalog).write().unwrap()
}

fn ambiguous_table_names_file() -> Vec<u8> {
    let catalog = TableCatalog {
        flags: 0,
        tables: vec![
            TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 0,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                )],
            },
            TableEntry {
                table_id: 2,
                namespace: "archive".into(),
                name: "events".into(),
                row_count: 0,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![column(
                    1,
                    "id",
                    CoveLogicalType::Int64,
                    CovePhysicalKind::NumCode,
                    false,
                )],
            },
        ],
    };
    ScanProfileCoveWriter::new(catalog).write().unwrap()
}

fn column(
    column_id: u32,
    name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    nullable: bool,
) -> ColumnEntry {
    ColumnEntry {
        column_id,
        name: name.into(),
        logical,
        physical,
        nullable,
        sort_order: 0,
        collation_id: 0,
        precision: 0,
        scale: 0,
        flags: 0,
    }
}

fn column_with_collation(
    column_id: u32,
    name: &str,
    logical: CoveLogicalType,
    physical: CovePhysicalKind,
    nullable: bool,
    collation_id: u16,
) -> ColumnEntry {
    let mut column = column(column_id, name, logical, physical, nullable);
    column.collation_id = collation_id;
    column
}

fn numcode_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::NumCode as u32)
}

fn nullable_numcode_page(values: &[Option<i64>]) -> ScanPageSpec {
    let row_count = values.len() as u32;
    let mut null_bitmap = vec![0u8; values.len().div_ceil(8)];
    let mut non_null_count = 0u32;
    let mut payload_values = Vec::with_capacity(values.len() * 8);
    for (index, value) in values.iter().enumerate() {
        match value {
            Some(value) => {
                non_null_count += 1;
                payload_values.extend_from_slice(&(*value as u64).to_le_bytes());
            }
            None => {
                null_bitmap[index / 8] |= 1u8 << (index % 8);
                payload_values.extend_from_slice(&0u64.to_le_bytes());
            }
        }
    }
    let null_count = row_count - non_null_count;
    let mut payload = Vec::with_capacity(null_bitmap.len() + payload_values.len());
    if null_count != 0 {
        payload.extend_from_slice(&null_bitmap);
    }
    payload.extend_from_slice(&payload_values);
    ScanPageSpec::new(row_count, payload)
        .with_encoding_root(CoveEncodingKind::NumCode as u32)
        .with_counts(non_null_count, null_count)
}

fn varbytes_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::VarBytes as u32)
}

fn bool_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::PlainFixed as u32)
}

fn fixedbytes_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::PlainFixed as u32)
}

fn filecode_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::FileCode as u32)
}

fn local_codebook_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
    ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::LocalCodebook as u32)
}

fn has_redacted_entries(dictionary: &FileDictionary) -> bool {
    dictionary
        .entries
        .iter()
        .any(|entry| entry.storage_class == StorageClass::Redacted as u8)
}

fn redaction_manifest_section() -> SectionPayload {
    let manifest = RedactionManifest {
        entries: vec![RedactionEntry {
            redaction_id: 1,
            section_id: 2,
            local_ref: 0,
            reason_code: 1,
            policy_id: b"test/redacted".to_vec(),
            audit_ref: b"native_single_file".to_vec(),
            created_at_us: 0,
        }],
    };
    SectionPayload {
        section_kind: SectionKind::RedactionManifest as u16,
        profile: PrimaryProfile::Mixed as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_REDACTIONS,
        optional_features: 0,
        data: manifest.serialize().unwrap(),
    }
}

fn column_domain_section() -> SectionPayload {
    let domain = ColumnDomain::from_sorted_present_codes(
        &[1, 0],
        2,
        7,
        1,
        CoveLogicalType::Utf8 as u16,
        1,
        0,
    )
    .unwrap();
    SectionPayload {
        section_kind: SectionKind::ColumnDomain as u16,
        profile: PrimaryProfile::TableScan as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: FEATURE_COLUMN_DOMAINS,
        data: domain.serialize().unwrap(),
    }
}

fn filecode_zone_stats_section() -> SectionPayload {
    let entries = vec![
        filecode_zone_stats_entry(0, 1),
        filecode_zone_stats_entry(1, 0),
    ];
    let section = ZoneStatsSection { entries };
    SectionPayload {
        section_kind: SectionKind::ZoneStats as u16,
        profile: PrimaryProfile::TableScan as u8,
        flags: 0,
        item_count: 2,
        row_count: 2,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: section.serialize().unwrap(),
    }
}

fn lookup_index_section() -> SectionPayload {
    let index = LookupIndex {
        header: LookupIndexHeaderV1 {
            table_id: 7,
            column_id: 1,
            key_kind: LookupKeyKind::FileCode,
            index_kind: LookupIndexKind::SparseSorted,
            uniqueness: LookupUniqueness::NonUnique,
            flags: 0,
            entry_count: 0,
            entries_offset: 0,
            entries_length: 0,
            rowref_offset: 0,
            rowref_length: 0,
            checksum: 0,
        },
        entries: vec![
            LookupEntry {
                key: 0,
                rows: vec![RowRef {
                    table_id: 7,
                    segment_id: 0,
                    morsel_id: 0,
                    row_in_morsel: 0,
                }],
            },
            LookupEntry {
                key: 1,
                rows: vec![RowRef {
                    table_id: 7,
                    segment_id: 0,
                    morsel_id: 0,
                    row_in_morsel: 1,
                }],
            },
        ],
    };
    SectionPayload {
        section_kind: SectionKind::LookupIndex as u16,
        profile: PrimaryProfile::ArchiveAcceleration as u8,
        flags: 0,
        item_count: 2,
        row_count: 2,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: index.serialize().unwrap(),
    }
}

fn numcode_lookup_index_section() -> SectionPayload {
    let index = LookupIndex {
        header: LookupIndexHeaderV1 {
            table_id: 8,
            column_id: 1,
            key_kind: LookupKeyKind::NumCode,
            index_kind: LookupIndexKind::SparseSorted,
            uniqueness: LookupUniqueness::NonUnique,
            flags: 0,
            entry_count: 0,
            entries_offset: 0,
            entries_length: 0,
            rowref_offset: 0,
            rowref_length: 0,
            checksum: 0,
        },
        entries: vec![LookupEntry {
            key: 2,
            rows: vec![RowRef {
                table_id: 8,
                segment_id: 0,
                morsel_id: 0,
                row_in_morsel: 1,
            }],
        }],
    };
    SectionPayload {
        section_kind: SectionKind::LookupIndex as u16,
        profile: PrimaryProfile::ArchiveAcceleration as u8,
        flags: 0,
        item_count: 1,
        row_count: 3,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: index.serialize().unwrap(),
    }
}

fn cove_e_sections(supported_execution_code: bool) -> Vec<SectionPayload> {
    let descriptor = ExecutionCodeDescriptorV1 {
        descriptor_id: 1,
        code_kind: if supported_execution_code {
            ExecutionCodeKind::DictionaryKey
        } else {
            ExecutionCodeKind::OpaqueBytes
        },
        code_width_bits: if supported_execution_code { 32 } else { 128 },
        byte_order: 0,
        lifetime: ExecutionCodeLifetime::Scan,
        comparison_scope: ExecutionCodeComparisonScope::File,
        canonicality: ExecutionCodeCanonicality::Transient,
        null_code_policy: NullCodePolicy::NullBitmapOnly,
        flags: 0,
        scope_ref: 0,
        code_space_ref: 0,
        checksum: 0,
    };
    let policy = EngineMountPolicyV1 {
        policy_id: 2,
        filecode_mapping_kind: FileCodeMappingKind::MapToArrowDictionary,
        missing_value_policy: MissingValuePolicy::DecodeValueOnly,
        stale_mapping_policy: StaleMappingPolicy::IgnoreIfOptional,
        reverse_lookup_policy: ReverseLookupPolicy::BuildFromDictionary,
        flags: 0,
        dictionary_digest_ref: 0,
        code_space_ref: 0,
        cache_key_ref: 0,
        private_payload_ref: 0,
        checksum: 0,
    };
    let registry = EngineProfileRegistry {
        flags: 0,
        profiles: vec![EngineProfileEntryV1 {
            profile_id: 3,
            namespace: "org.apache.datafusion".into(),
            profile_name: "arrow-dictionary".into(),
            version_major: 1,
            version_minor: 0,
            required_features: 0,
            optional_features: 0,
            execution_descriptor_ref: 1,
            mount_policy_ref: 2,
            private_payload_ref: 0,
            checksum: 0,
        }],
    };
    vec![
        cove_e_section(
            SectionKind::EngineProfileRegistry,
            1,
            registry.serialize().unwrap(),
        ),
        cove_e_section(
            SectionKind::ExecutionCodeDescriptor,
            1,
            descriptor.serialize().to_vec(),
        ),
        cove_e_section(
            SectionKind::EngineMountPolicy,
            1,
            policy.serialize().to_vec(),
        ),
    ]
}

fn cove_e_section(kind: SectionKind, item_count: u64, data: Vec<u8>) -> SectionPayload {
    SectionPayload {
        section_kind: kind as u16,
        profile: PrimaryProfile::EngineExecution as u8,
        flags: 0,
        item_count,
        row_count: 0,
        compression: 0,
        alignment_log2: 0,
        required_features: FEATURE_ENGINE_PROFILE,
        optional_features: 0,
        data,
    }
}

fn inverted_index_section() -> SectionPayload {
    let index = InvertedMorselIndex {
        header: InvertedMorselIndexHeaderV1 {
            table_id: 7,
            column_id: 1,
            key_kind: InvertedKeyKind::FileCode,
            flags: 0,
            representation: 0,
            reserved: 0,
            entry_count: 0,
            entries_offset: 0,
            bitmap_data_offset: 0,
            checksum: 0,
        },
        entries: vec![InvertedEntry {
            key: 0,
            morsel_bitmap_offset: 0,
            morsel_bitmap_length: 1,
            row_bitmap_offset: 0,
            row_bitmap_length: 0,
        }],
        bitmap_data: vec![0b0000_0001],
    };
    SectionPayload {
        section_kind: SectionKind::InvertedMorselIndex as u16,
        profile: PrimaryProfile::ArchiveAcceleration as u8,
        flags: 0,
        item_count: 1,
        row_count: 2,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: index.serialize(),
    }
}

fn ambiguous_inverted_index_section() -> SectionPayload {
    let index = InvertedMorselIndex {
        header: InvertedMorselIndexHeaderV1 {
            table_id: 7,
            column_id: 1,
            key_kind: InvertedKeyKind::FileCode,
            flags: 0,
            representation: 0,
            reserved: 0,
            entry_count: 0,
            entries_offset: 0,
            bitmap_data_offset: 0,
            checksum: 0,
        },
        entries: vec![
            InvertedEntry {
                key: 0,
                morsel_bitmap_offset: 0,
                morsel_bitmap_length: 1,
                row_bitmap_offset: 0,
                row_bitmap_length: 0,
            },
            InvertedEntry {
                key: 1,
                morsel_bitmap_offset: 1,
                morsel_bitmap_length: 1,
                row_bitmap_offset: 0,
                row_bitmap_length: 0,
            },
        ],
        bitmap_data: vec![0b0000_0001, 0b0000_0010],
    };
    SectionPayload {
        section_kind: SectionKind::InvertedMorselIndex as u16,
        profile: PrimaryProfile::ArchiveAcceleration as u8,
        flags: 0,
        item_count: 2,
        row_count: 2,
        compression: 0,
        alignment_log2: 0,
        required_features: 0,
        optional_features: 0,
        data: index.serialize(),
    }
}

fn filecode_zone_stats_entry(segment_id: u32, rank: u32) -> ZoneStatsEntry {
    ZoneStatsEntry {
        table_id: 7,
        segment_id,
        morsel_id: 0,
        column_id: 1,
        non_null_count: 1,
        distinct_count: 1,
        run_count: 1,
        stats: ZoneStats {
            scope: cove_core::zone_stats::ZoneScope::Morsel,
            row_count: 1,
            null_count: 0,
            min: None,
            max: None,
            flags: ZoneStatFlags::HAS_DOMAIN_RANGE | ZoneStatFlags::CONSTANT,
        },
        min_domain_rank: rank,
        max_domain_rank: rank,
        exact_set_ref: u32::MAX,
        bloom_ref: u32::MAX,
    }
}

fn numcode_i64(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| (*value as u64).to_le_bytes())
        .collect()
}

fn numcode_f32(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| u64::from(value.to_bits()).to_le_bytes())
        .collect()
}

fn numcode_f64(values: &[f64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

fn varbytes(values: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn cfs2_payload(values: &[&str]) -> Vec<u8> {
    let page = LogicalPage {
        values: values
            .iter()
            .map(|value| Some(value.as_bytes().to_vec()))
            .collect(),
    };
    encode_registered_row_bytes(b"CFS2", &page)
}

fn encode_registered_row_bytes(magic: &[u8; 4], page: &LogicalPage) -> Vec<u8> {
    let mut value_bytes = Vec::new();
    let mut offsets = Vec::with_capacity(page.values.len() + 1);
    offsets.push(0u32);
    for value in &page.values {
        if let Some(value) = value {
            let next = offsets.last().copied().unwrap() + value.len() as u32;
            offsets.push(next);
            value_bytes.extend_from_slice(value);
        } else {
            offsets.push(*offsets.last().unwrap());
        }
    }
    let mut null_bitmap = vec![0u8; page.values.len().div_ceil(8)];
    for (index, value) in page.values.iter().enumerate() {
        if value.is_none() {
            null_bitmap[index / 8] |= 1u8 << (index % 8);
        }
    }
    let offsets_len = offsets.len() * 4;
    let mut out = Vec::new();
    out.extend_from_slice(magic);
    out.extend_from_slice(&(page.values.len() as u32).to_le_bytes());
    out.extend_from_slice(&(null_bitmap.len() as u32).to_le_bytes());
    out.extend_from_slice(&(offsets_len as u32).to_le_bytes());
    out.extend_from_slice(&null_bitmap);
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&value_bytes);
    out
}

fn stable_fsst_descriptor() -> CodecExtensionDescriptorV2 {
    CodecExtensionDescriptorV2 {
        codec_id: 1,
        namespace: "org.coveformat.codec".into(),
        name: "fsst-utf8".into(),
        version_major: 2,
        version_minor: 0,
        codec_family: 0,
        logical_type_mask: 1u64 << (CoveLogicalType::Utf8 as u32),
        physical_kind_mask: 1u64 << (CovePhysicalKind::VarBytes as u32),
        requirement: CodecRequirementV2::OptionalWithFallback,
        fallback_policy: CodecFallbackPolicyV2::CoreEncodingPayloadPresent,
        parameter_schema_kind: 0,
        flags: 0,
        specification_status: CodecSpecificationStatusV2::StableRegistered,
        required_feature_bit: 0,
        optional_feature_bit: FEATURE_REGISTERED_ENCODINGS,
        spec_digest_algorithm: 1,
        spec_digest: b"COVE-FSST-UTF8-V2-SPEC-DIGEST".to_vec(),
        conformance_vector_ref: ABSENT_REF,
        fallback_ref: 0,
        private_payload_ref: ABSENT_REF,
        checksum: 0,
    }
}

fn varbinary(values: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value);
    }
    out
}

fn bools(values: &[bool]) -> Vec<u8> {
    values.iter().map(|value| u8::from(*value)).collect()
}

fn filecodes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn sample_dictionary() -> FileDictionary {
    FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("red"), inline_utf8_entry("blue")],
        payload: Vec::new(),
    }
}

fn redacted_dictionary() -> FileDictionary {
    FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![redacted_utf8_entry(), inline_utf8_entry("blue")],
        payload: Vec::new(),
    }
}

fn swapped_dictionary() -> FileDictionary {
    FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("blue"), inline_utf8_entry("red")],
        payload: Vec::new(),
    }
}

fn inline_utf8_entry(value: &str) -> FileDictionaryIndexEntryV1 {
    let canonical = canonical_utf8(value);
    inline_canonical_entry(ValueTag::Utf8, &canonical)
}

fn inline_bool_entry(value_tag: ValueTag) -> FileDictionaryIndexEntryV1 {
    assert!(matches!(
        value_tag,
        ValueTag::BoolFalse | ValueTag::BoolTrue
    ));
    inline_canonical_entry(value_tag, &[])
}

fn inline_canonical_entry(value_tag: ValueTag, canonical: &[u8]) -> FileDictionaryIndexEntryV1 {
    let mut inline_data = [0u8; 16];
    inline_data[..canonical.len()].copy_from_slice(canonical);
    FileDictionaryIndexEntryV1 {
        value_tag: value_tag as u16,
        storage_class: StorageClass::Inline as u8,
        flags: 0,
        inline_len: canonical.len() as u8,
        reserved0: [0; 3],
        inline_data,
        payload_offset: 0,
        payload_length: 0,
        canonical_hash64: 0,
        reserved1: 0,
    }
}

fn inline_binary_entry(value: &[u8]) -> FileDictionaryIndexEntryV1 {
    let mut canonical = wire::encode_u64_leb128(value.len() as u64);
    canonical.extend_from_slice(value);
    let mut inline_data = [0u8; 16];
    inline_data[..canonical.len()].copy_from_slice(&canonical);
    FileDictionaryIndexEntryV1 {
        value_tag: ValueTag::Binary as u16,
        storage_class: StorageClass::Inline as u8,
        flags: 0,
        inline_len: canonical.len() as u8,
        reserved0: [0; 3],
        inline_data,
        payload_offset: 0,
        payload_length: 0,
        canonical_hash64: 0,
        reserved1: 0,
    }
}

fn redacted_utf8_entry() -> FileDictionaryIndexEntryV1 {
    FileDictionaryIndexEntryV1 {
        value_tag: ValueTag::Utf8 as u16,
        storage_class: StorageClass::Redacted as u8,
        flags: 0,
        inline_len: 0,
        reserved0: [0; 3],
        inline_data: [0; 16],
        payload_offset: 0,
        payload_length: 0,
        canonical_hash64: 0,
        reserved1: 0,
    }
}

fn redacted_binary_entry() -> FileDictionaryIndexEntryV1 {
    FileDictionaryIndexEntryV1 {
        value_tag: ValueTag::Binary as u16,
        storage_class: StorageClass::Redacted as u8,
        flags: 0,
        inline_len: 0,
        reserved0: [0; 3],
        inline_data: [0; 16],
        payload_offset: 0,
        payload_length: 0,
        canonical_hash64: 0,
        reserved1: 0,
    }
}

fn canonical_utf8(value: &str) -> Vec<u8> {
    let mut canonical = wire::encode_u64_leb128(value.len() as u64);
    canonical.extend_from_slice(value.as_bytes());
    canonical
}

fn covemap_section(kind: SectionKind, value: Value) -> CovemapSection {
    let payload = serde_json::to_vec_pretty(&covemap_payload_value(kind, value)).unwrap();
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

fn covemap_payload_value(kind: SectionKind, mut value: Value) -> Value {
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
    value
}

fn showcase_multi_source_covemap() -> CovemapFile {
    CovemapFile {
        header: CovemapHeaderV1::new([0x53; 16], 0),
        mapping_version: "test/showcase.v1".into(),
        sections: vec![
            covemap_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "test/showcase.v1",
                    "sources": [
                        {"source_id": "crm", "row_identity_rules": ["person_by_id"], "source_priority": 10},
                        {"source_id": "directory", "row_identity_rules": ["person_by_id"], "source_priority": 20},
                        {"source_id": "subscription", "row_identity_rules": ["person_by_id"], "source_priority": 1}
                    ]
                }),
            ),
            covemap_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "test/showcase.v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            ),
            covemap_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "test/showcase.v1",
                    "identity_rules": [
                        {
                            "rule_id": "person_by_id",
                            "object_type": "Person",
                            "semantic_role": "subject",
                            "confidence_class": "authoritative",
                            "candidate_only": false,
                            "property_conflicts_declared": true,
                            "function_ids": ["identity"],
                            "join_keys": [{
                                "role_id": "person_id",
                                "source_column": "id",
                                "logical_type": "utf8",
                                "canonicalization": "identity",
                                "null_policy": "reject",
                                "ordering": "declared"
                            }]
                        }
                    ],
                    "do_not_merge": []
                }),
            ),
            covemap_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "test/showcase.v1",
                    "rules": [
                        {
                            "rule_id": "upsert_person_name_crm",
                            "source_id": "crm",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_crm"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_crm",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_directory",
                            "source_id": "directory",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_directory"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_directory",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        },
                        {
                            "rule_id": "upsert_person_name_subscription",
                            "source_id": "subscription",
                            "identity_rule_id": "person_by_id",
                            "row_semantics_kind": "Object",
                            "assertion_kinds": ["object", "property", "evidence"],
                            "function_ids": ["identity"],
                            "output_assertion_ids": ["person_name_assertion_subscription"],
                            "association_endpoints": [],
                            "property_bindings": [{
                                "assertion_id": "person_name_assertion_subscription",
                                "property_id": "name",
                                "property_name": "name",
                                "source_column": "name",
                                "logical_type": "utf8",
                                "nullable": true,
                                "missing_policy": "reject",
                                "conflict_policy": "source_priority_wins"
                            }]
                        }
                    ]
                }),
            ),
            covemap_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "showcase-map",
                    "mapping_version": "test/showcase.v1",
                    "projections": [
                        {
                            "projection_id": "person_projection",
                            "output_table": "people_projection",
                            "row_grain": "one_row_per_object",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "person_goid", "logical_type": "uuid", "value": "object.goid"},
                                {"name": "name", "logical_type": "utf8", "value": "name"}
                            ]
                        },
                        {
                            "projection_id": "evidence_projection",
                            "output_table": "evidence_projection",
                            "row_grain": "one_row_per_evidence_assertion",
                            "anchor": {"object_type": "Person"},
                            "temporal_mode": {"as_of": "latest_committed"},
                            "multi_value_policy": "reject",
                            "missing_policy": "null",
                            "output_modes": ["json", "arrow", "cove-t", "cove-o"],
                            "columns": [
                                {"name": "source_id", "logical_type": "utf8", "value": "evidence.source_id"},
                                {"name": "source_row_identity", "logical_type": "utf8", "value": "evidence.source_row_identity"},
                                {"name": "output_object_id", "logical_type": "uuid", "value": "evidence.output_object_id"}
                            ]
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

fn showcase_directory_name_batch() -> ArrowRecordBatch {
    ArrowRecordBatch::try_from_iter(vec![
        (
            "id",
            Arc::new(StringArray::from(vec!["p1", "p2"])) as ArrowArrayRef,
        ),
        (
            "name",
            Arc::new(StringArray::from(vec!["Ada Directory", "Linus Directory"])) as ArrowArrayRef,
        ),
    ])
    .unwrap()
}

fn write_parquet_batch(batch: ArrowRecordBatch) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut bytes, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }
    bytes
}

fn write_temp_cove(label: &str, bytes: Vec<u8>) -> PathBuf {
    let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cove-datafusion-{label}-{}-{id}.cove",
        std::process::id()
    ));
    fs::write(&path, bytes).unwrap();
    path
}

fn conformance_accept_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/accept")
        .join(name)
}

fn write_temp_mapped_cove(label: &str, mapping: &str, sources: &[&str]) -> PathBuf {
    let map_path = conformance_accept_path(mapping);
    let source_paths = sources
        .iter()
        .map(|source| conformance_accept_path(source))
        .collect::<Vec<_>>();
    let bytes = cove_o_from_paths(&map_path, &source_paths).unwrap();
    write_temp_cove(label, bytes)
}

#[cfg(feature = "covm")]
fn write_covm_manifest(path: &std::path::Path, files: Vec<CovmFileEntryV1>) {
    let manifest = CovmFile {
        header: CovmHeaderV1::new([0xC0; 16], 1, files.len() as u32, 0),
        files,
        postscript: CovmPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    };
    fs::write(path, manifest.serialize().unwrap()).unwrap();
}

#[cfg(feature = "covm")]
fn covm_entry_for_path(uri: &str, path: &std::path::Path) -> CovmFileEntryV1 {
    let state = bootstrap_local_file(path).unwrap();
    CovmFileEntryV1 {
        file_id: *state.file_id(),
        uri: uri.to_string(),
        file_len: state.file_len(),
        footer_crc32c: state.footer_crc32c(),
        digest_algorithm: DigestAlgorithm::None as u16,
        digest: Vec::new(),
        row_count: state.table().row_count,
        segment_count: state.segments().len() as u32,
        file_stats_ref: u32::MAX,
        file_exact_set_ref: u32::MAX,
        flags: 0,
    }
}

#[cfg(all(feature = "covm", feature = "covx"))]
fn write_covx_sidecar(path: &std::path::Path, referenced_files: Vec<CovxReferencedFileV1>) {
    let sidecar = CovxFile {
        header: CovxHeaderV1::new([0xC1; 16], referenced_files.len() as u32, 0),
        referenced_files,
        postscript: CovxPostscriptV1 {
            header_offset: 0,
            header_len: 0,
            entries_offset: 0,
            entries_len: 0,
            file_len: 0,
            flags: 0,
            checksum: 0,
        },
    };
    fs::write(path, sidecar.serialize().unwrap()).unwrap();
}

#[cfg(all(feature = "covm", feature = "covx"))]
fn covx_entry_for_path(path: &std::path::Path) -> CovxReferencedFileV1 {
    let state = bootstrap_local_file(path).unwrap();
    CovxReferencedFileV1 {
        file_id: *state.file_id(),
        file_len: state.file_len(),
        footer_crc32c: state.footer_crc32c(),
        digest_algorithm: DigestAlgorithm::None as u16,
        digest: Vec::new(),
    }
}

fn make_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cove-datafusion-{label}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[derive(Debug, Clone, Copy)]
struct QueryCounts {
    full_gets: usize,
    range_gets: usize,
    bytes_returned: usize,
}

async fn query_counting_store(sql: &str) -> QueryCounts {
    let inner = Arc::new(InMemory::new());
    inner
        .put_opts(
            &Path::from("dataset/part1.cove"),
            primitive_events_file().into(),
            PutOptions::default(),
        )
        .await
        .unwrap();
    let store = Arc::new(CountingObjectStore::new(inner));
    let ctx = SessionContext::new();
    ctx.register_object_store(
        &Url::parse("cove-test://bucket").unwrap(),
        store.clone() as Arc<dyn ObjectStore>,
    );
    register_cove_listing_table(&ctx, "events", "cove-test://bucket/dataset/")
        .await
        .unwrap();
    let batches = ctx.sql(sql).await.unwrap().collect().await.unwrap();
    assert!(!batches.is_empty());
    store.counts()
}

#[derive(Debug)]
struct CountingObjectStore {
    inner: Arc<dyn ObjectStore>,
    full_gets: std::sync::atomic::AtomicUsize,
    range_gets: std::sync::atomic::AtomicUsize,
    bytes_returned: std::sync::atomic::AtomicUsize,
}

impl CountingObjectStore {
    fn new(inner: Arc<dyn ObjectStore>) -> Self {
        Self {
            inner,
            full_gets: std::sync::atomic::AtomicUsize::new(0),
            range_gets: std::sync::atomic::AtomicUsize::new(0),
            bytes_returned: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn counts(&self) -> QueryCounts {
        QueryCounts {
            full_gets: self.full_gets.load(Ordering::SeqCst),
            range_gets: self.range_gets.load(Ordering::SeqCst),
            bytes_returned: self.bytes_returned.load(Ordering::SeqCst),
        }
    }
}

impl fmt::Display for CountingObjectStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CountingObjectStore")
    }
}

#[async_trait]
impl ObjectStore for CountingObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> datafusion::object_store::Result<PutResult> {
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> datafusion::object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> datafusion::object_store::Result<GetResult> {
        self.full_gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get_opts(location, options).await
    }

    async fn get_ranges(
        &self,
        location: &Path,
        ranges: &[Range<u64>],
    ) -> datafusion::object_store::Result<Vec<bytes::Bytes>> {
        self.range_gets.fetch_add(ranges.len(), Ordering::SeqCst);
        let chunks = self.inner.get_ranges(location, ranges).await?;
        let bytes = chunks.iter().map(|chunk| chunk.len()).sum::<usize>();
        self.bytes_returned.fetch_add(bytes, Ordering::SeqCst);
        Ok(chunks)
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, datafusion::object_store::Result<Path>>,
    ) -> BoxStream<'static, datafusion::object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> BoxStream<'static, datafusion::object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> datafusion::object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: CopyOptions,
    ) -> datafusion::object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }
}
