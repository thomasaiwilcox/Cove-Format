use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeNullOrder {
    First,
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeDenseGroupCounts {
    pub counts: Vec<u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeHashGroupCounts {
    pub counts: HashMap<u32, u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64HashGroupCounts {
    pub counts: HashMap<i64, u64>,
    pub null_count: u64,
    pub rows_grouped: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeU32I64HashGroupAggregates {
    pub aggregates: HashMap<u32, NativeI64Aggregates>,
    pub row_counts: HashMap<u32, u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64I64HashGroupAggregates {
    pub aggregates: HashMap<i64, NativeI64Aggregates>,
    pub row_counts: HashMap<i64, u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeInnerJoinPairs {
    pub left_rows: Vec<u32>,
    pub right_rows: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeI64Aggregates {
    pub count: u64,
    pub null_count: u64,
    pub sum: i128,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeDenseI64GroupAggregates {
    pub aggregates: Vec<NativeI64Aggregates>,
    pub row_counts: Vec<u64>,
    pub null_aggregate: NativeI64Aggregates,
    pub null_row_count: u64,
}

pub fn group_count_u32_dense(
    values: &[u32],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::try_from(value).map_err(|_| CoveError::ArithOverflow)?;
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u8_dense(
    values: &[u8],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::from(value);
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u16_dense(
    values: &[u16],
    validity: ValidityRef<'_>,
    group_count: usize,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; group_count],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        let group = usize::from(value);
        let Some(count) = groups.counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {value} is outside declared group count {group_count}"
            )));
        };
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped += 1;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_bool(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
    validate_bool_bytes(values, row_count, validity)?;
    let mut groups = NativeDenseGroupCounts {
        counts: vec![0; 2],
        ..NativeDenseGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        stats.rows_valid += 1;
        let value = values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        let count = groups
            .counts
            .get_mut(usize::from(value))
            .ok_or(CoveError::PageCorrupt)?;
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn group_count_u32_hash(
    values: &[u32],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeHashGroupCounts, KernelStats), CoveError> {
    let capacity = base.map_or(values.len(), SelectionBitmap::count_ones);
    let mut groups = NativeHashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(values.len())),
        ..NativeHashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn group_count_u32_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeHashGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeHashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeHashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        let offset = row * std::mem::size_of::<u32>();
        let value = wire::read_u32_le_checked(bytes, offset)?;
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn group_count_i64_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64HashGroupCounts, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64HashGroupCounts {
        counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64HashGroupCounts::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            groups.null_count = groups
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
            continue;
        }
        let offset = row * std::mem::size_of::<u64>();
        let value = wire::read_i64_le_checked(bytes, offset)?;
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        let count = groups.counts.entry(value).or_default();
        *count = count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups.rows_grouped = groups
            .rows_grouped
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
    }
    Ok((groups, stats))
}

pub fn distinct_u32(
    values: &[u32],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(Vec<u32>, KernelStats), CoveError> {
    let mut seen = HashSet::with_capacity(base.map_or(values.len(), SelectionBitmap::count_ones));
    let mut distinct = Vec::new();
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if seen.insert(value) {
            distinct.push(value);
            stats.rows_matched += 1;
        }
    }
    Ok((distinct, stats))
}

pub fn inner_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code inner join requires a proven shared code equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

pub fn inner_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code inner join requires a proven shared code equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

pub fn inner_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 inner join requires a proven shared equality domain".into(),
        ));
    }
    inner_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
    )
}

#[derive(Debug, Clone, Copy)]
struct NativeJoinBucket {
    start: usize,
    len: usize,
    filled: usize,
}

pub(super) fn inner_join_eq<K>(
    left_values: &[K],
    left_validity: ValidityRef<'_>,
    right_values: &[K],
    right_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeInnerJoinPairs, KernelStats), CoveError>
where
    K: Copy + Eq + Hash,
{
    let mut counts = HashMap::<K, usize>::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            *counts.entry(value).or_default() += 1;
        }
    }

    let mut buckets = HashMap::<K, NativeJoinBucket>::with_capacity(counts.len());
    let mut right_row_ids = Vec::<u32>::new();
    for (value, len) in counts {
        let start = right_row_ids.len();
        right_row_ids.resize(start + len, 0);
        buckets.insert(
            value,
            NativeJoinBucket {
                start,
                len,
                filled: 0,
            },
        );
    }

    for (row, value) in right_values.iter().copied().enumerate() {
        if !right_validity.is_valid(row) {
            continue;
        }
        let Some(bucket) = buckets.get_mut(&value) else {
            return Err(CoveError::PageCorrupt);
        };
        let slot = bucket
            .start
            .checked_add(bucket.filled)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(target) = right_row_ids.get_mut(slot) else {
            return Err(CoveError::PageCorrupt);
        };
        *target = u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        bucket.filled += 1;
    }

    let mut pairs = NativeInnerJoinPairs::default();
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len()) * std::mem::size_of::<K>()
            + right_row_ids.len() * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (left_row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(left_row)) {
            continue;
        }
        if !left_validity.is_valid(left_row) {
            continue;
        }
        stats.rows_valid += 1;
        let Some(bucket) = buckets.get(&value) else {
            continue;
        };
        debug_assert_eq!(bucket.filled, bucket.len);
        let left_row = u32::try_from(left_row).map_err(|_| CoveError::ArithOverflow)?;
        let end = bucket
            .start
            .checked_add(bucket.len)
            .ok_or(CoveError::ArithOverflow)?;
        let right_matches = right_row_ids
            .get(bucket.start..end)
            .ok_or(CoveError::PageCorrupt)?;
        pairs.left_rows.reserve(right_matches.len());
        pairs.right_rows.reserve(right_matches.len());
        for right_row in right_matches {
            pairs.left_rows.push(left_row);
            pairs.right_rows.push(*right_row);
        }
        stats.rows_matched = stats
            .rows_matched
            .checked_add(right_matches.len())
            .ok_or(CoveError::ArithOverflow)?;
    }
    stats.bytes_touched_estimate += pairs.left_rows.len() * 2 * std::mem::size_of::<u32>();
    Ok((pairs, stats))
}

