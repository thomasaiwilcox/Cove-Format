#![allow(clippy::unnecessary_lazy_evaluations)]

use cove_core::{
    encoding::local_codebook::{LocalCodebookPayload, LocalCodebookValues},
    native::{
        filter_bool_eq, filter_fixed_bytes_eq, filter_fixed_bytes_in,
        filter_length_prefixed_varbytes_eq, filter_length_prefixed_varbytes_in,
        filter_length_prefixed_varbytes_prefix, filter_local_u16_membership,
        filter_local_u32_membership, filter_local_u8_membership, filter_numcode_le_in_typed,
        filter_numcode_le_not_in_typed, filter_numcode_le_typed, filter_u32_in_sorted,
        filter_u32_le_in_sorted, filter_u32_le_not_in_sorted, filter_u32_not_in_sorted,
        filter_validity, filter_varbytes_eq, filter_varbytes_in, filter_varbytes_prefix,
        native_numcode_matches, KernelStats, LaneRef, NativeKernelDispatch, NativeNumericLiteral,
        NativeNumericPredicateOp, ValidityRef,
    },
};

use super::*;

pub(super) fn plan_has_row_predicate(plan: &ScanPlan) -> bool {
    plan.filters.iter().any(|filter| {
        matches!(
            filter.predicate,
            Some(
                CovePredicate::Null { .. }
                    | CovePredicate::Numeric { .. }
                    | CovePredicate::NumericIn { .. }
                    | CovePredicate::NumericNotIn { .. }
                    | CovePredicate::FileCodeIn { .. }
                    | CovePredicate::FileCodeNotIn { .. }
                    | CovePredicate::FixedBytesEq { .. }
                    | CovePredicate::FixedBytesIn { .. }
                    | CovePredicate::VarBytesEq { .. }
                    | CovePredicate::VarBytesIn { .. }
                    | CovePredicate::VarBytesPrefix { .. }
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
                    CovePredicate::Null { .. }
                        | CovePredicate::Numeric { .. }
                        | CovePredicate::NumericIn { .. }
                        | CovePredicate::NumericNotIn { .. }
                        | CovePredicate::FileCodeIn { .. }
                        | CovePredicate::FileCodeNotIn { .. }
                        | CovePredicate::FixedBytesEq { .. }
                        | CovePredicate::FixedBytesIn { .. }
                        | CovePredicate::VarBytesEq { .. }
                        | CovePredicate::VarBytesIn { .. }
                        | CovePredicate::VarBytesPrefix { .. }
                )
            )
    })
}

