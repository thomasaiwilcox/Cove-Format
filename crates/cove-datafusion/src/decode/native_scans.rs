use super::*;

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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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

pub(super) fn local_filecode_groups_to_canonical(
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
