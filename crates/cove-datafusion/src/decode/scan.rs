use std::{collections::BTreeSet, path::Path, sync::Arc};

use cove_core::CoveError;

use crate::{
    dataset_state::DatasetState, planner::ScanPlan, range_reader::MemoryRangeReader,
    task_graph::ScanTask,
};

use super::{
    decode_scan, decode_scan_to_sink, decode_scan_with_reader_cached,
    decode_scan_with_reader_tasks_to_sink_cached, plan_selects_no_rows, DecodeSink, DecodeStats,
    DecodedScan, ScanExecutionCache, VecDecodeSink,
};

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
