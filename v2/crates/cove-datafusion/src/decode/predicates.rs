use super::*;

pub(super) fn plan_has_row_predicate(plan: &ScanPlan) -> bool {
    plan.filters.iter().any(|filter| {
        matches!(
            filter.predicate,
            Some(
                CovePredicate::Numeric { .. }
                    | CovePredicate::FileCodeIn { .. }
                    | CovePredicate::VarBytesEq { .. }
            )
        )
    })
}

pub(super) fn plan_has_exact_row_predicate(plan: &ScanPlan) -> bool {
    plan.filters.iter().any(|filter| {
        filter.use_kind == CoveFilterUse::FullRowPredicateExact
            && matches!(
                filter.predicate,
                Some(
                    CovePredicate::Numeric { .. }
                        | CovePredicate::FileCodeIn { .. }
                        | CovePredicate::VarBytesEq { .. }
                )
            )
    })
}

pub(super) fn lookup_selection_for_morsel(
    state: &DatasetState,
    segment_id: u32,
    morsel_id: u32,
    row_count: u32,
    plan: &ScanPlan,
    stats: &mut DecodeStats,
    scratch: &mut DecodeScratch,
) -> Result<bool, CoveError> {
    let mut saw_lookup_filter = false;
    scratch.selected_mask.fill_all(row_count as usize);
    for filter in &plan.filters {
        let (column_index, key_kind, keys) = match &filter.predicate {
            Some(CovePredicate::FileCodeIn {
                column_index,
                file_codes,
                ..
            }) => (
                *column_index,
                LookupKeyKind::FileCode,
                file_codes
                    .iter()
                    .copied()
                    .map(u64::from)
                    .collect::<Vec<_>>(),
            ),
            Some(CovePredicate::Numeric {
                column_index,
                op: NumericPredicateOp::Eq,
                literal,
            }) => {
                let column = &state.table().columns[*column_index];
                let keys = numeric_lookup_keys(column.logical, *literal);
                if keys.is_empty() {
                    continue;
                }
                (*column_index, LookupKeyKind::NumCode, keys)
            }
            _ => continue,
        };
        let column = &state.table().columns[column_index];
        let Some(index) = state.lookup_for(column.column_id) else {
            if saw_lookup_filter && key_kind == LookupKeyKind::FileCode {
                stats.index_fallbacks += 1;
                return Ok(false);
            }
            continue;
        };
        if index.header.key_kind != key_kind {
            stats.index_fallbacks += 1;
            return Ok(false);
        }
        saw_lookup_filter = true;
        scratch.filter_mask.fill_none(row_count as usize);
        for key in keys {
            match index.rows_for(key) {
                Some(rows) if !rows.is_empty() => {
                    stats.lookup_index_hits += 1;
                    for row in rows {
                        if row.table_id != state.table().table_id
                            || row.segment_id != segment_id
                            || row.morsel_id != morsel_id
                        {
                            continue;
                        }
                        let row_index = usize::from(row.row_in_morsel);
                        if row_index >= scratch.filter_mask.len {
                            stats.index_fallbacks += 1;
                            return Ok(false);
                        }
                        scratch.filter_mask.set(row_index);
                    }
                }
                _ => stats.lookup_index_misses += 1,
            }
        }
        scratch.selected_mask.and_inplace(&scratch.filter_mask);
        if scratch.selected_mask.all_zero() {
            break;
        }
    }
    if saw_lookup_filter {
        stats.index_rows_selected += scratch.selected_mask.count_ones();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[inline]
pub(super) fn predicate_is_index_covered(state: &DatasetState, predicate: &CovePredicate) -> bool {
    match predicate {
        CovePredicate::FileCodeIn { column_index, .. } => {
            let column = &state.table().columns[*column_index];
            state
                .lookup_for(column.column_id)
                .map(|index| index.header.key_kind == LookupKeyKind::FileCode)
                .unwrap_or(false)
        }
        CovePredicate::Numeric {
            column_index,
            op: NumericPredicateOp::Eq,
            literal,
        } => {
            let column = &state.table().columns[*column_index];
            if numeric_lookup_keys(column.logical, *literal).is_empty() {
                return false;
            }
            state
                .lookup_for(column.column_id)
                .map(|index| index.header.key_kind == LookupKeyKind::NumCode)
                .unwrap_or(false)
        }
        _ => false,
    }
}

#[inline]
pub(crate) fn numeric_lookup_keys(logical: CoveLogicalType, literal: PredicateLiteral) -> Vec<u64> {
    match logical {
        CoveLogicalType::Bool => bool_numeric_lookup_key(literal).into_iter().collect(),
        CoveLogicalType::Float32 => {
            let Some(original) = predicate_literal_as_f64(literal) else {
                return Vec::new();
            };
            let value = original as f32;
            if !value.is_finite() {
                Vec::new()
            } else if f64::from(value) != original {
                Vec::new()
            } else if value == 0.0 {
                vec![u64::from(0.0f32.to_bits()), u64::from((-0.0f32).to_bits())]
            } else {
                vec![u64::from(value.to_bits())]
            }
        }
        CoveLogicalType::Float64 => {
            let Some(value) = predicate_literal_as_f64(literal) else {
                return Vec::new();
            };
            if !value.is_finite() {
                Vec::new()
            } else if value == 0.0 {
                vec![0.0f64.to_bits(), (-0.0f64).to_bits()]
            } else {
                vec![value.to_bits()]
            }
        }
        _ => integer_numeric_lookup_key(logical, literal)
            .into_iter()
            .collect(),
    }
}

#[inline]
fn bool_numeric_lookup_key(literal: PredicateLiteral) -> Option<u64> {
    match literal {
        PredicateLiteral::Int64(value) => match value {
            0 => Some(0),
            1 => Some(1),
            _ => None,
        },
        PredicateLiteral::UInt64(value) => match value {
            0 => Some(0),
            1 => Some(1),
            _ => None,
        },
        PredicateLiteral::Float64(value) if value == 0.0 => Some(0),
        PredicateLiteral::Float64(value) if value == 1.0 => Some(1),
        PredicateLiteral::Float64(_) => None,
    }
}

#[inline]
fn integer_numeric_lookup_key(logical: CoveLogicalType, literal: PredicateLiteral) -> Option<u64> {
    match literal {
        PredicateLiteral::Int64(value) => signed_literal_lookup_key(logical, value),
        PredicateLiteral::UInt64(value) => unsigned_literal_lookup_key(logical, value),
        PredicateLiteral::Float64(value) => f64_to_i64_exact(value)
            .and_then(|value| signed_literal_lookup_key(logical, value))
            .or_else(|| {
                f64_to_u64_exact(value)
                    .and_then(|value| unsigned_literal_lookup_key(logical, value))
            }),
    }
}

const I64_MAX_PLUS_ONE_AS_F64: f64 = 9_223_372_036_854_775_808.0;
const U64_MAX_PLUS_ONE_AS_F64: f64 = 18_446_744_073_709_551_616.0;

#[inline]
fn f64_to_i64_exact(value: f64) -> Option<i64> {
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
fn f64_to_u64_exact(value: f64) -> Option<u64> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value >= U64_MAX_PLUS_ONE_AS_F64
    {
        return None;
    }
    let candidate = value as u64;
    ((candidate as f64) == value).then_some(candidate)
}

#[inline]
fn signed_literal_lookup_key(logical: CoveLogicalType, value: i64) -> Option<u64> {
    match logical {
        CoveLogicalType::Int8 => i8::try_from(value).ok().map(|value| value as i64 as u64),
        CoveLogicalType::Int16 => i16::try_from(value).ok().map(|value| value as i64 as u64),
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => {
            i32::try_from(value).ok().map(|value| value as i64 as u64)
        }
        CoveLogicalType::Int64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => Some(value as u64),
        CoveLogicalType::UInt8 => u8::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt16 => u16::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt32 => u32::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt64 => u64::try_from(value).ok(),
        _ => None,
    }
}

#[inline]
fn unsigned_literal_lookup_key(logical: CoveLogicalType, value: u64) -> Option<u64> {
    match logical {
        CoveLogicalType::Int8 => i8::try_from(value).ok().map(|value| value as i64 as u64),
        CoveLogicalType::Int16 => i16::try_from(value).ok().map(|value| value as i64 as u64),
        CoveLogicalType::Int32 | CoveLogicalType::DateDays => {
            i32::try_from(value).ok().map(|value| value as i64 as u64)
        }
        CoveLogicalType::Int64
        | CoveLogicalType::Decimal64
        | CoveLogicalType::TimestampMicros
        | CoveLogicalType::TimestampNanos => i64::try_from(value).ok().map(|value| value as u64),
        CoveLogicalType::UInt8 => u8::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt16 => u16::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt32 => u32::try_from(value).ok().map(u64::from),
        CoveLogicalType::UInt64 => Some(value),
        _ => None,
    }
}

pub(super) fn plan_has_residual(plan: &ScanPlan) -> bool {
    plan.filters
        .iter()
        .any(|filter| filter.use_kind == CoveFilterUse::PruningOnly)
}

#[inline]
pub(super) fn predicate_column_index(predicate: &CovePredicate) -> Option<usize> {
    match predicate {
        CovePredicate::Null { column_index, .. }
        | CovePredicate::Numeric { column_index, .. }
        | CovePredicate::FileCodeIn { column_index, .. }
        | CovePredicate::VarBytesEq { column_index, .. } => Some(*column_index),
    }
}

pub(super) fn apply_predicate_to_selection(
    predicate: &CovePredicate,
    prepared: &PreparedEncodedArray<'_>,
    selected: &mut SelectionMask,
    scratch: &mut SelectionMask,
) -> Result<bool, CoveError> {
    if let Some(()) =
        try_apply_numcode_predicate_to_selection(predicate, prepared.array(), selected, scratch)?
    {
        return Ok(true);
    }
    if let Some(()) = try_apply_varbytes_eq_predicate_to_selection(
        predicate,
        prepared.array(),
        selected,
        scratch,
    )? {
        return Ok(true);
    }
    for word_index in 0..selected.words.len() {
        let mut remaining = selected.words[word_index];
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let index = word_index
                .checked_mul(64)
                .and_then(|base| base.checked_add(bit))
                .ok_or_else(|| CoveError::ArithOverflow)?;
            if index >= selected.len {
                break;
            }
            let row = u64::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
            let array = prepared.array();
            let keep = match predicate {
                CovePredicate::Null { kind, .. } => {
                    let is_null = array.is_null(row)?;
                    match kind {
                        NullPredicateKind::IsNull => is_null,
                        NullPredicateKind::IsNotNull => !is_null,
                    }
                }
                CovePredicate::Numeric { op, literal, .. } => {
                    let value = prepared.decode_row(row)?;
                    match compare_numeric_value(array.logical, &value, *op, *literal)? {
                        Some(value) => value,
                        None => return Ok(false),
                    }
                }
                CovePredicate::FileCodeIn { file_codes, .. } => {
                    if array.is_null(row)? {
                        false
                    } else {
                        let code = match raw_file_code_at(array, row)? {
                            Some(code) => Some(code),
                            None => match prepared.decode_row(row)? {
                                CoveArrayValue::FileCode(code) => Some(code),
                                _ => None,
                            },
                        };
                        match code {
                            Some(code) => file_codes.binary_search(&code).is_ok(),
                            None => return Ok(false),
                        }
                    }
                }
                CovePredicate::VarBytesEq { literal, .. } => {
                    if array.is_null(row)? {
                        false
                    } else {
                        match prepared.decode_row(row)? {
                            CoveArrayValue::Bytes(bytes) => bytes == literal.as_slice(),
                            CoveArrayValue::OwnedBytes(bytes) => {
                                bytes.as_slice() == literal.as_slice()
                            }
                            _ => return Ok(false),
                        }
                    }
                }
            };
            if !keep {
                selected.clear_bit(index);
            }
            remaining &= remaining - 1;
        }
    }
    Ok(true)
}

pub(super) fn try_apply_raw_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    if let Some(()) = try_apply_numcode_predicate_to_selection(predicate, array, selected, scratch)?
    {
        return Ok(Some(()));
    }
    if let Some(()) =
        try_apply_varbytes_eq_predicate_to_selection(predicate, array, selected, scratch)?
    {
        return Ok(Some(()));
    }
    Ok(None)
}