pub fn aggregate_i64(
    values: &[i64],
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> (NativeI64Aggregates, KernelStats) {
    let mut aggregates = NativeI64Aggregates::default();
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            aggregates.null_count += 1;
            continue;
        }
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        aggregates.count += 1;
        aggregates.sum += i128::from(value);
        aggregates.min = Some(aggregates.min.map_or(value, |min| min.min(value)));
        aggregates.max = Some(aggregates.max.map_or(value, |max| max.max(value)));
    }
    (aggregates, stats)
}

pub fn aggregate_i64_le_bytes(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64Aggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    let mut aggregates = NativeI64Aggregates::default();
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            aggregates.null_count += 1;
            continue;
        }
        let offset = row * std::mem::size_of::<u64>();
        let value = wire::read_i64_le_checked(bytes, offset)?;
        stats.rows_valid += 1;
        stats.rows_matched += 1;
        aggregates.count = aggregates
            .count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        aggregates.sum = aggregates
            .sum
            .checked_add(i128::from(value))
            .ok_or(CoveError::ArithOverflow)?;
        aggregates.min = Some(aggregates.min.map_or(value, |min| min.min(value)));
        aggregates.max = Some(aggregates.max.map_or(value, |max| max.max(value)));
    }
    Ok((aggregates, stats))
}