#[inline]
fn optional_base_for_selection(selected: &SelectionMask) -> Option<&SelectionMask> {
    (!selected.all_set()).then_some(selected)
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
            Some(CovePredicate::NumericIn {
                column_index,
                literals,
            }) => {
                let column = &state.table().columns[*column_index];
                let keys = numeric_in_lookup_keys(column.logical, literals);
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
                        if row_index >= scratch.filter_mask.len() {
                            stats.index_fallbacks += 1;
                            return Ok(false);
                        }
                        scratch.filter_mask.set(row_index);
                    }
                }
                _ => stats.lookup_index_misses += 1,
            }
        }
        let dispatch = scratch
            .selected_mask
            .and_inplace_dispatch(&scratch.filter_mask, NativeKernelDispatch::Auto);
        stats.record_native_bitmap_dispatch(dispatch);
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
        CovePredicate::NumericIn {
            column_index,
            literals,
        } => {
            let column = &state.table().columns[*column_index];
            if numeric_in_lookup_keys(column.logical, literals).is_empty() {
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
            if !value.is_finite() || f64::from(value) != original {
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

fn numeric_in_lookup_keys(logical: CoveLogicalType, literals: &[PredicateLiteral]) -> Vec<u64> {
    let mut keys = Vec::new();
    for literal in literals {
        for key in numeric_lookup_keys(logical, *literal) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
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
        PredicateLiteral::Float64(0.0) => Some(0),
        PredicateLiteral::Float64(1.0) => Some(1),
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
    if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..U64_MAX_PLUS_ONE_AS_F64).contains(&value)
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
        | CovePredicate::NumericIn { column_index, .. }
        | CovePredicate::NumericNotIn { column_index, .. }
        | CovePredicate::FileCodeIn { column_index, .. }
        | CovePredicate::FileCodeNotIn { column_index, .. }
        | CovePredicate::FixedBytesEq { column_index, .. }
        | CovePredicate::FixedBytesIn { column_index, .. }
        | CovePredicate::VarBytesEq { column_index, .. }
        | CovePredicate::VarBytesIn { column_index, .. }
        | CovePredicate::VarBytesPrefix { column_index, .. } => Some(*column_index),
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
    if let Some(()) = try_apply_local_codebook_predicate_to_selection(
        predicate,
        prepared.array(),
        selected,
        scratch,
    )? {
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
    if let Some(()) =
        try_apply_varbytes_prefix_predicate_to_selection(predicate, prepared.array(), selected)?
    {
        return Ok(true);
    }
    if let Some(()) =
        try_apply_fixed_bytes_eq_predicate_to_selection(predicate, prepared.array(), selected)?
    {
        return Ok(true);
    }
    for word_index in 0..selected.words().len() {
        let mut remaining = selected.words()[word_index];
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let index = word_index
                .checked_mul(64)
                .and_then(|base| base.checked_add(bit))
                .ok_or_else(|| CoveError::ArithOverflow)?;
            if index >= selected.len() {
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
                CovePredicate::NumericIn { literals, .. } => {
                    let value = prepared.decode_row(row)?;
                    match compare_numeric_in_value(array.logical, &value, literals)? {
                        Some(value) => value,
                        None => return Ok(false),
                    }
                }
                CovePredicate::NumericNotIn { literals, .. } => {
                    let value = prepared.decode_row(row)?;
                    if matches!(value, CoveArrayValue::Null) {
                        false
                    } else {
                        match compare_numeric_in_value(array.logical, &value, literals)? {
                            Some(matched) => !matched,
                            None => return Ok(false),
                        }
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
                CovePredicate::FileCodeNotIn { file_codes, .. } => {
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
                            Some(code) => file_codes.binary_search(&code).is_err(),
                            None => return Ok(false),
                        }
                    }
                }
                CovePredicate::FixedBytesEq { literal, .. } => {
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
                CovePredicate::FixedBytesIn { literals, .. } => {
                    if array.is_null(row)? {
                        false
                    } else {
                        match prepared.decode_row(row)? {
                            CoveArrayValue::Bytes(bytes) => {
                                literals.iter().any(|literal| bytes == literal.as_slice())
                            }
                            CoveArrayValue::OwnedBytes(bytes) => literals
                                .iter()
                                .any(|literal| bytes.as_slice() == literal.as_slice()),
                            _ => return Ok(false),
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
                CovePredicate::VarBytesIn { literals, .. } => {
                    if array.is_null(row)? {
                        false
                    } else {
                        match prepared.decode_row(row)? {
                            CoveArrayValue::Bytes(bytes) => {
                                literals.iter().any(|literal| bytes == literal.as_slice())
                            }
                            CoveArrayValue::OwnedBytes(bytes) => literals
                                .iter()
                                .any(|literal| bytes.as_slice() == literal.as_slice()),
                            _ => return Ok(false),
                        }
                    }
                }
                CovePredicate::VarBytesPrefix { prefix, .. } => {
                    if array.is_null(row)? {
                        false
                    } else {
                        match prepared.decode_row(row)? {
                            CoveArrayValue::Bytes(bytes) => bytes.starts_with(prefix),
                            CoveArrayValue::OwnedBytes(bytes) => bytes.starts_with(prefix),
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
        try_apply_local_codebook_predicate_to_selection(predicate, array, selected, scratch)?
    {
        return Ok(Some(()));
    }
    if let Some(()) =
        try_apply_varbytes_eq_predicate_to_selection(predicate, array, selected, scratch)?
    {
        return Ok(Some(()));
    }
    if let Some(()) = try_apply_varbytes_prefix_predicate_to_selection(predicate, array, selected)?
    {
        return Ok(Some(()));
    }
    if let Some(()) = try_apply_fixed_bytes_eq_predicate_to_selection(predicate, array, selected)? {
        return Ok(Some(()));
    }
    Ok(None)
}

pub(super) fn try_apply_native_lane_predicate_to_selection(
    predicate: &CovePredicate,
    lane: &LaneRef<'_>,
    selected: &mut SelectionMask,
) -> Result<Option<KernelStats>, CoveError> {
    if lane.row_count() != selected.len() {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match native lane row count {}",
            selected.len(),
            lane.row_count()
        )));
    }
    if let CovePredicate::Null { kind, .. } = predicate {
        let Some(validity) = lane.validity() else {
            return Ok(None);
        };
        let want_valid = matches!(kind, NullPredicateKind::IsNotNull);
        let (filtered, stats) =
            filter_validity(lane.row_count(), validity, want_valid, Some(selected));
        selected.clone_from_mask(&filtered);
        return Ok(Some(stats));
    }
    if matches!(lane, LaneRef::DecodeBoundary { .. }) {
        return Ok(None);
    }
    let (filtered, kernel_stats) = match lane {
        LaneRef::Bool {
            values,
            row_count,
            validity,
            ..
        } => {
            let (false_matches, true_matches) = match predicate {
                CovePredicate::Numeric { op, literal, .. } => {
                    let false_matches = match native_numcode_matches(
                        CoveLogicalType::Bool,
                        0,
                        native_numeric_op(*op),
                        native_numeric_literal(*literal),
                    ) {
                        Ok(value) => value,
                        Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                        Err(error) => return Err(error),
                    };
                    let true_matches = match native_numcode_matches(
                        CoveLogicalType::Bool,
                        1,
                        native_numeric_op(*op),
                        native_numeric_literal(*literal),
                    ) {
                        Ok(value) => value,
                        Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                        Err(error) => return Err(error),
                    };
                    (false_matches, true_matches)
                }
                CovePredicate::NumericIn { literals, .. } => {
                    let Some(false_matches) =
                        native_numcode_matches_any(CoveLogicalType::Bool, 0, literals)?
                    else {
                        return Ok(None);
                    };
                    let Some(true_matches) =
                        native_numcode_matches_any(CoveLogicalType::Bool, 1, literals)?
                    else {
                        return Ok(None);
                    };
                    (false_matches, true_matches)
                }
                CovePredicate::NumericNotIn { literals, .. } => {
                    let Some(false_matches) =
                        native_numcode_matches_any(CoveLogicalType::Bool, 0, literals)?
                    else {
                        return Ok(None);
                    };
                    let Some(true_matches) =
                        native_numcode_matches_any(CoveLogicalType::Bool, 1, literals)?
                    else {
                        return Ok(None);
                    };
                    (!false_matches, !true_matches)
                }
                _ => return Ok(None),
            };
            match (false_matches, true_matches) {
                (true, true) => filter_validity(*row_count, *validity, true, Some(selected)),
                (true, false) => filter_bool_eq(
                    values,
                    *row_count,
                    *validity,
                    false,
                    optional_base_for_selection(selected),
                )?,
                (false, true) => filter_bool_eq(
                    values,
                    *row_count,
                    *validity,
                    true,
                    optional_base_for_selection(selected),
                )?,
                (false, false) => {
                    let (valid, mut stats) =
                        filter_validity(*row_count, *validity, true, Some(selected));
                    stats.rows_matched = 0;
                    (SelectionMask::none(valid.len()), stats)
                }
            }
        }
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            logical_type,
            ..
        } => match predicate {
            CovePredicate::Numeric { op, literal, .. } => match filter_numcode_le_typed(
                bytes,
                *row_count,
                *validity,
                *logical_type,
                native_numeric_op(*op),
                native_numeric_literal(*literal),
                optional_base_for_selection(selected),
            ) {
                Ok((filtered, stats)) => (filtered, stats),
                Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                Err(error) => return Err(error),
            },
            CovePredicate::NumericIn { literals, .. } => {
                let literals = native_numeric_literals(literals);
                match filter_numcode_le_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    &literals,
                    optional_base_for_selection(selected),
                ) {
                    Ok((filtered, stats)) => (filtered, stats),
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
            CovePredicate::NumericNotIn { literals, .. } => {
                let literals = native_numeric_literals(literals);
                match filter_numcode_le_not_in_typed(
                    bytes,
                    *row_count,
                    *validity,
                    *logical_type,
                    &literals,
                    optional_base_for_selection(selected),
                ) {
                    Ok((filtered, stats)) => (filtered, stats),
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
            _ => return Ok(None),
        },
        LaneRef::FileCodeU32 {
            values, validity, ..
        } => match predicate {
            CovePredicate::FileCodeIn { file_codes, .. } => filter_u32_in_sorted(
                values,
                *validity,
                file_codes,
                optional_base_for_selection(selected),
            ),
            CovePredicate::FileCodeNotIn { file_codes, .. } => filter_u32_not_in_sorted(
                values,
                *validity,
                file_codes,
                optional_base_for_selection(selected),
            ),
            _ => return Ok(None),
        },
        LaneRef::FileCodeU32LeBytes {
            bytes,
            row_count,
            validity,
            ..
        } => match predicate {
            CovePredicate::FileCodeIn { file_codes, .. } => filter_u32_le_in_sorted(
                bytes,
                *row_count,
                *validity,
                file_codes,
                optional_base_for_selection(selected),
            )?,
            CovePredicate::FileCodeNotIn { file_codes, .. } => filter_u32_le_not_in_sorted(
                bytes,
                *row_count,
                *validity,
                file_codes,
                optional_base_for_selection(selected),
            )?,
            _ => return Ok(None),
        },
        LaneRef::FixedBytes {
            values,
            row_count,
            width,
            validity,
            ..
        } => match predicate {
            CovePredicate::FixedBytesEq { literal, .. } => filter_fixed_bytes_eq(
                values,
                *row_count,
                *width,
                *validity,
                literal,
                Some(selected),
            )?,
            CovePredicate::FixedBytesIn { literals, .. } => {
                let needles = flatten_fixed_bytes_literals(*width, literals)?;
                filter_fixed_bytes_in(
                    values,
                    *row_count,
                    *width,
                    *validity,
                    &needles,
                    Some(selected),
                )?
            }
            _ => return Ok(None),
        },
        LaneRef::LocalCodeU8 {
            values,
            validity,
            local_to_global,
            logical_type,
            physical_kind,
            ..
        } => {
            let Some(membership) =
                native_local_membership(predicate, *logical_type, *physical_kind, local_to_global)?
            else {
                return Ok(None);
            };
            filter_local_u8_membership(
                values,
                *validity,
                &membership,
                optional_base_for_selection(selected),
            )
        }
        LaneRef::LocalCodeU16 {
            values,
            validity,
            local_to_global,
            logical_type,
            physical_kind,
            ..
        } => {
            let Some(membership) =
                native_local_membership(predicate, *logical_type, *physical_kind, local_to_global)?
            else {
                return Ok(None);
            };
            filter_local_u16_membership(
                values,
                *validity,
                &membership,
                optional_base_for_selection(selected),
            )
        }
        LaneRef::LocalCodeU32 {
            values,
            validity,
            local_to_global,
            logical_type,
            physical_kind,
            ..
        } => {
            let Some(membership) =
                native_local_membership(predicate, *logical_type, *physical_kind, local_to_global)?
            else {
                return Ok(None);
            };
            filter_local_u32_membership(
                values,
                *validity,
                &membership,
                optional_base_for_selection(selected),
            )
        }
        LaneRef::VarBytes {
            row_offsets,
            values,
            validity,
            ..
        } => match predicate {
            CovePredicate::VarBytesEq { literal, .. } => {
                filter_varbytes_eq(row_offsets, values, *validity, literal, Some(selected))?
            }
            CovePredicate::VarBytesIn { literals, .. } => {
                let needles = borrowed_varbytes_literals(literals);
                filter_varbytes_in(row_offsets, values, *validity, &needles, Some(selected))?
            }
            CovePredicate::VarBytesPrefix { prefix, .. } => {
                filter_varbytes_prefix(row_offsets, values, *validity, prefix, Some(selected))?
            }
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    selected.clone_from_mask(&filtered);
    Ok(Some(kernel_stats))
}

fn try_apply_numcode_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    _scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    if array.encoding != CoveEncodingKind::NumCode || array.physical != CovePhysicalKind::NumCode {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len() != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match NumCode row count {row_count}",
            selected.len()
        )));
    }
    let validity = native_validity_for_array(array)?;
    let filtered = match predicate {
        CovePredicate::Numeric { op, literal, .. } => match filter_numcode_le_typed(
            array.data,
            row_count,
            validity,
            array.logical,
            native_numeric_op(*op),
            native_numeric_literal(*literal),
            Some(selected),
        ) {
            Ok((filtered, _stats)) => filtered,
            Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
            Err(error) => return Err(error),
        },
        CovePredicate::NumericIn { literals, .. } => {
            let literals = native_numeric_literals(literals);
            match filter_numcode_le_in_typed(
                array.data,
                row_count,
                validity,
                array.logical,
                &literals,
                Some(selected),
            ) {
                Ok((filtered, _stats)) => filtered,
                Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        CovePredicate::NumericNotIn { literals, .. } => {
            let literals = native_numeric_literals(literals);
            match filter_numcode_le_not_in_typed(
                array.data,
                row_count,
                validity,
                array.logical,
                &literals,
                Some(selected),
            ) {
                Ok((filtered, _stats)) => filtered,
                Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        _ => return Ok(None),
    };
    selected.clone_from_mask(&filtered);
    Ok(Some(()))
}

fn try_apply_local_codebook_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    _scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    if array.encoding != CoveEncodingKind::LocalCodebook {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len() != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match LocalCodebook row count {row_count}",
            selected.len()
        )));
    }

    let payload = LocalCodebookPayload::parse(array.data)?;
    let local_indexes = payload.decode_local_indexes()?;
    if local_indexes.len() != row_count {
        return Err(CoveError::PageCorrupt);
    }
    let validity = native_validity_for_array(array)?;
    let membership = match (predicate, array.physical, &payload.values) {
        (
            CovePredicate::FileCodeIn { file_codes, .. },
            CovePhysicalKind::FileCode,
            LocalCodebookValues::FileCode(values),
        ) => values
            .iter()
            .map(|value| file_codes.binary_search(value).is_ok())
            .collect::<Vec<_>>(),
        (
            CovePredicate::FileCodeNotIn { file_codes, .. },
            CovePhysicalKind::FileCode,
            LocalCodebookValues::FileCode(values),
        ) => values
            .iter()
            .map(|value| file_codes.binary_search(value).is_err())
            .collect::<Vec<_>>(),
        (
            CovePredicate::Numeric { op, literal, .. },
            CovePhysicalKind::NumCode,
            LocalCodebookValues::NumCode(values),
        ) => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                match native_numcode_matches(
                    array.logical,
                    *value,
                    native_numeric_op(*op),
                    native_numeric_literal(*literal),
                ) {
                    Ok(value) => membership.push(value),
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
            membership
        }
        (
            CovePredicate::NumericIn { literals, .. },
            CovePhysicalKind::NumCode,
            LocalCodebookValues::NumCode(values),
        ) => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                let Some(matches) = native_numcode_matches_any(array.logical, *value, literals)?
                else {
                    return Ok(None);
                };
                membership.push(matches);
            }
            membership
        }
        (
            CovePredicate::NumericNotIn { literals, .. },
            CovePhysicalKind::NumCode,
            LocalCodebookValues::NumCode(values),
        ) => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                let Some(matches) = native_numcode_matches_any(array.logical, *value, literals)?
                else {
                    return Ok(None);
                };
                membership.push(!matches);
            }
            membership
        }
        (
            CovePredicate::Numeric { op, literal, .. },
            CovePhysicalKind::Boolean,
            LocalCodebookValues::Boolean(values),
        ) if array.logical == CoveLogicalType::Bool => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                match native_numcode_matches(
                    CoveLogicalType::Bool,
                    u64::from(u8::from(*value)),
                    native_numeric_op(*op),
                    native_numeric_literal(*literal),
                ) {
                    Ok(value) => membership.push(value),
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
            membership
        }
        (
            CovePredicate::NumericIn { literals, .. },
            CovePhysicalKind::Boolean,
            LocalCodebookValues::Boolean(values),
        ) if array.logical == CoveLogicalType::Bool => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                let Some(matches) = native_numcode_matches_any(
                    CoveLogicalType::Bool,
                    u64::from(u8::from(*value)),
                    literals,
                )?
                else {
                    return Ok(None);
                };
                membership.push(matches);
            }
            membership
        }
        (
            CovePredicate::NumericNotIn { literals, .. },
            CovePhysicalKind::Boolean,
            LocalCodebookValues::Boolean(values),
        ) if array.logical == CoveLogicalType::Bool => {
            let mut membership = Vec::with_capacity(values.len());
            for value in values {
                let Some(matches) = native_numcode_matches_any(
                    CoveLogicalType::Bool,
                    u64::from(u8::from(*value)),
                    literals,
                )?
                else {
                    return Ok(None);
                };
                membership.push(!matches);
            }
            membership
        }
        (
            CovePredicate::VarBytesEq { literal, .. },
            CovePhysicalKind::VarBytes,
            LocalCodebookValues::VarBytes(values),
        ) => values
            .iter()
            .map(|value| value.as_slice() == literal.as_slice())
            .collect::<Vec<_>>(),
        (
            CovePredicate::VarBytesIn { literals, .. },
            CovePhysicalKind::VarBytes,
            LocalCodebookValues::VarBytes(values),
        ) => values
            .iter()
            .map(|value| {
                literals
                    .iter()
                    .any(|literal| value.as_slice() == literal.as_slice())
            })
            .collect::<Vec<_>>(),
        (
            CovePredicate::VarBytesPrefix { prefix, .. },
            CovePhysicalKind::VarBytes,
            LocalCodebookValues::VarBytes(values),
        ) => values
            .iter()
            .map(|value| value.starts_with(prefix))
            .collect::<Vec<_>>(),
        _ => return Ok(None),
    };

    let filtered = filter_decoded_local_indexes(
        &local_indexes,
        validity,
        &membership,
        optional_base_for_selection(selected),
    )?;
    selected.clone_from_mask(&filtered);
    Ok(Some(()))
}

fn filter_decoded_local_indexes(
    local_indexes: &[u32],
    validity: ValidityRef<'_>,
    membership: &[bool],
    base: Option<&SelectionMask>,
) -> Result<SelectionMask, CoveError> {
    if membership.len() <= (u8::MAX as usize) + 1 {
        let values = local_indexes
            .iter()
            .copied()
            .map(|value| u8::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        let (selected, _) = filter_local_u8_membership(&values, validity, membership, base);
        return Ok(selected);
    }

    if membership.len() <= (u16::MAX as usize) + 1 {
        let values = local_indexes
            .iter()
            .copied()
            .map(|value| u16::try_from(value).map_err(|_| CoveError::PageCorrupt))
            .collect::<Result<Vec<_>, _>>()?;
        let (selected, _) = filter_local_u16_membership(&values, validity, membership, base);
        return Ok(selected);
    }

    let (selected, _) = filter_local_u32_membership(local_indexes, validity, membership, base);
    Ok(selected)
}

fn native_local_membership(
    predicate: &CovePredicate,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    local_to_global: &[u64],
) -> Result<Option<Vec<bool>>, CoveError> {
    match (predicate, physical_kind) {
        (CovePredicate::FileCodeIn { file_codes, .. }, CovePhysicalKind::FileCode) => Ok(Some(
            local_to_global
                .iter()
                .copied()
                .map(|code| {
                    u32::try_from(code)
                        .ok()
                        .is_some_and(|code| file_codes.binary_search(&code).is_ok())
                })
                .collect(),
        )),
        (CovePredicate::FileCodeNotIn { file_codes, .. }, CovePhysicalKind::FileCode) => Ok(Some(
            local_to_global
                .iter()
                .copied()
                .map(|code| {
                    u32::try_from(code)
                        .ok()
                        .is_some_and(|code| file_codes.binary_search(&code).is_err())
                })
                .collect(),
        )),
        (CovePredicate::Numeric { op, literal, .. }, CovePhysicalKind::NumCode)
        | (CovePredicate::Numeric { op, literal, .. }, CovePhysicalKind::Boolean) => {
            let mut membership = Vec::with_capacity(local_to_global.len());
            for code in local_to_global {
                match native_numcode_matches(
                    logical_type,
                    *code,
                    native_numeric_op(*op),
                    native_numeric_literal(*literal),
                ) {
                    Ok(value) => membership.push(value),
                    Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
            }
            Ok(Some(membership))
        }
        (CovePredicate::NumericIn { literals, .. }, CovePhysicalKind::NumCode)
        | (CovePredicate::NumericIn { literals, .. }, CovePhysicalKind::Boolean) => {
            let mut membership = Vec::with_capacity(local_to_global.len());
            for code in local_to_global {
                let Some(matches) = native_numcode_matches_any(logical_type, *code, literals)?
                else {
                    return Ok(None);
                };
                membership.push(matches);
            }
            Ok(Some(membership))
        }
        (CovePredicate::NumericNotIn { literals, .. }, CovePhysicalKind::NumCode)
        | (CovePredicate::NumericNotIn { literals, .. }, CovePhysicalKind::Boolean) => {
            let mut membership = Vec::with_capacity(local_to_global.len());
            for code in local_to_global {
                let Some(matches) = native_numcode_matches_any(logical_type, *code, literals)?
                else {
                    return Ok(None);
                };
                membership.push(!matches);
            }
            Ok(Some(membership))
        }
        _ => Ok(None),
    }
}

fn native_validity_for_array<'a>(
    array: &'a EncodedArray<'a>,
) -> Result<ValidityRef<'a>, CoveError> {
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    match array.validity {
        Some(bitmap) => {
            if bitmap.row_count() != array.row_count {
                return Err(CoveError::PageCorrupt);
            }
            bitmap.validate_len(array.row_count)?;
            Ok(ValidityRef::CoveNullBitmap {
                bytes: bitmap.bytes(),
                row_count,
            })
        }
        None => Ok(ValidityRef::AllValid { row_count }),
    }
}

#[inline]
fn native_numeric_op(op: NumericPredicateOp) -> NativeNumericPredicateOp {
    match op {
        NumericPredicateOp::Eq => NativeNumericPredicateOp::Eq,
        NumericPredicateOp::Lt => NativeNumericPredicateOp::Lt,
        NumericPredicateOp::LtEq => NativeNumericPredicateOp::LtEq,
        NumericPredicateOp::Gt => NativeNumericPredicateOp::Gt,
        NumericPredicateOp::GtEq => NativeNumericPredicateOp::GtEq,
    }
}

#[inline]
fn native_numeric_literal(literal: PredicateLiteral) -> NativeNumericLiteral {
    match literal {
        PredicateLiteral::Int64(value) => NativeNumericLiteral::Int64(value),
        PredicateLiteral::UInt64(value) => NativeNumericLiteral::UInt64(value),
        PredicateLiteral::Float64(value) => NativeNumericLiteral::Float64(value),
    }
}

fn native_numeric_literals(literals: &[PredicateLiteral]) -> Vec<NativeNumericLiteral> {
    literals
        .iter()
        .copied()
        .map(native_numeric_literal)
        .collect()
}

fn native_numcode_matches_any(
    logical: CoveLogicalType,
    value: u64,
    literals: &[PredicateLiteral],
) -> Result<Option<bool>, CoveError> {
    for literal in literals {
        match native_numcode_matches(
            logical,
            value,
            NativeNumericPredicateOp::Eq,
            native_numeric_literal(*literal),
        ) {
            Ok(true) => return Ok(Some(true)),
            Ok(false) => {}
            Err(CoveError::UnsupportedEncoding(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(false))
}

fn try_apply_varbytes_eq_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
    _scratch: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    if !matches!(
        predicate,
        CovePredicate::VarBytesEq { .. } | CovePredicate::VarBytesIn { .. }
    ) {
        return Ok(None);
    }
    if array.encoding != CoveEncodingKind::VarBytes || array.physical != CovePhysicalKind::VarBytes
    {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len() != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match VarBytes row count {row_count}",
            selected.len()
        )));
    }
    let validity = native_validity_for_array(array)?;
    let filtered = match predicate {
        CovePredicate::VarBytesEq { literal, .. } => {
            filter_length_prefixed_varbytes_eq(
                array.data,
                row_count,
                validity,
                literal,
                Some(selected),
            )?
            .0
        }
        CovePredicate::VarBytesIn { literals, .. } => {
            let needles = borrowed_varbytes_literals(literals);
            filter_length_prefixed_varbytes_in(
                array.data,
                row_count,
                validity,
                &needles,
                Some(selected),
            )?
            .0
        }
        _ => unreachable!("VarBytes predicate checked above"),
    };
    selected.clone_from_mask(&filtered);
    Ok(Some(()))
}

fn borrowed_varbytes_literals(literals: &[Vec<u8>]) -> Vec<&[u8]> {
    literals.iter().map(Vec::as_slice).collect()
}

fn try_apply_varbytes_prefix_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    let CovePredicate::VarBytesPrefix { prefix, .. } = predicate else {
        return Ok(None);
    };
    if array.encoding != CoveEncodingKind::VarBytes || array.physical != CovePhysicalKind::VarBytes
    {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len() != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match VarBytes row count {row_count}",
            selected.len()
        )));
    }
    let validity = native_validity_for_array(array)?;
    let (filtered, _stats) = filter_length_prefixed_varbytes_prefix(
        array.data,
        row_count,
        validity,
        prefix,
        Some(selected),
    )?;
    selected.clone_from_mask(&filtered);
    Ok(Some(()))
}

