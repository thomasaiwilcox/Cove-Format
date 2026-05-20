//! Stable scan task generation from planned candidates.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use cove_core::{
    index::lookup::LookupKeyKind,
    segment::{
        RowMorselDirectory, RowMorselEntryV1, TableSegmentHeaderV1, TableSegmentIndexEntryV1,
        ROW_MORSEL_ENTRY_LEN, TABLE_SEGMENT_HEADER_LEN,
    },
    wire, CoveError,
};

use crate::{
    dataset_state::DatasetState,
    decode::numeric_lookup_keys,
    planner::{CovePredicate, NumericPredicateOp, ScanPlan},
    prune,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanTask {
    pub file_ordinal: usize,
    pub segment_index: usize,
    pub segment_id: u32,
    pub morsel_id: u32,
    pub row_start: u64,
    pub row_count: u32,
    pub split_id: Option<u32>,
    pub cluster_id: Option<u32>,
    pub output_columns: Vec<usize>,
    pub predicate_columns: Vec<usize>,
    pub row_selection: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskGraph {
    pub tasks: Vec<ScanTask>,
    pub partitions: Vec<TaskPartition>,
    pub morsels_considered: usize,
    pub morsels_pruned: usize,
    pub scan_splits_used: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskPartition {
    pub tasks: Vec<ScanTask>,
}

/// Build a stable task list using metadata-only pruning. This function must
/// not read or decode page payloads; execution remains responsible for row
/// predicate evaluation and materialization.
pub fn build_task_graph(state: &DatasetState, plan: &ScanPlan) -> Result<TaskGraph, CoveError> {
    if plan.scan_program.lookup_rowref_eligible {
        if let Some(graph) = build_lookup_rowref_task_graph(state, plan)? {
            return Ok(graph);
        }
    }

    let mut graph = TaskGraph::default();
    let mut split_partitions = Vec::new();
    let mut fallback_tasks = Vec::new();
    for file_ordinal in 0..state.file_count() {
        let file_state = state.single_file_view(file_ordinal)?;
        let file_plan = state.resolved_plan_for_file(plan, file_ordinal)?;
        if let Some(splits) = file_state.file(0)?.layout().scan_splits.as_deref() {
            let segment_by_id = file_state
                .segments()
                .iter()
                .enumerate()
                .map(|(segment_index, segment)| (segment.segment_id, segment_index))
                .collect::<BTreeMap<_, _>>();
            for split in &splits.entries {
                let mut partition = TaskPartition::default();
                let segment_end = split
                    .first_segment_id
                    .checked_add(split.segment_count)
                    .ok_or(CoveError::ArithOverflow)?;
                let morsel_end = split
                    .first_morsel_id
                    .checked_add(split.morsel_count)
                    .ok_or(CoveError::ArithOverflow)?;
                for segment_id in split.first_segment_id..segment_end {
                    let Some(segment_index) = segment_by_id.get(&segment_id).copied() else {
                        continue;
                    };
                    let segment = file_state
                        .segments()
                        .get(segment_index)
                        .ok_or(CoveError::SegmentCorrupt)?;
                    let morsels = segment_morsels_for_task_graph(&file_state, segment)?;
                    for morsel in morsels.iter().filter(|morsel| {
                        morsel_in_split_range(morsel, split.first_morsel_id, morsel_end)
                    }) {
                        maybe_push_task(
                            &file_state,
                            &file_plan,
                            file_ordinal,
                            segment_index,
                            segment,
                            morsel,
                            Some(split.split_id),
                            single_cluster_id(split.first_cluster_id, split.cluster_count),
                            &mut graph,
                            Some(&mut partition),
                            None,
                        )?;
                    }
                }
                if !partition.tasks.is_empty() {
                    graph.scan_splits_used += 1;
                    split_partitions.push(partition);
                }
            }
        } else {
            for (segment_index, segment) in file_state.segments().iter().enumerate() {
                let morsels = segment_morsels_for_task_graph(&file_state, segment)?;
                for morsel in &morsels {
                    maybe_push_task(
                        &file_state,
                        &file_plan,
                        file_ordinal,
                        segment_index,
                        segment,
                        morsel,
                        None,
                        None,
                        &mut graph,
                        None,
                        Some(&mut fallback_tasks),
                    )?;
                }
            }
        }
    }
    if split_partitions.is_empty() {
        finalize_partitions(&mut graph, state.target_morsels_per_partition());
    } else {
        graph.partitions.extend(split_partitions);
        graph.partitions.extend(partition_tasks(
            &fallback_tasks,
            state.target_morsels_per_partition(),
        ));
        if graph.partitions.is_empty() {
            graph.partitions.push(TaskPartition::default());
        }
    }
    Ok(graph)
}

#[allow(clippy::too_many_arguments)]
fn maybe_push_task(
    file_state: &DatasetState,
    file_plan: &ScanPlan,
    file_ordinal: usize,
    segment_index: usize,
    segment: &TableSegmentIndexEntryV1,
    morsel: &RowMorselEntryV1,
    split_id: Option<u32>,
    cluster_id: Option<u32>,
    graph: &mut TaskGraph,
    partition: Option<&mut TaskPartition>,
    fallback_tasks: Option<&mut Vec<ScanTask>>,
) -> Result<(), CoveError> {
    graph.morsels_considered += 1;
    let row_start = morsel_row_start(segment, morsel)?;
    let row_count = morsel.row_count;
    if file_state.file(0)?.visibility().morsel_all_hidden(
        row_start,
        row_count,
        file_state.table().row_count,
    )? || prune::morsel_pruned(file_state, segment.segment_id, morsel.morsel_id, file_plan)?
    {
        graph.morsels_pruned += 1;
        return Ok(());
    }
    let task = ScanTask {
        file_ordinal,
        segment_index,
        segment_id: segment.segment_id,
        morsel_id: morsel.morsel_id,
        row_start,
        row_count,
        split_id,
        cluster_id,
        output_columns: file_plan.column_plan.output_columns.clone(),
        predicate_columns: file_plan.column_plan.predicate_columns.clone(),
        row_selection: None,
    };
    if let Some(partition) = partition {
        partition.tasks.push(task.clone());
    }
    if let Some(fallback_tasks) = fallback_tasks {
        fallback_tasks.push(task.clone());
    }
    graph.tasks.push(task);
    Ok(())
}

fn single_cluster_id(first_cluster_id: u32, cluster_count: u32) -> Option<u32> {
    if cluster_count == 1 {
        Some(first_cluster_id)
    } else {
        None
    }
}

fn build_lookup_rowref_task_graph(
    state: &DatasetState,
    plan: &ScanPlan,
) -> Result<Option<TaskGraph>, CoveError> {
    let mut graph = TaskGraph::default();
    for file_ordinal in 0..state.file_count() {
        let file_state = state.single_file_view(file_ordinal)?;
        let file_plan = state.resolved_plan_for_file(plan, file_ordinal)?;
        let Some((column_index, key_kind, keys)) = lookup_keys_for_plan(&file_state, &file_plan)
        else {
            return Ok(None);
        };
        let Some(column) = file_state.table().columns.get(column_index) else {
            return Ok(None);
        };
        let Some(index) = file_state.lookup_for(column.column_id) else {
            return Ok(None);
        };
        if index.header.key_kind != key_kind {
            return Ok(None);
        }

        let segment_by_id = file_state
            .segments()
            .iter()
            .enumerate()
            .map(|(segment_index, segment)| (segment.segment_id, segment_index))
            .collect::<BTreeMap<_, _>>();
        let mut morsels_by_segment = BTreeMap::<u32, Vec<RowMorselEntryV1>>::new();
        let mut rows_by_morsel: BTreeMap<RowrefMorselKey, BTreeSet<u32>> = BTreeMap::new();
        for key in keys {
            let Some(rows) = index.rows_for(key) else {
                continue;
            };
            for row in rows {
                if row.table_id != file_state.table().table_id {
                    continue;
                }
                let Some(segment_index) = segment_by_id.get(&row.segment_id).copied() else {
                    return Ok(None);
                };
                let segment = file_state
                    .segments()
                    .get(segment_index)
                    .ok_or(CoveError::SegmentCorrupt)?;
                if !morsels_by_segment.contains_key(&row.segment_id) {
                    let morsels = segment_morsels_for_task_graph(&file_state, segment)?;
                    morsels_by_segment.insert(row.segment_id, morsels);
                }
                let morsels = morsels_by_segment
                    .get(&row.segment_id)
                    .ok_or(CoveError::SegmentCorrupt)?;
                let morsel = morsel_by_id(morsels, row.morsel_id)?;
                let row_count = morsel.row_count;
                let row_in_morsel = u32::from(row.row_in_morsel);
                if row_in_morsel >= row_count {
                    return Ok(None);
                }
                let row_start = morsel_row_start(segment, morsel)?;
                let morsel_key = RowrefMorselKey {
                    file_ordinal,
                    segment_index,
                    segment_id: segment.segment_id,
                    morsel_id: row.morsel_id,
                    row_start,
                    row_count,
                };
                rows_by_morsel
                    .entry(morsel_key)
                    .or_default()
                    .insert(row_in_morsel);
            }
        }

        for (key, rows) in rows_by_morsel {
            if file_state.file(0)?.visibility().morsel_all_hidden(
                key.row_start,
                key.row_count,
                file_state.table().row_count,
            )? {
                graph.morsels_pruned += 1;
                continue;
            }
            graph.morsels_considered += 1;
            graph.tasks.push(ScanTask {
                file_ordinal: key.file_ordinal,
                segment_index: key.segment_index,
                segment_id: key.segment_id,
                morsel_id: key.morsel_id,
                row_start: key.row_start,
                row_count: key.row_count,
                split_id: None,
                cluster_id: None,
                output_columns: file_plan.column_plan.output_columns.clone(),
                predicate_columns: file_plan.column_plan.predicate_columns.clone(),
                row_selection: Some(rows.into_iter().collect()),
            });
        }
    }
    finalize_partitions(&mut graph, state.target_morsels_per_partition());
    Ok(Some(graph))
}

fn lookup_keys_for_plan(
    state: &DatasetState,
    plan: &ScanPlan,
) -> Option<(usize, LookupKeyKind, Vec<u64>)> {
    if plan.filters.len() != 1 {
        return None;
    }
    match plan.filters.first()?.predicate.as_ref()? {
        CovePredicate::FileCodeIn {
            column_index,
            file_codes,
            ..
        } => Some((
            *column_index,
            LookupKeyKind::FileCode,
            file_codes.iter().copied().map(u64::from).collect(),
        )),
        CovePredicate::Numeric {
            column_index,
            op: NumericPredicateOp::Eq,
            literal,
        } => {
            let column = state.table().columns.get(*column_index)?;
            let keys = numeric_lookup_keys(column.logical, *literal);
            if keys.is_empty() {
                return None;
            }
            Some((*column_index, LookupKeyKind::NumCode, keys))
        }
        _ => None,
    }
}

fn finalize_partitions(graph: &mut TaskGraph, target_morsels_per_partition: usize) {
    graph.partitions = partition_tasks(&graph.tasks, target_morsels_per_partition);
    if graph.partitions.is_empty() {
        graph.partitions.push(TaskPartition::default());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RowrefMorselKey {
    file_ordinal: usize,
    segment_index: usize,
    segment_id: u32,
    morsel_id: u32,
    row_start: u64,
    row_count: u32,
}

fn partition_tasks(tasks: &[ScanTask], target_morsels_per_partition: usize) -> Vec<TaskPartition> {
    let target = target_morsels_per_partition.max(1);
    if tasks.is_empty() {
        return Vec::new();
    }
    tasks
        .chunks(target)
        .map(|chunk| TaskPartition {
            tasks: chunk.to_vec(),
        })
        .collect()
}

fn segment_morsels_for_task_graph(
    file_state: &DatasetState,
    segment: &TableSegmentIndexEntryV1,
) -> Result<Vec<RowMorselEntryV1>, CoveError> {
    let file_bytes = if let Ok(bytes) = file_state.full_file_bytes() {
        Cow::Borrowed(bytes)
    } else if Path::new(file_state.source()).is_file() {
        Cow::Owned(std::fs::read(file_state.source())?)
    } else {
        Cow::Borrowed(file_state.full_file_bytes()?)
    };
    let segment_start = usize::try_from(segment.offset).map_err(|_| CoveError::OffsetRange)?;
    let segment_len = usize::try_from(segment.length).map_err(|_| CoveError::OffsetRange)?;
    let segment_bytes = wire::read_range_checked(file_bytes.as_ref(), segment_start, segment_len)?;
    let header = TableSegmentHeaderV1::parse(segment_bytes)?;
    if header.table_id != segment.table_id
        || header.segment_id != segment.segment_id
        || header.row_start != segment.row_start
        || header.row_count != segment.row_count
        || header.morsel_count != segment.morsel_count
        || header.morsel_row_count != segment.morsel_row_count
        || header.column_count != segment.column_count
    {
        return Err(CoveError::SegmentCorrupt);
    }
    let morsel_offset =
        usize::try_from(header.morsel_directory_offset).map_err(|_| CoveError::OffsetRange)?;
    if morsel_offset < TABLE_SEGMENT_HEADER_LEN || morsel_offset > segment_bytes.len() {
        return Err(CoveError::SegmentCorrupt);
    }
    let morsel_len = (header.morsel_count as usize)
        .checked_mul(ROW_MORSEL_ENTRY_LEN)
        .ok_or(CoveError::ArithOverflow)?;
    let morsel_end = morsel_offset
        .checked_add(morsel_len)
        .ok_or(CoveError::ArithOverflow)?;
    if morsel_end > segment_bytes.len() {
        return Err(CoveError::SegmentCorrupt);
    }
    let directory = RowMorselDirectory::parse(
        &segment_bytes[morsel_offset..morsel_end],
        header.morsel_count,
    )?;
    if directory.sum_rows() != u64::from(header.row_count) {
        return Err(CoveError::SegmentCorrupt);
    }
    Ok(directory.entries)
}

fn morsel_by_id(
    morsels: &[RowMorselEntryV1],
    morsel_id: u32,
) -> Result<&RowMorselEntryV1, CoveError> {
    morsels
        .iter()
        .find(|morsel| morsel.morsel_id == morsel_id)
        .ok_or(CoveError::SegmentCorrupt)
}

fn morsel_in_split_range(morsel: &RowMorselEntryV1, first_morsel_id: u32, morsel_end: u32) -> bool {
    morsel.morsel_id >= first_morsel_id && morsel.morsel_id < morsel_end
}

fn morsel_row_start(
    segment: &TableSegmentIndexEntryV1,
    morsel: &RowMorselEntryV1,
) -> Result<u64, CoveError> {
    segment
        .row_start
        .checked_add(u64::from(morsel.first_row_in_segment))
        .ok_or(CoveError::ArithOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cove_core::{
        compression,
        constants::{
            CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile, SectionKind,
        },
        index::lookup::{
            LookupEntry, LookupIndex, LookupIndexHeaderV1, LookupIndexKind, LookupKeyKind,
            LookupUniqueness,
        },
        page::{ColumnPageIndexEntryV1, COLUMN_PAGE_INDEX_ENTRY_LEN},
        reader,
        row_ref::RowRef,
        segment::{RowMorselEntryV1, TableSegmentHeaderV1, TableSegmentIndexEntryV1},
        table::{ColumnEntry, TableCatalog, TableEntry},
        writer::{
            MinimalCoveWriter, ScanPageSpec, ScanProfileCoveWriter, ScanSegment, SectionPayload,
        },
    };
    use cove_layout::build_default_scan_split_index;

    use crate::planner::{plan_scan, FilterPlan, PredicateLiteral};

    fn split_test_table() -> TableCatalog {
        TableCatalog {
            flags: 0,
            tables: vec![TableEntry {
                table_id: 1,
                namespace: "public".into(),
                name: "events".into(),
                row_count: 4,
                primary_sort_key_count: 0,
                clustering_key_count: 0,
                flags: 0,
                columns: vec![ColumnEntry {
                    column_id: 1,
                    name: "id".into(),
                    logical: CoveLogicalType::Int64,
                    physical: CovePhysicalKind::NumCode,
                    nullable: false,
                    sort_order: 0,
                    collation_id: 0,
                    precision: 0,
                    scale: 0,
                    flags: 0,
                }],
            }],
        }
    }

    fn one_segment_test_table() -> TableCatalog {
        let mut catalog = split_test_table();
        catalog.tables[0].row_count = 2;
        catalog
    }

    fn split_authority_segments() -> Vec<TableSegmentIndexEntryV1> {
        vec![
            TableSegmentIndexEntryV1 {
                table_id: 1,
                segment_id: 1,
                row_start: 0,
                row_count: 2,
                morsel_count: 2,
                morsel_row_count: 1,
                column_count: 1,
                offset: 0,
                length: 1,
                stats_ref: 0,
                flags: 0,
                checksum: 0,
            },
            TableSegmentIndexEntryV1 {
                table_id: 1,
                segment_id: 2,
                row_start: 2,
                row_count: 2,
                morsel_count: 2,
                morsel_row_count: 1,
                column_count: 1,
                offset: 0,
                length: 1,
                stats_ref: 0,
                flags: 0,
                checksum: 0,
            },
        ]
    }

    fn split_test_page(value: i64) -> ScanPageSpec {
        ScanPageSpec::new(1, (value as u64).to_le_bytes().to_vec())
            .with_encoding_root(CoveEncodingKind::NumCode as u32)
    }

    fn split_test_file(mut split_payload: Option<Vec<u8>>) -> Vec<u8> {
        let mut writer = ScanProfileCoveWriter::new(split_test_table());
        let mut first = ScanSegment::new(1, 1, 0, 2, 1);
        first.morsel_row_count = 1;
        first.set_column_pages(1, vec![split_test_page(1), split_test_page(2)]);
        writer.push_segment(first);

        let mut second = ScanSegment::new(1, 2, 2, 2, 1);
        second.morsel_row_count = 1;
        second.set_column_pages(1, vec![split_test_page(3), split_test_page(4)]);
        writer.push_segment(second);

        if let Some(data) = split_payload.take() {
            writer.push_extra_section(SectionPayload {
                section_kind: SectionKind::ScanSplitIndex as u16,
                profile: PrimaryProfile::LayoutPlanning as u8,
                flags: 0,
                item_count: 2,
                row_count: 4,
                compression: cove_core::constants::CompressionCodec::None as u8,
                alignment_log2: 0,
                required_features: 0,
                optional_features: cove_core::constants::FEATURE_SCAN_SPLIT_INDEX,
                data,
            });
        }

        writer.write().unwrap()
    }

    fn one_segment_two_morsel_file(with_lookup: bool) -> Vec<u8> {
        let mut writer = ScanProfileCoveWriter::new(one_segment_test_table());
        let mut segment = ScanSegment::new(1, 0, 0, 2, 1);
        segment.morsel_row_count = 1;
        segment.set_column_pages(1, vec![split_test_page(1), split_test_page(2)]);
        writer.push_segment(segment);
        if with_lookup {
            writer.push_extra_section(sparse_lookup_index_section());
        }
        writer.write().unwrap()
    }

    fn sparse_morsel_file(with_lookup: bool) -> Vec<u8> {
        rewrite_first_segment_data(one_segment_two_morsel_file(with_lookup), |data| {
            set_morsel_and_page_ids(data, &[10, 20]);
        })
    }

    fn sparse_lookup_index_section() -> SectionPayload {
        let index = LookupIndex {
            header: LookupIndexHeaderV1 {
                table_id: 1,
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
                    table_id: 1,
                    segment_id: 0,
                    morsel_id: 20,
                    row_in_morsel: 0,
                }],
            }],
        };
        SectionPayload {
            section_kind: SectionKind::LookupIndex as u16,
            profile: PrimaryProfile::ArchiveAcceleration as u8,
            flags: 0,
            item_count: 1,
            row_count: 2,
            compression: cove_core::constants::CompressionCodec::None as u8,
            alignment_log2: 0,
            required_features: 0,
            optional_features: 0,
            data: index.serialize().unwrap(),
        }
    }

    fn rewrite_first_segment_data<F>(bytes: Vec<u8>, mut mutate: F) -> Vec<u8>
    where
        F: FnMut(&mut Vec<u8>),
    {
        let validated = reader::validate_bytes(&bytes).unwrap();
        let mut writer = MinimalCoveWriter::new();
        writer.created_at_us = validated.header.created_at_us;
        writer.file_id = validated.header.file_id;
        writer.producer_scope_id = validated.header.producer_scope_id;
        writer.producer_scope_kind = validated.header.producer_scope_kind;
        writer.primary_profile = validated.header.primary_profile;
        writer.required_features = validated.header.required_features;
        writer.optional_features = validated.header.optional_features;
        writer.metadata_json = validated.footer.metadata_json.clone();

        let mut mutated = false;
        for entry in &validated.footer.sections {
            let mut data = compression::section_payload(&bytes, entry)
                .unwrap()
                .into_owned();
            if !mutated && entry.section_kind == SectionKind::TableSegmentData as u16 {
                mutate(&mut data);
                mutated = true;
            }
            writer.sections.push(SectionPayload {
                section_kind: entry.section_kind,
                profile: entry.profile,
                flags: entry.flags,
                item_count: entry.item_count,
                row_count: entry.row_count,
                compression: entry.compression,
                alignment_log2: entry.alignment_log2,
                required_features: entry.required_features,
                optional_features: entry.optional_features,
                data,
            });
        }
        assert!(
            mutated,
            "fixture should contain a table segment data section"
        );
        writer.write().unwrap()
    }

    fn set_morsel_and_page_ids(segment_data: &mut [u8], ids: &[u32]) {
        let header = TableSegmentHeaderV1::parse(segment_data).unwrap();
        assert_eq!(header.morsel_count as usize, ids.len());
        let morsel_dir = header.morsel_directory_offset as usize;
        for (index, morsel_id) in ids.iter().copied().enumerate() {
            let offset = morsel_dir + index * cove_core::segment::ROW_MORSEL_ENTRY_LEN;
            let mut entry = RowMorselEntryV1::parse(&segment_data[offset..]).unwrap();
            entry.morsel_id = morsel_id;
            segment_data[offset..offset + cove_core::segment::ROW_MORSEL_ENTRY_LEN]
                .copy_from_slice(&entry.serialize());
        }
        let page_index = header.page_index_offset as usize;
        for (index, morsel_id) in ids.iter().copied().enumerate() {
            let offset = page_index + index * COLUMN_PAGE_INDEX_ENTRY_LEN;
            let mut page = ColumnPageIndexEntryV1::parse(&segment_data[offset..]).unwrap();
            page.morsel_id = morsel_id;
            segment_data[offset..offset + COLUMN_PAGE_INDEX_ENTRY_LEN]
                .copy_from_slice(&page.serialize());
        }
    }

    #[test]
    fn valid_scan_splits_drive_one_partition_per_split() {
        let catalog = split_test_table();
        let table = &catalog.tables[0];
        let split_index = build_default_scan_split_index(table, &split_authority_segments())
            .unwrap()
            .serialize()
            .unwrap();
        let state =
            DatasetState::from_bytes("split-test", split_test_file(Some(split_index))).unwrap();
        let plan = plan_scan(&state, None, Vec::new()).unwrap();

        let graph = build_task_graph(&state, &plan).unwrap();

        assert_eq!(graph.partitions.len(), 2);
        assert_eq!(graph.scan_splits_used, 2);
        assert_eq!(graph.tasks.len(), 4);
        assert!(graph.partitions[0]
            .tasks
            .iter()
            .all(|task| task.split_id == Some(1)));
        assert!(graph.partitions[1]
            .tasks
            .iter()
            .all(|task| task.split_id == Some(2)));
    }

    #[test]
    fn invalid_optional_scan_splits_fall_back_to_default_partitioning() {
        let catalog = split_test_table();
        let table = &catalog.tables[0];
        let mut split_index =
            build_default_scan_split_index(table, &split_authority_segments()).unwrap();
        split_index.entries[0].row_count = 99;
        let state = DatasetState::from_bytes(
            "bad-split-test",
            split_test_file(Some(split_index.serialize().unwrap())),
        )
        .unwrap();
        let plan = plan_scan(&state, None, Vec::new()).unwrap();

        let graph = build_task_graph(&state, &plan).unwrap();

        assert_eq!(graph.partitions.len(), 1);
        assert_eq!(graph.scan_splits_used, 0);
        assert!(graph.tasks.iter().all(|task| task.split_id.is_none()));
        assert_eq!(state.bootstrap_stats().covel_sections_ignored, 1);
    }

    #[test]
    fn sparse_morsel_scan_uses_real_ids_and_directory_row_starts() {
        let state = DatasetState::from_bytes("sparse-morsels", sparse_morsel_file(false)).unwrap();
        let plan = plan_scan(&state, None, Vec::new()).unwrap();

        let graph = build_task_graph(&state, &plan).unwrap();

        assert_eq!(graph.tasks.len(), 2);
        assert_eq!(
            graph
                .tasks
                .iter()
                .map(|task| (task.morsel_id, task.row_start, task.row_count))
                .collect::<Vec<_>>(),
            vec![(10, 0, 1), (20, 1, 1)]
        );
    }

    #[test]
    fn sparse_morsel_split_filtering_uses_id_range() {
        let left = RowMorselEntryV1 {
            morsel_id: 10,
            first_row_in_segment: 0,
            row_count: 1,
            flags: 0,
            stats_ref: 0,
            checksum: 0,
        };
        let right = RowMorselEntryV1 {
            morsel_id: 20,
            first_row_in_segment: 1,
            row_count: 1,
            flags: 0,
            stats_ref: 0,
            checksum: 0,
        };

        assert!(!morsel_in_split_range(&left, 20, 21));
        assert!(morsel_in_split_range(&right, 20, 21));
    }

    #[test]
    fn lookup_rowref_sparse_morsel_uses_directory_row_start() {
        let state = DatasetState::from_bytes("sparse-lookup", sparse_morsel_file(true)).unwrap();
        let filter = FilterPlan::pruning_numeric(
            0,
            NumericPredicateOp::Eq,
            PredicateLiteral::Int64(2),
            "id = 2",
        );
        let plan = plan_scan(&state, None, vec![filter]).unwrap();

        let graph = build_task_graph(&state, &plan).unwrap();

        assert_eq!(graph.tasks.len(), 1);
        assert_eq!(graph.tasks[0].morsel_id, 20);
        assert_eq!(graph.tasks[0].row_start, 1);
        assert_eq!(graph.tasks[0].row_selection.as_deref(), Some(&[0][..]));
    }
}