fn try_apply_numcode_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    let CovePredicate::Numeric { op, literal, .. } = predicate else {
        return Ok(None);
    };
    if array.encoding != CoveEncodingKind::NumCode || array.physical != CovePhysicalKind::NumCode {
        return Ok(None);
    }

    scratch.clone_from_mask(selected);
    for word_index in 0..selected.words.len() {
        let mut remaining = selected.words[word_index];
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let index = word_index
                .checked_mul(64)
                .and_then(|base| base.checked_add(bit))
                .ok_or_else(|| CoveError::ArithOverflow)?;
            if index >= selected.len {
                break;
            }
            let row = u64::try_from(index).map_err(|_| CoveError::ArithOverflow)?;
            let keep = if array.is_null(row)? {
                false
            } else {
                let code = raw_numcode_at(array, row)?;
                match compare_numcode_value(array.logical, code, *op, *literal) {
                    Ok(value) => value,
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            };
            if !keep {
                scratch.clear_bit(index);
            }
            remaining &= remaining - 1;
        }
    }
    std::mem::swap(selected, scratch);
    Ok(Some(()))
}

fn try_apply_varbytes_eq_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    let CovePredicate::VarBytesEq { literal, .. } = predicate else {
        return Ok(None);
    };
    if array.encoding != CoveEncodingKind::VarBytes || array.physical != CovePhysicalKind::VarBytes
    {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match VarBytes row count {row_count}",
            selected.len
        )));
    }

    scratch.clone_from_mask(selected);
    let all_non_null = array.validity.is_none();
    let all_selected = selected.count_ones() == row_count;
    let data = array.data;
    let mut offset = 0usize;
    for row in 0..row_count {
        let header_end = offset
            .checked_add(4)
            .ok_or_else(|| CoveError::ArithOverflow)?;
        if header_end > data.len() {
            return Err(CoveError::OffsetRange);
        }
        let len = u32::from_le_bytes(data[offset..header_end].try_into().unwrap()) as usize;
        offset = header_end;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| CoveError::ArithOverflow)?;
        if end > data.len() {
            return Err(CoveError::OffsetRange);
        }
        let value = &data[offset..end];

        let is_selected = if all_selected {
            true
        } else {
            let word = selected.words.get(row / 64).copied().unwrap_or(0);
            (word & (1u64 << (row % 64))) != 0
        };
        if is_selected {
            let is_non_null = if all_non_null {
                true
            } else {
                let row_u64 = u64::try_from(row).map_err(|_| CoveError::ArithOverflow)?;
                !array.is_null(row_u64)?
            };
            let keep = is_non_null && value.len() == literal.len() && value == literal.as_slice();
            if !keep {
                scratch.clear_bit(row);
            }
        }
        offset = end;
    }
    if offset != array.data.len() {
        return Err(CoveError::PageCorrupt);
    }
    std::mem::swap(selected, scratch);
    Ok(Some(()))
}