pub fn aggregate_i64_by_bool(
    key_values: &[u8],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    validate_fixed_le_width_for_validity(key_values, row_count, 1, key_validity)?;
    validate_bool_bytes(key_values, row_count, key_validity)?;
    if value_validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); 2],
        row_counts: vec![0; 2],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (1 + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in value_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_bool_group_for_row(key_values, key_validity, &mut groups, row)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_bool(
    key_values: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(key_values, row_count, 1, key_validity)?;
    validate_bool_bytes(key_values, row_count, key_validity)?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); 2],
        row_counts: vec![0; 2],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (1 + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_bool_group_for_row(key_values, key_validity, &mut groups, row)?;
        if value_validity.is_valid(row) {
            let offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = wire::read_i64_le_checked(value_bytes, offset)?;
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_by_u8_dense(
    key_values: &[u8],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_by_u16_dense(
    key_values: &[u16],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_by_u32_dense(
    key_values: &[u32],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_by_dense_key(
        key_values,
        value_values,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub(super) fn aggregate_i64_by_dense_key<K>(
    key_values: &[K],
    value_values: &[i64],
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError>
where
    K: Copy + TryInto<usize>,
{
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); group_count],
        row_counts: vec![0; group_count],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (std::mem::size_of::<K>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_key_group_for_row(
            &mut groups,
            key_validity,
            row,
            key.try_into().map_err(|_| CoveError::ArithOverflow)?,
        )?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_u8_dense(
    key_values: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_le_bytes_by_u16_dense(
    key_values: &[u16],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub fn aggregate_i64_le_bytes_by_u32_dense(
    key_values: &[u32],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError> {
    aggregate_i64_le_bytes_by_dense_key(
        key_values,
        value_bytes,
        row_count,
        group_count,
        key_validity,
        value_validity,
        base,
    )
}

pub(super) fn aggregate_i64_le_bytes_by_dense_key<K>(
    key_values: &[K],
    value_bytes: &[u8],
    row_count: usize,
    group_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeDenseI64GroupAggregates, KernelStats), CoveError>
where
    K: Copy + TryInto<usize>,
{
    if key_values.len() != row_count || key_validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let mut groups = NativeDenseI64GroupAggregates {
        aggregates: vec![NativeI64Aggregates::default(); group_count],
        row_counts: vec![0; group_count],
        ..NativeDenseI64GroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count * (std::mem::size_of::<K>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, key) in key_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = dense_key_group_for_row(
            &mut groups,
            key_validity,
            row,
            key.try_into().map_err(|_| CoveError::ArithOverflow)?,
        )?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = wire::read_i64_le_checked(value_bytes, value_offset)?;
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub(super) fn dense_bool_group_for_row<'a>(
    key_values: &[u8],
    key_validity: ValidityRef<'_>,
    groups: &'a mut NativeDenseI64GroupAggregates,
    row: usize,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let key = key_values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        let group = usize::from(key);
        let row_count = groups
            .row_counts
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)?;
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups
            .aggregates
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub(super) fn dense_key_group_for_row<'a>(
    groups: &'a mut NativeDenseI64GroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    group: usize,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let declared_group_count = groups.row_counts.len();
        let Some(row_count) = groups.row_counts.get_mut(group) else {
            return Err(CoveError::BadSchema(format!(
                "dense group code {group} is outside declared group count {declared_group_count}"
            )));
        };
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        groups
            .aggregates
            .get_mut(group)
            .ok_or(CoveError::PageCorrupt)
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub(super) fn accumulate_i64(
    aggregate: &mut NativeI64Aggregates,
    value: i64,
) -> Result<(), CoveError> {
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

pub fn aggregate_i64_by_i64(
    key_values: &[i64],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64I64HashGroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<i64>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = i64_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_i64_le_bytes(
    key_bytes: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeI64I64HashGroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(
        key_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        key_validity,
    )?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeI64I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeI64I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u64>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let key_offset = row
            .checked_mul(std::mem::size_of::<u64>())
            .ok_or(CoveError::ArithOverflow)?;
        let key = wire::read_i64_le_checked(key_bytes, key_offset)?;
        let aggregate = i64_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = wire::read_i64_le_checked(value_bytes, value_offset)?;
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub(super) fn i64_group_for_row<'a>(
    groups: &'a mut NativeI64I64HashGroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    key: i64,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let row_count = groups.row_counts.entry(key).or_default();
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        Ok(groups.aggregates.entry(key).or_default())
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub fn aggregate_i64_by_u32(
    key_values: &[u32],
    value_values: &[i64],
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeU32I64HashGroupAggregates, KernelStats), CoveError> {
    let row_count = value_values.len();
    if key_values.len() != row_count
        || key_validity.row_count() != row_count
        || value_validity.row_count() != row_count
    {
        return Err(CoveError::PageCorrupt);
    }
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeU32I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeU32I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u32>() + std::mem::size_of::<i64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, (key, value)) in key_values
        .iter()
        .copied()
        .zip(value_values.iter().copied())
        .enumerate()
    {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let aggregate = u32_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub fn aggregate_i64_le_bytes_by_u32_le_bytes(
    key_bytes: &[u8],
    value_bytes: &[u8],
    row_count: usize,
    key_validity: ValidityRef<'_>,
    value_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
) -> Result<(NativeU32I64HashGroupAggregates, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(
        key_bytes,
        row_count,
        std::mem::size_of::<u32>(),
        key_validity,
    )?;
    validate_fixed_le_width_for_validity(
        value_bytes,
        row_count,
        std::mem::size_of::<u64>(),
        value_validity,
    )?;
    let capacity = base.map_or(row_count, SelectionBitmap::count_ones);
    let mut groups = NativeU32I64HashGroupAggregates {
        aggregates: HashMap::with_capacity(capacity.min(row_count)),
        row_counts: HashMap::with_capacity(capacity.min(row_count)),
        ..NativeU32I64HashGroupAggregates::default()
    };
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64) * 2,
        bytes_touched_estimate: row_count
            * (std::mem::size_of::<u32>() + std::mem::size_of::<u64>()),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        let key_offset = row
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or(CoveError::ArithOverflow)?;
        let key = wire::read_u32_le_checked(key_bytes, key_offset)?;
        let aggregate = u32_group_for_row(&mut groups, key_validity, row, key)?;
        if value_validity.is_valid(row) {
            let value_offset = row
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or(CoveError::ArithOverflow)?;
            let value = wire::read_i64_le_checked(value_bytes, value_offset)?;
            stats.rows_valid += 1;
            accumulate_i64(aggregate, value)?;
        } else {
            aggregate.null_count = aggregate
                .null_count
                .checked_add(1)
                .ok_or(CoveError::ArithOverflow)?;
        }
        stats.rows_matched += 1;
    }
    Ok((groups, stats))
}

pub(super) fn u32_group_for_row<'a>(
    groups: &'a mut NativeU32I64HashGroupAggregates,
    key_validity: ValidityRef<'_>,
    row: usize,
    key: u32,
) -> Result<&'a mut NativeI64Aggregates, CoveError> {
    if key_validity.is_valid(row) {
        let row_count = groups.row_counts.entry(key).or_default();
        *row_count = row_count.checked_add(1).ok_or(CoveError::ArithOverflow)?;
        Ok(groups.aggregates.entry(key).or_default())
    } else {
        groups.null_row_count = groups
            .null_row_count
            .checked_add(1)
            .ok_or(CoveError::ArithOverflow)?;
        Ok(&mut groups.null_aggregate)
    }
}

pub fn sort_rows_i64(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
) -> Result<SelectionVector, CoveError> {
    sort_rows_i64_with_stats(values, validity, direction, null_order, base)
        .map(|(selection, _stats)| selection)
}

pub fn sort_rows_i64_with_stats(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionVector, KernelStats), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut rows = Vec::with_capacity(base.map_or(values.len(), SelectionBitmap::count_ones));
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..values.len() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            stats.rows_valid += 1;
        }
        stats.rows_matched += 1;
        rows.push(u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?);
    }
    stats.bytes_touched_estimate = stats
        .bytes_touched_estimate
        .checked_add(
            rows.len()
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(CoveError::ArithOverflow)?,
        )
        .ok_or(CoveError::ArithOverflow)?;
    rows.sort_by(|left, right| {
        compare_i64_rows(
            values,
            validity,
            *left as usize,
            *right as usize,
            direction,
            null_order,
        )
    });
    Ok((SelectionVector::from_rows(rows), stats))
}

pub fn top_n_rows_i64_with_stats(
    values: &[i64],
    validity: ValidityRef<'_>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
    base: Option<&SelectionBitmap>,
    limit: usize,
) -> Result<(SelectionVector, KernelStats), CoveError> {
    if validity.row_count() != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if limit == 0 {
        return Ok((SelectionVector::new(), stats));
    }

    let mut heap = BinaryHeap::<I64TopRow<'_>>::with_capacity(limit);
    for row in 0..values.len() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if validity.is_valid(row) {
            stats.rows_valid += 1;
        }
        stats.rows_matched += 1;

        let row = u32::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
        let item = I64TopRow {
            row,
            values,
            validity,
            direction,
            null_order,
        };
        if heap.len() < limit {
            heap.push(item);
            continue;
        }
        let candidate_beats_worst = heap
            .peek()
            .is_some_and(|worst| item.desired_cmp(worst) == Ordering::Less);
        if candidate_beats_worst {
            heap.pop();
            heap.push(item);
        }
    }

    let mut rows = heap.into_iter().map(|row| row.row).collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        compare_i64_rows(
            values,
            validity,
            *left as usize,
            *right as usize,
            direction,
            null_order,
        )
    });
    stats.bytes_touched_estimate = stats
        .bytes_touched_estimate
        .checked_add(
            rows.len()
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(CoveError::ArithOverflow)?,
        )
        .ok_or(CoveError::ArithOverflow)?;
    Ok((SelectionVector::from_rows(rows), stats))
}

#[derive(Clone, Copy)]
struct I64TopRow<'a> {
    row: u32,
    values: &'a [i64],
    validity: ValidityRef<'a>,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
}

impl I64TopRow<'_> {
    fn desired_cmp(&self, other: &Self) -> Ordering {
        compare_i64_rows(
            self.values,
            self.validity,
            self.row as usize,
            other.row as usize,
            self.direction,
            self.null_order,
        )
    }
}

impl PartialEq for I64TopRow<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl Eq for I64TopRow<'_> {}

impl PartialOrd for I64TopRow<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for I64TopRow<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.desired_cmp(other)
    }
}

pub fn semi_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code semi-join requires a proven shared code equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<u32>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn anti_join_u32_eq(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_u32_eq_left_nulls_unmatched(
    left_values: &[u32],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u32],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u32 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

pub fn semi_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code semi-join requires a proven shared code equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn semi_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 semi-join requires a proven shared equality domain".into(),
        ));
    }
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len())
            * std::mem::size_of::<i64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn anti_join_u64_eq(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_i64_eq(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 anti-join requires a proven shared equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        false,
    )
}

