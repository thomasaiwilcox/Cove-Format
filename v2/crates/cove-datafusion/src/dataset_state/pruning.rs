use std::collections::BTreeMap;

use cove_core::{
    constants::CovePhysicalKind,
    domain::ColumnDomain,
    index::{
        aggregate::{AggregateEntry, SynopsisAccuracy, SynopsisKind},
        bloom::BloomFilterIndex,
        composite::{CompositeIndex, CompositeTransformKind},
        exact_set::ExactSetIndex,
        inverted::InvertedMorselIndex,
        lookup::LookupIndex,
        topn::TopNSummary,
    },
    segment::TableSegmentIndexEntryV1,
    zone_stats::ZoneStatsEntry,
    CoveError,
};

use super::{DatasetState, FileMetadata, PruningMetadata, TableEntry};

#[derive(Debug, Clone, Default)]
pub(super) struct PlanningCache {
    column_by_id: BTreeMap<u32, usize>,
    zone_stat_by_key: BTreeMap<(u32, u32, u32), (usize, usize)>,
    column_domain_by_id: BTreeMap<u32, usize>,
    exact_set_by_id: BTreeMap<u32, usize>,
    bloom_by_id: BTreeMap<u32, usize>,
    lookup_by_id: BTreeMap<u32, usize>,
    inverted_by_id: BTreeMap<u32, usize>,
}

impl PlanningCache {
    pub(super) fn build(table: &TableEntry, pruning: &PruningMetadata) -> Self {
        let mut cache = Self::default();
        for (index, column) in table.columns.iter().enumerate() {
            cache.column_by_id.entry(column.column_id).or_insert(index);
        }
        for (section_index, section) in pruning.zone_stats.iter().enumerate() {
            for (entry_index, entry) in section.entries.iter().enumerate() {
                if entry.table_id == table.table_id {
                    cache
                        .zone_stat_by_key
                        .entry((entry.segment_id, entry.morsel_id, entry.column_id))
                        .or_insert((section_index, entry_index));
                }
            }
        }
        for (index, domain) in pruning.column_domains.iter().enumerate() {
            if domain.header.table_or_object_id == table.table_id && domain.is_safe() {
                cache
                    .column_domain_by_id
                    .entry(domain.header.column_or_property_id)
                    .or_insert(index);
            }
        }
        for (index, exact_set) in pruning.exact_sets.iter().enumerate() {
            if exact_set.header.table_id == table.table_id {
                cache
                    .exact_set_by_id
                    .entry(exact_set.header.column_id)
                    .or_insert(index);
            }
        }
        for (index, bloom) in pruning.blooms.iter().enumerate() {
            if bloom.header.table_id == table.table_id {
                cache
                    .bloom_by_id
                    .entry(bloom.header.column_id)
                    .or_insert(index);
            }
        }
        for (index, lookup) in pruning.lookups.iter().enumerate() {
            if lookup.header.table_id == table.table_id {
                cache
                    .lookup_by_id
                    .entry(lookup.header.column_id)
                    .or_insert(index);
            }
        }
        for (index, inverted) in pruning.inverted.iter().enumerate() {
            if inverted.header.table_id == table.table_id {
                cache
                    .inverted_by_id
                    .entry(inverted.header.column_id)
                    .or_insert(index);
            }
        }
        cache
    }
}

impl DatasetState {
    #[inline]
    pub fn zone_stats_for(
        &self,
        segment_id: u32,
        morsel_id: u32,
        column_id: u32,
    ) -> Option<&ZoneStatsEntry> {
        let (section_index, entry_index) = self
            .planning_cache
            .zone_stat_by_key
            .get(&(segment_id, morsel_id, column_id))
            .copied()?;
        self.pruning
            .zone_stats
            .get(section_index)?
            .entries
            .get(entry_index)
    }

    pub fn segment_zone_stats_for(
        &self,
        segment_id: u32,
        column_id: u32,
    ) -> Option<&ZoneStatsEntry> {
        self.zone_stats_for(segment_id, u32::MAX, column_id)
    }

    #[inline]
    pub fn column_domain_for(&self, column_id: u32) -> Option<&ColumnDomain> {
        self.planning_cache
            .column_domain_by_id
            .get(&column_id)
            .and_then(|index| self.pruning.column_domains.get(*index))
    }

    #[inline]
    pub fn exact_set_for(&self, column_id: u32) -> Option<&ExactSetIndex> {
        self.planning_cache
            .exact_set_by_id
            .get(&column_id)
            .and_then(|index| self.pruning.exact_sets.get(*index))
    }

    #[inline]
    pub fn bloom_for(&self, column_id: u32) -> Option<&BloomFilterIndex> {
        self.planning_cache
            .bloom_by_id
            .get(&column_id)
            .and_then(|index| self.pruning.blooms.get(*index))
    }

    #[inline]
    pub fn lookup_for(&self, column_id: u32) -> Option<&LookupIndex> {
        self.planning_cache
            .lookup_by_id
            .get(&column_id)
            .and_then(|index| self.pruning.lookups.get(*index))
    }

