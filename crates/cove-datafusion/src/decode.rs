//! Decode and materialization kernels shared by execution modes.

mod cache;
mod materialize;
mod morsels;
mod predicates;
mod pruning;
mod selection;
mod stats;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::Arc,
};

use arrow_array::{
    types::UInt32Type, Array, ArrayRef, DictionaryArray, RecordBatch, RecordBatchOptions,
};
use arrow_schema::SchemaRef;
use cove_arrow::arrow::{
    arrow_buffer_owner, encoded_array_to_arrow_with_row_selection_options_and_owner,
    encoded_filecode_array_to_arrow_dictionary_remapped,
    encoded_filecode_array_to_arrow_dictionary_with_values,
    file_dictionary_entries_compatible_with_logical, file_dictionary_values_to_arrow,
    filecode_dictionary_value_export_options, nested_page_payload_to_arrow_array, ArrowBufferOwner,
    ArrowDictionaryPolicy, ArrowExportOptions, ArrowRowSelection, ArrowStringValidationPolicy,
    ArrowVarBytesExportPolicy,
};
use cove_core::{
    array::{logical_type_fixed_width, CoveArrayValue, EncodedArray, PreparedEncodedArray},
    compression,
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind},
    dictionary::{DictionaryValue, FileDictionary},
    index::{lookup::LookupKeyKind, topn::TopNDirection},
    materialize_stats_only_constant_page_payload,
    native::{
        aggregate_i64, aggregate_i64_by_bool, aggregate_i64_by_i64, aggregate_i64_by_u16_dense,
        aggregate_i64_by_u32, aggregate_i64_by_u32_dense, aggregate_i64_by_u8_dense,
        aggregate_i64_le_bytes, aggregate_i64_le_bytes_by_bool,
        aggregate_i64_le_bytes_by_i64_le_bytes, aggregate_i64_le_bytes_by_u16_dense,
        aggregate_i64_le_bytes_by_u32_dense, aggregate_i64_le_bytes_by_u32_le_bytes,
        aggregate_i64_le_bytes_by_u8_dense, group_count_bool, group_count_i64_le_bytes,
        group_count_u16_dense, group_count_u32_dense, group_count_u32_hash,
        group_count_u32_le_bytes, group_count_u8_dense, native_table_batch_from_page_refs,
        sort_rows_i64_with_stats, top_n_rows_i64_with_stats, KernelStats, LaneRef,
        NativeCodeDomain, NativeDenseGroupCounts, NativeDenseI64GroupAggregates,
        NativeHashGroupCounts, NativeI64Aggregates, NativeI64HashGroupCounts,
        NativeI64I64HashGroupAggregates, NativeKernelDispatch, NativeNullOrder,
        NativeSortDirection, NativeTablePageRef, NativeU32I64HashGroupAggregates, ValidityRef,
    },
    nested_schema::NestedSchemaNodeV1,
    page::{
        page_uses_payload_elision, ColumnPageIndex, ColumnPageIndexEntryV1,
        PAGE_FLAG_STATS_ONLY_CONSTANT,
    },
    page_payload::{ColumnPagePayloadV1, PageBufferKind, RetainedColumnPagePayloadV1},
    retained_bytes::RetainedBytes,
    segment::{
        RowMorselDirectory, RowMorselEntryV1, TableColumnDirectoryEntryV1, TableSegmentHeaderV1,
        TableSegmentIndexEntryV1, TableSegmentPayloadV1, TABLE_COLUMN_DIRECTORY_ENTRY_LEN,
        TABLE_SEGMENT_HEADER_LEN,
    },
    table::ColumnEntry,
    validity::ValidityBitmap,
    wire, CoveError, StatsOnlyPageMaterializationContext,
};
use cove_layout::{ZeroCopyCompatibilityV2, ZeroCopyMaterializationReasonV2};

use crate::{
    dataset_state::{DatasetBootstrapStats, DatasetState},
    options::{LocalFileReadPolicy, PagePayloadValidationPolicy},
    planner::{
        plan_scan, CoveFilterUse, CovePredicate, NullPredicateKind, NumericPredicateOp,
        PredicateLiteral, ScanPlan,
    },
    prune,
    range_reader::{
        build_layout_aware_coalesced_range_plan, read_coalesced_range_buffers_for_plan,
        CoveRangeReader, LocalFileRangeReader, MemoryRangeReader, MmapFileRangeReader,
        RangeReadKind, RangeReadMode, RangeReadPlan,
    },
    task_graph::ScanTask,
};
pub use stats::{DecodeStats, DecodedScan};