fn try_apply_fixed_bytes_eq_predicate_to_selection(
    predicate: &CovePredicate,
    array: &EncodedArray<'_>,
    selected: &mut SelectionMask,
) -> Result<Option<()>, CoveError> {
    if !matches!(
        predicate,
        CovePredicate::FixedBytesEq { .. } | CovePredicate::FixedBytesIn { .. }
    ) {
        return Ok(None);
    }
    if array.encoding != CoveEncodingKind::PlainFixed
        || array.physical != CovePhysicalKind::FixedBytes
    {
        return Ok(None);
    }
    let row_count = usize::try_from(array.row_count).map_err(|_| CoveError::ArithOverflow)?;
    if selected.len() != row_count {
        return Err(CoveError::BadSection(format!(
            "selection length {} does not match FixedBytes row count {row_count}",
            selected.len()
        )));
    }
    let width = cove_core::array::logical_type_fixed_width(array.logical).ok_or_else(|| {
        CoveError::UnsupportedEncoding(format!(
            "FixedBytes predicate cannot infer width for {:?}",
            array.logical
        ))
    })?;
    let validity = native_validity_for_array(array)?;
    let filtered = match predicate {
        CovePredicate::FixedBytesEq { literal, .. } => {
            filter_fixed_bytes_eq(
                array.data,
                row_count,
                width,
                validity,
                literal,
                Some(selected),
            )?
            .0
        }
        CovePredicate::FixedBytesIn { literals, .. } => {
            let needles = flatten_fixed_bytes_literals(width, literals)?;
            filter_fixed_bytes_in(
                array.data,
                row_count,
                width,
                validity,
                &needles,
                Some(selected),
            )?
            .0
        }
        _ => unreachable!("fixed-byte predicate checked above"),
    };
    selected.clone_from_mask(&filtered);
    Ok(Some(()))
}