    #[inline]
    pub fn inverted_for(&self, column_id: u32) -> Option<&InvertedMorselIndex> {
        self.planning_cache
            .inverted_by_id
            .get(&column_id)
            .and_then(|index| self.pruning.inverted.get(*index))
    }

    pub fn aggregate_entries_for(&self, column_id: u32) -> Vec<&AggregateEntry> {
        self.pruning
            .aggregates
            .iter()
            .flat_map(|synopsis| synopsis.entries.iter())
            .filter(|entry| entry.table_id == self.table.table_id && entry.column_id == column_id)
            .collect()
    }

    pub fn composite_indexes(&self) -> impl Iterator<Item = &CompositeIndex> {
        self.pruning.composites.iter().filter(|index| {
            index.header.table_id == self.table.table_id
                && index.header.transform_kind == CompositeTransformKind::Tuple
        })
    }

    pub fn topn_for(&self, column_id: u32) -> Vec<&TopNSummary> {
        self.pruning
            .topn
            .iter()
            .filter(|summary| {
                summary.table_id == self.table.table_id && summary.column_id == column_id
            })
            .collect()
    }

    pub fn exact_global_count(
        &self,
        column_index: Option<usize>,
    ) -> Result<Option<u64>, CoveError> {
        let mut total = 0u64;
        for file in self.files() {
            let visible = file.visibility().visible_count(file.table().row_count)?;
            if visible == 0 {
                continue;
            }
            match column_index {
                None => {
                    total = total.checked_add(visible).ok_or(CoveError::ArithOverflow)?;
                }
                Some(index) => {
                    let column = file.table().columns.get(index).ok_or_else(|| {
                        CoveError::BadSchema(format!(
                            "COUNT column index {index} is out of bounds for {} columns",
                            file.table().columns.len()
                        ))
                    })?;
                    if column.physical == CovePhysicalKind::FileCode && file_has_redaction(file) {
                        return Ok(None);
                    }
                    if !column.nullable {
                        total = total.checked_add(visible).ok_or(CoveError::ArithOverflow)?;
                        continue;
                    }
                    if !file.visibility().is_all() {
                        return Ok(None);
                    }
                    let Some(file_count) = exact_count_for_file_column(file, column.column_id)?
                    else {
                        return Ok(None);
                    };
                    total = total
                        .checked_add(file_count)
                        .ok_or(CoveError::ArithOverflow)?;
                }
            }
        }
        Ok(Some(total))
    }

    pub fn exact_visible_row_count(&self) -> Result<u64, CoveError> {
        self.exact_global_count(None)?
            .ok_or(CoveError::ArithOverflow)
    }
}

impl FileMetadata {
    pub fn has_redaction(&self) -> bool {
        file_has_redaction(self)
    }
}

fn file_has_redaction(file: &FileMetadata) -> bool {
    file.mounted()
        .reverse_lookup
        .as_ref()
        .map(|lookup| !lookup.redacted_filecodes.is_empty())
        .unwrap_or(false)
}