use cache::{ArrowDictionaryValuesCacheKey, SegmentMetadataCacheKey, Utf8ProofKey};
pub(crate) use cache::{ScanExecutionCache, Utf8ProofCache};
#[cfg(test)]
use materialize::DecodedArrowColumn;
use materialize::{
    arrow_encoded_columns_for_payloads, encoded_array_for_page, materialize_page_payload,
    materialize_page_payload_from_wire, record_batch_for_selection,
};
use morsels::{
    ordered_morsels, prepare_segment_payload, read_segment_metadata, PreparedSegmentColumn,
};
#[cfg(test)]
use predicates::apply_predicate_to_selection;
pub(crate) use predicates::numeric_lookup_keys;
use predicates::{plan_has_exact_row_predicate, plan_has_residual};
use pruning::{
    apply_overlay_to_selection, covi_morsel_pruned, selected_rows_for_morsel,
    selected_rows_for_morsel_metadata, should_prune_morsel, should_prune_morsel_metadata,
};
use selection::{DecodeScratch, Selection, SelectionMask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeControl {
    Continue,
    Stop,
}

pub(crate) trait DecodeSink {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError>;

    fn should_stop(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
struct VecDecodeSink {
    batches: Vec<RecordBatch>,
}

impl VecDecodeSink {
    fn finish(self, stats: DecodeStats) -> DecodedScan {
        DecodedScan {
            batches: self.batches,
            stats,
        }
    }
}

impl DecodeSink for VecDecodeSink {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError> {
        stats.rows_materialized += batch.num_rows();
        self.batches.push(batch);
        Ok(DecodeControl::Continue)
    }
}

#[derive(Debug)]
pub(crate) struct FetchLimitedDecodeSink<S> {
    inner: S,
    remaining: Option<usize>,
    stopped: bool,
}

impl<S> FetchLimitedDecodeSink<S> {
    pub(crate) fn new(inner: S, fetch: Option<usize>) -> Self {
        Self {
            inner,
            remaining: fetch,
            stopped: fetch == Some(0),
        }
    }
}

impl<S: DecodeSink> DecodeSink for FetchLimitedDecodeSink<S> {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError> {
        if self.stopped {
            return Ok(DecodeControl::Stop);
        }

        let batch = match self.remaining {
            Some(0) => {
                self.stopped = true;
                return Ok(DecodeControl::Stop);
            }
            Some(remaining) if batch.num_rows() > remaining => batch.slice(0, remaining),
            _ => batch,
        };
        let emitted_rows = batch.num_rows();
        let control = self.inner.emit_batch(batch, stats)?;
        if let Some(remaining) = self.remaining.as_mut() {
            *remaining = remaining.saturating_sub(emitted_rows);
            if *remaining == 0 {
                self.stopped = true;
                return Ok(DecodeControl::Stop);
            }
        }
        if control == DecodeControl::Stop {
            self.stopped = true;
        }
        Ok(control)
    }

    fn should_stop(&self) -> bool {
        self.stopped || self.inner.should_stop()
    }
}

fn emit_batch<S: DecodeSink + ?Sized>(
    sink: &mut S,
    stats: &mut DecodeStats,
    batch: RecordBatch,
) -> Result<DecodeControl, CoveError> {
    sink.emit_batch(batch, stats)
}

pub fn decode_local_dataset_scan(
    state: &DatasetState,
    plan: &ScanPlan,
) -> Result<DecodedScan, CoveError> {
    let cache = Arc::new(ScanExecutionCache::default());
    decode_local_dataset_scan_with_cache(state, plan, cache)
}

fn decode_local_dataset_scan_with_cache(
    state: &DatasetState,
    plan: &ScanPlan,
    cache: Arc<ScanExecutionCache>,
) -> Result<DecodedScan, CoveError> {
    let mut decoded = DecodedScan {
        batches: Vec::new(),
        stats: DecodeStats::default(),
    };
    decoded.stats.record_bootstrap(state.bootstrap_stats());

    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        decoded.stats.execution_code_profiles_used += execution_stats.supported_files;
        decoded.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        decoded.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            decoded.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_decoded = if file_state.has_full_file_bytes() {
            decode_scan(&file_state, &file_plan)?
        } else if Path::new(file_state.source()).is_file() {
            let reader = cache.local_reader(
                file_ordinal,
                file_state.local_file_read_policy(),
                file_state.source(),
            )?;
            futures::executor::block_on(decode_scan_with_reader_cached(
                &file_state,
                &file_plan,
                reader.as_ref(),
                Some(cache.as_ref()),
                file_ordinal,
            ))?
        } else {
            decode_scan(&file_state, &file_plan)?
        };
        decoded.stats.add_decode(file_decoded.stats);
        decoded.batches.extend(file_decoded.batches);
    }
    Ok(decoded)
}

pub fn decode_local_dataset_scan_tasks(
    state: &DatasetState,
    plan: &ScanPlan,
    tasks: &[ScanTask],
    partition_index: usize,
    partition_count: usize,
) -> Result<DecodedScan, CoveError> {
    let cache = Arc::new(ScanExecutionCache::default());
    decode_local_dataset_scan_tasks_with_cache(
        state,
        plan,
        tasks,
        partition_index,
        partition_count,
        cache,
    )
}

pub(crate) fn decode_local_dataset_scan_tasks_with_cache(
    state: &DatasetState,
    plan: &ScanPlan,
    tasks: &[ScanTask],
    partition_index: usize,
    partition_count: usize,
    cache: Arc<ScanExecutionCache>,
) -> Result<DecodedScan, CoveError> {
    let mut sink = VecDecodeSink::default();
    let stats = decode_local_dataset_scan_tasks_with_sink(
        state,
        plan,
        tasks,
        partition_index,
        partition_count,
        cache,
        &mut sink,
    )?;
    Ok(sink.finish(stats))
}

pub(crate) fn decode_local_dataset_scan_tasks_with_sink<S: DecodeSink + ?Sized>(
    state: &DatasetState,
    plan: &ScanPlan,
    tasks: &[ScanTask],
    partition_index: usize,
    partition_count: usize,
    cache: Arc<ScanExecutionCache>,
    sink: &mut S,
) -> Result<DecodeStats, CoveError> {
    let mut stats = DecodeStats {
        scan_tasks: tasks.len(),
        scan_partitions: usize::from(partition_index == 0) * partition_count,
        covel_scan_splits_used: tasks
            .iter()
            .filter_map(|task| task.split_id.map(|split_id| (task.file_ordinal, split_id)))
            .collect::<BTreeSet<_>>()
            .len(),
        ..DecodeStats::default()
    };
    if partition_index == 0 {
        stats.record_bootstrap(state.bootstrap_stats());
    }
    if sink.should_stop() {
        return Ok(stats);
    }

    let mut task_start = 0usize;
    while task_start < tasks.len() {
        let file_ordinal = tasks[task_start].file_ordinal;
        let task_end = tasks[task_start..]
            .iter()
            .position(|task| task.file_ordinal != file_ordinal)
            .map(|offset| task_start + offset)
            .unwrap_or(tasks.len());
        let file_tasks = &tasks[task_start..task_end];
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        stats.execution_code_profiles_used += execution_stats.supported_files;
        stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            stats.files_pruned += 1;
            task_start = task_end;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_stats = if let Some(bytes) = file_state
            .files()
            .first()
            .and_then(|file| file.full_file_bytes_arc())
        {
            let reader = MemoryRangeReader::from_arc(bytes);
            futures::executor::block_on(decode_scan_with_reader_tasks_to_sink_cached(
                &file_state,
                &file_plan,
                &reader,
                file_tasks,
                Some(cache.as_ref()),
                file_ordinal,
                sink,
            ))?
        } else if Path::new(file_state.source()).is_file() {
            let reader = cache.local_reader(
                file_ordinal,
                file_state.local_file_read_policy(),
                file_state.source(),
            )?;
            futures::executor::block_on(decode_scan_with_reader_tasks_to_sink_cached(
                &file_state,
                &file_plan,
                reader.as_ref(),
                file_tasks,
                Some(cache.as_ref()),
                file_ordinal,
                sink,
            ))?
        } else {
            decode_scan_to_sink(&file_state, &file_plan, sink)?
        };
        stats.add_decode(file_stats);
        if sink.should_stop() {
            return Ok(stats);
        }
        task_start = task_end;
    }
    Ok(stats)
}

/// Decode a planned native single-file scan into Arrow record batches.
///
/// INVARIANT: this routine emits rows in segment order and morsel order, and it
/// delegates scalar COVE-to-Arrow representation rules to `cove-arrow`. FileCode
/// predicates are resolved against this concrete single-file view before
/// pruning or residual filtering begins.
pub fn decode_scan(state: &DatasetState, plan: &ScanPlan) -> Result<DecodedScan, CoveError> {
    let mut sink = VecDecodeSink::default();
    let stats = decode_scan_to_sink(state, plan, &mut sink)?;
    Ok(sink.finish(stats))
}

#[derive(Debug, Clone, Default)]
pub struct NativeI64AggregateScan {
    pub aggregate: NativeI64Aggregates,
    pub stats: DecodeStats,
}

pub fn native_unfiltered_i64_aggregate_scan(
    state: &DatasetState,
    column_index: usize,
) -> Result<NativeI64AggregateScan, CoveError> {
    let projection = Vec::new();
    let plan = plan_scan(state, Some(&projection), Vec::new())?;
    native_i64_aggregate_scan(state, column_index, &plan)
}

pub fn native_i64_aggregate_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64AggregateScan, CoveError> {
    let mut out = NativeI64AggregateScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_i64_aggregate_single_file(&file_state, column_index, &file_plan)?;
        merge_i64_aggregates(&mut out.aggregate, &file_scan.aggregate)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
pub struct NativeI64GroupCountScan {
    pub groups: NativeI64HashGroupCounts,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeGroupCounts {
    pub counts: BTreeMap<Vec<u8>, u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeI64GroupAggregate {
    pub row_count: u64,
    pub aggregate: NativeI64Aggregates,
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeI64GroupAggregates {
    pub groups: BTreeMap<Vec<u8>, NativeFileCodeI64GroupAggregate>,
    pub null_row_count: u64,
    pub null_aggregate: NativeI64Aggregates,
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeGroupCountScan {
    pub groups: NativeFileCodeGroupCounts,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeBoolGroupCountScan {
    pub groups: NativeDenseGroupCounts,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeBoolI64GroupAggregateScan {
    pub groups: NativeDenseI64GroupAggregates,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeI64I64GroupAggregateScan {
    pub groups: NativeI64I64HashGroupAggregates,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeI64GroupAggregateScan {
    pub groups: NativeFileCodeI64GroupAggregates,
    pub stats: DecodeStats,
}

pub fn native_unfiltered_i64_group_count_scan(
    state: &DatasetState,
    column_index: usize,
) -> Result<NativeI64GroupCountScan, CoveError> {
    let projection = Vec::new();
    let plan = plan_scan(state, Some(&projection), Vec::new())?;
    native_i64_group_count_scan(state, column_index, &plan)
}

pub fn native_i64_group_count_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64GroupCountScan, CoveError> {
    let mut out = NativeI64GroupCountScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_i64_group_count_single_file(&file_state, column_index, &file_plan)?;
        merge_i64_group_counts(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

pub fn native_bool_group_count_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeBoolGroupCountScan, CoveError> {
    let mut out = NativeBoolGroupCountScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_bool_group_count_single_file(&file_state, column_index, &file_plan)?;
        merge_bool_group_counts(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

pub fn native_bool_i64_group_aggregate_scan(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeBoolI64GroupAggregateScan, CoveError> {
    let mut out = NativeBoolI64GroupAggregateScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_bool_i64_group_aggregate_single_file(
            &file_state,
            key_column_index,
            value_column_index,
            &file_plan,
        )?;
        merge_bool_i64_group_aggregates(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

pub fn native_i64_i64_group_aggregate_scan(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64I64GroupAggregateScan, CoveError> {
    let mut out = NativeI64I64GroupAggregateScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_i64_i64_group_aggregate_single_file(
            &file_state,
            key_column_index,
            value_column_index,
            &file_plan,
        )?;
        merge_i64_i64_group_aggregates(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

pub fn native_filecode_i64_group_aggregate_scan(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeI64GroupAggregateScan, CoveError> {
    let mut out = NativeFileCodeI64GroupAggregateScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_filecode_i64_group_aggregate_single_file(
            &file_state,
            key_column_index,
            value_column_index,
            &file_plan,
        )?;
        merge_filecode_i64_group_aggregates(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

pub fn native_filecode_group_count_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeGroupCountScan, CoveError> {
    let mut out = NativeFileCodeGroupCountScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan =
            native_filecode_group_count_single_file(&file_state, column_index, &file_plan)?;
        merge_filecode_group_counts(&mut out.groups, &file_scan.groups)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
pub struct NativeI64OrderScan {
    pub values: Vec<Option<i64>>,
    pub stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NativeI64DenseLane {
    values: Vec<i64>,
    null_bitmap: Vec<u8>,
    null_count: usize,
}

impl NativeI64DenseLane {
    fn push(&mut self, value: Option<i64>) -> Result<(), CoveError> {
        let row = self.values.len();
        if row.is_multiple_of(8) {
            self.null_bitmap.push(0);
        }
        match value {
            Some(value) => self.values.push(value),
            None => {
                self.values.push(0);
                self.null_count = self
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
                let byte = self.null_bitmap.last_mut().ok_or(CoveError::PageCorrupt)?;
                *byte |= 1 << (row % 8);
            }
        }
        Ok(())
    }

    fn extend(&mut self, other: NativeI64DenseLane) -> Result<(), CoveError> {
        let old_len = self.values.len();
        let other_len = other.values.len();
        if other_len == 0 {
            return Ok(());
        }
        if old_len == 0 {
            *self = other;
            return Ok(());
        }

        let new_len = old_len
            .checked_add(other_len)
            .ok_or(CoveError::ArithOverflow)?;
        let other_null_bitmap = other.null_bitmap;
        let other_null_count = other.null_count;
        self.values.extend(other.values);
        self.null_bitmap.resize(new_len.div_ceil(8), 0);
        self.null_count = self
            .null_count
            .checked_add(other_null_count)
            .ok_or(CoveError::ArithOverflow)?;
        if other_null_count == 0 {
            return Ok(());
        }

        for (byte_index, byte) in other_null_bitmap.iter().copied().enumerate() {
            let mut live = byte;
            while live != 0 {
                let bit = live.trailing_zeros() as usize;
                let source_row = byte_index
                    .checked_mul(8)
                    .and_then(|base| base.checked_add(bit))
                    .ok_or(CoveError::ArithOverflow)?;
                if source_row < other_len {
                    self.set_null(
                        old_len
                            .checked_add(source_row)
                            .ok_or(CoveError::ArithOverflow)?,
                    )?;
                }
                live &= live - 1;
            }
        }
        Ok(())
    }

    fn set_null(&mut self, row: usize) -> Result<(), CoveError> {
        let byte = self
            .null_bitmap
            .get_mut(row / 8)
            .ok_or(CoveError::PageCorrupt)?;
        *byte |= 1 << (row % 8);
        Ok(())
    }

    pub(crate) fn values(&self) -> &[i64] {
        &self.values
    }

    pub(crate) fn validity(&self) -> ValidityRef<'_> {
        let row_count = self.values.len();
        if self.null_count == 0 {
            ValidityRef::AllValid { row_count }
        } else if self.null_count == row_count {
            ValidityRef::AllNull { row_count }
        } else {
            ValidityRef::CoveNullBitmap {
                bytes: &self.null_bitmap,
                row_count,
            }
        }
    }

    pub(crate) fn value_at(&self, row: usize) -> Result<Option<i64>, CoveError> {
        let value = self
            .values
            .get(row)
            .copied()
            .ok_or(CoveError::PageCorrupt)?;
        Ok(self.validity().is_valid(row).then_some(value))
    }

    pub(crate) fn values_for_rows(&self, rows: &[u32]) -> Result<Vec<Option<i64>>, CoveError> {
        let mut out = Vec::with_capacity(rows.len());
        for &row in rows {
            out.push(self.value_at(usize::try_from(row).map_err(|_| CoveError::ArithOverflow)?)?);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NativeFileCodeDenseLane {
    codes: Vec<u32>,
    null_bitmap: Vec<u8>,
    null_count: usize,
    canonical_values: Vec<Vec<u8>>,
    canonical_to_code: HashMap<Vec<u8>, u32>,
}

impl NativeFileCodeDenseLane {
    fn push(&mut self, value: Option<Vec<u8>>) -> Result<(), CoveError> {
        let row = self.codes.len();
        if row.is_multiple_of(8) {
            self.null_bitmap.push(0);
        }
        match value {
            Some(value) => {
                let code = self.intern(value)?;
                self.codes.push(code);
            }
            None => {
                self.codes.push(0);
                self.null_count = self
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
                let byte = self.null_bitmap.last_mut().ok_or(CoveError::PageCorrupt)?;
                *byte |= 1 << (row % 8);
            }
        }
        Ok(())
    }

    fn intern(&mut self, value: Vec<u8>) -> Result<u32, CoveError> {
        if let Some(code) = self.canonical_to_code.get(value.as_slice()).copied() {
            return Ok(code);
        }
        let code =
            u32::try_from(self.canonical_values.len()).map_err(|_| CoveError::ArithOverflow)?;
        self.canonical_values.push(value.clone());
        self.canonical_to_code.insert(value, code);
        Ok(code)
    }

    fn extend(&mut self, other: NativeFileCodeDenseLane) -> Result<(), CoveError> {
        for row in 0..other.codes.len() {
            if other.validity().is_valid(row) {
                let code = *other.codes.get(row).ok_or(CoveError::PageCorrupt)?;
                let canonical = other
                    .canonical_values
                    .get(usize::try_from(code).map_err(|_| CoveError::ArithOverflow)?)
                    .cloned()
                    .ok_or(CoveError::PageCorrupt)?;
                self.push(Some(canonical))?;
            } else {
                self.push(None)?;
            }
        }
        Ok(())
    }

    pub(crate) fn codes(&self) -> &[u32] {
        &self.codes
    }

    pub(crate) fn canonical_values(&self) -> &[Vec<u8>] {
        &self.canonical_values
    }

    pub(crate) fn validity(&self) -> ValidityRef<'_> {
        let row_count = self.codes.len();
        if self.null_count == 0 {
            ValidityRef::AllValid { row_count }
        } else if self.null_count == row_count {
            ValidityRef::AllNull { row_count }
        } else {
            ValidityRef::CoveNullBitmap {
                bytes: &self.null_bitmap,
                row_count,
            }
        }
    }

    pub(crate) fn value_at(&self, row: usize) -> Result<Option<&[u8]>, CoveError> {
        let code = self.codes.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        if !self.validity().is_valid(row) {
            return Ok(None);
        }
        self.canonical_values
            .get(usize::try_from(code).map_err(|_| CoveError::ArithOverflow)?)
            .map(|value| Some(value.as_slice()))
            .ok_or(CoveError::PageCorrupt)
    }
}

#[derive(Debug, Clone, Default)]
struct NativeI64DenseScan {
    lane: NativeI64DenseLane,
    stats: DecodeStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeI64KeyKernelKind {
    SortInput,
    Join,
}

pub fn native_i64_order_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
    descending: bool,
    nulls_first: bool,
    fetch: Option<usize>,
) -> Result<NativeI64OrderScan, CoveError> {
    let mut out = NativeI64OrderScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    if fetch == Some(0) {
        return Ok(out);
    }

    let mut lane = NativeI64DenseLane::default();
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_i64_values_single_file(
            &file_state,
            column_index,
            &file_plan,
            NativeI64KeyKernelKind::SortInput,
        )?;
        lane.extend(file_scan.lane)?;
        out.stats.add_decode(file_scan.stats);
    }

    let direction = if descending {
        NativeSortDirection::Descending
    } else {
        NativeSortDirection::Ascending
    };
    let null_order = if nulls_first {
        NativeNullOrder::First
    } else {
        NativeNullOrder::Last
    };
    let (rows, kernel_stats) = match fetch {
        Some(fetch) => top_n_rows_i64_with_stats(
            &lane.values,
            lane.validity(),
            direction,
            null_order,
            None,
            fetch,
        )?,
        None => {
            sort_rows_i64_with_stats(&lane.values, lane.validity(), direction, null_order, None)?
        }
    };
    out.stats.record_native_sort_kernel(kernel_stats);
    out.values = lane.values_for_rows(rows.rows())?;
    Ok(out)
}

#[derive(Debug, Clone, Default)]
pub struct NativeI64ValuesScan {
    pub(crate) lane: NativeI64DenseLane,
    pub stats: DecodeStats,
}

impl NativeI64ValuesScan {
    pub fn values(&self) -> Result<Vec<Option<i64>>, CoveError> {
        (0..self.lane.values().len())
            .map(|row| self.lane.value_at(row))
            .collect()
    }
}

pub fn native_i64_values_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64ValuesScan, CoveError> {
    let mut out = NativeI64ValuesScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());

    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_i64_values_single_file(
            &file_state,
            column_index,
            &file_plan,
            NativeI64KeyKernelKind::Join,
        )?;
        out.lane.extend(file_scan.lane)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
pub struct NativeFileCodeValuesScan {
    pub(crate) lane: NativeFileCodeDenseLane,
    pub stats: DecodeStats,
}

impl NativeFileCodeValuesScan {
    pub fn values(&self) -> Result<Vec<Option<Vec<u8>>>, CoveError> {
        (0..self.lane.codes().len())
            .map(|row| {
                self.lane
                    .value_at(row)
                    .map(|value| value.map(|value| value.to_vec()))
            })
            .collect()
    }
}

pub fn native_filecode_values_scan(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeValuesScan, CoveError> {
    let mut out = NativeFileCodeValuesScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());

    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_filecode_values_single_file(&file_state, column_index, &file_plan)?;
        out.lane.extend(file_scan.lane)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

#[derive(Debug, Clone, Default)]
struct NativeFileCodeDenseScan {
    lane: NativeFileCodeDenseLane,
    stats: DecodeStats,
}

#[derive(Debug, Clone, Default)]
pub struct NativeRowCountScan {
    pub count: u64,
    pub stats: DecodeStats,
}

pub fn native_row_count_scan(
    state: &DatasetState,
    plan: &ScanPlan,
) -> Result<NativeRowCountScan, CoveError> {
    let mut out = NativeRowCountScan::default();
    out.stats.record_bootstrap(state.bootstrap_stats());
    for file_ordinal in 0..state.file_count() {
        let (file_plan, execution_stats) =
            state.resolved_plan_for_file_with_stats(plan, file_ordinal)?;
        out.stats.execution_code_profiles_used += execution_stats.supported_files;
        out.stats.execution_code_profile_fallbacks += execution_stats.fallback_files;
        out.stats.execution_code_literal_resolutions += execution_stats.literal_resolutions;
        if plan_selects_no_rows(&file_plan) {
            out.stats.files_pruned += 1;
            continue;
        }
        let file_state = state.single_file_view(file_ordinal)?;
        let file_scan = native_row_count_single_file(&file_state, &file_plan)?;
        out.count = out
            .count
            .checked_add(file_scan.count)
            .ok_or(CoveError::ArithOverflow)?;
        out.stats.add_decode(file_scan.stats);
    }
    Ok(out)
}

fn native_row_count_single_file(
    state: &DatasetState,
    plan: &ScanPlan,
) -> Result<NativeRowCountScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native row count scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeRowCountScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            let selected_len = scratch.selection.len();
            out.stats.record_native_count_scan(
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                selected_len,
            );
            if selected_len == 0 {
                continue;
            }
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }
            out.count = out
                .count
                .checked_add(u64::try_from(selected_len).map_err(|_| CoveError::ArithOverflow)?)
                .ok_or(CoveError::ArithOverflow)?;
        }
    }
    Ok(out)
}

fn native_i64_aggregate_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64AggregateScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native aggregate scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Int64 || column.physical != CovePhysicalKind::NumCode {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native i64 aggregate requires Int64/NumCode, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeI64AggregateScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut aggregate_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut aggregate_base,
            )?;
            let page_aggregate = native_i64_aggregate_for_page(
                state,
                &segment,
                segment_column,
                page,
                &payload,
                base,
                &mut out.stats,
            )?;
            merge_i64_aggregates(&mut out.aggregate, &page_aggregate)?;
        }
    }
    Ok(out)
}

fn native_i64_group_count_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64GroupCountScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native group count scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Int64 || column.physical != CovePhysicalKind::NumCode {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native i64 group count requires Int64/NumCode, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeI64GroupCountScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut group_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut group_base,
            )?;
            let page_groups = native_i64_group_count_for_page(
                &segment,
                segment_column,
                page,
                &payload,
                base,
                &mut out.stats,
            )?;
            merge_i64_group_counts(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_bool_group_count_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeBoolGroupCountScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native bool group count scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Bool || column.physical != CovePhysicalKind::Boolean {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native bool group count requires Bool/Boolean, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeBoolGroupCountScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut group_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut group_base,
            )?;
            let page_groups = native_bool_group_count_for_page(
                &segment,
                segment_column,
                page,
                &payload,
                base,
                &mut out.stats,
            )?;
            merge_bool_group_counts(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_bool_i64_group_aggregate_single_file(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeBoolI64GroupAggregateScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native bool/i64 group aggregate scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let key_column = state.table().columns.get(key_column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {key_column_index} out of bounds"))
    })?;
    if key_column.logical != CoveLogicalType::Bool
        || key_column.physical != CovePhysicalKind::Boolean
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native bool/i64 group aggregate key requires Bool/Boolean, got {:?}/{:?}",
            key_column.logical, key_column.physical
        )));
    }
    let value_column = state
        .table()
        .columns
        .get(value_column_index)
        .ok_or_else(|| {
            CoveError::BadSchema(format!("column index {value_column_index} out of bounds"))
        })?;
    if value_column.logical != CoveLogicalType::Int64
        || value_column.physical != CovePhysicalKind::NumCode
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native bool/i64 group aggregate value requires Int64/NumCode, got {:?}/{:?}",
            value_column.logical, value_column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeBoolI64GroupAggregateScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut aggregate_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let key_segment_column = prepared_segment.column(key_column.column_id)?;
        let value_segment_column = prepared_segment.column(value_column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let key_page =
                prepared_segment.page_for_morsel(key_segment_column, morsel.morsel_id)?;
            let value_page =
                prepared_segment.page_for_morsel(value_segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, key_page)?;
            state.reject_table_scan_page_feature_use(segment_ref, value_page)?;
            let key_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                key_column,
                key_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            let value_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                value_column,
                value_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded +=
                usize::from(key_page.page_length != 0) + usize::from(value_page.page_length != 0);
            let key_page_len =
                usize::try_from(key_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            let value_page_len =
                usize::try_from(value_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(key_page_len)
                .and_then(|bytes| bytes.checked_add(value_page_len))
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut aggregate_base,
            )?;
            let page_groups = native_bool_i64_group_aggregate_for_pages(
                &segment,
                key_segment_column,
                key_page,
                &key_payload,
                value_segment_column,
                value_page,
                &value_payload,
                base,
                &mut out.stats,
            )?;
            merge_bool_i64_group_aggregates(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_i64_i64_group_aggregate_single_file(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeI64I64GroupAggregateScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native i64/i64 group aggregate scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let key_column = state.table().columns.get(key_column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {key_column_index} out of bounds"))
    })?;
    if key_column.logical != CoveLogicalType::Int64
        || key_column.physical != CovePhysicalKind::NumCode
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native i64/i64 group aggregate key requires Int64/NumCode, got {:?}/{:?}",
            key_column.logical, key_column.physical
        )));
    }
    let value_column = state
        .table()
        .columns
        .get(value_column_index)
        .ok_or_else(|| {
            CoveError::BadSchema(format!("column index {value_column_index} out of bounds"))
        })?;
    if value_column.logical != CoveLogicalType::Int64
        || value_column.physical != CovePhysicalKind::NumCode
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native i64/i64 group aggregate value requires Int64/NumCode, got {:?}/{:?}",
            value_column.logical, value_column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeI64I64GroupAggregateScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut aggregate_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let key_segment_column = prepared_segment.column(key_column.column_id)?;
        let value_segment_column = prepared_segment.column(value_column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let key_page =
                prepared_segment.page_for_morsel(key_segment_column, morsel.morsel_id)?;
            let value_page =
                prepared_segment.page_for_morsel(value_segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, key_page)?;
            state.reject_table_scan_page_feature_use(segment_ref, value_page)?;
            let key_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                key_column,
                key_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            let value_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                value_column,
                value_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded +=
                usize::from(key_page.page_length != 0) + usize::from(value_page.page_length != 0);
            let key_page_len =
                usize::try_from(key_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            let value_page_len =
                usize::try_from(value_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(key_page_len)
                .and_then(|bytes| bytes.checked_add(value_page_len))
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut aggregate_base,
            )?;
            let page_groups = native_i64_i64_group_aggregate_for_pages(
                &segment,
                key_segment_column,
                key_page,
                &key_payload,
                value_segment_column,
                value_page,
                &value_payload,
                base,
                &mut out.stats,
            )?;
            merge_i64_i64_group_aggregates(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_filecode_i64_group_aggregate_single_file(
    state: &DatasetState,
    key_column_index: usize,
    value_column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeI64GroupAggregateScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native FileCode/i64 group aggregate scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let key_column = state.table().columns.get(key_column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {key_column_index} out of bounds"))
    })?;
    if key_column.logical != CoveLogicalType::Utf8
        || key_column.physical != CovePhysicalKind::FileCode
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native FileCode/i64 group aggregate key requires Utf8/FileCode, got {:?}/{:?}",
            key_column.logical, key_column.physical
        )));
    }
    let value_column = state
        .table()
        .columns
        .get(value_column_index)
        .ok_or_else(|| {
            CoveError::BadSchema(format!("column index {value_column_index} out of bounds"))
        })?;
    if value_column.logical != CoveLogicalType::Int64
        || value_column.physical != CovePhysicalKind::NumCode
    {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native FileCode/i64 group aggregate value requires Int64/NumCode, got {:?}/{:?}",
            value_column.logical, value_column.physical
        )));
    }
    if state.file(0)?.has_redaction() {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode/i64 group aggregate rejects redacted dictionaries".into(),
        ));
    }
    let Some(dictionary) = state.mounted().dictionary.as_ref() else {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode/i64 group aggregate requires a file dictionary".into(),
        ));
    };
    validate_scan_plan(state, plan)?;

    let mut out = NativeFileCodeI64GroupAggregateScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut aggregate_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let key_segment_column = prepared_segment.column(key_column.column_id)?;
        let value_segment_column = prepared_segment.column(value_column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let key_page =
                prepared_segment.page_for_morsel(key_segment_column, morsel.morsel_id)?;
            let value_page =
                prepared_segment.page_for_morsel(value_segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, key_page)?;
            state.reject_table_scan_page_feature_use(segment_ref, value_page)?;
            let key_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                key_column,
                key_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            let value_payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                value_column,
                value_page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded +=
                usize::from(key_page.page_length != 0) + usize::from(value_page.page_length != 0);
            let key_page_len =
                usize::try_from(key_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            let value_page_len =
                usize::try_from(value_page.page_length).map_err(|_| CoveError::OffsetRange)?;
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(key_page_len)
                .and_then(|bytes| bytes.checked_add(value_page_len))
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut aggregate_base,
            )?;
            let page_groups = native_filecode_i64_group_aggregate_for_pages(
                &segment,
                key_segment_column,
                key_page,
                &key_payload,
                value_segment_column,
                value_page,
                &value_payload,
                dictionary,
                base,
                &mut out.stats,
            )?;
            merge_filecode_i64_group_aggregates(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_filecode_group_count_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeGroupCountScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native FileCode group count scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Utf8 || column.physical != CovePhysicalKind::FileCode {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native FileCode group count requires Utf8/FileCode, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    if state.file(0)?.has_redaction() {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode group count rejects redacted dictionaries".into(),
        ));
    }
    let Some(dictionary) = state.mounted().dictionary.as_ref() else {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode group count requires a file dictionary".into(),
        ));
    };
    validate_scan_plan(state, plan)?;

    let mut out = NativeFileCodeGroupCountScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut group_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut group_base,
            )?;
            let page_groups = native_filecode_group_count_for_page(
                &segment,
                segment_column,
                page,
                &payload,
                dictionary,
                base,
                &mut out.stats,
            )?;
            merge_filecode_group_counts(&mut out.groups, &page_groups)?;
        }
    }
    Ok(out)
}

fn native_i64_values_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
    kernel_kind: NativeI64KeyKernelKind,
) -> Result<NativeI64DenseScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native i64 value scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Int64 || column.physical != CovePhysicalKind::NumCode {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native i64 value scan requires Int64/NumCode, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    validate_scan_plan(state, plan)?;

    let mut out = NativeI64DenseScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut value_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut value_base,
            )?;
            collect_native_i64_values_for_page(
                &segment,
                segment_column,
                page,
                &payload,
                base,
                &mut out.lane,
                &mut out.stats,
                kernel_kind,
            )?;
        }
    }
    Ok(out)
}

