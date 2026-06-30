use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeNumericPredicateOp {
    Eq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeNumericLiteral {
    Int64(i64),
    UInt64(u64),
    Float64(f64),
}

pub fn filter_numcode_le_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    op: NativeNumericPredicateOp,
    literal: NativeNumericLiteral,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if matches!(logical_type, CoveLogicalType::Int64) {
        if let NativeNumericLiteral::Int64(needle) = literal {
            if let Some((out, dispatch)) = try_filter_i64_le_cmp_simd(
                bytes,
                row_count,
                validity,
                op,
                needle,
                base,
                NativeKernelDispatch::Auto,
            ) {
                let rows_matched = out.count_ones();
                return Ok((
                    out,
                    KernelStats {
                        rows_seen: row_count,
                        rows_valid: row_count,
                        rows_matched,
                        bitmap_words_touched: row_count.div_ceil(64),
                        bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
                        dispatch,
                    },
                ));
            }
        }
    }
    let mut out = SelectionBitmap::none(row_count);
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
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = wire::read_u64_le_checked(bytes, offset)?;
        if compare_native_numcode_value(logical_type, value, op, literal)? {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_numcode_le_in_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if let [literal] = literals {
        return filter_numcode_le_typed(
            bytes,
            row_count,
            validity,
            logical_type,
            NativeNumericPredicateOp::Eq,
            *literal,
            base,
        );
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<u64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if literals.is_empty() {
        return Ok((out, stats));
    }
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = wire::read_u64_le_checked(bytes, offset)?;
        let mut matched = false;
        for literal in literals {
            if compare_native_numcode_value(
                logical_type,
                value,
                NativeNumericPredicateOp::Eq,
                *literal,
            )? {
                matched = true;
                break;
            }
        }
        if matched {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_numcode_le_not_in_typed(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    logical_type: CoveLogicalType,
    literals: &[NativeNumericLiteral],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u64>(), validity)?;
    if base.is_none() {
        if let [literal] = literals {
            let (matched, mut stats) = filter_numcode_le_typed(
                bytes,
                row_count,
                validity,
                logical_type,
                NativeNumericPredicateOp::Eq,
                *literal,
                None,
            )?;
            let selected = valid_rows_except(row_count, validity, &matched);
            stats.rows_matched = selected.count_ones();
            return Ok((selected, stats));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
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
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u64>();
        let value = wire::read_u64_le_checked(bytes, offset)?;
        let mut matched = false;
        for literal in literals {
            if compare_native_numcode_value(
                logical_type,
                value,
                NativeNumericPredicateOp::Eq,
                *literal,
            )? {
                matched = true;
                break;
            }
        }
        if !matched {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_i64_le_range(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<i64>(), validity)?;
    if let Some((out, dispatch)) =
        try_filter_i64_le_range_simd(bytes, row_count, validity, lower, upper, base)
    {
        let rows_matched = out.count_ones();
        return Ok((
            out,
            KernelStats {
                rows_seen: row_count,
                rows_valid: row_count,
                rows_matched,
                bitmap_words_touched: row_count.div_ceil(64),
                bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
                dispatch,
            },
        ));
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count * std::mem::size_of::<i64>(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<i64>();
        let value = wire::read_i64_le_checked(bytes, offset)?;
        if !i64_value_in_range(value, lower, upper) {
            continue;
        }
        out.set(row);
        stats.rows_matched += 1;
    }
    Ok((out, stats))
}

pub fn filter_u32_le_in_sorted(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    if let [needle] = needles {
        return filter_u32_le_eq_dispatch(
            bytes,
            row_count,
            validity,
            *needle,
            base,
            NativeKernelDispatch::Auto,
        );
    }
    let mut out = SelectionBitmap::none(row_count);
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
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u32>();
        let value = wire::read_u32_le_checked(bytes, offset)?;
        if needles.binary_search(&value).is_ok() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_u32_le_not_in_sorted(
    bytes: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[u32],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(bytes, row_count, std::mem::size_of::<u32>(), validity)?;
    if base.is_none() {
        if let [needle] = needles {
            let (matched, mut stats) = filter_u32_le_eq_dispatch(
                bytes,
                row_count,
                validity,
                *needle,
                None,
                NativeKernelDispatch::Auto,
            )?;
            let selected = valid_rows_except(row_count, validity, &matched);
            stats.rows_matched = selected.count_ones();
            return Ok((selected, stats));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
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
            continue;
        }
        stats.rows_valid += 1;
        let offset = row * std::mem::size_of::<u32>();
        let value = wire::read_u32_le_checked(bytes, offset)?;
        if needles.binary_search(&value).is_err() {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_i64_range(
    values: &[i64],
    validity: ValidityRef<'_>,
    lower: Option<(i64, BoundInclusive)>,
    upper: Option<(i64, BoundInclusive)>,
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    let mut out = SelectionBitmap::none(values.len());
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
        if lower
            .is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, false))
        {
            continue;
        }
        if upper
            .is_some_and(|(bound, inclusive)| !compare_bound(value.cmp(&bound), inclusive, true))
        {
            continue;
        }
        out.set(row);
        stats.rows_matched += 1;
    }
    (out, stats)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundInclusive {
    Inclusive,
    Exclusive,
}

pub fn local_membership_u8(local_to_global: &[u64], global_needles: &[u64]) -> Vec<bool> {
    let mut membership = vec![false; local_to_global.len()];
    for (local, global) in local_to_global.iter().copied().enumerate() {
        if global_needles.binary_search(&global).is_ok() {
            membership[local] = true;
        }
    }
    membership
}

pub fn filter_local_u8_membership(
    values: &[u8],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u8_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: values.len(),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(local as usize)
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_local_u16_membership(
    values: &[u16],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u16_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(local as usize)
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_local_u32_membership(
    values: &[u32],
    validity: ValidityRef<'_>,
    local_membership: &[bool],
    base: Option<&SelectionBitmap>,
) -> (SelectionBitmap, KernelStats) {
    if let Some((out, dispatch)) =
        try_filter_local_u32_membership_simd(values, validity, local_membership, base)
    {
        let rows_matched = out.count_ones();
        return (
            out,
            KernelStats {
                rows_seen: values.len(),
                rows_valid: values.len(),
                rows_matched,
                bitmap_words_touched: values.len().div_ceil(64),
                bytes_touched_estimate: std::mem::size_of_val(values),
                dispatch,
            },
        );
    }
    let mut out = SelectionBitmap::none(values.len());
    let mut stats = KernelStats {
        rows_seen: values.len(),
        bitmap_words_touched: values.len().div_ceil(64),
        bytes_touched_estimate: std::mem::size_of_val(values),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for (row, local) in values.iter().copied().enumerate() {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if local_membership
            .get(usize::try_from(local).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(false)
        {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    (out, stats)
}

pub fn filter_bool_eq(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: bool,
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    validate_fixed_le_width_for_validity(values, row_count, 1, validity)?;
    let needle = u8::from(needle);
    if base.is_none()
        && matches!(validity, ValidityRef::AllValid { row_count: valid_rows } if valid_rows == row_count)
    {
        validate_bool_bytes(values, row_count, validity)?;
        if let Some((out, dispatch)) = try_filter_bool_eq_simd(values, row_count, needle) {
            let rows_matched = out.count_ones();
            return Ok((
                out,
                KernelStats {
                    rows_seen: row_count,
                    rows_valid: row_count,
                    rows_matched,
                    bitmap_words_touched: row_count.div_ceil(64),
                    bytes_touched_estimate: row_count,
                    dispatch,
                },
            ));
        }
    }
    let mut out = SelectionBitmap::none(row_count);
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
            continue;
        }
        stats.rows_valid += 1;
        let value = values.get(row).copied().ok_or(CoveError::PageCorrupt)?;
        if !matches!(value, 0 | 1) {
            return Err(CoveError::PageCorrupt);
        }
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_fixed_bytes_eq(
    values: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if width == 0 || needle.len() != width {
        return Err(CoveError::BadSchema(
            "fixed-byte equality requires a non-zero width matching the literal".into(),
        ));
    }
    validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count
            .checked_mul(width)
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let value = values
            .get(offset..offset + width)
            .ok_or(CoveError::PageCorrupt)?;
        if value == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_fixed_bytes_in(
    values: &[u8],
    row_count: usize,
    width: usize,
    validity: ValidityRef<'_>,
    needles: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if width == 0 || needles.is_empty() || !needles.len().is_multiple_of(width) {
        return Err(CoveError::BadSchema(
            "fixed-byte IN requires non-empty literals with a width multiple".into(),
        ));
    }
    validate_fixed_le_width_for_validity(values, row_count, width, validity)?;
    let literal_count = needles.len() / width;
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: row_count
            .checked_mul(width)
            .and_then(|value| value.checked_add(needles.len()))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let offset = row.checked_mul(width).ok_or(CoveError::ArithOverflow)?;
        let value = values
            .get(offset..offset + width)
            .ok_or(CoveError::PageCorrupt)?;
        for literal_index in 0..literal_count {
            let literal_offset = literal_index
                .checked_mul(width)
                .ok_or(CoveError::ArithOverflow)?;
            let needle = needles
                .get(literal_offset..literal_offset + width)
                .ok_or(CoveError::PageCorrupt)?;
            if value == needle {
                out.set(row);
                stats.rows_matched += 1;
                break;
            }
        }
    }
    Ok((out, stats))
}

pub fn filter_varbytes_eq(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if varbytes_payload_at(row_offsets, values, row)? == needle {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_varbytes_in(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    needles: &[&[u8]],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if needles.is_empty() {
        return Err(CoveError::BadSchema(
            "VarBytes IN requires at least one literal".into(),
        ));
    }
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .and_then(|value| {
                needles
                    .iter()
                    .try_fold(value, |acc, needle| acc.checked_add(needle.len()))
            })
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        let value = varbytes_payload_at(row_offsets, values, row)?;
        if needles.contains(&value) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_eq(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needle: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if values.get(offset..len_end).is_none() {
            return Err(CoveError::BufferTooShort);
        }
        let len = wire::read_u32_le_checked(values, offset)? as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if value == needle {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_in(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    needles: &[&[u8]],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if needles.is_empty() {
        return Err(CoveError::BadSchema(
            "VarBytes IN requires at least one literal".into(),
        ));
    }
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: needles
            .iter()
            .try_fold(values.len(), |acc, needle| acc.checked_add(needle.len()))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if values.get(offset..len_end).is_none() {
            return Err(CoveError::BufferTooShort);
        }
        let len = wire::read_u32_le_checked(values, offset)? as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if needles.contains(&value) {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_length_prefixed_varbytes_prefix(
    values: &[u8],
    row_count: usize,
    validity: ValidityRef<'_>,
    prefix: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values.len(),
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    if matches!(validity, ValidityRef::AllNull { .. }) && values.is_empty() {
        return Ok((out, stats));
    }

    let all_selected = base.is_none_or(|bitmap| bitmap.count_ones() == row_count);
    let mut offset = 0usize;
    for row in 0..row_count {
        let len_end = offset.checked_add(4).ok_or(CoveError::ArithOverflow)?;
        if values.get(offset..len_end).is_none() {
            return Err(CoveError::BufferTooShort);
        }
        let len = wire::read_u32_le_checked(values, offset)? as usize;
        let value_start = len_end;
        let value_end = value_start
            .checked_add(len)
            .ok_or(CoveError::ArithOverflow)?;
        let Some(value) = values.get(value_start..value_end) else {
            return Err(CoveError::BufferTooShort);
        };

        if (all_selected || base.is_some_and(|bitmap| bitmap.contains(row)))
            && validity.is_valid(row)
        {
            stats.rows_valid += 1;
            if value.starts_with(prefix) {
                out.set(row);
                stats.rows_matched += 1;
            }
        }
        offset = value_end;
    }
    if offset != values.len() {
        return Err(CoveError::PageCorrupt);
    }
    Ok((out, stats))
}

pub fn filter_varbytes_prefix(
    row_offsets: &[u32],
    values: &[u8],
    validity: ValidityRef<'_>,
    prefix: &[u8],
    base: Option<&SelectionBitmap>,
) -> Result<(SelectionBitmap, KernelStats), CoveError> {
    let row_count = row_offsets.len();
    if validity.row_count() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let mut out = SelectionBitmap::none(row_count);
    let mut stats = KernelStats {
        rows_seen: row_count,
        bitmap_words_touched: row_count.div_ceil(64),
        bytes_touched_estimate: values
            .len()
            .checked_add(std::mem::size_of_val(row_offsets))
            .ok_or(CoveError::ArithOverflow)?,
        dispatch: NativeKernelDispatch::Scalar,
        ..KernelStats::default()
    };
    for row in 0..row_count {
        if base.is_some_and(|bitmap| !bitmap.contains(row)) {
            continue;
        }
        if !validity.is_valid(row) {
            continue;
        }
        stats.rows_valid += 1;
        if varbytes_payload_at(row_offsets, values, row)?.starts_with(prefix) {
            out.set(row);
            stats.rows_matched += 1;
        }
    }
    Ok((out, stats))
}