pub fn anti_join_i64_eq_left_nulls_unmatched(
    left_values: &[i64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[i64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "i64 anti-join requires a proven shared equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

pub fn anti_join_u64_eq_left_nulls_unmatched(
    left_values: &[u64],
    left_validity: ValidityRef<'_>,
    left_domain: &NativeCodeDomain,
    right_values: &[u64],
    right_validity: ValidityRef<'_>,
    right_domain: &NativeCodeDomain,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if !left_domain.code_equality_compatible(right_domain) {
        return Err(CoveError::BadSchema(
            "u64 code anti-join requires a proven shared code equality domain".into(),
        ));
    }
    anti_join_eq(
        left_values,
        left_validity,
        right_values,
        right_validity,
        base,
        true,
    )
}

pub(super) fn anti_join_eq<K>(
    left_values: &[K],
    left_validity: ValidityRef<'_>,
    right_values: &[K],
    right_validity: ValidityRef<'_>,
    base: Option<&SelectionBitmap>,
    left_nulls_unmatched: bool,
) -> Result<(SelectionBitmap, KernelStats), CoveError>
where
    K: Copy + Eq + Hash,
{
    let mut build = HashSet::with_capacity(right_values.len());
    for (row, value) in right_values.iter().copied().enumerate() {
        if right_validity.is_valid(row) {
            build.insert(value);
        }
    }

    let mut out = SelectionBitmap::none(left_values.len());
    let mut stats = KernelStats {
        rows_seen: left_values.len(),
        bitmap_words_touched: left_values.len().div_ceil(64),
        bytes_touched_estimate: (left_values.len() + right_values.len()) * std::mem::size_of::<K>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, value) in left_values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !left_validity.is_valid(row) {
            if left_nulls_unmatched {
                out.set(row);
                stats.rows_matched += 1;
            }
            continue;
        }
        stats.rows_valid += 1;
        if !build.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub(super) fn compare_bound(ordering: Ordering, inclusive: BoundInclusive, upper: bool) -> bool {
    !matches!(
        (upper, inclusive, ordering),
        (false, BoundInclusive::Inclusive, Ordering::Less)
            | (
                false,
                BoundInclusive::Exclusive,
                Ordering::Less | Ordering::Equal
            )
            | (true, BoundInclusive::Inclusive, Ordering::Greater)
            | (
                true,
                BoundInclusive::Exclusive,
                Ordering::Greater | Ordering::Equal
            )
    )
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NativeTypedNumericValue {
    Signed(i128),
    Unsigned(u128),
    Float(f64),
}

const I64_MAX_PLUS_ONE_AS_F64: f64 = 9_223_372_036_854_775_808.0;
const U64_MAX_PLUS_ONE_AS_F64: f64 = 18_446_744_073_709_551_616.0;

pub(super) fn compare_native_numcode_value(
    logical_type: CoveLogicalType,
    value: u64,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, CoveError> {
    let Some(value) = typed_native_numcode_value(logical_type, value) else {
        return Err(CoveError::UnsupportedEncoding(format!(
            "numeric predicate for {logical_type:?} NumCode"
        )));
    };
    Ok(compare_native_typed_numeric_value(value, op, literal))
}

pub fn native_numcode_matches(
    logical_type: CoveLogicalType,
    value: u64,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> Result<bool, CoveError> {
    compare_native_numcode_value(logical_type, value, op, literal)
}

pub(super) fn typed_native_numcode_value(
    logical_type: CoveLogicalType,
    value: u64,
) -> Option<NativeTypedNumericValue> {
    use crate::types;

    let value = match logical_type {
        CoveLogicalType::Bool => match value {
            0 => NativeTypedNumericValue::Unsigned(0),
            1 => NativeTypedNumericValue::Unsigned(1),
            _ => return None,
        },
        CoveLogicalType::Int8 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i8(value) as i128)
        }
        CoveLogicalType::Int16 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i16(value) as i128)
        }
        CoveLogicalType::Int32 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i32(value) as i128)
        }
        CoveLogicalType::Int64 => {
            NativeTypedNumericValue::Signed(types::numcode_as_i64(value) as i128)
        }
        CoveLogicalType::UInt8 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u8(value) as u128)
        }
        CoveLogicalType::UInt16 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u16(value) as u128)
        }
        CoveLogicalType::UInt32 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u32(value) as u128)
        }
        CoveLogicalType::UInt64 => {
            NativeTypedNumericValue::Unsigned(types::numcode_as_u64(value) as u128)
        }
        CoveLogicalType::Float32 => {
            let value = types::numcode_as_f32(value);
            if value.is_nan() {
                return Some(NativeTypedNumericValue::Float(f64::NAN));
            }
            NativeTypedNumericValue::Float(f64::from(value))
        }
        CoveLogicalType::Float64 => {
            let value = types::numcode_as_f64(value);
            if value.is_nan() {
                return Some(NativeTypedNumericValue::Float(f64::NAN));
            }
            NativeTypedNumericValue::Float(value)
        }
        CoveLogicalType::Decimal64 => {
            NativeTypedNumericValue::Signed(types::numcode_as_decimal64(value) as i128)
        }
        CoveLogicalType::DateDays => {
            NativeTypedNumericValue::Signed(types::numcode_as_date_days(value) as i128)
        }
        CoveLogicalType::TimestampMicros => {
            NativeTypedNumericValue::Signed(types::numcode_as_timestamp_micros(value) as i128)
        }
        CoveLogicalType::TimestampNanos => {
            NativeTypedNumericValue::Signed(types::numcode_as_timestamp_nanos(value) as i128)
        }
        _ => return None,
    };
    Some(value)
}