pub(super) fn try_apply_predicate_to_selection(
    predicate: &CovePredicate,
    prepared: &PreparedEncodedArray<'_>,
    selected: &mut SelectionMask,
    scratch: &mut SelectionMask,
) -> Result<bool, CoveError> {
    match apply_predicate_to_selection(predicate, prepared, selected, scratch) {
        Ok(value) => Ok(value),
        Err(CoveError::UnsupportedEncoding(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

fn compare_numcode_value(
    logical: CoveLogicalType,
    value: u64,
    op: NumericPredicateOp,
    literal: PredicateLiteral,
) -> Result<bool, CoveError> {
    let Some(value) = typed_numcode_value(logical, value) else {
        return Err(CoveError::UnsupportedEncoding(format!(
            "numeric predicate for {logical:?} NumCode"
        )));
    };
    Ok(compare_typed_numeric_value(value, op, literal))
}

fn compare_numeric_value(
    logical: CoveLogicalType,
    value: &CoveArrayValue<'_>,
    op: NumericPredicateOp,
    literal: PredicateLiteral,
) -> Result<Option<bool>, CoveError> {
    if matches!(value, CoveArrayValue::Null) {
        return Ok(Some(false));
    }
    if let CoveArrayValue::NumCode(value) = value {
        let Some(value) = typed_numcode_value(logical, *value) else {
            return Ok(None);
        };
        return Ok(Some(compare_typed_numeric_value(value, op, literal)));
    }
    match literal {
        PredicateLiteral::Int64(literal) => {
            let Some(value) = value_as_i64(value)? else {
                return Ok(None);
            };
            Ok(Some(compare_ordered(value, op, literal)))
        }
        PredicateLiteral::UInt64(literal) => {
            let Some(value) = value_as_u64(value)? else {
                return Ok(None);
            };
            Ok(Some(compare_ordered(value, op, literal)))
        }
        PredicateLiteral::Float64(literal) => {
            let Some(value) = value_as_f64(logical, value)? else {
                return Ok(None);
            };
            Ok(Some(compare_ordered(value, op, literal)))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TypedNumericValue {
    Signed(i128),
    Unsigned(u128),
    Float(f64),
}

fn typed_numcode_value(logical: CoveLogicalType, value: u64) -> Option<TypedNumericValue> {
    use cove_core::types;

    let value = match logical {
        CoveLogicalType::Bool => match value {
            0 => TypedNumericValue::Unsigned(0),
            1 => TypedNumericValue::Unsigned(1),
            _ => return None,
        },
        CoveLogicalType::Int8 => TypedNumericValue::Signed(types::numcode_as_i8(value) as i128),
        CoveLogicalType::Int16 => TypedNumericValue::Signed(types::numcode_as_i16(value) as i128),
        CoveLogicalType::Int32 => TypedNumericValue::Signed(types::numcode_as_i32(value) as i128),
        CoveLogicalType::Int64 => TypedNumericValue::Signed(types::numcode_as_i64(value) as i128),
        CoveLogicalType::UInt8 => TypedNumericValue::Unsigned(types::numcode_as_u8(value) as u128),
        CoveLogicalType::UInt16 => {
            TypedNumericValue::Unsigned(types::numcode_as_u16(value) as u128)
        }
        CoveLogicalType::UInt32 => {
            TypedNumericValue::Unsigned(types::numcode_as_u32(value) as u128)
        }
        CoveLogicalType::UInt64 => {
            TypedNumericValue::Unsigned(types::numcode_as_u64(value) as u128)
        }
        CoveLogicalType::Float32 => {
            let value = types::numcode_as_f32(value);
            if value.is_nan() {
                return Some(TypedNumericValue::Float(f64::NAN));
            }
            TypedNumericValue::Float(value as f64)
        }
        CoveLogicalType::Float64 => {
            let value = types::numcode_as_f64(value);
            if value.is_nan() {
                return Some(TypedNumericValue::Float(f64::NAN));
            }
            TypedNumericValue::Float(value)
        }
        CoveLogicalType::Decimal64 => {
            TypedNumericValue::Signed(types::numcode_as_decimal64(value) as i128)
        }
        CoveLogicalType::DateDays => {
            TypedNumericValue::Signed(types::numcode_as_date_days(value) as i128)
        }
        CoveLogicalType::TimestampMicros => {
            TypedNumericValue::Signed(types::numcode_as_timestamp_micros(value) as i128)
        }
        CoveLogicalType::TimestampNanos => {
            TypedNumericValue::Signed(types::numcode_as_timestamp_nanos(value) as i128)
        }
        _ => return None,
    };
    Some(value)
}

fn compare_typed_numeric_value(
    value: TypedNumericValue,
    op: NumericPredicateOp,
    literal: PredicateLiteral,
) -> bool {
    match value {
        TypedNumericValue::Signed(value) => compare_signed_numeric_value(value, op, literal),
        TypedNumericValue::Unsigned(value) => compare_unsigned_numeric_value(value, op, literal),
        TypedNumericValue::Float(value) => {
            let Some(literal) = predicate_literal_as_f64(literal) else {
                return false;
            };
            if value.is_nan() || literal.is_nan() {
                false
            } else {
                compare_ordered(value, op, literal)
            }
        }
    }
}

fn compare_signed_numeric_value(
    value: i128,
    op: NumericPredicateOp,
    literal: PredicateLiteral,
) -> bool {
    match literal {
        PredicateLiteral::Int64(literal) => compare_ordered(value, op, literal as i128),
        PredicateLiteral::UInt64(literal) => compare_signed_unsigned(value, op, literal as u128),
        PredicateLiteral::Float64(literal) => compare_signed_float_literal(value, op, literal),
    }
}

fn compare_unsigned_numeric_value(
    value: u128,
    op: NumericPredicateOp,
    literal: PredicateLiteral,
) -> bool {
    match literal {
        PredicateLiteral::Int64(literal) if literal < 0 => match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Lt | NumericPredicateOp::LtEq => false,
            NumericPredicateOp::Gt | NumericPredicateOp::GtEq => true,
        },
        PredicateLiteral::Int64(literal) => compare_ordered(value, op, literal as u128),
        PredicateLiteral::UInt64(literal) => compare_ordered(value, op, literal as u128),
        PredicateLiteral::Float64(literal) => compare_unsigned_float_literal(value, op, literal),
    }
}

fn compare_signed_float_literal(value: i128, op: NumericPredicateOp, literal: f64) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_i64_exact(literal) {
        return compare_ordered(value, op, rhs as i128);
    }
    if literal < i64::MIN as f64 {
        return match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Lt | NumericPredicateOp::LtEq => false,
            NumericPredicateOp::Gt | NumericPredicateOp::GtEq => true,
        };
    }
    if literal >= I64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Gt | NumericPredicateOp::GtEq => false,
            NumericPredicateOp::Lt | NumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as i128;
    let ceil = literal.ceil() as i128;
    compare_integer_to_fractional_bounds(value, op, floor, ceil)
}

fn compare_unsigned_float_literal(value: u128, op: NumericPredicateOp, literal: f64) -> bool {
    if !literal.is_finite() {
        return false;
    }
    if let Some(rhs) = f64_to_u64_exact(literal) {
        return compare_ordered(value, op, rhs as u128);
    }
    if literal < 0.0 {
        return match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Lt | NumericPredicateOp::LtEq => false,
            NumericPredicateOp::Gt | NumericPredicateOp::GtEq => true,
        };
    }
    if literal >= U64_MAX_PLUS_ONE_AS_F64 {
        return match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Gt | NumericPredicateOp::GtEq => false,
            NumericPredicateOp::Lt | NumericPredicateOp::LtEq => true,
        };
    }
    let floor = literal.floor() as u128;
    let ceil = literal.ceil() as u128;
    compare_integer_to_fractional_bounds(value, op, floor, ceil)
}

fn compare_integer_to_fractional_bounds<T>(
    value: T,
    op: NumericPredicateOp,
    floor: T,
    ceil: T,
) -> bool
where
    T: PartialOrd,
{
    match op {
        NumericPredicateOp::Eq => false,
        NumericPredicateOp::Lt | NumericPredicateOp::LtEq => value <= floor,
        NumericPredicateOp::Gt | NumericPredicateOp::GtEq => value >= ceil,
    }
}

fn compare_signed_unsigned(value: i128, op: NumericPredicateOp, literal: u128) -> bool {
    if value < 0 {
        return match op {
            NumericPredicateOp::Eq | NumericPredicateOp::Gt | NumericPredicateOp::GtEq => false,
            NumericPredicateOp::Lt | NumericPredicateOp::LtEq => true,
        };
    }
    compare_ordered(value as u128, op, literal)
}

fn compare_ordered<T: PartialOrd + PartialEq>(left: T, op: NumericPredicateOp, right: T) -> bool {
    match op {
        NumericPredicateOp::Eq => left == right,
        NumericPredicateOp::Lt => left < right,
        NumericPredicateOp::LtEq => left <= right,
        NumericPredicateOp::Gt => left > right,
        NumericPredicateOp::GtEq => left >= right,
    }
}

fn value_as_i64(value: &CoveArrayValue<'_>) -> Result<Option<i64>, CoveError> {
    match value {
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => i64::try_from(*value)
            .map(Some)
            .map_err(|_| CoveError::UnsupportedEncoding("NumCode value exceeds i64".into())),
        CoveArrayValue::Int64(value) => Ok(Some(*value)),
        CoveArrayValue::Bytes(bytes) if bytes.len() == 8 => {
            Ok(Some(i64::from_le_bytes((*bytes).try_into().unwrap())))
        }
        _ => Ok(None),
    }
}

fn value_as_u64(value: &CoveArrayValue<'_>) -> Result<Option<u64>, CoveError> {
    match value {
        CoveArrayValue::NumCode(value) | CoveArrayValue::Varint(value) => Ok(Some(*value)),
        CoveArrayValue::Int64(value) => u64::try_from(*value).map(Some).map_err(|_| {
            CoveError::UnsupportedEncoding("negative value cannot compare as u64".into())
        }),
        CoveArrayValue::Bytes(bytes) if bytes.len() == 8 => {
            Ok(Some(u64::from_le_bytes((*bytes).try_into().unwrap())))
        }
        _ => Ok(None),
    }
}

fn predicate_literal_as_f64(literal: PredicateLiteral) -> Option<f64> {
    match literal {
        PredicateLiteral::Int64(value) => Some(value as f64),
        PredicateLiteral::UInt64(value) => Some(value as f64),
        PredicateLiteral::Float64(value) if !value.is_nan() => Some(value),
        PredicateLiteral::Float64(_) => None,
    }
}

fn numcode_as_f64(logical: CoveLogicalType, value: u64) -> Option<f64> {
    let value = match logical {
        CoveLogicalType::Float32 => f32::from_bits(value as u32) as f64,
        CoveLogicalType::Float64 => f64::from_bits(value),
        _ => value as f64,
    };
    (!value.is_nan()).then_some(value)
}

fn value_as_f64(
    logical: CoveLogicalType,
    value: &CoveArrayValue<'_>,
) -> Result<Option<f64>, CoveError> {
    match value {
        CoveArrayValue::NumCode(value) => Ok(numcode_as_f64(logical, *value)),
        CoveArrayValue::Varint(value) => Ok(Some(*value as f64)),
        CoveArrayValue::Int64(value) => Ok(Some(*value as f64)),
        CoveArrayValue::Bytes(bytes) if bytes.len() == 8 => {
            let value = f64::from_bits(u64::from_le_bytes((*bytes).try_into().unwrap()));
            if value.is_nan() {
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        _ => Ok(None),
    }
}

fn raw_file_code_at(array: &EncodedArray<'_>, row: u64) -> Result<Option<u32>, CoveError> {
    if array.encoding != CoveEncodingKind::FileCode {
        return Ok(None);
    }
    let offset = usize::try_from(row)
        .map_err(|_| CoveError::ArithOverflow)?
        .checked_mul(4)
        .ok_or_else(|| CoveError::ArithOverflow)?;
    let bytes = wire::read_range_checked(array.data, offset, 4)?;
    Ok(Some(u32::from_le_bytes(bytes.try_into().unwrap())))
}

fn raw_numcode_at(array: &EncodedArray<'_>, row: u64) -> Result<u64, CoveError> {
    let offset = usize::try_from(row)
        .map_err(|_| CoveError::ArithOverflow)?
        .checked_mul(8)
        .ok_or_else(|| CoveError::ArithOverflow)?;
    let bytes = wire::read_range_checked(array.data, offset, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_integer_lookup_keys_require_exact_representation() {
        assert_eq!(
            numeric_lookup_keys(
                CoveLogicalType::UInt64,
                PredicateLiteral::Float64((1u64 << 53) as f64)
            ),
            vec![1u64 << 53]
        );
        assert!(numeric_lookup_keys(
            CoveLogicalType::Int64,
            PredicateLiteral::Float64(i64::MAX as f64)
        )
        .is_empty());
        assert!(numeric_lookup_keys(
            CoveLogicalType::UInt64,
            PredicateLiteral::Float64(u64::MAX as f64)
        )
        .is_empty());
        assert!(
            numeric_lookup_keys(CoveLogicalType::UInt64, PredicateLiteral::Float64(1.5)).is_empty()
        );
        assert!(numeric_lookup_keys(
            CoveLogicalType::UInt64,
            PredicateLiteral::Float64(U64_MAX_PLUS_ONE_AS_F64)
        )
        .is_empty());
    }

    #[test]
    fn integer_float_comparisons_preserve_exact_integer_domain() {
        let rounded = (1u64 << 53) as f64;
        assert!(!compare_unsigned_numeric_value(
            (1u128 << 53) + 1,
            NumericPredicateOp::Eq,
            PredicateLiteral::Float64(rounded)
        ));
        assert!(!compare_unsigned_numeric_value(
            (1u128 << 53) + 1,
            NumericPredicateOp::LtEq,
            PredicateLiteral::Float64(rounded)
        ));
        assert!(compare_unsigned_numeric_value(
            (1u128 << 53) + 1,
            NumericPredicateOp::Gt,
            PredicateLiteral::Float64(rounded)
        ));
        assert!(compare_signed_numeric_value(
            i64::MAX as i128,
            NumericPredicateOp::Lt,
            PredicateLiteral::Float64(i64::MAX as f64)
        ));
        assert!(!compare_unsigned_numeric_value(
            u64::MAX as u128,
            NumericPredicateOp::Eq,
            PredicateLiteral::Float64(u64::MAX as f64)
        ));
        assert!(compare_unsigned_numeric_value(
            u64::MAX as u128,
            NumericPredicateOp::Lt,
            PredicateLiteral::Float64(u64::MAX as f64)
        ));
        assert!(compare_unsigned_numeric_value(
            2,
            NumericPredicateOp::GtEq,
            PredicateLiteral::Float64(1.5)
        ));
        assert!(!compare_signed_numeric_value(
            1,
            NumericPredicateOp::Gt,
            PredicateLiteral::Float64(1.5)
        ));
    }
}