fn native_filecode_values_single_file(
    state: &DatasetState,
    column_index: usize,
    plan: &ScanPlan,
) -> Result<NativeFileCodeDenseScan, CoveError> {
    if state.file_count() != 1 {
        return Err(CoveError::BadSchema(format!(
            "native FileCode value scan requires a single-file view, found {} files",
            state.file_count()
        )));
    }
    let column = state.table().columns.get(column_index).ok_or_else(|| {
        CoveError::BadSchema(format!("column index {column_index} out of bounds"))
    })?;
    if column.logical != CoveLogicalType::Utf8 || column.physical != CovePhysicalKind::FileCode {
        return Err(CoveError::UnsupportedEncoding(format!(
            "native FileCode value scan requires Utf8/FileCode, got {:?}/{:?}",
            column.logical, column.physical
        )));
    }
    if state.file(0)?.has_redaction() {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode value scan rejects redacted dictionaries".into(),
        ));
    }
    let Some(dictionary) = state.mounted().dictionary.as_ref() else {
        return Err(CoveError::UnsupportedEncoding(
            "native FileCode value scan requires a file dictionary".into(),
        ));
    };
    validate_scan_plan(state, plan)?;

    let mut out = NativeFileCodeDenseScan::default();
    record_plan_predicates(plan, &mut out.stats);
    let mut scratch = DecodeScratch::default();
    let mut value_base = SelectionMask::default();
    let file_bytes = local_full_file_bytes_for_native_aggregate(state)?;
    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            file_bytes.as_ref(),
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;
        let segment_column = prepared_segment.column(column.column_id)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            plan,
        ) {
            out.stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                out.stats.morsels_pruned += 1;
                out.stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                out.stats.morsels_pruned += 1;
                out.stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    plan,
                    &mut out.stats,
                )?
            {
                out.stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                plan,
                &mut out.stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut out.stats,
            )?;
            scratch.selection.record(&mut out.stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            out.stats.rows_selected += selected_len;
            if plan_has_residual(plan) {
                out.stats.residual_rows += selected_len;
            }

            let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
            state.reject_table_scan_page_feature_use(segment_ref, page)?;
            let payload = materialize_page_payload(
                segment_bytes,
                segment_ref.table_id,
                segment_ref.segment_id,
                column,
                page,
                state.pruning().zone_stats_entries.as_slice(),
                state.pruning().codec_descriptors.as_slice(),
                state
                    .mounted()
                    .dictionary
                    .as_ref()
                    .map(|dictionary| dictionary.len()),
                state.page_payload_validation_policy(),
            )?;
            out.stats.pages_decoded += usize::from(page.page_length != 0);
            out.stats.data_bytes_read = out
                .stats
                .data_bytes_read
                .checked_add(usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?)
                .ok_or(CoveError::ArithOverflow)?;

            let base = aggregate_base_for_selection(
                &scratch.selection,
                usize::try_from(morsel.row_count).map_err(|_| CoveError::OffsetRange)?,
                &mut value_base,
            )?;
            collect_native_filecode_values_for_page(
                &segment,
                segment_column,
                page,
                &payload,
                dictionary,
                base,
                &mut out.lane,
                &mut out.stats,
            )?;
        }
    }
    Ok(out)
}

fn aggregate_base_for_selection<'a>(
    selection: &'a Selection,
    row_count: usize,
    scratch: &'a mut SelectionMask,
) -> Result<Option<&'a SelectionMask>, CoveError> {
    match selection {
        Selection::None => {
            scratch.fill_none(row_count);
            Ok(Some(scratch))
        }
        Selection::AllRows { len } if *len == row_count => Ok(None),
        Selection::AllRows { len } => Err(CoveError::BadSection(format!(
            "selection length {len} does not match morsel row count {row_count}"
        ))),
        Selection::Bitset(mask) => {
            if mask.len() != row_count {
                return Err(CoveError::BadSection(format!(
                    "selection length {} does not match morsel row count {row_count}",
                    mask.len()
                )));
            }
            Ok(Some(mask))
        }
        Selection::RowIndices(rows) => {
            scratch.fill_none(row_count);
            for row in rows {
                let index = usize::try_from(*row).map_err(|_| CoveError::ArithOverflow)?;
                if index >= row_count {
                    return Err(CoveError::BadSection(format!(
                        "selected row {index} outside morsel row count {row_count}"
                    )));
                }
                scratch.set(index);
            }
            Ok(Some(scratch))
        }
    }
}

fn local_full_file_bytes_for_native_aggregate(
    state: &DatasetState,
) -> Result<Cow<'_, [u8]>, CoveError> {
    if let Ok(bytes) = state.full_file_bytes() {
        return Ok(Cow::Borrowed(bytes));
    }
    if Path::new(state.source()).is_file() {
        return Ok(Cow::Owned(std::fs::read(state.source())?));
    }
    Err(CoveError::BadSection(
        "native aggregate scan requires local file bytes or a local file source".into(),
    ))
}

fn native_i64_group_count_for_page(
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64HashGroupCounts, CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            logical_type: CoveLogicalType::Int64,
            ..
        } => {
            let (groups, kernel_stats) =
                group_count_i64_le_bytes(bytes, *row_count, *validity, base)?;
            stats.record_native_group_kernel(kernel_stats);
            Ok(groups)
        }
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            group_count_i64_page_fallback(page, payload, base, stats)
        }
        _ => group_count_i64_page_fallback(page, payload, base, stats),
    }
}

fn native_bool_group_count_for_page(
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeDenseGroupCounts, CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::Bool {
            values,
            row_count,
            validity,
            ..
        } => {
            let (groups, kernel_stats) = group_count_bool(values, *row_count, *validity, base)?;
            stats.record_native_group_kernel(kernel_stats);
            Ok(groups)
        }
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            group_count_bool_page_fallback(page, payload, base, stats)
        }
        _ => group_count_bool_page_fallback(page, payload, base, stats),
    }
}

fn native_bool_i64_group_aggregate_for_pages(
    segment: &TableSegmentPayloadV1,
    key_segment_column: &PreparedSegmentColumn,
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_segment_column: &PreparedSegmentColumn,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeDenseI64GroupAggregates, CoveError> {
    let key_ref = NativeTablePageRef {
        column: key_segment_column.directory(),
        page: key_page,
        payload: Some(key_payload),
    };
    let value_ref = NativeTablePageRef {
        column: value_segment_column.directory(),
        page: value_page,
        payload: Some(value_payload),
    };
    let batch = native_table_batch_from_page_refs(
        segment,
        &[key_ref, value_ref],
        NativeCodeDomain::default(),
    )?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let [key_native_page, value_native_page] = batch.column_pages.as_slice() else {
        return Err(CoveError::PageCorrupt);
    };
    match (&key_native_page.lane, &value_native_page.lane) {
        (
            LaneRef::Bool {
                values: key_values,
                row_count,
                validity: key_validity,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if row_count == value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_bool(
                key_values,
                value_bytes,
                *row_count,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(groups)
        }
        (
            LaneRef::Bool {
                values: key_values,
                row_count,
                validity: key_validity,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if *row_count == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_bool(
                key_values,
                value_values,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(groups)
        }
        (LaneRef::DecodeBoundary { .. }, _) | (_, LaneRef::DecodeBoundary { .. }) => {
            stats.native_table_decode_boundaries += 1;
            group_aggregate_bool_i64_page_fallback(
                key_page,
                key_payload,
                value_page,
                value_payload,
                base,
                stats,
            )
        }
        _ => group_aggregate_bool_i64_page_fallback(
            key_page,
            key_payload,
            value_page,
            value_payload,
            base,
            stats,
        ),
    }
}

fn native_i64_i64_group_aggregate_for_pages(
    segment: &TableSegmentPayloadV1,
    key_segment_column: &PreparedSegmentColumn,
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_segment_column: &PreparedSegmentColumn,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64I64HashGroupAggregates, CoveError> {
    let key_ref = NativeTablePageRef {
        column: key_segment_column.directory(),
        page: key_page,
        payload: Some(key_payload),
    };
    let value_ref = NativeTablePageRef {
        column: value_segment_column.directory(),
        page: value_page,
        payload: Some(value_payload),
    };
    let batch = native_table_batch_from_page_refs(
        segment,
        &[key_ref, value_ref],
        NativeCodeDomain::default(),
    )?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let [key_native_page, value_native_page] = batch.column_pages.as_slice() else {
        return Err(CoveError::PageCorrupt);
    };
    match (&key_native_page.lane, &value_native_page.lane) {
        (
            LaneRef::NumCodeU64LeBytes {
                bytes: key_bytes,
                row_count,
                validity: key_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if row_count == value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_i64_le_bytes(
                key_bytes,
                value_bytes,
                *row_count,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(groups)
        }
        (
            LaneRef::NumCodeI64 {
                values: key_values,
                validity: key_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_i64(
                key_values,
                value_values,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(groups)
        }
        (LaneRef::DecodeBoundary { .. }, _) | (_, LaneRef::DecodeBoundary { .. }) => {
            stats.native_table_decode_boundaries += 1;
            group_aggregate_i64_i64_page_fallback(
                key_page,
                key_payload,
                value_page,
                value_payload,
                base,
                stats,
            )
        }
        _ => group_aggregate_i64_i64_page_fallback(
            key_page,
            key_payload,
            value_page,
            value_payload,
            base,
            stats,
        ),
    }
}

fn native_filecode_i64_group_aggregate_for_pages(
    segment: &TableSegmentPayloadV1,
    key_segment_column: &PreparedSegmentColumn,
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_segment_column: &PreparedSegmentColumn,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeFileCodeI64GroupAggregates, CoveError> {
    let key_ref = NativeTablePageRef {
        column: key_segment_column.directory(),
        page: key_page,
        payload: Some(key_payload),
    };
    let value_ref = NativeTablePageRef {
        column: value_segment_column.directory(),
        page: value_page,
        payload: Some(value_payload),
    };
    let batch = native_table_batch_from_page_refs(
        segment,
        &[key_ref, value_ref],
        NativeCodeDomain::default(),
    )?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let [key_native_page, value_native_page] = batch.column_pages.as_slice() else {
        return Err(CoveError::PageCorrupt);
    };
    match (&key_native_page.lane, &value_native_page.lane) {
        (
            LaneRef::FileCodeU32 {
                values: key_values,
                validity: key_validity,
                logical_type: CoveLogicalType::Utf8,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_u32(
                key_values,
                value_values,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            filecode_u32_i64_groups_to_canonical(dictionary, groups)
        }
        (
            LaneRef::FileCodeU32LeBytes {
                bytes: key_bytes,
                row_count,
                validity: key_validity,
                logical_type: CoveLogicalType::Utf8,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if row_count == value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_u32_le_bytes(
                key_bytes,
                value_bytes,
                *row_count,
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            filecode_u32_i64_groups_to_canonical(dictionary, groups)
        }
        (
            LaneRef::LocalCodeU8 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == *value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_u8_dense(
                key_values,
                value_bytes,
                *value_row_count,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (
            LaneRef::LocalCodeU8 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_u8_dense(
                key_values,
                value_values,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (
            LaneRef::LocalCodeU16 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == *value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_u16_dense(
                key_values,
                value_bytes,
                *value_row_count,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (
            LaneRef::LocalCodeU16 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_u16_dense(
                key_values,
                value_values,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (
            LaneRef::LocalCodeU32 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeU64LeBytes {
                bytes: value_bytes,
                row_count: value_row_count,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == *value_row_count => {
            let (groups, kernel_stats) = aggregate_i64_le_bytes_by_u32_dense(
                key_values,
                value_bytes,
                *value_row_count,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (
            LaneRef::LocalCodeU32 {
                values: key_values,
                validity: key_validity,
                local_to_global,
                logical_type: CoveLogicalType::Utf8,
                physical_kind: CovePhysicalKind::FileCode,
                ..
            },
            LaneRef::NumCodeI64 {
                values: value_values,
                validity: value_validity,
                logical_type: CoveLogicalType::Int64,
                ..
            },
        ) if key_values.len() == value_values.len() => {
            let (groups, kernel_stats) = aggregate_i64_by_u32_dense(
                key_values,
                value_values,
                local_to_global.len(),
                *key_validity,
                *value_validity,
                base,
            )?;
            stats.record_native_aggregate_kernel(kernel_stats);
            local_filecode_i64_groups_to_canonical(dictionary, groups, local_to_global)
        }
        (LaneRef::DecodeBoundary { .. }, _) | (_, LaneRef::DecodeBoundary { .. }) => {
            stats.native_table_decode_boundaries += 1;
            group_aggregate_filecode_i64_page_fallback(
                key_page,
                key_payload,
                value_page,
                value_payload,
                dictionary,
                base,
                stats,
            )
        }
        _ => group_aggregate_filecode_i64_page_fallback(
            key_page,
            key_payload,
            value_page,
            value_payload,
            dictionary,
            base,
            stats,
        ),
    }
}

fn native_filecode_group_count_for_page(
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeFileCodeGroupCounts, CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::FileCodeU32 {
            values,
            validity,
            logical_type: CoveLogicalType::Utf8,
            ..
        } => {
            let (groups, kernel_stats) = group_count_u32_hash(values, *validity, base)?;
            stats.record_native_group_kernel(kernel_stats);
            filecode_u32_groups_to_canonical(dictionary, groups)
        }
        LaneRef::FileCodeU32LeBytes {
            bytes,
            row_count,
            validity,
            logical_type: CoveLogicalType::Utf8,
            ..
        } => {
            let (groups, kernel_stats) =
                group_count_u32_le_bytes(bytes, *row_count, *validity, base)?;
            stats.record_native_group_kernel(kernel_stats);
            filecode_u32_groups_to_canonical(dictionary, groups)
        }
        LaneRef::LocalCodeU8 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => {
            let (groups, kernel_stats) =
                group_count_u8_dense(values, *validity, local_to_global.len(), base)?;
            stats.record_native_group_kernel(kernel_stats);
            local_filecode_groups_to_canonical(dictionary, groups, local_to_global)
        }
        LaneRef::LocalCodeU16 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => {
            let (groups, kernel_stats) =
                group_count_u16_dense(values, *validity, local_to_global.len(), base)?;
            stats.record_native_group_kernel(kernel_stats);
            local_filecode_groups_to_canonical(dictionary, groups, local_to_global)
        }
        LaneRef::LocalCodeU32 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => {
            let (groups, kernel_stats) =
                group_count_u32_dense(values, *validity, local_to_global.len(), base)?;
            stats.record_native_group_kernel(kernel_stats);
            local_filecode_groups_to_canonical(dictionary, groups, local_to_global)
        }
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            group_count_filecode_page_fallback(page, payload, dictionary, base, stats)
        }
        _ => group_count_filecode_page_fallback(page, payload, dictionary, base, stats),
    }
}

fn native_i64_aggregate_for_page(
    state: &DatasetState,
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64Aggregates, CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            logical_type: CoveLogicalType::Int64,
            ..
        } => {
            let (aggregate, kernel_stats) =
                aggregate_i64_le_bytes(bytes, *row_count, *validity, base)?;
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(aggregate)
        }
        LaneRef::NumCodeI64 {
            values,
            validity,
            logical_type: CoveLogicalType::Int64,
            ..
        } => {
            let (aggregate, kernel_stats) = aggregate_i64(values, *validity, base);
            stats.record_native_aggregate_kernel(kernel_stats);
            Ok(aggregate)
        }
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            aggregate_i64_page_fallback(state, page, payload, base, stats)
        }
        _ => aggregate_i64_page_fallback(state, page, payload, base, stats),
    }
}

fn collect_native_i64_values_for_page(
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    out: &mut NativeI64DenseLane,
    stats: &mut DecodeStats,
    kernel_kind: NativeI64KeyKernelKind,
) -> Result<(), CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            logical_type: CoveLogicalType::Int64,
            ..
        } => collect_i64_le_values(bytes, *row_count, *validity, base, out, stats, kernel_kind),
        LaneRef::NumCodeI64 {
            values,
            validity,
            logical_type: CoveLogicalType::Int64,
            ..
        } => collect_i64_values(values, *validity, base, out, stats, kernel_kind),
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            collect_i64_page_fallback(page, payload, base, out, stats, kernel_kind)
        }
        _ => collect_i64_page_fallback(page, payload, base, out, stats, kernel_kind),
    }
}

fn collect_native_filecode_values_for_page(
    segment: &TableSegmentPayloadV1,
    segment_column: &PreparedSegmentColumn,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    out: &mut NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    let page_ref = NativeTablePageRef {
        column: segment_column.directory(),
        page,
        payload: Some(payload),
    };
    let batch =
        native_table_batch_from_page_refs(segment, &[page_ref], NativeCodeDomain::default())?;
    stats.native_table_batches += 1;
    stats.native_table_pages += batch.column_pages.len();
    let Some(native_page) = batch.column_pages.first() else {
        return Err(CoveError::PageCorrupt);
    };
    match &native_page.lane {
        LaneRef::FileCodeU32 {
            values,
            validity,
            logical_type: CoveLogicalType::Utf8,
            ..
        } => collect_filecode_codes(values, *validity, dictionary, base, out, stats),
        LaneRef::FileCodeU32LeBytes {
            bytes,
            row_count,
            validity,
            logical_type: CoveLogicalType::Utf8,
            ..
        } => collect_filecode_le_codes(bytes, *row_count, *validity, dictionary, base, out, stats),
        LaneRef::LocalCodeU8 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => collect_local_filecode_codes(
            values,
            *validity,
            local_to_global,
            dictionary,
            base,
            out,
            stats,
        ),
        LaneRef::LocalCodeU16 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => collect_local_filecode_codes(
            values,
            *validity,
            local_to_global,
            dictionary,
            base,
            out,
            stats,
        ),
        LaneRef::LocalCodeU32 {
            values,
            validity,
            local_to_global,
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            ..
        } => collect_local_filecode_codes(
            values,
            *validity,
            local_to_global,
            dictionary,
            base,
            out,
            stats,
        ),
        LaneRef::DecodeBoundary { .. } => {
            stats.native_table_decode_boundaries += 1;
            collect_filecode_page_fallback(page, payload, dictionary, base, out, stats)
        }
        _ => collect_filecode_page_fallback(page, payload, dictionary, base, out, stats),
    }
}

fn collect_i64_le_values(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionMask>,
    out: &mut NativeI64DenseLane,
    stats: &mut DecodeStats,
    kernel_kind: NativeI64KeyKernelKind,
) -> Result<(), CoveError> {
    let expected_len = row_count
        .checked_mul(std::mem::size_of::<u64>())
        .ok_or(CoveError::ArithOverflow)?;
    if bytes.len() != expected_len || validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut kernel_stats = KernelStats {
        rows_seen: row_count,
        bytes_touched_estimate: expected_len,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let value = if validity.is_valid(row) {
            kernel_stats.rows_valid += 1;
            let offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            Some(i64::from_le_bytes(
                bytes[offset..offset + std::mem::size_of::<u64>()]
                    .try_into()
                    .unwrap(),
            ))
        } else {
            None
        };
        out.push(value)?;
        kernel_stats.rows_matched += 1;
    }
    record_native_i64_key_kernel(stats, kernel_kind, kernel_stats);
    Ok(())
}

fn collect_i64_values(
    values: &[i64],
    validity: ValidityRef<'_>,
    base: Option<&SelectionMask>,
    out: &mut NativeI64DenseLane,
    stats: &mut DecodeStats,
    kernel_kind: NativeI64KeyKernelKind,
) -> Result<(), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut kernel_stats = KernelStats {
        rows_seen: values.len(),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let value = if validity.is_valid(row) {
            kernel_stats.rows_valid += 1;
            Some(value)
        } else {
            None
        };
        out.push(value)?;
        kernel_stats.rows_matched += 1;
    }
    record_native_i64_key_kernel(stats, kernel_kind, kernel_stats);
    Ok(())
}

fn collect_filecode_codes(
    values: &[u32],
    validity: ValidityRef<'_>,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    out: &mut NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut code_cache = filecode_decode_cache(dictionary)?;
    let mut kernel_stats = KernelStats {
        rows_seen: values.len(),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, file_code) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            kernel_stats.rows_valid += 1;
            let canonical = cached_filecode_canonical_key(dictionary, &mut code_cache, file_code)?;
            out.push(Some(canonical))?;
        } else {
            out.push(None)?;
        }
        kernel_stats.rows_matched += 1;
    }
    stats.record_native_join_kernel(kernel_stats);
    Ok(())
}

fn collect_filecode_le_codes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    out: &mut NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    if bytes.len() != row_count * std::mem::size_of::<u32>() || validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut code_cache = filecode_decode_cache(dictionary)?;
    let mut kernel_stats = KernelStats {
        rows_seen: row_count,
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            let offset = row
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(CoveError::ArithOverflow)?;
            let file_code = u32::from_le_bytes(
                bytes[offset..offset + std::mem::size_of::<u32>()]
                    .try_into()
                    .unwrap(),
            );
            kernel_stats.rows_valid += 1;
            let canonical = cached_filecode_canonical_key(dictionary, &mut code_cache, file_code)?;
            out.push(Some(canonical))?;
        } else {
            out.push(None)?;
        }
        kernel_stats.rows_matched += 1;
    }
    stats.record_native_join_kernel(kernel_stats);
    Ok(())
}

fn collect_local_filecode_codes<K>(
    values: &[K],
    validity: ValidityRef<'_>,
    local_to_global: &[u64],
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    out: &mut NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<(), CoveError>
where
    K: Copy + TryInto<usize>,
{
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut code_cache = filecode_decode_cache(dictionary)?;
    let mut kernel_stats = KernelStats {
        rows_seen: values.len(),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local_code) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            let local_code = local_code
                .try_into()
                .map_err(|_| CoveError::ArithOverflow)?;
            let global_code = *local_to_global
                .get(local_code)
                .ok_or(CoveError::PageCorrupt)?;
            let file_code = u32::try_from(global_code).map_err(|_| CoveError::BadDomain)?;
            kernel_stats.rows_valid += 1;
            let canonical = cached_filecode_canonical_key(dictionary, &mut code_cache, file_code)?;
            out.push(Some(canonical))?;
        } else {
            out.push(None)?;
        }
        kernel_stats.rows_matched += 1;
    }
    stats.record_native_join_kernel(kernel_stats);
    Ok(())
}

fn filecode_decode_cache(dictionary: &FileDictionary) -> Result<Vec<Option<Vec<u8>>>, CoveError> {
    let len = usize::try_from(dictionary.len()).map_err(|_| CoveError::ArithOverflow)?;
    Ok(vec![None; len])
}

fn cached_filecode_canonical_key(
    dictionary: &FileDictionary,
    cache: &mut [Option<Vec<u8>>],
    file_code: u32,
) -> Result<Vec<u8>, CoveError> {
    let index = usize::try_from(file_code).map_err(|_| CoveError::ArithOverflow)?;
    let slot = cache.get_mut(index).ok_or(CoveError::BadFileCode)?;
    if let Some(value) = slot {
        return Ok(value.clone());
    }
    let value = match dictionary.decode_value(file_code)? {
        DictionaryValue::RawBytes(bytes) => bytes,
        DictionaryValue::RedactedPresent => {
            return Err(CoveError::UnsupportedEncoding(
                "native FileCode value scan rejects redacted dictionary values".into(),
            ));
        }
        _ => {
            return Err(CoveError::UnsupportedEncoding(
                "native FileCode value scan decoded unsupported future dictionary value".into(),
            ));
        }
    };
    *slot = Some(value.clone());
    Ok(value)
}

fn group_count_i64_page_fallback(
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64HashGroupCounts, CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, None)?;
    let prepared = array.prepare()?;
    let mut groups = NativeI64HashGroupCounts::default();
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        match prepared.decode_row(u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?)? {
            CoveArrayValue::Null => {
                groups.null_count = groups
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::NumCode(code) => {
                merge_i64_group_value(&mut groups, code as i64)?;
            }
            CoveArrayValue::Int64(value) => {
                merge_i64_group_value(&mut groups, value)?;
            }
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native i64 group fallback decoded non-i64 value {other:?}"
                )));
            }
        }
    }
    Ok(groups)
}

fn group_count_bool_page_fallback(
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeDenseGroupCounts, CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, None)?;
    let prepared = array.prepare()?;
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; 2],
        ..NativeDenseGroupCounts::default()
    };
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        match prepared.decode_row(u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?)? {
            CoveArrayValue::Null => {
                groups.null_count = groups
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::Boolean(value) => {
                merge_bool_group_value(&mut groups, value)?;
            }
            CoveArrayValue::Bytes(bytes) => match bytes {
                [0] => merge_bool_group_value(&mut groups, false)?,
                [1] => merge_bool_group_value(&mut groups, true)?,
                _ => {
                    return Err(CoveError::UnsupportedEncoding(format!(
                        "native bool group fallback decoded invalid bool bytes {bytes:?}"
                    )));
                }
            },
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native bool group fallback decoded non-bool value {other:?}"
                )));
            }
        }
    }
    Ok(groups)
}