pub(super) fn compare_native_typed_numeric_value(
    value: NativeTypedNumericValue,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match value {
        NativeTypedNumericValue::Signed(value) => {
            compare_native_signed_numeric_value(value, op, literal)
        }
        NativeTypedNumericValue::Unsigned(value) => {
            compare_native_unsigned_numeric_value(value, op, literal)
        }
        NativeTypedNumericValue::Float(value) => {
            let Some(literal) = native_literal_as_f64(literal) else {
                return false;
            };
            if value.is_nan() || literal.is_nan() {
                false
            } else {
                compare_native_ordered(value, op, literal)
            }
        }
    }
}

pub(super) fn compare_native_signed_numeric_value(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match literal {
        NativeNumericLiteral::Int64(literal) => compare_native_ordered(value, op, literal as i128),
        NativeNumericLiteral::UInt64(literal) => {
            compare_native_signed_unsigned(value, op, literal as u128)
        }
        NativeNumericLiteral::Float64(literal) => {
            compare_native_signed_float_literal(value, op, literal)
        }
    }
}

pub(super) fn compare_native_unsigned_numeric_value(
    value: u128,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
) -> bool {
    match literal {
        NativeNumericLiteral::Int64(literal) if literal < 0 => match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        },
        NativeNumericLiteral::Int64(literal) => compare_native_ordered(value, op, literal as u128),
        NativeNumericLiteral::UInt64(literal) => compare_native_ordered(value, op, literal as u128),
        NativeNumericLiteral::Float64(literal) => {
            compare_native_unsigned_float_literal(value, op, literal)
        }
    }
}