fn flatten_fixed_bytes_literals(width: usize, literals: &[Vec<u8>]) -> Result<Vec<u8>, CoveError> {
    if width == 0 || literals.is_empty() {
        return Err(CoveError::BadSchema(
            "fixed-byte IN requires a non-empty literal set".into(),
        ));
    }
    let mut needles = Vec::with_capacity(
        width
            .checked_mul(literals.len())
            .ok_or(CoveError::ArithOverflow)?,
    );
    for literal in literals {
        if literal.len() != width {
            return Err(CoveError::BadSchema(format!(
                "fixed-byte IN literal has width {}, expected {width}",
                literal.len()
            )));
        }
        needles.extend_from_slice(literal);
    }
    Ok(needles)
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

fn compare_numeric_in_value(
    logical: CoveLogicalType,
    value: &CoveArrayValue<'_>,
    literals: &[PredicateLiteral],
) -> Result<Option<bool>, CoveError> {
    for literal in literals {
        match compare_numeric_value(logical, value, NumericPredicateOp::Eq, *literal)? {
            Some(true) => return Ok(Some(true)),
            Some(false) => {}
            None => return Ok(None),
        }
    }
    Ok(Some(false))
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

#[cfg(test)]
mod tests {
    use super::*;
    use cove_core::encoding::{
        bit_packed::BitPackedPayload,
        local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
    };
    use cove_core::native::{NativeCodeDomain, NativeLaneId};
    use std::borrow::Cow;

    fn assert_auto_dispatch(stats: &KernelStats) {
        #[cfg(target_arch = "aarch64")]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Neon);
        #[cfg(target_arch = "x86_64")]
        {
            let expected = if std::arch::is_x86_feature_detected!("avx2") {
                NativeKernelDispatch::Avx2
            } else {
                NativeKernelDispatch::Scalar
            };
            assert_eq!(stats.dispatch, expected);
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    }

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

    #[test]
    fn local_codebook_filecode_predicate_uses_local_membership() {
        let payload = LocalCodebookPayload {
            values: LocalCodebookValues::FileCode(vec![10, 20, 40]),
            indexes: LocalIndexPayload::BitPacked(
                BitPackedPayload::pack(&[0, 1, 2, 1], 2).unwrap(),
            ),
        }
        .encode();
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::FileCode,
            4,
            CoveEncodingKind::LocalCodebook,
            None,
            &payload,
            None,
        );
        let predicate = CovePredicate::FileCodeIn {
            column_index: 0,
            file_codes: vec![20, 40],
            canonical_values: Vec::new(),
            canonical_keys: Vec::new(),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(4);
        let mut scratch = SelectionMask::default();

        assert_eq!(
            try_apply_raw_predicate_to_selection(&predicate, &array, &mut selected, &mut scratch)
                .unwrap(),
            Some(())
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }

    #[test]
    fn native_local_codebook_lane_elides_all_set_base_for_simd_dispatch() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| (row % 16) as u8)
            .collect::<Vec<_>>();
        let local_to_global = (0..16u64).map(|value| value * 10).collect::<Vec<_>>();
        let lane = LaneRef::LocalCodeU8 {
            lane_id: NativeLaneId(0),
            values: Cow::Owned(values),
            validity: ValidityRef::AllValid { row_count },
            local_to_global: Cow::Owned(local_to_global),
            logical_type: CoveLogicalType::Utf8,
            physical_kind: CovePhysicalKind::FileCode,
            domain: NativeCodeDomain::default(),
        };
        let predicate = CovePredicate::FileCodeIn {
            column_index: 0,
            file_codes: vec![10, 30, 50],
            canonical_values: Vec::new(),
            canonical_keys: Vec::new(),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(row_count);

        let stats = try_apply_native_lane_predicate_to_selection(&predicate, &lane, &mut selected)
            .unwrap()
            .unwrap();

        assert_auto_dispatch(&stats);
        assert!(selected.contains(1));
        assert!(!selected.contains(2));
        assert!(selected.contains(3));
        assert!(selected.contains(5));
        assert_eq!(stats.rows_seen, row_count);
        assert_eq!(stats.rows_valid, row_count);
        assert_eq!(stats.rows_matched, selected.count_ones());
    }

    #[test]
    fn native_direct_filecode_u32_lane_elides_all_set_base_for_simd_dispatch() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| ((row % 16) * 10) as u32)
            .collect::<Vec<_>>();
        let lane = LaneRef::FileCodeU32 {
            lane_id: NativeLaneId(0),
            values: &values,
            validity: ValidityRef::AllValid { row_count },
            logical_type: CoveLogicalType::Utf8,
            domain: NativeCodeDomain::default(),
        };
        let predicate = CovePredicate::FileCodeIn {
            column_index: 0,
            file_codes: vec![10, 30, 50],
            canonical_values: Vec::new(),
            canonical_keys: Vec::new(),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(row_count);

        let stats = try_apply_native_lane_predicate_to_selection(&predicate, &lane, &mut selected)
            .unwrap()
            .unwrap();

        assert_auto_dispatch(&stats);
        assert!(selected.contains(1));
        assert!(!selected.contains(2));
        assert!(selected.contains(3));
        assert!(selected.contains(5));
        assert_eq!(stats.rows_seen, row_count);
        assert_eq!(stats.rows_valid, row_count);
        assert_eq!(stats.rows_matched, selected.count_ones());

        let predicate = CovePredicate::FileCodeNotIn {
            column_index: 0,
            file_codes: vec![10, 30, 50],
            canonical_values: Vec::new(),
            canonical_keys: Vec::new(),
        };
        selected.fill_all(row_count);

        let stats = try_apply_native_lane_predicate_to_selection(&predicate, &lane, &mut selected)
            .unwrap()
            .unwrap();

        assert_auto_dispatch(&stats);
        assert!(!selected.contains(1));
        assert!(selected.contains(2));
        assert!(!selected.contains(3));
        assert!(!selected.contains(5));
        assert_eq!(stats.rows_seen, row_count);
        assert_eq!(stats.rows_valid, row_count);
        assert_eq!(stats.rows_matched, selected.count_ones());
    }

    #[test]
    fn native_bool_lane_elides_all_set_base_for_simd_dispatch() {
        let row_count = 257;
        let values = (0..row_count)
            .map(|row| if row % 7 == 0 || row % 11 == 0 { 1 } else { 0 })
            .collect::<Vec<_>>();
        let lane = LaneRef::Bool {
            lane_id: NativeLaneId(0),
            values: &values,
            row_count,
            validity: ValidityRef::AllValid { row_count },
            domain: NativeCodeDomain::default(),
        };
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Eq,
            literal: PredicateLiteral::UInt64(1),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(row_count);

        let stats = try_apply_native_lane_predicate_to_selection(&predicate, &lane, &mut selected)
            .unwrap()
            .unwrap();

        assert_auto_dispatch(&stats);
        assert!(selected.contains(0));
        assert!(!selected.contains(1));
        assert!(selected.contains(7));
        assert!(selected.contains(11));
        assert_eq!(stats.rows_seen, row_count);
        assert_eq!(stats.rows_valid, row_count);
        assert_eq!(stats.rows_matched, selected.count_ones());
    }

    #[test]
    fn local_codebook_numcode_predicate_uses_typed_membership() {
        let payload = LocalCodebookPayload {
            values: LocalCodebookValues::NumCode(vec![3, 17, 255]),
            indexes: LocalIndexPayload::BitPacked(
                BitPackedPayload::pack(&[0, 1, 2, 1], 2).unwrap(),
            ),
        }
        .encode();
        let array = EncodedArray::new(
            CoveLogicalType::UInt64,
            CovePhysicalKind::NumCode,
            4,
            CoveEncodingKind::LocalCodebook,
            None,
            &payload,
            None,
        );
        let predicate = CovePredicate::Numeric {
            column_index: 0,
            op: NumericPredicateOp::Gt,
            literal: PredicateLiteral::UInt64(16),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(4);
        let mut scratch = SelectionMask::default();

        assert_eq!(
            try_apply_raw_predicate_to_selection(&predicate, &array, &mut selected, &mut scratch)
                .unwrap(),
            Some(())
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }

    #[test]
    fn local_codebook_varbytes_predicate_uses_local_membership() {
        let payload = LocalCodebookPayload {
            values: LocalCodebookValues::VarBytes(vec![
                b"red".to_vec(),
                b"blue".to_vec(),
                b"rose".to_vec(),
            ]),
            indexes: LocalIndexPayload::BitPacked(
                BitPackedPayload::pack(&[0, 1, 2, 1], 2).unwrap(),
            ),
        }
        .encode();
        let array = EncodedArray::new(
            CoveLogicalType::Utf8,
            CovePhysicalKind::VarBytes,
            4,
            CoveEncodingKind::LocalCodebook,
            None,
            &payload,
            None,
        );
        let predicate = CovePredicate::VarBytesEq {
            column_index: 0,
            literal: b"blue".to_vec(),
        };
        let mut selected = SelectionMask::default();
        selected.fill_all(4);
        let mut scratch = SelectionMask::default();

        assert_eq!(
            try_apply_raw_predicate_to_selection(&predicate, &array, &mut selected, &mut scratch)
                .unwrap(),
            Some(())
        );

        let mut rows = Vec::new();
        selected.write_selected_rows(&mut rows).unwrap();
        assert_eq!(rows, vec![1, 3]);
    }
}