fn group_aggregate_i64_i64_page_fallback(
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64I64HashGroupAggregates, CoveError> {
    stats.kernel_fallbacks += 1;
    let key_array = encoded_array_for_page(key_payload, key_page, None)?;
    let value_array = encoded_array_for_page(value_payload, value_page, None)?;
    if key_array.row_count != value_array.row_count {
        return Err(CoveError::PageCorrupt);
    }
    let key_prepared = key_array.prepare()?;
    let value_prepared = value_array.prepare()?;
    let mut groups = NativeI64I64HashGroupAggregates::default();
    let row_count = usize::try_from(key_array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let key = match key_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::NumCode(code) => Some(code as i64),
            CoveArrayValue::Int64(value) => Some(value),
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native i64/i64 group aggregate fallback decoded non-i64 key {other:?}"
                )));
            }
        };
        let value = match value_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::NumCode(code) => Some(code as i64),
            CoveArrayValue::Int64(value) => Some(value),
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native i64/i64 group aggregate fallback decoded non-i64 value {other:?}"
                )));
            }
        };
        merge_i64_i64_group_row(&mut groups, key, value)?;
    }
    Ok(groups)
}

fn group_aggregate_bool_i64_page_fallback(
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeDenseI64GroupAggregates, CoveError> {
    stats.kernel_fallbacks += 1;
    let key_array = encoded_array_for_page(key_payload, key_page, None)?;
    let value_array = encoded_array_for_page(value_payload, value_page, None)?;
    if key_array.row_count != value_array.row_count {
        return Err(CoveError::PageCorrupt);
    }
    let key_prepared = key_array.prepare()?;
    let value_prepared = value_array.prepare()?;
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); 2],
        row_counts: vec![0; 2],
        ..NativeDenseI64GroupAggregates::default()
    };
    let row_count = usize::try_from(key_array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let key = match key_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::Boolean(value) => Some(value),
            CoveArrayValue::Bytes(bytes) => match bytes {
                [0] => Some(false),
                [1] => Some(true),
                _ => {
                    return Err(CoveError::UnsupportedEncoding(format!(
                        "native bool/i64 group aggregate fallback decoded invalid bool key bytes {bytes:?}"
                    )));
                }
            },
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native bool/i64 group aggregate fallback decoded non-bool key {other:?}"
                )));
            }
        };
        let value = match value_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::NumCode(code) => Some(code as i64),
            CoveArrayValue::Int64(value) => Some(value),
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native bool/i64 group aggregate fallback decoded non-i64 value {other:?}"
                )));
            }
        };
        merge_bool_i64_group_row(&mut groups, key, value)?;
    }
    Ok(groups)
}

fn group_aggregate_filecode_i64_page_fallback(
    key_page: &ColumnPageIndexEntryV1,
    key_payload: &RetainedColumnPagePayloadV1,
    value_page: &ColumnPageIndexEntryV1,
    value_payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeFileCodeI64GroupAggregates, CoveError> {
    stats.kernel_fallbacks += 1;
    let key_array = encoded_array_for_page(key_payload, key_page, Some(dictionary))?;
    let value_array = encoded_array_for_page(value_payload, value_page, None)?;
    if key_array.row_count != value_array.row_count {
        return Err(CoveError::PageCorrupt);
    }
    let key_prepared = key_array.prepare()?;
    let value_prepared = value_array.prepare()?;
    let mut groups = NativeFileCodeI64GroupAggregates::default();
    let row_count = usize::try_from(key_array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let key = match key_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::FileCode(code) => Some(filecode_group_canonical_key(dictionary, code)?),
            CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => Some(bytes),
            CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
                return Err(CoveError::UnsupportedEncoding(
                    "native FileCode/i64 group aggregate fallback rejects redacted dictionary values"
                        .into(),
                ));
            }
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native FileCode/i64 group aggregate fallback decoded non-FileCode key {other:?}"
                )));
            }
        };
        let value = match value_prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => None,
            CoveArrayValue::NumCode(code) => Some(code as i64),
            CoveArrayValue::Int64(value) => Some(value),
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native FileCode/i64 group aggregate fallback decoded non-i64 value {other:?}"
                )));
            }
        };
        merge_filecode_i64_group_row(&mut groups, key, value)?;
    }
    Ok(groups)
}

fn group_count_filecode_page_fallback(
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeFileCodeGroupCounts, CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, Some(dictionary))?;
    let prepared = array.prepare()?;
    let mut groups = NativeFileCodeGroupCounts::default();
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        match prepared.decode_row(u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?)? {
            CoveArrayValue::Null => {
                groups.null_count = groups
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::FileCode(code) => {
                merge_filecode_group_code(&mut groups, dictionary, code, 1)?;
                groups.rows_grouped = groups
                    .rows_grouped
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
                merge_filecode_group_canonical(&mut groups, bytes, 1)?;
                groups.rows_grouped = groups
                    .rows_grouped
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
                return Err(CoveError::UnsupportedEncoding(
                    "native FileCode group fallback rejects redacted dictionary values".into(),
                ));
            }
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native FileCode group fallback decoded non-FileCode value {other:?}"
                )));
            }
        }
    }
    Ok(groups)
}

fn aggregate_i64_page_fallback(
    state: &DatasetState,
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    stats: &mut DecodeStats,
) -> Result<NativeI64Aggregates, CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, state.mounted().dictionary.as_ref())?;
    let prepared = array.prepare()?;
    let mut aggregate = NativeI64Aggregates::default();
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        match prepared.decode_row(u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?)? {
            CoveArrayValue::Null => {
                aggregate.null_count = aggregate
                    .null_count
                    .checked_add(1)
                    .ok_or(CoveError::ArithOverflow)?;
            }
            CoveArrayValue::NumCode(code) => {
                merge_i64_value(&mut aggregate, code as i64)?;
            }
            CoveArrayValue::Int64(value) => {
                merge_i64_value(&mut aggregate, value)?;
            }
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native i64 aggregate fallback decoded non-i64 value {other:?}"
                )));
            }
        }
    }
    Ok(aggregate)
}

fn collect_i64_page_fallback(
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    base: Option<&SelectionMask>,
    out: &mut NativeI64DenseLane,
    stats: &mut DecodeStats,
    kernel_kind: NativeI64KeyKernelKind,
) -> Result<(), CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, None)?;
    let prepared = array.prepare()?;
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    let mut kernel_stats = KernelStats {
        rows_seen: row_count,
        bytes_touched_estimate: usize::try_from(page.page_length)
            .map_err(|_| CoveError::OffsetRange)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let value =
            match prepared.decode_row(u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?)? {
                CoveArrayValue::Null => None,
                CoveArrayValue::NumCode(code) => {
                    kernel_stats.rows_valid += 1;
                    Some(code as i64)
                }
                CoveArrayValue::Int64(value) => {
                    kernel_stats.rows_valid += 1;
                    Some(value)
                }
                other => {
                    return Err(CoveError::UnsupportedEncoding(format!(
                        "native i64 order fallback decoded non-i64 value {other:?}"
                    )));
                }
            };
        out.push(value)?;
        kernel_stats.rows_matched += 1;
    }
    record_native_i64_key_kernel(stats, kernel_kind, kernel_stats);
    Ok(())
}

fn collect_filecode_page_fallback(
    page: &ColumnPageIndexEntryV1,
    payload: &RetainedColumnPagePayloadV1,
    dictionary: &FileDictionary,
    base: Option<&SelectionMask>,
    out: &mut NativeFileCodeDenseLane,
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    stats.kernel_fallbacks += 1;
    let array = encoded_array_for_page(payload, page, Some(dictionary))?;
    let prepared = array.prepare()?;
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::OffsetRange)?;
    let mut code_cache = filecode_decode_cache(dictionary)?;
    let mut kernel_stats = KernelStats {
        rows_seen: row_count,
        bytes_touched_estimate: usize::try_from(page.page_length)
            .map_err(|_| CoveError::OffsetRange)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        match prepared.decode_row(row_u64)? {
            CoveArrayValue::Null => out.push(None)?,
            CoveArrayValue::FileCode(code) => {
                kernel_stats.rows_valid += 1;
                let canonical = cached_filecode_canonical_key(dictionary, &mut code_cache, code)?;
                out.push(Some(canonical))?;
            }
            CoveArrayValue::DictValue(DictionaryValue::RawBytes(bytes)) => {
                kernel_stats.rows_valid += 1;
                out.push(Some(bytes))?;
            }
            CoveArrayValue::DictValue(DictionaryValue::RedactedPresent) => {
                return Err(CoveError::UnsupportedEncoding(
                    "native FileCode value fallback rejects redacted dictionary values".into(),
                ));
            }
            other => {
                return Err(CoveError::UnsupportedEncoding(format!(
                    "native FileCode value fallback decoded non-FileCode value {other:?}"
                )));
            }
        }
        kernel_stats.rows_matched += 1;
    }
    stats.record_native_join_kernel(kernel_stats);
    Ok(())
}

fn record_native_i64_key_kernel(
    stats: &mut DecodeStats,
    kernel_kind: NativeI64KeyKernelKind,
    kernel_stats: KernelStats,
) {
    match kernel_kind {
        NativeI64KeyKernelKind::SortInput => {
            let _ = kernel_stats;
        }
        NativeI64KeyKernelKind::Join => stats.record_native_join_kernel(kernel_stats),
    }
}

fn merge_i64_value(aggregate: &mut NativeI64Aggregates, value: i64) -> Result<(), CoveError> {
    aggregate.count = aggregate
        .count
        .checked_add(1)
        .ok_or(CoveError::ArithOverflow)?;
    aggregate.sum = aggregate
        .sum
        .checked_add(i128::from(value))
        .ok_or(CoveError::ArithOverflow)?;
    aggregate.min = Some(aggregate.min.map_or(value, |min| min.min(value)));
    aggregate.max = Some(aggregate.max.map_or(value, |max| max.max(value)));
    Ok(())
}

fn merge_i64_group_value(
    groups: &mut NativeI64HashGroupCounts,
    value: i64,
) -> Result<(), CoveError> {
    let count = groups.counts.entry(value).or_default();
    *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
    groups.rows_grouped = groups
        .rows_grouped
        .checked_add(1)
        .ok_or(CoveError::ArithOverflow)?;
    Ok(())
}