pub(super) fn compare_native_signed_float_literal(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: f64,
) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_i64_exact(literal) {
        return compare_native_ordered(value, op, rhs as i128);
    }
    if literal < i64::MIN as f64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        };
    }
    if literal >= I64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as i128;
    let ceil = literal.ceil() as i128;
    compare_native_integer_to_fractional_bounds(value, op, floor, ceil)
}

pub(super) fn compare_native_unsigned_float_literal(
    value: u128,
    op: NativeNumericPredicateOp,
    literal: f64,
) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_u64_exact(literal) {
        return compare_native_ordered(value, op, rhs as u128);
    }
    if literal < 0.0 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Lt
            | NativeNumericPredicateOp::LtEq => false,
            NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => true,
        };
    }
    if literal >= U64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as u128;
    let ceil = literal.ceil() as u128;
    compare_native_integer_to_fractional_bounds(value, op, floor, ceil)
}

pub(super) fn compare_native_integer_to_fractional_bounds<T>(
    value: T,
    op: NativeNumericPredicateOp,
    floor: T,
    ceil: T,
) -> bool
where
    T: PartialOrd,
{
    match op {
        NativeNumericPredicateOp::Eq => false,
        NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => value <= floor,
        NativeNumericPredicateOp::Gt | NativeNumericPredicateOp::GtEq => value >= ceil,
    }
}