fn exact_count_for_file_column(
    file: &FileMetadata,
    column_id: u32,
) -> Result<Option<u64>, CoveError> {
    let table_id = file.table().table_id;
    let expected_rows = file.table().row_count;

    let candidates = file
        .pruning()
        .aggregates
        .iter()
        .flat_map(|synopsis| synopsis.entries.iter())
        .filter(|entry| {
            entry.table_id == table_id
                && entry.column_id == column_id
                && entry.synopsis_kind == SynopsisKind::Count
                && entry.accuracy == SynopsisAccuracy::Exact
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return Ok(None);
    }

    let file_level = candidates
        .iter()
        .copied()
        .filter(|entry| entry.segment_id == u32::MAX && entry.morsel_id == u32::MAX)
        .collect::<Vec<_>>();
    if !file_level.is_empty() {
        if file_level.len() == 1 && file_level[0].row_count as u64 == expected_rows {
            return entry_count(file_level[0]).map(Some);
        }
        return Ok(None);
    }

    let segment_level = candidates
        .iter()
        .copied()
        .filter(|entry| entry.segment_id != u32::MAX && entry.morsel_id == u32::MAX)
        .collect::<Vec<_>>();
    if !segment_level.is_empty() {
        return exact_segment_count(file.segments(), &segment_level);
    }

    let morsel_level = candidates
        .into_iter()
        .filter(|entry| entry.segment_id != u32::MAX && entry.morsel_id != u32::MAX)
        .collect::<Vec<_>>();
    exact_morsel_count(file.segments(), &morsel_level)
}

fn exact_segment_count(
    segments: &[TableSegmentIndexEntryV1],
    entries: &[&AggregateEntry],
) -> Result<Option<u64>, CoveError> {
    if entries.len() != segments.len() {
        return Ok(None);
    }

    let mut by_segment = BTreeMap::new();
    for entry in entries {
        if by_segment.insert(entry.segment_id, *entry).is_some() {
            return Ok(None);
        }
    }

    let mut count = 0u64;
    for segment in segments {
        let Some(entry) = by_segment.remove(&segment.segment_id) else {
            return Ok(None);
        };
        if entry.row_count != segment.row_count {
            return Ok(None);
        }
        count = count
            .checked_add(entry_count(entry)?)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok(Some(count))
}

fn exact_morsel_count(
    segments: &[TableSegmentIndexEntryV1],
    entries: &[&AggregateEntry],
) -> Result<Option<u64>, CoveError> {
    // Morsel IDs are identifiers, not guaranteed dense ordinals. The segment
    // index carries only morsel_count/morsel_row_count, so this metadata path
    // cannot prove full morsel-level coverage without the row-morsel directory.
    // Fail closed to segment/file-level aggregate coverage instead.
    let _ = (segments, entries);
    return Ok(None);
}

#[allow(dead_code)]
fn dense_morsel_count_for_tests(
    segments: &[TableSegmentIndexEntryV1],
    entries: &[&AggregateEntry],
) -> Result<Option<u64>, CoveError> {
    let expected = segments.iter().try_fold(0usize, |acc, segment| {
        let count = usize::try_from(segment.morsel_count).map_err(|_| CoveError::ArithOverflow)?;
        acc.checked_add(count).ok_or(CoveError::ArithOverflow)
    })?;
    if entries.len() != expected {
        return Ok(None);
    }

    let mut by_morsel = BTreeMap::new();
    for entry in entries {
        if by_morsel
            .insert((entry.segment_id, entry.morsel_id), *entry)
            .is_some()
        {
            return Ok(None);
        }
    }

    let mut count = 0u64;
    for segment in segments {
        for morsel_id in 0..segment.morsel_count {
            let Some(entry) = by_morsel.remove(&(segment.segment_id, morsel_id)) else {
                return Ok(None);
            };
            if entry.row_count != expected_morsel_row_count(segment, morsel_id)? {
                return Ok(None);
            }
            count = count
                .checked_add(entry_count(entry)?)
                .ok_or(CoveError::ArithOverflow)?;
        }
    }
    Ok(Some(count))
}

fn expected_morsel_row_count(
    segment: &TableSegmentIndexEntryV1,
    morsel_id: u32,
) -> Result<u32, CoveError> {
    if morsel_id >= segment.morsel_count || segment.morsel_row_count == 0 {
        return Err(CoveError::BadIndex);
    }
    let consumed = morsel_id
        .checked_mul(segment.morsel_row_count)
        .ok_or(CoveError::ArithOverflow)?;
    if consumed >= segment.row_count {
        return Err(CoveError::BadIndex);
    }
    Ok(segment.morsel_row_count.min(segment.row_count - consumed))
}

fn entry_count(entry: &AggregateEntry) -> Result<u64, CoveError> {
    entry
        .row_count
        .checked_sub(entry.null_count)
        .map(u64::from)
        .ok_or(CoveError::BadIndex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(segment_id: u32, row_start: u64, row_count: u32) -> TableSegmentIndexEntryV1 {
        TableSegmentIndexEntryV1 {
            table_id: 1,
            segment_id,
            row_start,
            row_count,
            morsel_count: 2,
            morsel_row_count: 4,
            column_count: 1,
            offset: 0,
            length: 0,
            stats_ref: 0,
            flags: 0,
            checksum: 0,
        }
    }

    fn count_entry(segment_id: u32, morsel_id: u32, row_count: u32) -> AggregateEntry {
        AggregateEntry::new(
            1,
            segment_id,
            morsel_id,
            1,
            SynopsisKind::Count,
            0,
            SynopsisAccuracy::Exact,
            row_count,
            1,
        )
        .unwrap()
    }

    #[test]
    fn count_segment_coverage_rejects_duplicates_even_when_rows_sum() {
        let segments = vec![segment(10, 0, 4), segment(20, 4, 4)];
        let first = count_entry(10, u32::MAX, 4);
        let duplicate = count_entry(10, u32::MAX, 4);
        let selected = exact_segment_count(&segments, &[&first, &duplicate]).unwrap();
        assert_eq!(selected, None);
    }

    #[test]
    fn count_segment_coverage_requires_every_segment_once() {
        let segments = vec![segment(10, 0, 4), segment(20, 4, 4)];
        let first = count_entry(10, u32::MAX, 4);
        let second = count_entry(20, u32::MAX, 4);
        let selected = exact_segment_count(&segments, &[&first, &second]).unwrap();
        assert_eq!(selected, Some(6));
    }

    #[test]
    fn count_morsel_coverage_rejects_missing_final_morsel() {
        let segments = vec![segment(10, 0, 6)];
        let first = count_entry(10, 0, 4);
        let selected = exact_morsel_count(&segments, &[&first]).unwrap();
        assert_eq!(selected, None);
    }

    #[test]
    fn count_morsel_coverage_fails_closed_without_morsel_directory() {
        let segments = vec![segment(10, 0, 6)];
        let first = count_entry(10, 0, 4);
        let second = count_entry(10, 1, 2);
        let selected = exact_morsel_count(&segments, &[&first, &second]).unwrap();
        assert_eq!(selected, None);
    }
}