fn merge_i64_group_counts(
    target: &mut NativeI64HashGroupCounts,
    source: &NativeI64HashGroupCounts,
) -> Result<(), CoveError> {
    target.null_count = target
        .null_count
        .checked_add(source.null_count)
        .ok_or(CoveError::ArithOverflow)?;
    target.rows_grouped = target
        .rows_grouped
        .checked_add(source.rows_grouped)
        .ok_or(CoveError::ArithOverflow)?;
    for (value, count) in &source.counts {
        let slot = target.counts.entry(*value).or_default();
        *slot = slot.checked_add(*count).ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_i64_i64_group_row(
    groups: &mut NativeI64I64HashGroupAggregates,
    key: Option<i64>,
    value: Option<i64>,
) -> Result<(), CoveError> {
    let aggregate = if let Some(key) = key {
        let row_count = groups.row_counts.entry(key).or_default();
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.aggregates.entry(key).or_default()
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        &mut groups.null_aggregate
    };
    if let Some(value) = value {
        merge_i64_value(aggregate, value)?;
    } else {
        aggregate.null_count = aggregate
            .null_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_i64_i64_group_aggregate(
    groups: &mut NativeI64I64HashGroupAggregates,
    key: i64,
    row_count: u64,
    aggregate: &NativeI64Aggregates,
) -> Result<(), CoveError> {
    if row_count == 0 {
        return Ok(());
    }
    let target_row_count = groups.row_counts.entry(key).or_default();
    *target_row_count = target_row_count
        .checked_add(row_count)
        .ok_or(CoveError::ArithOverflow)?;
    merge_i64_aggregate_state(groups.aggregates.entry(key).or_default(), aggregate)
}

fn merge_i64_i64_group_aggregates(
    target: &mut NativeI64I64HashGroupAggregates,
    source: &NativeI64I64HashGroupAggregates,
) -> Result<(), CoveError> {
    target.null_row_count = target
        .null_row_count
        .checked_add(source.null_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    merge_i64_aggregate_state(&mut target.null_aggregate, &source.null_aggregate)?;
    for (key, aggregate) in &source.aggregates {
        let row_count = source.row_counts.get(key).copied().unwrap_or(0);
        merge_i64_i64_group_aggregate(target, *key, row_count, aggregate)?;
    }
    Ok(())
}

fn merge_bool_group_value(
    groups: &mut NativeDenseGroupCounts,
    value: bool,
) -> Result<(), CoveError> {
    if groups.counts.len() != 2 {
        return Err(CoveError::PageCorrupt);
    }
    let slot = &mut groups.counts[usize::from(value)];
    *slot = slot.checked_add(1).ok_or(CoveError::ArithOverflow)?;
    groups.rows_grouped = groups
        .rows_grouped
        .checked_add(1)
        .ok_or(CoveError::ArithOverflow)?;
    Ok(())
}

fn merge_bool_group_counts(
    target: &mut NativeDenseGroupCounts,
    source: &NativeDenseGroupCounts,
) -> Result<(), CoveError> {
    if target.counts.is_empty() {
        target.counts.resize(2, 0);
    }
    if target.counts.len() != 2 || source.counts.len() != 2 {
        return Err(CoveError::PageCorrupt);
    }
    target.null_count = target
        .null_count
        .checked_add(source.null_count)
        .ok_or(CoveError::ArithOverflow)?;
    target.rows_grouped = target
        .rows_grouped
        .checked_add(source.rows_grouped)
        .ok_or(CoveError::ArithOverflow)?;
    for (target_count, source_count) in target.counts.iter_mut().zip(source.counts.iter()) {
        *target_count = target_count
            .checked_add(*source_count)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_bool_i64_group_row(
    groups: &mut NativeDenseI64GroupAggregates,
    key: Option<bool>,
    value: Option<i64>,
) -> Result<(), CoveError> {
    if groups.aggregates.is_empty() {
        groups.aggregates.resize(2, NativeI64Aggregates::default());
    }
    if groups.row_counts.is_empty() {
        groups.row_counts.resize(2, 0);
    }
    if groups.aggregates.len() != 2 || groups.row_counts.len() != 2 {
        return Err(CoveError::PageCorrupt);
    }
    let aggregate = if let Some(key) = key {
        let group = usize::from(key);
        groups.row_counts[group] = groups.row_counts[group]
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        &mut groups.aggregates[group]
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        &mut groups.null_aggregate
    };
    if let Some(value) = value {
        merge_i64_value(aggregate, value)?;
    } else {
        aggregate.null_count = aggregate
            .null_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_bool_i64_group_aggregates(
    target: &mut NativeDenseI64GroupAggregates,
    source: &NativeDenseI64GroupAggregates,
) -> Result<(), CoveError> {
    if target.aggregates.is_empty() {
        target.aggregates.resize(2, NativeI64Aggregates::default());
    }
    if target.row_counts.is_empty() {
        target.row_counts.resize(2, 0);
    }
    if target.aggregates.len() != 2
        || target.row_counts.len() != 2
        || source.aggregates.len() != 2
        || source.row_counts.len() != 2
    {
        return Err(CoveError::PageCorrupt);
    }
    for group in 0..2 {
        target.row_counts[group] = target.row_counts[group]
            .checked_add(source.row_counts[group])
            .ok_or(CoveError::ArithOverflow)?;
        merge_i64_aggregate_state(&mut target.aggregates[group], &source.aggregates[group])?;
    }
    target.null_row_count = target
        .null_row_count
        .checked_add(source.null_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    merge_i64_aggregate_state(&mut target.null_aggregate, &source.null_aggregate)?;
    Ok(())
}

fn merge_i64_aggregate_state(
    target: &mut NativeI64Aggregates,
    source: &NativeI64Aggregates,
) -> Result<(), CoveError> {
    target.count = target
        .count
        .checked_add(source.count)
        .ok_or(CoveError::ArithOverflow)?;
    target.null_count = target
        .null_count
        .checked_add(source.null_count)
        .ok_or(CoveError::ArithOverflow)?;
    target.sum = target
        .sum
        .checked_add(source.sum)
        .ok_or(CoveError::ArithOverflow)?;
    if let Some(source_min) = source.min {
        target.min = Some(
            target
                .min
                .map_or(source_min, |target_min| target_min.min(source_min)),
        );
    }
    if let Some(source_max) = source.max {
        target.max = Some(
            target
                .max
                .map_or(source_max, |target_max| target_max.max(source_max)),
        );
    }
    Ok(())
}

fn filecode_u32_i64_groups_to_canonical(
    dictionary: &FileDictionary,
    groups: NativeU32I64HashGroupAggregates,
) -> Result<NativeFileCodeI64GroupAggregates, CoveError> {
    let mut out = NativeFileCodeI64GroupAggregates {
        null_row_count: groups.null_row_count,
        null_aggregate: groups.null_aggregate,
        ..NativeFileCodeI64GroupAggregates::default()
    };
    for (file_code, aggregate) in groups.aggregates {
        let row_count = groups.row_counts.get(&file_code).copied().unwrap_or(0);
        let canonical_key = filecode_group_canonical_key(dictionary, file_code)?;
        merge_filecode_i64_group_aggregate(&mut out, canonical_key, row_count, &aggregate)?;
    }
    Ok(out)
}

fn local_filecode_i64_groups_to_canonical(
    dictionary: &FileDictionary,
    groups: NativeDenseI64GroupAggregates,
    local_to_global: &[u64],
) -> Result<NativeFileCodeI64GroupAggregates, CoveError> {
    if groups.aggregates.len() != local_to_global.len()
        || groups.row_counts.len() != local_to_global.len()
    {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = NativeFileCodeI64GroupAggregates {
        null_row_count: groups.null_row_count,
        null_aggregate: groups.null_aggregate,
        ..NativeFileCodeI64GroupAggregates::default()
    };
    for (local_code, aggregate) in groups.aggregates.into_iter().enumerate() {
        let row_count = groups.row_counts.get(local_code).copied().unwrap_or(0);
        if row_count == 0 {
            continue;
        }
        let file_code =
            u32::try_from(local_to_global[local_code]).map_err(|_| CoveError::BadDomain)?;
        let canonical_key = filecode_group_canonical_key(dictionary, file_code)?;
        merge_filecode_i64_group_aggregate(&mut out, canonical_key, row_count, &aggregate)?;
    }
    Ok(out)
}

fn filecode_group_canonical_key(
    dictionary: &FileDictionary,
    file_code: u32,
) -> Result<Vec<u8>, CoveError> {
    match dictionary.decode_value(file_code)? {
        DictionaryValue::RawBytes(bytes) => Ok(bytes),
        DictionaryValue::RedactedPresent => Err(CoveError::UnsupportedEncoding(
            "native FileCode/i64 group aggregate rejects redacted dictionary values".into(),
        )),
        _ => Err(CoveError::UnsupportedEncoding(
            "native FileCode/i64 group aggregate decoded unsupported future dictionary value"
                .into(),
        )),
    }
}

fn merge_filecode_i64_group_row(
    groups: &mut NativeFileCodeI64GroupAggregates,
    key: Option<Vec<u8>>,
    value: Option<i64>,
) -> Result<(), CoveError> {
    let aggregate = if let Some(key) = key {
        let slot = groups.groups.entry(key).or_default();
        slot.row_count = slot
            .row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        &mut slot.aggregate
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        &mut groups.null_aggregate
    };
    if let Some(value) = value {
        merge_i64_value(aggregate, value)?;
    } else {
        aggregate.null_count = aggregate
            .null_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_filecode_i64_group_aggregate(
    groups: &mut NativeFileCodeI64GroupAggregates,
    canonical_key: Vec<u8>,
    row_count: u64,
    aggregate: &NativeI64Aggregates,
) -> Result<(), CoveError> {
    if row_count == 0 {
        return Ok(());
    }
    let slot = groups.groups.entry(canonical_key).or_default();
    slot.row_count = slot
        .row_count
        .checked_add(row_count)
        .ok_or(CoveError::ArithOverflow)?;
    merge_i64_aggregate_state(&mut slot.aggregate, aggregate)
}

fn merge_filecode_i64_group_aggregates(
    target: &mut NativeFileCodeI64GroupAggregates,
    source: &NativeFileCodeI64GroupAggregates,
) -> Result<(), CoveError> {
    target.null_row_count = target
        .null_row_count
        .checked_add(source.null_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    merge_i64_aggregate_state(&mut target.null_aggregate, &source.null_aggregate)?;
    for (canonical_key, source_group) in &source.groups {
        merge_filecode_i64_group_aggregate(
            target,
            canonical_key.clone(),
            source_group.row_count,
            &source_group.aggregate,
        )?;
    }
    Ok(())
}

fn filecode_u32_groups_to_canonical(
    dictionary: &FileDictionary,
    groups: NativeHashGroupCounts,
) -> Result<NativeFileCodeGroupCounts, CoveError> {
    let mut out = NativeFileCodeGroupCounts {
        null_count: groups.null_count,
        rows_grouped: groups.rows_grouped,
        ..NativeFileCodeGroupCounts::default()
    };
    for (file_code, count) in groups.counts {
        merge_filecode_group_code(&mut out, dictionary, file_code, count)?;
    }
    Ok(out)
}

fn local_filecode_groups_to_canonical(
    dictionary: &FileDictionary,
    groups: NativeDenseGroupCounts,
    local_to_global: &[u64],
) -> Result<NativeFileCodeGroupCounts, CoveError> {
    if groups.counts.len() != local_to_global.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = NativeFileCodeGroupCounts {
        null_count: groups.null_count,
        rows_grouped: groups.rows_grouped,
        ..NativeFileCodeGroupCounts::default()
    };
    for (local_code, count) in groups.counts.into_iter().enumerate() {
        if count == 0 {
            continue;
        }
        let file_code =
            u32::try_from(local_to_global[local_code]).map_err(|_| CoveError::BadDomain)?;
        merge_filecode_group_code(&mut out, dictionary, file_code, count)?;
    }
    Ok(out)
}

fn merge_filecode_group_code(
    groups: &mut NativeFileCodeGroupCounts,
    dictionary: &FileDictionary,
    file_code: u32,
    count: u64,
) -> Result<(), CoveError> {
    match dictionary.decode_value(file_code)? {
        DictionaryValue::RawBytes(bytes) => merge_filecode_group_canonical(groups, bytes, count),
        DictionaryValue::RedactedPresent => Err(CoveError::UnsupportedEncoding(
            "native FileCode group count rejects redacted dictionary values".into(),
        )),
        _ => Err(CoveError::UnsupportedEncoding(
            "native FileCode group count decoded unsupported future dictionary value".into(),
        )),
    }
}

fn merge_filecode_group_canonical(
    groups: &mut NativeFileCodeGroupCounts,
    canonical_value: Vec<u8>,
    count: u64,
) -> Result<(), CoveError> {
    if count == 0 {
        return Ok(());
    }
    let slot = groups.counts.entry(canonical_value).or_default();
    *slot = slot.checked_add(count).ok_or(CoveError::ArithOverflow)?;
    Ok(())
}

fn merge_filecode_group_counts(
    target: &mut NativeFileCodeGroupCounts,
    source: &NativeFileCodeGroupCounts,
) -> Result<(), CoveError> {
    target.null_count = target
        .null_count
        .checked_add(source.null_count)
        .ok_or(CoveError::ArithOverflow)?;
    target.rows_grouped = target
        .rows_grouped
        .checked_add(source.rows_grouped)
        .ok_or(CoveError::ArithOverflow)?;
    for (canonical_value, count) in &source.counts {
        let slot = target.counts.entry(canonical_value.clone()).or_default();
        *slot = slot.checked_add(*count).ok_or(CoveError::ArithOverflow)?;
    }
    Ok(())
}

fn merge_i64_aggregates(
    target: &mut NativeI64Aggregates,
    source: &NativeI64Aggregates,
) -> Result<(), CoveError> {
    target.count = target
        .count
        .checked_add(source.count)
        .ok_or(CoveError::ArithOverflow)?;
    target.null_count = target
        .null_count
        .checked_add(source.null_count)
        .ok_or(CoveError::ArithOverflow)?;
    target.sum = target
        .sum
        .checked_add(source.sum)
        .ok_or(CoveError::ArithOverflow)?;
    if let Some(min) = source.min {
        target.min = Some(target.min.map_or(min, |current| current.min(min)));
    }
    if let Some(max) = source.max {
        target.max = Some(target.max.map_or(max, |current| current.max(max)));
    }
    Ok(())
}

pub(crate) fn decode_scan_to_sink<S: DecodeSink + ?Sized>(
    state: &DatasetState,
    plan: &ScanPlan,
    sink: &mut S,
) -> Result<DecodeStats, CoveError> {
    let plan = state.resolved_plan_for_current_state(plan)?;
    validate_scan_plan(state, &plan)?;
    let mut stats = DecodeStats::default();
    record_plan_predicates(&plan, &mut stats);
    let mut scratch = DecodeScratch::default();
    if sink.should_stop() {
        return Ok(stats);
    }

    for segment_ref in state.segments() {
        let segment_bytes = wire::read_range_checked(
            state.full_file_bytes()?,
            usize::try_from(segment_ref.offset).map_err(|_| CoveError::OffsetRange)?,
            usize::try_from(segment_ref.length).map_err(|_| CoveError::OffsetRange)?,
        )?;
        let segment = TableSegmentPayloadV1::parse_with_required_features(
            segment_bytes,
            state.mounted().header.required_features,
        )?;
        let prepared_segment = prepare_segment_payload(segment_bytes, &segment)?;

        for morsel in ordered_morsels(
            state,
            segment.header.segment_id,
            prepared_segment.morsel_entries(),
            &plan,
        ) {
            stats.morsels_considered += 1;
            let row_start = segment
                .header
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                stats.morsels_pruned += 1;
                stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                &plan,
                segment.header.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                stats.morsels_pruned += 1;
                stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment.header.segment_id, morsel.morsel_id, &plan)?
                || should_prune_morsel(
                    state,
                    segment_ref,
                    &prepared_segment,
                    morsel.morsel_id,
                    &plan,
                    &mut stats,
                )?
            {
                stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel(
                state,
                segment_bytes,
                &prepared_segment,
                segment_ref,
                segment.header.segment_id,
                morsel.morsel_id,
                &plan,
                &mut stats,
                &mut scratch,
            )?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut stats,
            )?;
            scratch.selection.record(&mut stats);
            if scratch.selection.is_empty() {
                stats.rows_selected += 0;
                continue;
            }
            let selected_len = scratch.selection.len();
            stats.rows_selected += selected_len;
            if plan_has_residual(&plan) {
                stats.residual_rows += selected_len;
            }
            record_late_materialization(
                &plan,
                morsel.row_count as usize,
                selected_len,
                &mut stats,
            )?;

            if plan.scan_projection.is_empty() {
                let options = RecordBatchOptions::new().with_row_count(Some(selected_len));
                let batch = RecordBatch::try_new_with_options(
                    plan.output_schema.clone(),
                    Vec::new(),
                    &options,
                )
                .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch: {err}")))?;
                if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                    return Ok(stats);
                }
                continue;
            }

            let mut page_payloads = Vec::with_capacity(plan.scan_projection.len());
            let mut page_indexes = Vec::with_capacity(plan.scan_projection.len());
            let mut page_refs = Vec::with_capacity(plan.scan_projection.len());
            let mut columns = Vec::with_capacity(plan.scan_projection.len());
            let mut segment_columns = Vec::with_capacity(plan.scan_projection.len());
            for projection_index in &plan.scan_projection {
                let column = &state.table().columns[*projection_index];
                let segment_column = prepared_segment.column(column.column_id)?;
                let page = prepared_segment.page_for_morsel(segment_column, morsel.morsel_id)?;
                let page_ref =
                    prepared_segment.page_ref_for_morsel(segment_column, morsel.morsel_id)?;
                state.reject_table_scan_page_feature_use(segment_ref, page)?;
                let payload = materialize_page_payload(
                    segment_bytes,
                    segment_ref.table_id,
                    segment_ref.segment_id,
                    column,
                    page,
                    state.pruning().zone_stats_entries.as_slice(),
                    state.pruning().codec_descriptors.as_slice(),
                    state
                        .mounted()
                        .dictionary
                        .as_ref()
                        .map(|dictionary| dictionary.len()),
                    state.page_payload_validation_policy(),
                )?;
                stats.pages_decoded += usize::from(page.page_length != 0);
                stats.data_bytes_read = stats
                    .data_bytes_read
                    .checked_add(
                        usize::try_from(page.page_length).map_err(|_| CoveError::OffsetRange)?,
                    )
                    .ok_or(CoveError::ArithOverflow)?;
                page_payloads.push(payload);
                page_indexes.push(page.clone());
                page_refs.push(page_ref);
                columns.push(column);
                segment_columns.push(segment_column);
            }

            record_native_projection_batch(
                &segment_columns,
                &page_indexes,
                &page_payloads,
                &mut stats,
            )?;
            let mut encoded_columns = Vec::with_capacity(columns.len());
            for ((column, page), payload) in columns
                .iter()
                .zip(page_indexes.iter())
                .zip(page_payloads.iter())
            {
                encoded_columns.push((
                    column.name.as_str(),
                    encoded_array_for_page(payload, page, state.mounted().dictionary.as_ref())?,
                ));
            }
            let arrow_options = state.arrow_export_options();
            let column_refs = arrow_encoded_columns_for_payloads(
                state,
                segment.header.segment_id,
                &columns,
                &encoded_columns,
                &page_indexes,
                &page_refs,
                &page_payloads,
                arrow_options,
            );
            let batch = record_batch_for_selection(
                state,
                &column_refs,
                &scratch.selection,
                plan.output_schema.clone(),
                arrow_options,
                None,
                &mut stats,
            )?
            .value;
            if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                return Ok(stats);
            }
        }
    }

    Ok(stats)
}

fn plan_selects_no_rows(plan: &ScanPlan) -> bool {
    if plan
        .covi_candidates
        .as_ref()
        .map(Vec::is_empty)
        .unwrap_or(false)
    {
        return true;
    }
    plan.filters.iter().any(|filter| {
        matches!(
            filter.predicate,
            Some(CovePredicate::FileCodeIn { ref file_codes, .. }) if file_codes.is_empty()
        ) || matches!(
            filter.predicate,
            Some(CovePredicate::NumericIn { ref literals, .. }) if literals.is_empty()
        )
    })
}

fn record_plan_predicates(plan: &ScanPlan, stats: &mut DecodeStats) {
    stats.exact_predicates += plan.scan_program.exact_filters;
    stats.residual_predicates += plan.scan_program.inexact_filters;
    stats.predicate_orderings += usize::from(plan.scan_program.predicate_ordered);
}

fn record_range_plan(plan: RangeReadPlan, stats: &mut DecodeStats) {
    match plan.mode {
        RangeReadMode::Sparse => stats.range_plan_sparse += 1,
        RangeReadMode::Mixed => stats.range_plan_mixed += 1,
        RangeReadMode::Dense => stats.range_plan_dense += 1,
    }
}

fn record_late_materialization(
    plan: &ScanPlan,
    row_count: usize,
    selected_len: usize,
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    if plan.scan_projection.is_empty()
        || selected_len >= row_count
        || !plan_has_exact_row_predicate(plan)
    {
        return Ok(());
    }
    let skipped_rows = row_count
        .checked_sub(selected_len)
        .ok_or(CoveError::ArithOverflow)?;
    let skipped_cells = skipped_rows
        .checked_mul(plan.scan_projection.len())
        .ok_or(CoveError::ArithOverflow)?;
    stats.late_materialization_morsels += 1;
    stats.late_materialization_rows_skipped = stats
        .late_materialization_rows_skipped
        .checked_add(skipped_rows)
        .ok_or(CoveError::ArithOverflow)?;
    stats.late_materialization_cells_skipped = stats
        .late_materialization_cells_skipped
        .checked_add(skipped_cells)
        .ok_or(CoveError::ArithOverflow)?;
    Ok(())
}

fn record_native_projection_batch(
    segment_columns: &[&PreparedSegmentColumn],
    page_indexes: &[ColumnPageIndexEntryV1],
    page_payloads: &[RetainedColumnPagePayloadV1],
    stats: &mut DecodeStats,
) -> Result<(), CoveError> {
    if page_payloads.is_empty() {
        return Ok(());
    }
    if segment_columns.len() != page_indexes.len() || page_indexes.len() != page_payloads.len() {
        return Err(CoveError::PageCorrupt);
    }
    stats.native_projection_batches += 1;
    stats.native_projection_pages += page_payloads.len();
    for ((segment_column, page), payload) in segment_columns
        .iter()
        .zip(page_indexes.iter())
        .zip(page_payloads.iter())
    {
        let column = segment_column.directory();
        if page.column_id != column.column_id || payload.header.row_count != page.row_count {
            return Err(CoveError::PageCorrupt);
        }
        let root = payload.root_node()?;
        if root.logical_type != column.logical_type
            || root.physical_kind != column.physical_kind
            || root.logical_len != page.row_count
        {
            return Err(CoveError::PageCorrupt);
        }
        stats.native_projection_decode_boundaries +=
            usize::from(native_projection_page_is_decode_boundary(root));
    }
    Ok(())
}

fn native_projection_page_is_decode_boundary(
    root: &cove_core::page_payload::CoveEncodingNodeV1,
) -> bool {
    match (root.encoding_kind, root.physical_kind) {
        (CoveEncodingKind::NumCode, CovePhysicalKind::NumCode)
        | (CoveEncodingKind::FileCode, CovePhysicalKind::FileCode)
        | (CoveEncodingKind::PlainFixed, CovePhysicalKind::Boolean)
        | (CoveEncodingKind::VarBytes, CovePhysicalKind::VarBytes) => false,
        (CoveEncodingKind::PlainFixed, CovePhysicalKind::FixedBytes) => {
            logical_type_fixed_width(root.logical_type).is_none()
        }
        (CoveEncodingKind::LocalCodebook, CovePhysicalKind::VarBytes) => true,
        (CoveEncodingKind::LocalCodebook, _) => false,
        _ => true,
    }
}

pub async fn decode_scan_with_reader<R: CoveRangeReader + ?Sized>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
) -> Result<DecodedScan, CoveError> {
    let mut sink = VecDecodeSink::default();
    let stats =
        decode_scan_with_reader_to_sink_cached(state, plan, reader, None, 0, &mut sink).await?;
    Ok(sink.finish(stats))
}

async fn decode_scan_with_reader_cached<R: CoveRangeReader + ?Sized>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
    cache: Option<&ScanExecutionCache>,
    file_ordinal: usize,
) -> Result<DecodedScan, CoveError> {
    let mut sink = VecDecodeSink::default();
    let stats =
        decode_scan_with_reader_to_sink_cached(state, plan, reader, cache, file_ordinal, &mut sink)
            .await?;
    Ok(sink.finish(stats))
}

pub(crate) async fn decode_scan_with_reader_to_sink<
    R: CoveRangeReader + ?Sized,
    S: DecodeSink + ?Sized,
>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
    sink: &mut S,
) -> Result<DecodeStats, CoveError> {
    decode_scan_with_reader_to_sink_cached(state, plan, reader, None, 0, sink).await
}

async fn decode_scan_with_reader_to_sink_cached<
    R: CoveRangeReader + ?Sized,
    S: DecodeSink + ?Sized,
>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
    cache: Option<&ScanExecutionCache>,
    file_ordinal: usize,
    sink: &mut S,
) -> Result<DecodeStats, CoveError> {
    let plan = state.resolved_plan_for_current_state(plan)?;
    validate_scan_plan(state, &plan)?;
    let mut stats = DecodeStats::default();
    record_plan_predicates(&plan, &mut stats);
    let mut scratch = DecodeScratch::default();
    if sink.should_stop() {
        return Ok(stats);
    }

    for segment_ref in state.segments() {
        let segment =
            read_segment_metadata(reader, state, segment_ref, &mut stats, cache, file_ordinal)
                .await?;

        for morsel in ordered_morsels(
            state,
            segment_ref.segment_id,
            segment.morsel_entries(),
            &plan,
        ) {
            stats.morsels_considered += 1;
            let row_start = segment_ref
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                stats.morsels_pruned += 1;
                stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                &plan,
                segment_ref.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                stats.morsels_pruned += 1;
                stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment_ref.segment_id, morsel.morsel_id, &plan)?
                || should_prune_morsel_metadata(
                    state,
                    segment_ref,
                    &segment,
                    morsel.morsel_id,
                    &plan,
                    &mut stats,
                )?
            {
                stats.morsels_pruned += 1;
                continue;
            }

            selected_rows_for_morsel_metadata(
                state,
                &segment,
                segment_ref,
                morsel.morsel_id,
                &plan,
                reader,
                &mut stats,
                &mut scratch,
            )
            .await?;
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut stats,
            )?;
            scratch.selection.record(&mut stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            stats.rows_selected += selected_len;
            if plan_has_residual(&plan) {
                stats.residual_rows += selected_len;
            }

            if plan.scan_projection.is_empty() {
                let options = RecordBatchOptions::new().with_row_count(Some(selected_len));
                let batch = RecordBatch::try_new_with_options(
                    plan.output_schema.clone(),
                    Vec::new(),
                    &options,
                )
                .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch: {err}")))?;
                if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                    return Ok(stats);
                }
                continue;
            }

            let mut page_indexes = Vec::with_capacity(plan.scan_projection.len());
            let mut page_refs = Vec::with_capacity(plan.scan_projection.len());
            let mut columns = Vec::with_capacity(plan.scan_projection.len());
            let mut segment_columns = Vec::with_capacity(plan.scan_projection.len());
            let mut ranges = Vec::new();
            let mut range_hints = Vec::new();
            let mut range_slots = Vec::with_capacity(plan.scan_projection.len());
            for projection_index in &plan.scan_projection {
                let column = &state.table().columns[*projection_index];
                let segment_column = segment.column(column.column_id)?;
                let page = segment.page_for_morsel(segment_column, morsel.morsel_id)?;
                let page_ref = segment.page_ref_for_morsel(segment_column, morsel.morsel_id)?;
                state.reject_table_scan_page_feature_use(segment_ref, page)?;
                if page.page_length == 0 {
                    range_slots.push(None);
                } else {
                    let start = segment_ref
                        .offset
                        .checked_add(page.page_offset)
                        .ok_or(CoveError::ArithOverflow)?;
                    let end = start
                        .checked_add(page.page_length)
                        .ok_or(CoveError::ArithOverflow)?;
                    range_slots.push(Some(ranges.len()));
                    range_hints.push(state.range_cluster_hint(
                        segment_ref.segment_id,
                        morsel.morsel_id,
                        start,
                        end,
                    ));
                    ranges.push(start..end);
                }
                stats.pages_decoded += usize::from(page.page_length != 0);
                page_indexes.push(page.clone());
                page_refs.push(page_ref);
                columns.push(column);
                segment_columns.push(segment_column);
            }

            let coalesced_plan = build_layout_aware_coalesced_range_plan(
                &ranges,
                &range_hints,
                state.range_coalescing(),
            )?;
            let range_stats = coalesced_plan.stats();
            record_range_plan(
                RangeReadPlan::choose(
                    selected_len,
                    morsel.row_count as usize,
                    range_stats.original_ranges,
                    range_stats.coalesced_ranges,
                ),
                &mut stats,
            );
            stats.original_range_requests += range_stats.original_ranges;
            stats.range_requests += range_stats.coalesced_ranges;
            stats.range_bytes_requested = stats
                .range_bytes_requested
                .checked_add(range_stats.coalesced_bytes)
                .ok_or(CoveError::ArithOverflow)?;
            stats.range_bytes_used = stats
                .range_bytes_used
                .checked_add(range_stats.original_bytes)
                .ok_or(CoveError::ArithOverflow)?;
            if range_stats.coalesced_ranges < range_stats.original_ranges {
                stats.coalesced_range_requests += range_stats.coalesced_ranges;
            }
            let wires =
                read_coalesced_range_buffers_for_plan(reader, RangeReadKind::Data, &coalesced_plan)
                    .await?;
            stats.data_bytes_read = stats
                .data_bytes_read
                .checked_add(wires.iter().map(RetainedBytes::len).sum::<usize>())
                .ok_or(CoveError::ArithOverflow)?;
            let mut wire_slots = wires.into_iter().map(Some).collect::<Vec<_>>();
            let mut page_payloads = Vec::with_capacity(page_indexes.len());
            for ((column, page), slot) in columns.iter().zip(page_indexes.iter()).zip(range_slots) {
                let wire = slot.and_then(|index| wire_slots[index].take());
                page_payloads.push(materialize_page_payload_from_wire(
                    segment_ref.table_id,
                    segment_ref.segment_id,
                    column,
                    page,
                    wire,
                    state.pruning().zone_stats_entries.as_slice(),
                    state.pruning().codec_descriptors.as_slice(),
                    state
                        .mounted()
                        .dictionary
                        .as_ref()
                        .map(|dictionary| dictionary.len()),
                    state.page_payload_validation_policy(),
                )?);
            }

            record_native_projection_batch(
                &segment_columns,
                &page_indexes,
                &page_payloads,
                &mut stats,
            )?;
            let mut encoded_columns = Vec::with_capacity(columns.len());
            for ((column, page), payload) in columns
                .iter()
                .zip(page_indexes.iter())
                .zip(page_payloads.iter())
            {
                encoded_columns.push((
                    column.name.as_str(),
                    encoded_array_for_page(payload, page, state.mounted().dictionary.as_ref())?,
                ));
            }
            let arrow_options = state.arrow_export_options();
            let column_refs = arrow_encoded_columns_for_payloads(
                state,
                segment_ref.segment_id,
                &columns,
                &encoded_columns,
                &page_indexes,
                &page_refs,
                &page_payloads,
                arrow_options,
            );
            let batch = record_batch_for_selection(
                state,
                &column_refs,
                &scratch.selection,
                plan.output_schema.clone(),
                arrow_options,
                cache,
                &mut stats,
            )?
            .value;
            if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                return Ok(stats);
            }
        }
    }

    Ok(stats)
}

pub async fn decode_scan_with_reader_tasks<R: CoveRangeReader + ?Sized>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
    tasks: &[ScanTask],
) -> Result<DecodedScan, CoveError> {
    let mut sink = VecDecodeSink::default();
    let stats = decode_scan_with_reader_tasks_to_sink_cached(
        state, plan, reader, tasks, None, 0, &mut sink,
    )
    .await?;
    Ok(sink.finish(stats))
}

async fn decode_scan_with_reader_tasks_to_sink_cached<
    R: CoveRangeReader + ?Sized,
    S: DecodeSink + ?Sized,
>(
    state: &DatasetState,
    plan: &ScanPlan,
    reader: &R,
    tasks: &[ScanTask],
    cache: Option<&ScanExecutionCache>,
    file_ordinal: usize,
    sink: &mut S,
) -> Result<DecodeStats, CoveError> {
    let plan = state.resolved_plan_for_current_state(plan)?;
    validate_scan_plan(state, &plan)?;
    let mut stats = DecodeStats::default();
    record_plan_predicates(&plan, &mut stats);
    let mut scratch = DecodeScratch::default();
    if sink.should_stop() {
        return Ok(stats);
    }

    let mut task_start = 0usize;
    while task_start < tasks.len() {
        let segment_index = tasks[task_start].segment_index;
        let segment_ref = state
            .segments()
            .get(segment_index)
            .ok_or(CoveError::SegmentCorrupt)?;
        let segment =
            read_segment_metadata(reader, state, segment_ref, &mut stats, cache, file_ordinal)
                .await?;
        let task_end = tasks[task_start..]
            .iter()
            .position(|task| task.segment_index != segment_index)
            .map(|offset| task_start + offset)
            .unwrap_or(tasks.len());

        for task in &tasks[task_start..task_end] {
            stats.morsels_considered += 1;
            let morsel = segment.morsel(task.morsel_id)?;
            let row_start = segment_ref
                .row_start
                .checked_add(u64::from(morsel.first_row_in_segment))
                .ok_or(CoveError::ArithOverflow)?;
            if state.file(0)?.visibility().morsel_all_hidden(
                row_start,
                morsel.row_count,
                state.table().row_count,
            )? {
                stats.morsels_pruned += 1;
                stats.overlay_morsels_pruned += 1;
                continue;
            }
            if covi_morsel_pruned(
                &plan,
                segment_ref.segment_id,
                morsel.morsel_id,
                row_start,
                morsel.row_count,
            ) {
                stats.morsels_pruned += 1;
                stats.covi_candidate_pruned += 1;
                continue;
            }
            if prune::morsel_pruned(state, segment_ref.segment_id, morsel.morsel_id, &plan)?
                || should_prune_morsel_metadata(
                    state,
                    segment_ref,
                    &segment,
                    morsel.morsel_id,
                    &plan,
                    &mut stats,
                )?
            {
                stats.morsels_pruned += 1;
                continue;
            }

            if let Some(rows) = &task.row_selection {
                scratch.selection = Selection::from_rows(rows, morsel.row_count as usize);
                scratch.selected_rows.clear();
                stats.lookup_rowref_tasks += 1;
            } else {
                selected_rows_for_morsel_metadata(
                    state,
                    &segment,
                    segment_ref,
                    morsel.morsel_id,
                    &plan,
                    reader,
                    &mut stats,
                    &mut scratch,
                )
                .await?;
            }
            apply_overlay_to_selection(
                state,
                row_start,
                morsel.row_count,
                &mut scratch,
                &mut stats,
            )?;
            scratch.selection.record(&mut stats);
            if scratch.selection.is_empty() {
                continue;
            }
            let selected_len = scratch.selection.len();
            stats.rows_selected += selected_len;
            if plan_has_residual(&plan) {
                stats.residual_rows += selected_len;
            }
            record_late_materialization(
                &plan,
                morsel.row_count as usize,
                selected_len,
                &mut stats,
            )?;

            if plan.scan_projection.is_empty() {
                let options = RecordBatchOptions::new().with_row_count(Some(selected_len));
                let batch = RecordBatch::try_new_with_options(
                    plan.output_schema.clone(),
                    Vec::new(),
                    &options,
                )
                .map_err(|err| CoveError::BadSection(format!("Arrow RecordBatch: {err}")))?;
                if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                    return Ok(stats);
                }
                continue;
            }

            let mut page_indexes = Vec::with_capacity(plan.scan_projection.len());
            let mut page_refs = Vec::with_capacity(plan.scan_projection.len());
            let mut columns = Vec::with_capacity(plan.scan_projection.len());
            let mut segment_columns = Vec::with_capacity(plan.scan_projection.len());
            let mut ranges = Vec::new();
            let mut range_hints = Vec::new();
            let mut range_slots = Vec::with_capacity(plan.scan_projection.len());
            for projection_index in &plan.scan_projection {
                let column = &state.table().columns[*projection_index];
                let segment_column = segment.column(column.column_id)?;
                let page = segment.page_for_morsel(segment_column, morsel.morsel_id)?;
                let page_ref = segment.page_ref_for_morsel(segment_column, morsel.morsel_id)?;
                state.reject_table_scan_page_feature_use(segment_ref, page)?;
                if page.page_length == 0 {
                    range_slots.push(None);
                } else {
                    let start = segment_ref
                        .offset
                        .checked_add(page.page_offset)
                        .ok_or(CoveError::ArithOverflow)?;
                    let end = start
                        .checked_add(page.page_length)
                        .ok_or(CoveError::ArithOverflow)?;
                    range_slots.push(Some(ranges.len()));
                    range_hints.push(state.range_cluster_hint(
                        segment_ref.segment_id,
                        morsel.morsel_id,
                        start,
                        end,
                    ));
                    ranges.push(start..end);
                }
                stats.pages_decoded += usize::from(page.page_length != 0);
                page_indexes.push(page.clone());
                page_refs.push(page_ref);
                columns.push(column);
                segment_columns.push(segment_column);
            }

            let coalesced_plan = build_layout_aware_coalesced_range_plan(
                &ranges,
                &range_hints,
                state.range_coalescing(),
            )?;
            let range_stats = coalesced_plan.stats();
            record_range_plan(
                RangeReadPlan::choose(
                    selected_len,
                    morsel.row_count as usize,
                    range_stats.original_ranges,
                    range_stats.coalesced_ranges,
                ),
                &mut stats,
            );
            stats.original_range_requests += range_stats.original_ranges;
            stats.range_requests += range_stats.coalesced_ranges;
            stats.range_bytes_requested = stats
                .range_bytes_requested
                .checked_add(range_stats.coalesced_bytes)
                .ok_or(CoveError::ArithOverflow)?;
            stats.range_bytes_used = stats
                .range_bytes_used
                .checked_add(range_stats.original_bytes)
                .ok_or(CoveError::ArithOverflow)?;
            if range_stats.coalesced_ranges < range_stats.original_ranges {
                stats.coalesced_range_requests += range_stats.coalesced_ranges;
            }
            let wires =
                read_coalesced_range_buffers_for_plan(reader, RangeReadKind::Data, &coalesced_plan)
                    .await?;
            stats.data_bytes_read = stats
                .data_bytes_read
                .checked_add(wires.iter().map(RetainedBytes::len).sum::<usize>())
                .ok_or(CoveError::ArithOverflow)?;
            let mut wire_slots = wires.into_iter().map(Some).collect::<Vec<_>>();
            let mut page_payloads = Vec::with_capacity(page_indexes.len());
            for ((column, page), slot) in columns.iter().zip(page_indexes.iter()).zip(range_slots) {
                let wire = slot.and_then(|index| wire_slots[index].take());
                page_payloads.push(materialize_page_payload_from_wire(
                    segment_ref.table_id,
                    segment_ref.segment_id,
                    column,
                    page,
                    wire,
                    state.pruning().zone_stats_entries.as_slice(),
                    state.pruning().codec_descriptors.as_slice(),
                    state
                        .mounted()
                        .dictionary
                        .as_ref()
                        .map(|dictionary| dictionary.len()),
                    state.page_payload_validation_policy(),
                )?);
            }

            record_native_projection_batch(
                &segment_columns,
                &page_indexes,
                &page_payloads,
                &mut stats,
            )?;
            let mut encoded_columns = Vec::with_capacity(columns.len());
            for ((column, page), payload) in columns
                .iter()
                .zip(page_indexes.iter())
                .zip(page_payloads.iter())
            {
                encoded_columns.push((
                    column.name.as_str(),
                    encoded_array_for_page(payload, page, state.mounted().dictionary.as_ref())?,
                ));
            }
            let arrow_options = state.arrow_export_options();
            let column_refs = arrow_encoded_columns_for_payloads(
                state,
                segment_ref.segment_id,
                &columns,
                &encoded_columns,
                &page_indexes,
                &page_refs,
                &page_payloads,
                arrow_options,
            );
            let batch = record_batch_for_selection(
                state,
                &column_refs,
                &scratch.selection,
                plan.output_schema.clone(),
                arrow_options,
                cache,
                &mut stats,
            )?
            .value;
            if emit_batch(sink, &mut stats, batch)? == DecodeControl::Stop {
                return Ok(stats);
            }
        }
        if sink.should_stop() {
            return Ok(stats);
        }
        task_start = task_end;
    }

    Ok(stats)
}

fn validate_scan_plan(state: &DatasetState, plan: &ScanPlan) -> Result<(), CoveError> {
    for index in &plan.scan_projection {
        if *index >= state.table().columns.len() {
            return Err(CoveError::BadSchema(format!(
                "scan projection index {index} is out of bounds for {} columns",
                state.table().columns.len()
            )));
        }
    }
    for index in &plan.predicate_columns {
        if *index >= state.table().columns.len() {
            return Err(CoveError::BadSchema(format!(
                "predicate column index {index} is out of bounds for {} columns",
                state.table().columns.len()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        options::CoveTableOptions,
        planner::{plan_scan, FilterPlan},
    };
    use arrow_array::Array;
    use cove_arrow::arrow::ArrowStringValidationPolicy;
    use cove_core::{
        array::EncodedArray,
        checksum,
        constants::{
            CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, StorageClass,
            ValueTag,
        },
        dictionary::{FileDictionary, FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
        page_payload::{COLUMN_PAGE_PAYLOAD_HEADER_LEN, COVE_ENCODING_NODE_LEN},
        table::{TableCatalog, TableEntry},
        wire,
        writer::{ScanPageSpec, ScanProfileCoveWriter, ScanSegment},
    };
    use std::sync::Arc;

    fn bool_test_column() -> ColumnEntry {
        ColumnEntry {
            column_id: 0,
            name: "flag".to_string(),
            logical: CoveLogicalType::Bool,
            physical: CovePhysicalKind::Boolean,
            nullable: false,
            sort_order: 0,
            collation_id: 0,
            precision: 0,
            scale: 0,
            flags: 0,
        }
    }

    fn bool_test_page(payload: &[u8], checksum: u32) -> ColumnPageIndexEntryV1 {
        ColumnPageIndexEntryV1 {
            column_id: 0,
            morsel_id: 0,
            row_count: 3,
            non_null_count: 3,
            null_count: 0,
            encoding_root: 0,
            page_offset: 0,
            page_length: payload.len() as u64,
            uncompressed_length: payload.len() as u64,
            stats_ref: 0,
            flags: CompressionCodec::None as u32,
            checksum,
        }
    }

    fn bool_test_payload() -> Vec<u8> {
        ColumnPagePayloadV1::build_single_node(
            3,
            CoveEncodingKind::PlainFixed,
            CoveLogicalType::Bool,
            CovePhysicalKind::Boolean,
            None,
            vec![0, 1, 0],
        )
        .unwrap()
    }

    fn test_utf8_dictionary(values: &[&str]) -> FileDictionary {
        FileDictionary {
            header: FileDictionaryHeaderV1 {
                entry_count: values.len() as u32,
                flags: 0,
                index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
                value_hash_algorithm: 0,
                payload_length: 0,
                reserved: [0; 24],
            },
            entries: values
                .iter()
                .map(|value| test_inline_utf8_entry(value))
                .collect(),
            payload: Vec::new(),
        }
    }

    fn test_inline_utf8_entry(value: &str) -> FileDictionaryIndexEntryV1 {
        let canonical = test_canonical_utf8(value);
        let mut inline_data = [0u8; 16];
        inline_data[..canonical.len()].copy_from_slice(&canonical);
        FileDictionaryIndexEntryV1 {
            value_tag: ValueTag::Utf8 as u16,
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

    fn test_canonical_utf8(value: &str) -> Vec<u8> {
        let mut out = wire::encode_u64_leb128(value.len() as u64);
        out.extend_from_slice(value.as_bytes());
        out
    }

    #[test]
    fn local_filecode_group_counts_merge_through_local_to_global_map() {
        let dictionary = test_utf8_dictionary(&["red", "blue", "green"]);
        let local_groups = NativeDenseGroupCounts {
            counts: vec![2, 0, 3],
            null_count: 1,
            rows_grouped: 5,
        };
        let local_to_global = [2, 0, 1];

        let groups =
            local_filecode_groups_to_canonical(&dictionary, local_groups, &local_to_global)
                .unwrap();

        assert_eq!(groups.counts.get(&test_canonical_utf8("green")), Some(&2));
        assert_eq!(groups.counts.get(&test_canonical_utf8("blue")), Some(&3));
        assert_eq!(groups.counts.get(&test_canonical_utf8("red")), None);
        assert_eq!(groups.null_count, 1);
        assert_eq!(groups.rows_grouped, 5);
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

    fn numcode_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
        ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::NumCode as u32)
    }

    fn varbytes_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
        ScanPageSpec::new(row_count, payload).with_encoding_root(CoveEncodingKind::VarBytes as u32)
    }

    fn bool_page(row_count: u32, payload: Vec<u8>) -> ScanPageSpec {
        ScanPageSpec::new(row_count, payload)
            .with_encoding_root(CoveEncodingKind::PlainFixed as u32)
    }

    fn numcode_i64(values: &[i64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| (*value as u64).to_le_bytes())
            .collect()
    }

    fn numcode_i32(values: &[i32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| (*value as i64 as u64).to_le_bytes())
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

    fn bools(values: &[bool]) -> Vec<u8> {
        values.iter().map(|value| u8::from(*value)).collect()
    }

    fn primitive_events_file() -> Vec<u8> {
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
        writer.write().unwrap()
    }

    #[test]
    fn cove_table_options_default_to_trusted_page_payload_validation() {
        assert_eq!(
            CoveTableOptions::default().page_payload_validation_policy(),
            PagePayloadValidationPolicy::Trusted
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_strict_page_payload_validation()
                .page_payload_validation_policy(),
            PagePayloadValidationPolicy::Strict
        );
    }

    #[test]
    fn cove_table_options_default_to_cached_proof_arrow_string_validation() {
        assert_eq!(
            CoveTableOptions::default()
                .arrow_export_options()
                .string_validation_policy,
            ArrowStringValidationPolicy::StrictOrCachedProof
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_trusted_arrow_string_validation()
                .arrow_export_options()
                .string_validation_policy,
            ArrowStringValidationPolicy::TrustedPageProof
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_trusted_arrow_string_validation()
                .with_strict_arrow_string_validation()
                .arrow_export_options()
                .string_validation_policy,
            ArrowStringValidationPolicy::Strict
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_strict_arrow_string_validation()
                .with_cached_proof_arrow_string_validation()
                .arrow_export_options()
                .string_validation_policy,
            ArrowStringValidationPolicy::StrictOrCachedProof
        );
    }

    #[test]
    fn cove_table_options_default_to_mmap_local_file_reads() {
        assert_eq!(
            CoveTableOptions::default().local_file_read_policy(),
            LocalFileReadPolicy::Mmap
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_positioned_local_file_reads()
                .local_file_read_policy(),
            LocalFileReadPolicy::PositionedReads
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_local_file_mmap_reads()
                .local_file_read_policy(),
            LocalFileReadPolicy::Mmap
        );
        assert_eq!(
            CoveTableOptions::default()
                .with_local_file_mmap_reads()
                .with_positioned_local_file_reads()
                .local_file_read_policy(),
            LocalFileReadPolicy::PositionedReads
        );
    }

    #[test]
    fn trusted_arrow_string_validation_and_local_file_mmap_reads_sets_both_knobs() {
        let options = CoveTableOptions::default()
            .with_trusted_arrow_string_validation_and_local_file_mmap_reads();
        assert_eq!(
            options.arrow_export_options().string_validation_policy,
            ArrowStringValidationPolicy::TrustedPageProof
        );
        assert_eq!(options.local_file_read_policy(), LocalFileReadPolicy::Mmap);
        assert_eq!(
            options
                .clone()
                .with_strict_arrow_string_validation()
                .arrow_export_options()
                .string_validation_policy,
            ArrowStringValidationPolicy::Strict
        );
        assert_eq!(
            options
                .with_positioned_local_file_reads()
                .local_file_read_policy(),
            LocalFileReadPolicy::PositionedReads
        );
    }

    #[test]
    fn utf8_proof_cache_reuses_verified_pages_within_one_dataset_state() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let plan = plan_scan(&state, None, Vec::new()).unwrap();

        let first = decode_scan(&state, &plan).unwrap();
        assert_eq!(first.stats.utf8_proof_hits, 0);
        assert_eq!(first.stats.utf8_proof_misses, 2);
        assert_eq!(first.stats.utf8_proofs_earned, 2);

        let second = decode_scan(&state, &plan).unwrap();
        assert_eq!(second.stats.utf8_proof_hits, 2);
        assert_eq!(second.stats.utf8_proof_misses, 0);
        assert_eq!(second.stats.utf8_proofs_earned, 0);

        let other_state = DatasetState::from_bytes("events-copy", primitive_events_file()).unwrap();
        let other_plan = plan_scan(&other_state, None, Vec::new()).unwrap();
        let third = decode_scan(&other_state, &other_plan).unwrap();
        assert_eq!(third.stats.utf8_proof_hits, 0);
        assert_eq!(third.stats.utf8_proof_misses, 2);
        assert_eq!(third.stats.utf8_proofs_earned, 2);
    }

    #[derive(Debug, Default)]
    struct StopAfterFirstBatchSink {
        batches: usize,
        rows: usize,
    }

    impl DecodeSink for StopAfterFirstBatchSink {
        fn emit_batch(
            &mut self,
            batch: RecordBatch,
            stats: &mut DecodeStats,
        ) -> Result<DecodeControl, CoveError> {
            let rows = batch.num_rows();
            stats.rows_materialized += rows;
            self.batches += 1;
            self.rows += rows;
            Ok(DecodeControl::Stop)
        }
    }

    #[test]
    fn decode_sink_stop_after_first_batch_stops_partition_cleanly() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let plan = plan_scan(&state, None, Vec::new()).unwrap();
        let full = decode_scan(&state, &plan).unwrap();
        let mut sink = StopAfterFirstBatchSink::default();

        let stats = decode_scan_to_sink(&state, &plan, &mut sink).unwrap();

        assert_eq!(sink.batches, 1);
        assert_eq!(sink.rows, 2);
        assert_eq!(stats.rows_materialized, 2);
        assert_eq!(stats.morsels_considered, 1);
        assert!(stats.morsels_considered < full.stats.morsels_considered);
    }

    #[test]
    fn fetch_limited_sink_slices_final_batch_and_stops() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let projection = vec![0];
        let plan = plan_scan(&state, Some(&projection), Vec::new()).unwrap();
        let inner = VecDecodeSink::default();
        let mut sink = FetchLimitedDecodeSink::new(inner, Some(1));

        let stats = decode_scan_to_sink(&state, &plan, &mut sink).unwrap();

        assert_eq!(stats.rows_materialized, 1);
        assert_eq!(stats.rows_selected, 2);
        assert_eq!(stats.morsels_considered, 1);
        assert_eq!(sink.inner.batches.len(), 1);
        assert_eq!(sink.inner.batches[0].num_rows(), 1);
    }

    #[test]
    fn exact_numeric_filter_records_late_materialization_for_projected_strings() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let projection = vec![1];
        let plan = plan_scan(
            &state,
            Some(&projection),
            vec![FilterPlan::pruning_numeric(
                0,
                NumericPredicateOp::Gt,
                PredicateLiteral::UInt64(1),
                "id > 1",
            )],
        )
        .unwrap();

        let decoded = decode_scan(&state, &plan).unwrap();

        assert_eq!(decoded.stats.exact_predicates, 1);
        assert_eq!(decoded.stats.rows_selected, 2);
        assert_eq!(decoded.stats.rows_materialized, 2);
        assert!(decoded.stats.native_table_batches >= 1);
        assert!(decoded.stats.native_table_pages >= 1);
        assert_eq!(decoded.stats.native_table_decode_boundaries, 0);
        assert!(decoded.stats.native_lane_predicates >= 1);
        assert!(decoded.stats.native_projection_batches >= 1);
        assert!(decoded.stats.native_projection_pages >= 1);
        assert_eq!(decoded.stats.native_projection_decode_boundaries, 0);
        assert_eq!(decoded.stats.late_materialization_morsels, 1);
        assert_eq!(decoded.stats.late_materialization_rows_skipped, 1);
        assert_eq!(decoded.stats.late_materialization_cells_skipped, 1);
        assert_eq!(decoded.stats.utf8_proof_misses, 2);
        assert_eq!(decoded.stats.utf8_proofs_earned, 2);
        assert_eq!(
            decoded
                .batches
                .iter()
                .map(|batch| batch.num_rows())
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn exact_numeric_filter_with_no_matches_does_not_materialize_projected_strings() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let projection = vec![1];
        let plan = plan_scan(
            &state,
            Some(&projection),
            vec![FilterPlan::pruning_numeric(
                0,
                NumericPredicateOp::Gt,
                PredicateLiteral::UInt64(99),
                "id > 99",
            )],
        )
        .unwrap();

        let decoded = decode_scan(&state, &plan).unwrap();

        assert_eq!(decoded.stats.rows_selected, 0);
        assert_eq!(decoded.stats.rows_materialized, 0);
        assert_eq!(decoded.stats.utf8_proof_misses, 0);
        assert_eq!(decoded.stats.utf8_proofs_earned, 0);
        assert!(decoded.batches.is_empty());
    }

    #[test]
    fn exact_numeric_filter_still_works_when_predicate_column_is_projected() {
        let state = DatasetState::from_bytes("events", primitive_events_file()).unwrap();
        let projection = vec![0, 1];
        let plan = plan_scan(
            &state,
            Some(&projection),
            vec![FilterPlan::pruning_numeric(
                0,
                NumericPredicateOp::Gt,
                PredicateLiteral::UInt64(1),
                "id > 1",
            )],
        )
        .unwrap();

        let decoded = decode_scan(&state, &plan).unwrap();

        assert_eq!(decoded.stats.rows_selected, 2);
        assert_eq!(decoded.stats.rows_materialized, 2);
        assert_eq!(decoded.stats.late_materialization_rows_skipped, 1);
        assert_eq!(decoded.stats.late_materialization_cells_skipped, 2);
        assert!(decoded.batches.iter().all(|batch| batch.num_columns() == 2));
    }

    #[test]
    fn scan_execution_cache_reuses_local_reader_by_file_ordinal() {
        let cache = ScanExecutionCache::default();
        let first = cache
            .local_reader(
                4,
                LocalFileReadPolicy::PositionedReads,
                "/tmp/cove-cache-a.cove",
            )
            .unwrap();
        let second = cache
            .local_reader(
                4,
                LocalFileReadPolicy::PositionedReads,
                "/tmp/cove-cache-a.cove",
            )
            .unwrap();
        let mmap = cache
            .local_reader(4, LocalFileReadPolicy::Mmap, "/tmp/cove-cache-a.cove")
            .unwrap();
        let other = cache
            .local_reader(
                5,
                LocalFileReadPolicy::PositionedReads,
                "/tmp/cove-cache-a.cove",
            )
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &mmap));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn trusted_page_payload_validation_skips_page_wire_crc_only() {
        let column = bool_test_column();
        let payload = bool_test_payload();
        let wrong_checksum = checksum::crc32c(&payload) ^ 1;
        let page = bool_test_page(&payload, wrong_checksum);

        assert!(materialize_page_payload_from_wire(
            1,
            1,
            &column,
            &page,
            Some(RetainedBytes::from_vec(payload.clone())),
            &[],
            &[],
            None,
            PagePayloadValidationPolicy::Trusted,
        )
        .is_ok());
        assert!(matches!(
            materialize_page_payload_from_wire(
                1,
                1,
                &column,
                &page,
                Some(RetainedBytes::from_vec(payload)),
                &[],
                &[],
                None,
                PagePayloadValidationPolicy::Strict,
            )
            .unwrap_err(),
            CoveError::ChecksumMismatch
        ));
    }

    #[test]
    fn trusted_page_payload_validation_skips_buffer_crc_only() {
        let column = bool_test_column();
        let mut payload = bool_test_payload();
        let checksum_offset = COLUMN_PAGE_PAYLOAD_HEADER_LEN + COVE_ENCODING_NODE_LEN + 24;
        payload[checksum_offset..checksum_offset + 4].copy_from_slice(&0u32.to_le_bytes());
        let page = bool_test_page(&payload, checksum::crc32c(&payload));

        assert!(materialize_page_payload_from_wire(
            1,
            1,
            &column,
            &page,
            Some(RetainedBytes::from_vec(payload.clone())),
            &[],
            &[],
            None,
            PagePayloadValidationPolicy::Trusted,
        )
        .is_ok());
        assert!(matches!(
            materialize_page_payload_from_wire(
                1,
                1,
                &column,
                &page,
                Some(RetainedBytes::from_vec(payload)),
                &[],
                &[],
                None,
                PagePayloadValidationPolicy::Strict,
            )
            .unwrap_err(),
            CoveError::ChecksumMismatch
        ));
    }

    #[test]
    fn selection_mask_and_row_extraction() {
        let mut selected = SelectionMask::default();
        selected.fill_all(10);
        selected.clear_bit(1);
        selected.clear_bit(8);

        let mut filter = SelectionMask::default();
        filter.fill_none(10);
        filter.set(0);
        filter.set(8);
        filter.set(9);

        selected.and_inplace(&filter);
        assert_eq!(selected.count_ones(), 2);

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![0, 9]);
    }

    #[test]
    fn numeric_predicate_filters_prepared_varint_rows() {
        let bytes = [
            wire::encode_u64_leb128(3),
            wire::encode_u64_leb128(17),
            wire::encode_u64_leb128(255),
        ]
        .concat();
        let array = EncodedArray::new(
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::PlainVarint,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Eq,
            literal: PredicateLiteral::UInt64(17),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn numeric_predicate_filters_numcode_rows_directly() {
        let bytes = [
            3u64.to_le_bytes(),
            17u64.to_le_bytes(),
            255u64.to_le_bytes(),
        ]
        .concat();
        let array = EncodedArray::new(
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Gt,
            literal: PredicateLiteral::UInt64(16),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1, 2]);
    }

    #[test]
    fn numeric_predicate_filters_signed_numcode_by_logical_type() {
        let bytes = numcode_i32(&[-8, -1, 3]);
        let array = EncodedArray::new(
            CoveLogicalType::Int32,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Lt,
            literal: PredicateLiteral::Int64(0),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![0, 1]);
    }

    #[test]
    fn numeric_predicate_filters_temporal_and_decimal_numcode_by_logical_type() {
        for logical in [
            CoveLogicalType::Decimal64,
            CoveLogicalType::TimestampMicros,
            CoveLogicalType::TimestampNanos,
        ] {
            let bytes = numcode_i64(&[-20, -1, 10]);
            let array = EncodedArray::new(
                logical,
                CovePhysicalKind::NumCode,
                3,
                CoveEncodingKind::NumCode,
                None,
                &bytes,
                None,
            );
            let prepared = array.prepare().unwrap();
            let predicate = CovePredicate::Numeric {
                column_index: 0,
                op: NumericPredicateOp::GtEq,
                literal: PredicateLiteral::Int64(-1),
            };
            let mut selected = SelectionMask::default();
            selected.fill_all(3);
            let mut scratch = SelectionMask::default();
            assert!(apply_predicate_to_selection(
                &predicate,
                &prepared,
                &mut selected,
                &mut scratch
            )
            .unwrap());

            let mut rows = Vec::new();
            selected.write_selected_rows(&mut rows).unwrap();
            assert_eq!(rows, vec![1, 2]);
        }

        let bytes = numcode_i32(&[-2, 0, 4]);
        let array = EncodedArray::new(
            CoveLogicalType::DateDays,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Eq,
            literal: PredicateLiteral::Int64(-2),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![0]);
    }

    #[test]
    fn numeric_predicate_filters_float_numcode_rows_by_value() {
        let bytes = numcode_f64(&[1.5, 2.25, -3.0]);
        let array = EncodedArray::new(
            CoveLogicalType::Float64,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Gt,
            literal: PredicateLiteral::Float64(2.0),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn numeric_predicate_filters_float32_numcode_rows_by_value() {
        let bytes = numcode_f32(&[1.5, 2.25, -3.0]);
        let array = EncodedArray::new(
            CoveLogicalType::Float32,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Eq,
            literal: PredicateLiteral::Float64(2.25),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(
            apply_predicate_to_selection(&predicate, &prepared, &mut selected, &mut scratch)
                .unwrap()
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn numeric_lookup_keys_use_logical_numcode_bits() {
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Float64, PredicateLiteral::Float64(2.25)),
            vec![2.25f64.to_bits()]
        );
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Float32, PredicateLiteral::Float64(2.25)),
            vec![u64::from(2.25f32.to_bits())]
        );
        assert!(
            numeric_lookup_keys(CoveLogicalType::Float32, PredicateLiteral::Float64(0.1))
                .is_empty()
        );
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Float64, PredicateLiteral::Float64(0.0)),
            vec![0.0f64.to_bits(), (-0.0f64).to_bits()]
        );
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Bool, PredicateLiteral::Int64(0)),
            vec![0]
        );
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Bool, PredicateLiteral::UInt64(1)),
            vec![1]
        );
        assert!(numeric_lookup_keys(CoveLogicalType::Bool, PredicateLiteral::Int64(2)).is_empty());
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::Int64, PredicateLiteral::Int64(-2)),
            vec![(-2i64) as u64]
        );
        assert_eq!(
            numeric_lookup_keys(CoveLogicalType::UInt64, PredicateLiteral::Float64(2.0)),
            vec![2]
        );
    }

    #[test]
    fn numeric_predicate_filters_bool_numcode_rows_by_numeric_value() {
        let bytes = [0u64.to_le_bytes(), 1u64.to_le_bytes(), 1u64.to_le_bytes()].concat();
        let array = EncodedArray::new(
            CoveLogicalType::Bool,
            CovePhysicalKind::NumCode,
            3,
            CoveEncodingKind::NumCode,
            None,
            &bytes,
            None,
        );
        let prepared = array.prepare().unwrap();
        let mut selected = SelectionMask::default();
        selected.fill_all(3);
        let mut scratch = SelectionMask::default();
        assert!(apply_predicate_to_selection(
            &CovePredicate::Numeric {
                column_index: 0,
                op: NumericPredicateOp::Lt,
                literal: PredicateLiteral::Int64(1),
            },
            &prepared,
            &mut selected,
            &mut scratch,
        )
        .unwrap());

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![0]);
    }

    #[test]
    fn record_batch_for_selection_exports_bitset_without_row_materialization() {
        let mut values = Vec::new();
        values.extend_from_slice(&1u32.to_le_bytes());
        values.extend_from_slice(b"a");
        values.extend_from_slice(&2u32.to_le_bytes());
        values.extend_from_slice(b"bb");
        values.extend_from_slice(&3u32.to_le_bytes());
        values.extend_from_slice(b"ccc");
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            3,
            CoveEncodingKind::VarBytes,
            None,
            &values,
            None,
        );
        let mut mask = SelectionMask::default();
        mask.fill_none(3);
        mask.set(0);
        mask.set(2);
        let selection = Selection::Bitset(mask);
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "word",
            arrow_schema::DataType::Utf8,
            false,
        )]));
        let state = DatasetState::from_bytes("selection", primitive_events_file()).unwrap();
        let mut stats = DecodeStats::default();

        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "word",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: None,
                utf8_proof_key: None,
                zero_copy: None,
            }],
            &selection,
            schema,
            ArrowExportOptions::default(),
            None,
            &mut stats,
        )
        .unwrap();
        let strings = result
            .value
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(strings.len(), 2);
        assert_eq!(strings.value(0), "a");
        assert_eq!(strings.value(1), "ccc");
    }

    #[test]
    fn compatible_zero_copy_map_records_direct_export_metric() {
        let values = Arc::new(varbytes(&["alpha", "beta"]));
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            2,
            CoveEncodingKind::VarBytes,
            None,
            values.as_slice(),
            None,
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "word",
            arrow_schema::DataType::Utf8View,
            false,
        )]));
        let state = DatasetState::from_bytes("zero-copy", primitive_events_file()).unwrap();
        let selection = Selection::AllRows { len: 2 };
        let mut stats = DecodeStats::default();

        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "word",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: Some(arrow_buffer_owner(Arc::clone(&values))),
                utf8_proof_key: None,
                zero_copy: Some(ZeroCopyCompatibilityV2::Compatible),
            }],
            &selection,
            schema,
            arrow_view_options(),
            None,
            &mut stats,
        )
        .unwrap();

        assert_eq!(result.value.num_rows(), 2);
        assert_eq!(
            result.value.column(0).data_type(),
            &arrow_schema::DataType::Utf8View
        );
        assert_eq!(stats.zero_copy_compatible_buffers, 1);
        assert_eq!(stats.zero_copy_materialized_buffers, 0);
    }

    #[test]
    fn compatible_zero_copy_map_reuses_direct_plainfixed_owner() {
        let mut raw_values = Vec::new();
        raw_values.extend_from_slice(&[1u8; 16]);
        raw_values.extend_from_slice(&[2u8; 16]);
        let values = Arc::new(raw_values);
        let array = EncodedArray::new(
            CoveLogicalType::Uuid,
            CovePhysicalKind::FixedBytes,
            2,
            CoveEncodingKind::PlainFixed,
            None,
            values.as_slice(),
            None,
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "uid",
            arrow_schema::DataType::FixedSizeBinary(16),
            false,
        )]));
        let state = DatasetState::from_bytes("zero-copy-fixed", primitive_events_file()).unwrap();
        let selection = Selection::AllRows { len: 2 };
        let mut stats = DecodeStats::default();

        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "uid",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: Some(arrow_buffer_owner(Arc::clone(&values))),
                utf8_proof_key: None,
                zero_copy: Some(ZeroCopyCompatibilityV2::Compatible),
            }],
            &selection,
            schema,
            ArrowExportOptions::default(),
            None,
            &mut stats,
        )
        .unwrap();

        let uuids = result
            .value
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
            .unwrap();
        assert_eq!(uuids.value(0), &[1u8; 16]);
        assert_eq!(uuids.value(1), &[2u8; 16]);
        assert_eq!(uuids.to_data().buffers()[0].as_ptr(), values.as_ptr());
        assert_eq!(stats.zero_copy_compatible_buffers, 1);
        assert_eq!(stats.zero_copy_materialized_buffers, 0);
    }

    #[test]
    fn compatible_zero_copy_map_reuses_direct_numcode_owner() {
        let values = Arc::new(vec![10u64, 20u64]);
        // SAFETY: the byte slice is read-only, covers exactly the `u64` vector
        // allocation, and the `Arc<Vec<u64>>` is retained as the Arrow owner.
        let value_bytes = unsafe {
            std::slice::from_raw_parts(
                values.as_ptr().cast::<u8>(),
                values.len() * std::mem::size_of::<u64>(),
            )
        };
        let array = EncodedArray::new(
            CoveLogicalType::Int64,
            CovePhysicalKind::NumCode,
            2,
            CoveEncodingKind::NumCode,
            None,
            value_bytes,
            None,
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "metric",
            arrow_schema::DataType::Int64,
            false,
        )]));
        let state = DatasetState::from_bytes("zero-copy-numcode", primitive_events_file()).unwrap();
        let selection = Selection::AllRows { len: 2 };
        let mut stats = DecodeStats::default();

        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "metric",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: Some(arrow_buffer_owner(Arc::clone(&values))),
                utf8_proof_key: None,
                zero_copy: Some(ZeroCopyCompatibilityV2::Compatible),
            }],
            &selection,
            schema,
            ArrowExportOptions::default(),
            None,
            &mut stats,
        )
        .unwrap();

        let metrics = result
            .value
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap();
        assert_eq!(metrics.values(), &[10, 20]);
        assert_eq!(
            metrics.to_data().buffers()[0].as_ptr(),
            values.as_ptr().cast::<u8>()
        );
        assert_eq!(stats.zero_copy_compatible_buffers, 1);
        assert_eq!(stats.zero_copy_materialized_buffers, 0);
    }

    #[test]
    fn compatible_zero_copy_map_with_selection_materializes_owned_view() {
        let values = Arc::new(varbytes(&["alpha", "hidden", "beta"]));
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            3,
            CoveEncodingKind::VarBytes,
            None,
            values.as_slice(),
            None,
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "word",
            arrow_schema::DataType::Utf8View,
            false,
        )]));
        let state = DatasetState::from_bytes("zero-copy", primitive_events_file()).unwrap();
        let selection = Selection::RowIndices(vec![0, 2]);
        let mut stats = DecodeStats::default();

        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "word",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: Some(arrow_buffer_owner(Arc::clone(&values))),
                utf8_proof_key: None,
                zero_copy: Some(ZeroCopyCompatibilityV2::Compatible),
            }],
            &selection,
            schema,
            arrow_view_options(),
            None,
            &mut stats,
        )
        .unwrap();

        let strings = result
            .value
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::StringViewArray>()
            .unwrap();
        assert_eq!(strings.value(0), "alpha");
        assert_eq!(strings.value(1), "beta");
        assert_eq!(stats.zero_copy_compatible_buffers, 0);
        assert_eq!(stats.zero_copy_materialized_buffers, 1);
        assert_eq!(stats.zero_copy_materialized_selection_mismatch, 1);
    }

    #[test]
    fn incompatible_zero_copy_maps_record_materialization_reasons() {
        for reason in [
            ZeroCopyMaterializationReasonV2::CompressedBuffer,
            ZeroCopyMaterializationReasonV2::NullPolarityMismatch,
            ZeroCopyMaterializationReasonV2::DictionaryMismatch,
            ZeroCopyMaterializationReasonV2::NestedLayoutMismatch,
            ZeroCopyMaterializationReasonV2::InsufficientLifetime,
            ZeroCopyMaterializationReasonV2::UnknownRole,
            ZeroCopyMaterializationReasonV2::ActiveVisibilityOverlay,
        ] {
            let stats = export_with_zero_copy_decision(Some(
                ZeroCopyCompatibilityV2::MaterializeRequired(reason),
            ));
            assert_eq!(stats.zero_copy_compatible_buffers, 0, "{reason:?}");
            assert_eq!(stats.zero_copy_materialized_buffers, 1, "{reason:?}");
            assert_eq!(zero_copy_reason_count(&stats, reason), 1, "{reason:?}");
        }
    }

    #[test]
    fn no_zero_copy_map_view_output_is_not_counted_as_attempted() {
        let stats = export_with_zero_copy_decision(None);
        assert_eq!(stats.zero_copy_compatible_buffers, 0);
        assert_eq!(stats.zero_copy_materialized_buffers, 0);
    }

    fn export_with_zero_copy_decision(decision: Option<ZeroCopyCompatibilityV2>) -> DecodeStats {
        let values = Arc::new(varbytes(&["alpha", "beta"]));
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            2,
            CoveEncodingKind::VarBytes,
            None,
            values.as_slice(),
            None,
        );
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "word",
            arrow_schema::DataType::Utf8View,
            false,
        )]));
        let state = DatasetState::from_bytes("zero-copy", primitive_events_file()).unwrap();
        let selection = Selection::AllRows { len: 2 };
        let mut stats = DecodeStats::default();
        let result = record_batch_for_selection(
            &state,
            &[DecodedArrowColumn {
                name: "word",
                array: &array,
                payload: None,
                nested_schema: None,
                data_owner: Some(arrow_buffer_owner(Arc::clone(&values))),
                utf8_proof_key: None,
                zero_copy: decision,
            }],
            &selection,
            schema,
            arrow_view_options(),
            None,
            &mut stats,
        )
        .unwrap();
        let strings = result
            .value
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::StringViewArray>()
            .unwrap();
        assert_eq!(strings.value(0), "alpha");
        assert_eq!(strings.value(1), "beta");
        stats
    }

    fn arrow_view_options() -> ArrowExportOptions {
        ArrowExportOptions {
            varbytes_policy: ArrowVarBytesExportPolicy::View,
            ..ArrowExportOptions::default()
        }
    }

    fn zero_copy_reason_count(
        stats: &DecodeStats,
        reason: ZeroCopyMaterializationReasonV2,
    ) -> usize {
        match reason {
            ZeroCopyMaterializationReasonV2::UnknownRole => {
                stats.zero_copy_materialized_unknown_role
            }
            ZeroCopyMaterializationReasonV2::NullPolarityMismatch => {
                stats.zero_copy_materialized_null_polarity_mismatch
            }
            ZeroCopyMaterializationReasonV2::CompressedBuffer => {
                stats.zero_copy_materialized_compressed_buffer
            }
            ZeroCopyMaterializationReasonV2::DictionaryMismatch => {
                stats.zero_copy_materialized_dictionary_mismatch
            }
            ZeroCopyMaterializationReasonV2::NestedLayoutMismatch => {
                stats.zero_copy_materialized_nested_layout_mismatch
            }
            ZeroCopyMaterializationReasonV2::InsufficientLifetime => {
                stats.zero_copy_materialized_insufficient_lifetime
            }
            ZeroCopyMaterializationReasonV2::ActiveVisibilityOverlay => {
                stats.zero_copy_materialized_active_visibility_overlay
            }
        }
    }
}