pub(super) fn compare_native_signed_unsigned(
    value: i128,
    op: NativeNumericPredicateOp,
    literal: u128,
) -> bool {
    if value < 0 {
        return match op {
            NativeNumericPredicateOp::Eq
            | NativeNumericPredicateOp::Gt
            | NativeNumericPredicateOp::GtEq => false,
            NativeNumericPredicateOp::Lt | NativeNumericPredicateOp::LtEq => true,
        };
    }
    compare_native_ordered(value as u128, op, literal)
}

pub(super) fn compare_native_ordered<T: PartialOrd + PartialEq>(
    left: T,
    op: NativeNumericPredicateOp,
    right: T,
) -> bool {
    match op {
        NativeNumericPredicateOp::Eq => left == right,
        NativeNumericPredicateOp::Lt => left < right,
        NativeNumericPredicateOp::LtEq => left <= right,
        NativeNumericPredicateOp::Gt => left > right,
        NativeNumericPredicateOp::GtEq => left >= right,
    }
}

pub(super) fn native_literal_as_f64(literal: NativeNumericLiteral) -> Option<f64> {
    match literal {
        NativeNumericLiteral::Int64(value) => Some(value as f64),
        NativeNumericLiteral::UInt64(value) => Some(value as f64),
        NativeNumericLiteral::Float64(value) if !value.is_nan() => Some(value),
        NativeNumericLiteral::Float64(_) => None,
    }
}

#[inline]
pub(super) fn f64_to_i64_exact(value: f64) -> Option<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value >= I64_MAX_PLUS_ONE_AS_F64
    {
        return None;
    }
    let candidate = value as i64;
    ((candidate as f64) == value).then_some(candidate)
}

#[inline]
pub(super) fn f64_to_u64_exact(value: f64) -> Option<u64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..U64_MAX_PLUS_ONE_AS_F64).contains(&value)
    {
        return None;
    }
    let candidate = value as u64;
    ((candidate as f64) == value).then_some(candidate)
}

pub(super) fn compare_i64_rows(
    values: &[i64],
    validity: ValidityRef<'_>,
    left: usize,
    right: usize,
    direction: NativeSortDirection,
    null_order: NativeNullOrder,
) -> Ordering {
    let left_valid = validity.is_valid(left);
    let right_valid = validity.is_valid(right);
    match (left_valid, right_valid) {
        (false, false) => left.cmp(&right),
        (false, true) => match null_order {
            NativeNullOrder::First => Ordering::Less,
            NativeNullOrder::Last => Ordering::Greater,
        },
        (true, false) => match null_order {
            NativeNullOrder::First => Ordering::Greater,
            NativeNullOrder::Last => Ordering::Less,
        },
        (true, true) => {
            let ordering = values[left]
                .cmp(&values[right])
                .then_with(|| left.cmp(&right));
            match direction {
                NativeSortDirection::Ascending => ordering,
                NativeSortDirection::Descending => ordering.reverse(),
            }
        }
    }
}
