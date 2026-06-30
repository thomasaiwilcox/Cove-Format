use super::*;
use crate::{
    checksum,
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind},
    encoding::{
        bit_packed::BitPackedPayload,
        local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
    },
    page::ColumnPageIndexEntryV1,
    page_payload::{ColumnPagePayloadV1, RetainedColumnPagePayloadV1},
    profile::cove_o::{
        RecordKind, RetainedTemporalPropertyColumn, RetainedTemporalPropertyPage,
        RetainedTemporalSegmentData, TemporalPropertyColumn, TemporalPropertyPage,
        TemporalRowEntryV1, TemporalSegmentData, TemporalSegmentHeaderV1,
    },
    retained_bytes::RetainedBytes,
    segment::{
        RowMorselDirectory, RowMorselEntryV1, TableColumnDirectoryEntryV1, TableSegmentHeaderV1,
        TableSegmentPayloadV1,
    },
};

fn assert_fixed_width_auto_dispatch(stats: &KernelStats) {
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

fn assert_local_membership_auto_dispatch(stats: &KernelStats) {
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
fn bitmap_to_selection_vector_uses_set_bits_only() {
    let mut bitmap = SelectionBitmap::all(130);
    bitmap.clear(0);
    bitmap.clear(64);
    bitmap.clear(129);
    let vector = bitmap.to_selection_vector();
    assert_eq!(vector.len(), 127);
    assert_eq!(vector.rows()[0], 1);
    assert_eq!(vector.rows()[63], 65);
    assert_eq!(vector.rows()[126], 128);
}

#[test]
fn compact_selection_bitmap_uses_dense_word_fast_path_and_tail_bits() {
    let mut bitmap = SelectionBitmap::all(130);
    bitmap.clear(64);
    bitmap.clear(129);
    let (vector, stats) = compact_selection_bitmap(&bitmap, NativeKernelDispatch::Auto).unwrap();

    assert_eq!(vector.len(), 128);
    assert_eq!(vector.rows()[0], 0);
    assert_eq!(vector.rows()[63], 63);
    assert_eq!(vector.rows()[64], 65);
    assert_eq!(vector.rows()[127], 128);
    assert_eq!(stats.rows_seen, 130);
    assert_eq!(stats.rows_matched, 128);
    assert_eq!(stats.rows_valid, 128);
    assert_fixed_width_auto_dispatch(&stats);
}

#[test]
fn validity_ref_uses_cove_null_polarity() {
    let bytes = [0b0000_0101u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &bytes,
        row_count: 4,
    };
    assert!(!validity.is_valid(0));
    assert!(validity.is_valid(1));
    assert!(!validity.is_valid(2));
    assert!(validity.is_valid(3));
    assert_eq!(validity.valid_count(), 2);
}

#[test]
fn validity_ref_all_null_never_accepts_rows() {
    let validity = ValidityRef::AllNull { row_count: 3 };
    assert_eq!(validity.row_count(), 3);
    assert_eq!(validity.valid_count(), 0);
    assert!(!validity.is_valid(0));
    assert!(!validity.is_valid(2));
}

#[test]
fn validity_filter_respects_base_and_null_polarity() {
    let nulls = [0b0000_1010u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: 5,
    };
    let mut base = SelectionBitmap::all(5);
    base.clear(2);

    let (is_null, stats) = filter_validity(5, validity, false, Some(&base));

    assert_eq!(is_null.to_selection_vector().rows(), &[1, 3]);
    assert_eq!(stats.rows_seen, 5);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn validity_filter_fast_paths_all_valid_and_all_null() {
    let mut base = SelectionBitmap::none(4);
    base.set(1);
    base.set(3);

    let (all_valid, _) =
        filter_validity(4, ValidityRef::AllValid { row_count: 4 }, true, Some(&base));
    let (no_nulls, _) = filter_validity(
        4,
        ValidityRef::AllValid { row_count: 4 },
        false,
        Some(&base),
    );
    let (all_null, _) =
        filter_validity(4, ValidityRef::AllNull { row_count: 4 }, false, Some(&base));

    assert_eq!(all_valid, base);
    assert_eq!(no_nulls.count_ones(), 0);
    assert_eq!(all_null.to_selection_vector().rows(), &[1, 3]);
}

#[test]
fn selection_bitmap_all_set_respects_tail_bits() {
    let mut selected = SelectionBitmap::all(130);
    assert!(selected.all_set());

    selected.clear(129);
    assert!(!selected.all_set());

    selected.set(129);
    assert!(selected.all_set());

    selected.words_mut()[2] |= !((1u64 << 2) - 1);
    assert!(!selected.all_set());

    let empty = SelectionBitmap::all(0);
    assert!(empty.all_set());
}

#[test]
fn u64_eq_filter_respects_validity_and_base() {
    let values = [7, 8, 7, 7, 9];
    let nulls = [0b0000_1000u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(0);
    let (selected, stats) = filter_u64_eq(&values, validity, 7, Some(&base));
    assert_eq!(selected.to_selection_vector().rows(), &[2]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn u64_le_eq_filter_reads_page_bytes_without_typed_casts() {
    let values = [7u64, 8, 7, 9];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };

    let (selected, stats) = filter_u64_le_eq(&bytes, values.len(), validity, 7, None).unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn u64_le_eq_dispatch_auto_matches_scalar_on_all_valid_page() {
    let row_count = 257;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u64>());
    let mut expected_rows = Vec::new();
    for row in 0..row_count {
        let value = if row % 29 == 0 { 42 } else { (row % 17) as u64 };
        if value == 42 {
            expected_rows.push(row as u32);
        }
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };

    let (scalar, scalar_stats) = filter_u64_le_eq_dispatch(
        &bytes,
        row_count,
        validity,
        42,
        None,
        NativeKernelDispatch::Scalar,
    )
    .unwrap();
    let (auto, auto_stats) = filter_u64_le_eq_dispatch(
        &bytes,
        row_count,
        validity,
        42,
        None,
        NativeKernelDispatch::Auto,
    )
    .unwrap();

    assert_eq!(auto, scalar);
    assert_eq!(auto.to_selection_vector().rows(), expected_rows.as_slice());
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_fixed_width_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn u64_le_eq_dispatch_auto_falls_back_with_validity_or_base() {
    let values = [42u64, 7, 42, 13, 42, 19];
    let mut bytes = Vec::with_capacity(values.len() * std::mem::size_of::<u64>());
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0001_0000u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let (with_validity, validity_stats) = filter_u64_le_eq_dispatch(
        &bytes,
        values.len(),
        validity,
        42,
        None,
        NativeKernelDispatch::Auto,
    )
    .unwrap();
    assert_eq!(with_validity.to_selection_vector().rows(), &[0, 2]);
    assert_eq!(validity_stats.dispatch, NativeKernelDispatch::Scalar);

    let mut base = SelectionBitmap::all(values.len());
    base.clear(0);
    let (with_base, base_stats) = filter_u64_le_eq_dispatch(
        &bytes,
        values.len(),
        ValidityRef::AllValid {
            row_count: values.len(),
        },
        42,
        Some(&base),
        NativeKernelDispatch::Auto,
    )
    .unwrap();
    assert_eq!(with_base.to_selection_vector().rows(), &[2, 4]);
    assert_eq!(base_stats.dispatch, NativeKernelDispatch::Scalar);
}

#[test]
fn u32_le_in_filter_respects_base_and_validity() {
    let values = [10u32, 20, 30, 40];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(1);

    let (selected, stats) =
        filter_u32_le_in_sorted(&bytes, values.len(), validity, &[20, 30, 40], Some(&base))
            .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[3]);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn u32_not_in_filter_respects_base_and_validity() {
    let values = [10u32, 20, 30, 40, 50];
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(4);

    let (selected, stats) = filter_u32_not_in_sorted(&values, validity, &[20, 40], Some(&base));

    assert_eq!(selected.to_selection_vector().rows(), &[0]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn direct_u32_in_and_not_in_auto_match_scalar_for_small_needles() {
    let row_count = 257;
    let values = (0..row_count)
        .map(|row| (row % 64) as u32)
        .collect::<Vec<_>>();
    let needles = [3u32, 7, 13, 31, 61];
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    let (scalar_in, scalar_in_stats) =
        filter_u32_in_sorted(&values, validity, &needles, Some(&base));
    let (auto_in, auto_in_stats) = filter_u32_in_sorted(&values, validity, &needles, None);
    assert_eq!(auto_in, scalar_in);
    assert_eq!(scalar_in_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto_in_stats);
    assert_eq!(auto_in_stats.rows_seen, row_count);
    assert_eq!(auto_in_stats.rows_valid, row_count);
    assert_eq!(auto_in_stats.rows_matched, scalar_in_stats.rows_matched);

    let (scalar_not_in, scalar_not_in_stats) =
        filter_u32_not_in_sorted(&values, validity, &needles, Some(&base));
    let (auto_not_in, auto_not_in_stats) =
        filter_u32_not_in_sorted(&values, validity, &needles, None);
    assert_eq!(auto_not_in, scalar_not_in);
    assert_eq!(scalar_not_in_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto_not_in_stats);
    assert_eq!(auto_not_in_stats.rows_seen, row_count);
    assert_eq!(auto_not_in_stats.rows_valid, row_count);
    assert_eq!(
        auto_not_in_stats.rows_matched,
        scalar_not_in_stats.rows_matched
    );
}

#[test]
fn u32_le_not_in_filter_respects_base_and_validity() {
    let values = [10u32, 20, 30, 40, 50];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(4);

    let (selected, stats) =
        filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20, 40], Some(&base))
            .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn u32_le_single_needle_not_in_filter_respects_nulls_without_base() {
    let values = [10u32, 20, 30, 20, 50];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };

    let (selected, stats) =
        filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20], None).unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
    assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn u32_le_single_needle_not_in_filter_masks_tail_bits() {
    let row_count = 130;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
    for row in 0..row_count {
        bytes.extend_from_slice(&((row as u32) + 1).to_le_bytes());
    }

    let (selected, stats) = filter_u32_le_not_in_sorted(
        &bytes,
        row_count,
        ValidityRef::AllValid { row_count },
        &[999],
        None,
    )
    .unwrap();

    assert!(selected.all_set());
    assert_eq!(selected.count_ones(), row_count);
    assert_fixed_width_auto_dispatch(&stats);
    assert_eq!(stats.rows_matched, row_count);
}

#[test]
fn u32_le_single_needle_not_in_filter_treats_missing_null_bytes_as_null() {
    let values = [10u32, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &[0],
        row_count: values.len(),
    };

    let (selected, stats) =
        filter_u32_le_not_in_sorted(&bytes, values.len(), validity, &[20], None).unwrap();

    assert_eq!(
        selected.to_selection_vector().rows(),
        &[0, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    assert_eq!(stats.rows_valid, 8);
    assert_eq!(stats.rows_matched, 7);
}

#[test]
fn u32_le_single_needle_in_filter_matches_scalar_eq_dispatch() {
    let row_count = 259;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
    let mut expected_rows = Vec::new();
    for row in 0..row_count {
        let value = if row % 31 == 0 { 99 } else { (row % 23) as u32 };
        if value == 99 {
            expected_rows.push(row as u32);
        }
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };

    let (scalar, scalar_stats) = filter_u32_le_eq_dispatch(
        &bytes,
        row_count,
        validity,
        99,
        None,
        NativeKernelDispatch::Scalar,
    )
    .unwrap();
    let (auto_in, auto_stats) =
        filter_u32_le_in_sorted(&bytes, row_count, validity, &[99], None).unwrap();

    assert_eq!(auto_in, scalar);
    assert_eq!(
        auto_in.to_selection_vector().rows(),
        expected_rows.as_slice()
    );
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_fixed_width_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn u32_le_single_needle_not_in_filter_matches_scalar_complement_dispatch() {
    let row_count = 259;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<u32>());
    let mut expected_rows = Vec::new();
    for row in 0..row_count {
        let value = if row % 31 == 0 { 99 } else { (row % 23) as u32 };
        if value != 99 {
            expected_rows.push(row as u32);
        }
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    let (scalar, scalar_stats) =
        filter_u32_le_not_in_sorted(&bytes, row_count, validity, &[99], Some(&base)).unwrap();
    let (auto_not_in, auto_stats) =
        filter_u32_le_not_in_sorted(&bytes, row_count, validity, &[99], None).unwrap();

    assert_eq!(auto_not_in, scalar);
    assert_eq!(
        auto_not_in.to_selection_vector().rows(),
        expected_rows.as_slice()
    );
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_fixed_width_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn numcode_le_not_in_typed_filters_valid_complement_rows() {
    let values = [1u64, 2, 3, 4, 5];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(4);

    let (selected, stats) = filter_numcode_le_not_in_typed(
        &bytes,
        values.len(),
        validity,
        CoveLogicalType::UInt64,
        &[
            NativeNumericLiteral::UInt64(2),
            NativeNumericLiteral::UInt64(4),
        ],
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn i64_range_filter_handles_inclusive_and_exclusive_bounds() {
    let values = [1, 2, 3, 4, 5];
    let (selected, stats) = filter_i64_range(
        &values,
        ValidityRef::AllValid {
            row_count: values.len(),
        },
        Some((2, BoundInclusive::Exclusive)),
        Some((5, BoundInclusive::Exclusive)),
        None,
    );
    assert_eq!(selected.to_selection_vector().rows(), &[2, 3]);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn i64_le_range_filter_handles_signed_bounds() {
    let values = [-3i64, -1, 0, 4, 9];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let (selected, stats) = filter_i64_le_range(
        &bytes,
        values.len(),
        ValidityRef::AllValid {
            row_count: values.len(),
        },
        Some((-1, BoundInclusive::Inclusive)),
        Some((9, BoundInclusive::Exclusive)),
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[1, 2, 3]);
    assert_eq!(stats.rows_matched, 3);
}

#[test]
fn i64_le_range_auto_matches_scalar_for_bound_variants() {
    let row_count = 257;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
    for row in 0..row_count {
        let value = match row {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => (row as i64 % 53) - 26,
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    for (lower, upper) in [
        (
            Some((-3, BoundInclusive::Inclusive)),
            Some((11, BoundInclusive::Exclusive)),
        ),
        (
            Some((-3, BoundInclusive::Exclusive)),
            Some((11, BoundInclusive::Inclusive)),
        ),
        (None, Some((0, BoundInclusive::Exclusive))),
        (Some((0, BoundInclusive::Inclusive)), None),
        (None, None),
    ] {
        let (scalar, scalar_stats) =
            filter_i64_le_range(&bytes, row_count, validity, lower, upper, Some(&base)).unwrap();
        let (auto, auto_stats) =
            filter_i64_le_range(&bytes, row_count, validity, lower, upper, None).unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }
}

#[test]
fn typed_numcode_filter_preserves_integer_float_edges() {
    let values = [0u64, 1u64 << 53, (1u64 << 53) + 1, u64::MAX];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    let (selected, stats) = filter_numcode_le_typed(
        &bytes,
        values.len(),
        ValidityRef::AllValid {
            row_count: values.len(),
        },
        CoveLogicalType::UInt64,
        NativeNumericPredicateOp::Gt,
        NativeNumericLiteral::Float64((1u64 << 53) as f64),
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[2, 3]);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn typed_numcode_int64_cmp_auto_matches_scalar_for_all_ops() {
    let row_count = 257;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
    for row in 0..row_count {
        let value = match row {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => (row as i64 % 41) - 20,
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    for op in [
        NativeNumericPredicateOp::Eq,
        NativeNumericPredicateOp::Lt,
        NativeNumericPredicateOp::LtEq,
        NativeNumericPredicateOp::Gt,
        NativeNumericPredicateOp::GtEq,
    ] {
        let (scalar, scalar_stats) = filter_numcode_le_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            op,
            NativeNumericLiteral::Int64(-3),
            Some(&base),
        )
        .unwrap();
        let (auto, auto_stats) = filter_numcode_le_typed(
            &bytes,
            row_count,
            validity,
            CoveLogicalType::Int64,
            op,
            NativeNumericLiteral::Int64(-3),
            None,
        )
        .unwrap();

        assert_eq!(auto, scalar);
        assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
        assert_fixed_width_auto_dispatch(&auto_stats);
        assert_eq!(auto_stats.rows_seen, row_count);
        assert_eq!(auto_stats.rows_valid, row_count);
        assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
    }
}

#[test]
fn typed_numcode_in_filter_scans_once_with_base_and_validity() {
    let values = [5u64, 12, 20, 12];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0010u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };
    let mut base = SelectionBitmap::all(values.len());
    base.clear(0);

    let (selected, stats) = filter_numcode_le_in_typed(
        &bytes,
        values.len(),
        validity,
        CoveLogicalType::UInt64,
        &[
            NativeNumericLiteral::UInt64(5),
            NativeNumericLiteral::UInt64(12),
        ],
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[3]);
    assert_eq!(stats.rows_seen, 4);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn typed_numcode_single_literal_in_reuses_equality_dispatch() {
    let row_count = 257;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
    for row in 0..row_count {
        let value = if row % 37 == 0 {
            -11
        } else {
            (row as i64 % 19) - 9
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };
    let literal = [NativeNumericLiteral::Int64(-11)];
    let base = SelectionBitmap::all(row_count);

    let (scalar, scalar_stats) = filter_numcode_le_in_typed(
        &bytes,
        row_count,
        validity,
        CoveLogicalType::Int64,
        &literal,
        Some(&base),
    )
    .unwrap();
    let (auto, auto_stats) = filter_numcode_le_in_typed(
        &bytes,
        row_count,
        validity,
        CoveLogicalType::Int64,
        &literal,
        None,
    )
    .unwrap();

    assert_eq!(auto, scalar);
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_fixed_width_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn typed_numcode_single_literal_not_in_reuses_equality_dispatch() {
    let row_count = 257;
    let mut bytes = Vec::with_capacity(row_count * std::mem::size_of::<i64>());
    for row in 0..row_count {
        let value = if row % 37 == 0 {
            -11
        } else {
            (row as i64 % 19) - 9
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let validity = ValidityRef::AllValid { row_count };
    let literal = [NativeNumericLiteral::Int64(-11)];
    let base = SelectionBitmap::all(row_count);

    let (scalar, scalar_stats) = filter_numcode_le_not_in_typed(
        &bytes,
        row_count,
        validity,
        CoveLogicalType::Int64,
        &literal,
        Some(&base),
    )
    .unwrap();
    let (auto, auto_stats) = filter_numcode_le_not_in_typed(
        &bytes,
        row_count,
        validity,
        CoveLogicalType::Int64,
        &literal,
        None,
    )
    .unwrap();

    assert_eq!(auto, scalar);
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_fixed_width_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn typed_numcode_single_literal_not_in_excludes_null_rows() {
    let values = [1u64, 2, 3, 2, 5];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let nulls = [0b0000_0100u8];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: values.len(),
    };

    let (selected, stats) = filter_numcode_le_not_in_typed(
        &bytes,
        values.len(),
        validity,
        CoveLogicalType::UInt64,
        &[NativeNumericLiteral::UInt64(2)],
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
    assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn local_u8_membership_translates_global_needles_once() {
    let local_to_global = [10, 20, 30, 40];
    let membership = local_membership_u8(&local_to_global, &[20, 40]);
    let values = [0, 1, 3, 2, 1];
    let (selected, stats) = filter_local_u8_membership(
        &values,
        ValidityRef::AllValid {
            row_count: values.len(),
        },
        &membership,
        None,
    );
    assert_eq!(selected.to_selection_vector().rows(), &[1, 2, 4]);
    assert_eq!(stats.rows_matched, 3);
}

#[test]
fn local_u8_membership_auto_matches_scalar_for_small_membership() {
    let row_count = 257;
    let values = (0..row_count)
        .map(|row| (row % 16) as u8)
        .collect::<Vec<_>>();
    let mut membership = vec![false; 16];
    for local in [1usize, 3, 5, 8, 13] {
        membership[local] = true;
    }
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    let (scalar, scalar_stats) =
        filter_local_u8_membership(&values, validity, &membership, Some(&base));
    let (auto, auto_stats) = filter_local_u8_membership(&values, validity, &membership, None);

    assert_eq!(auto, scalar);
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn fixed_bytes_in_filter_respects_base_and_validity() {
    let values = [1u8, 10, 2, 20, 3, 30, 4, 40];
    let nulls = [0b0000_0100];
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: 4,
    };
    let mut base = SelectionBitmap::all(4);
    base.clear(0);
    let needles = [1u8, 10, 3, 30, 4, 40];

    let (selected, stats) =
        filter_fixed_bytes_in(&values, 4, 2, validity, &needles, Some(&base)).unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[3]);
    assert_eq!(stats.rows_seen, 4);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn local_u16_and_u32_membership_use_local_codes_without_remap_per_row() {
    let local_to_global = [1, 3, 5, 7];
    let membership = local_membership_u8(&local_to_global, &[3, 7]);

    let values16 = [0u16, 1, 3, 2, 3];
    let (selected16, stats16) = filter_local_u16_membership(
        &values16,
        ValidityRef::AllValid {
            row_count: values16.len(),
        },
        &membership,
        None,
    );
    assert_eq!(selected16.to_selection_vector().rows(), &[1, 2, 4]);
    assert_eq!(stats16.rows_matched, 3);

    let values32 = [3u32, 2, 1, 0];
    let (selected32, stats32) = filter_local_u32_membership(
        &values32,
        ValidityRef::AllValid {
            row_count: values32.len(),
        },
        &membership,
        None,
    );
    assert_eq!(selected32.to_selection_vector().rows(), &[0, 2]);
    assert_eq!(stats32.rows_matched, 2);
}

#[test]
fn local_u16_and_u32_membership_auto_matches_scalar_for_small_membership() {
    let row_count = 259;
    let values16 = (0..row_count)
        .map(|row| (row % 64) as u16)
        .collect::<Vec<_>>();
    let values32 = (0..row_count)
        .map(|row| (row % 64) as u32)
        .collect::<Vec<_>>();
    let mut membership = vec![false; 64];
    for local in [2usize, 7, 13, 31, 61] {
        membership[local] = true;
    }
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    let (scalar16, scalar16_stats) =
        filter_local_u16_membership(&values16, validity, &membership, Some(&base));
    let (auto16, auto16_stats) =
        filter_local_u16_membership(&values16, validity, &membership, None);
    assert_eq!(auto16, scalar16);
    assert_eq!(scalar16_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto16_stats);
    assert_eq!(auto16_stats.rows_seen, row_count);
    assert_eq!(auto16_stats.rows_valid, row_count);
    assert_eq!(auto16_stats.rows_matched, scalar16_stats.rows_matched);

    let (scalar32, scalar32_stats) =
        filter_local_u32_membership(&values32, validity, &membership, Some(&base));
    let (auto32, auto32_stats) =
        filter_local_u32_membership(&values32, validity, &membership, None);
    assert_eq!(auto32, scalar32);
    assert_eq!(scalar32_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto32_stats);
    assert_eq!(auto32_stats.rows_seen, row_count);
    assert_eq!(auto32_stats.rows_valid, row_count);
    assert_eq!(auto32_stats.rows_matched, scalar32_stats.rows_matched);
}

#[test]
fn bool_filter_reads_plain_fixed_bytes() {
    let values = [1u8, 0, 1, 1];
    let nulls = [0b0000_0100u8];
    let (selected, stats) = filter_bool_eq(
        &values,
        values.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        true,
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0, 3]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn bool_eq_auto_matches_scalar_on_all_valid_page() {
    let row_count = 257;
    let values = (0..row_count)
        .map(|row| if row % 7 == 0 || row % 11 == 0 { 1 } else { 0 })
        .collect::<Vec<_>>();
    let validity = ValidityRef::AllValid { row_count };
    let base = SelectionBitmap::all(row_count);

    let (scalar, scalar_stats) =
        filter_bool_eq(&values, row_count, validity, true, Some(&base)).unwrap();
    let (auto, auto_stats) = filter_bool_eq(&values, row_count, validity, true, None).unwrap();

    assert_eq!(auto, scalar);
    assert_eq!(scalar_stats.dispatch, NativeKernelDispatch::Scalar);
    assert_local_membership_auto_dispatch(&auto_stats);
    assert_eq!(auto_stats.rows_seen, row_count);
    assert_eq!(auto_stats.rows_valid, row_count);
    assert_eq!(auto_stats.rows_matched, scalar_stats.rows_matched);
}

#[test]
fn bool_eq_auto_rejects_invalid_all_valid_byte() {
    let values = [1u8, 0, 2];

    assert_eq!(
        filter_bool_eq(
            &values,
            values.len(),
            ValidityRef::AllValid {
                row_count: values.len(),
            },
            true,
            None,
        )
        .unwrap_err(),
        CoveError::PageCorrupt
    );
}

#[test]
fn fixed_bytes_filter_compares_width_sized_payloads() {
    let mut values = Vec::new();
    values.extend_from_slice(&[1u8; 16]);
    values.extend_from_slice(&[2u8; 16]);
    values.extend_from_slice(&[1u8; 16]);
    let mut base = SelectionBitmap::all(3);
    base.clear(0);

    let (selected, stats) = filter_fixed_bytes_eq(
        &values,
        3,
        16,
        ValidityRef::AllValid { row_count: 3 },
        &[1u8; 16],
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[2]);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn varbytes_filters_use_prepared_length_prefix_offsets() {
    let values = length_prefixed_rows([b"hi".as_slice(), b"bye".as_slice(), b"hill".as_slice()]);
    let row_offsets =
        prepare_varbytes_row_offsets(&values, 3, ValidityRef::AllValid { row_count: 3 }).unwrap();

    let (equals, eq_stats) = filter_varbytes_eq(
        &row_offsets,
        &values,
        ValidityRef::AllValid { row_count: 3 },
        b"bye",
        None,
    )
    .unwrap();
    assert_eq!(equals.to_selection_vector().rows(), &[1]);
    assert_eq!(eq_stats.rows_matched, 1);

    let needles = [b"hi".as_slice(), b"hill".as_slice()];
    let (included, in_stats) = filter_varbytes_in(
        &row_offsets,
        &values,
        ValidityRef::AllValid { row_count: 3 },
        &needles,
        None,
    )
    .unwrap();
    assert_eq!(included.to_selection_vector().rows(), &[0, 2]);
    assert_eq!(in_stats.rows_matched, 2);

    let (prefixed, prefix_stats) = filter_varbytes_prefix(
        &row_offsets,
        &values,
        ValidityRef::AllValid { row_count: 3 },
        b"hi",
        None,
    )
    .unwrap();
    assert_eq!(prefixed.to_selection_vector().rows(), &[0, 2]);
    assert_eq!(prefix_stats.rows_matched, 2);
}

#[test]
fn length_prefixed_varbytes_eq_scans_without_offset_allocation() {
    let values = length_prefixed_rows([b"aa".as_slice(), b"bbb".as_slice(), b"aa".as_slice()]);
    let nulls = [0b0000_0100u8];

    let (selected, stats) = filter_length_prefixed_varbytes_eq(
        &values,
        3,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: 3,
        },
        b"aa",
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0]);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn length_prefixed_varbytes_in_scans_without_offset_allocation() {
    let values = length_prefixed_rows([
        b"aa".as_slice(),
        b"bbb".as_slice(),
        b"cc".as_slice(),
        b"aa".as_slice(),
    ]);
    let nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(4);
    base.clear(0);
    let needles = [b"aa".as_slice(), b"cc".as_slice()];

    let (selected, stats) = filter_length_prefixed_varbytes_in(
        &values,
        4,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: 4,
        },
        &needles,
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[3]);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn length_prefixed_varbytes_prefix_scans_without_offset_allocation() {
    let values =
        length_prefixed_rows([b"alpha".as_slice(), b"beta".as_slice(), b"alps".as_slice()]);
    let mut base = SelectionBitmap::all(3);
    base.clear(0);

    let (selected, stats) = filter_length_prefixed_varbytes_prefix(
        &values,
        3,
        ValidityRef::AllValid { row_count: 3 },
        b"al",
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[2]);
    assert_eq!(stats.rows_valid, 2);
    assert_eq!(stats.rows_matched, 1);
}

#[test]
fn dense_u32_group_count_uses_local_codes_without_hashing() {
    let values = [0u32, 2, 1, 2, 0, 3, 2];
    let nulls = [0b0010_0000u8];
    let (groups, stats) = group_count_u32_dense(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        4,
        None,
    )
    .unwrap();

    assert_eq!(groups.counts, vec![2, 1, 3, 0]);
    assert_eq!(groups.null_count, 1);
    assert_eq!(groups.rows_grouped, 6);
    assert_eq!(stats.rows_matched, 6);
}

#[test]
fn dense_u8_and_u16_group_counts_use_local_codes_without_widening() {
    let values_u8 = [0u8, 2, 1, 2, 0, 3, 2];
    let values_u16 = [0u16, 2, 1, 2, 0, 3, 2];
    let nulls = [0b0010_0000u8];

    let (groups_u8, stats_u8) = group_count_u8_dense(
        &values_u8,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values_u8.len(),
        },
        4,
        None,
    )
    .unwrap();
    let (groups_u16, stats_u16) = group_count_u16_dense(
        &values_u16,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values_u16.len(),
        },
        4,
        None,
    )
    .unwrap();

    assert_eq!(groups_u8, groups_u16);
    assert_eq!(groups_u8.counts, vec![2, 1, 3, 0]);
    assert_eq!(groups_u8.null_count, 1);
    assert_eq!(groups_u8.rows_grouped, 6);
    assert_eq!(stats_u8.rows_matched, 6);
    assert_eq!(stats_u16.rows_matched, 6);
}

#[test]
fn bool_group_count_uses_two_dense_buckets() {
    let values = [1u8, 0, 1, 1, 0, 1];
    let nulls = [0b0000_1000u8];
    let mut base = SelectionBitmap::none(values.len());
    base.set(0);
    base.set(1);
    base.set(3);
    base.set(4);
    base.set(5);

    let (groups, stats) = group_count_bool(
        &values,
        values.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.counts, vec![2, 2]);
    assert_eq!(groups.null_count, 1);
    assert_eq!(groups.rows_grouped, 4);
    assert_eq!(stats.rows_seen, values.len());
    assert_eq!(stats.rows_matched, 4);
    assert_eq!(stats.bytes_touched_estimate, values.len());
}

#[test]
fn bool_key_i64_aggregate_uses_dense_group_accumulators() {
    let keys = [1u8, 0, 1, 0, 1];
    let values = [10i64, 20, 30, 40, 50];
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let key_nulls = [0b0001_0000u8];
    let value_nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(keys.len());
    base.clear(3);

    let (groups, stats) = aggregate_i64_le_bytes_by_bool(
        &keys,
        &value_bytes,
        keys.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &key_nulls,
            row_count: keys.len(),
        },
        ValidityRef::CoveNullBitmap {
            bytes: &value_nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.row_counts, vec![1, 2]);
    assert_eq!(groups.aggregates[0].count, 1);
    assert_eq!(groups.aggregates[0].sum, 20);
    assert_eq!(groups.aggregates[1].count, 1);
    assert_eq!(groups.aggregates[1].null_count, 1);
    assert_eq!(groups.aggregates[1].sum, 10);
    assert_eq!(groups.null_row_count, 1);
    assert_eq!(groups.null_aggregate.count, 1);
    assert_eq!(groups.null_aggregate.sum, 50);
    assert_eq!(stats.rows_seen, keys.len());
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn dense_u8_key_i64_aggregate_uses_local_code_accumulators() {
    let keys = [2u8, 0, 2, 1, 0];
    let values = [10i64, 20, 30, 40, 50];
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let key_nulls = [0b0000_1000u8];
    let value_nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(keys.len());
    base.clear(4);

    let (groups, stats) = aggregate_i64_le_bytes_by_u8_dense(
        &keys,
        &value_bytes,
        keys.len(),
        3,
        ValidityRef::CoveNullBitmap {
            bytes: &key_nulls,
            row_count: keys.len(),
        },
        ValidityRef::CoveNullBitmap {
            bytes: &value_nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.row_counts, vec![1, 0, 2]);
    assert_eq!(groups.aggregates[0].sum, 20);
    assert_eq!(groups.aggregates[2].count, 1);
    assert_eq!(groups.aggregates[2].null_count, 1);
    assert_eq!(groups.aggregates[2].sum, 10);
    assert_eq!(groups.null_row_count, 1);
    assert_eq!(groups.null_aggregate.sum, 40);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn u32_key_i64_aggregate_hashes_codes_without_materializing_keys() {
    let keys = [7u32, 9, 7, 11, 9];
    let values = [10i64, 20, 30, 40, 50];
    let key_bytes = keys
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let key_nulls = [0b0000_1000u8];
    let value_nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(keys.len());
    base.clear(4);

    let (groups, stats) = aggregate_i64_le_bytes_by_u32_le_bytes(
        &key_bytes,
        &value_bytes,
        keys.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &key_nulls,
            row_count: keys.len(),
        },
        ValidityRef::CoveNullBitmap {
            bytes: &value_nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.row_counts.get(&7), Some(&2));
    assert_eq!(groups.aggregates.get(&7).unwrap().count, 1);
    assert_eq!(groups.aggregates.get(&7).unwrap().null_count, 1);
    assert_eq!(groups.aggregates.get(&7).unwrap().sum, 10);
    assert_eq!(groups.row_counts.get(&9), Some(&1));
    assert_eq!(groups.aggregates.get(&9).unwrap().sum, 20);
    assert_eq!(groups.null_row_count, 1);
    assert_eq!(groups.null_aggregate.sum, 40);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn i64_key_i64_aggregate_hashes_numeric_keys_without_payload_materialization() {
    let keys = [7i64, -2, 7, 11, -2];
    let values = [10i64, 20, 30, 40, 50];
    let key_bytes = keys
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let value_bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let key_nulls = [0b0000_1000u8];
    let value_nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(keys.len());
    base.clear(4);

    let (groups, stats) = aggregate_i64_le_bytes_by_i64_le_bytes(
        &key_bytes,
        &value_bytes,
        keys.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &key_nulls,
            row_count: keys.len(),
        },
        ValidityRef::CoveNullBitmap {
            bytes: &value_nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.row_counts.get(&7), Some(&2));
    assert_eq!(groups.aggregates.get(&7).unwrap().count, 1);
    assert_eq!(groups.aggregates.get(&7).unwrap().null_count, 1);
    assert_eq!(groups.aggregates.get(&7).unwrap().sum, 10);
    assert_eq!(groups.row_counts.get(&-2), Some(&1));
    assert_eq!(groups.aggregates.get(&-2).unwrap().sum, 20);
    assert_eq!(groups.null_row_count, 1);
    assert_eq!(groups.null_aggregate.sum, 40);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn hash_u32_group_count_handles_sparse_codes() {
    let values = [100u32, 7, 100, 42, 7, 9];
    let nulls = [0b0010_0000u8];
    let (groups, stats) = group_count_u32_hash(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        None,
    )
    .unwrap();

    assert_eq!(groups.counts.get(&100), Some(&2));
    assert_eq!(groups.counts.get(&7), Some(&2));
    assert_eq!(groups.counts.get(&42), Some(&1));
    assert_eq!(groups.counts.get(&9), None);
    assert_eq!(groups.null_count, 1);
    assert_eq!(groups.rows_grouped, 5);
    assert_eq!(stats.rows_matched, 5);
}

#[test]
fn u32_le_group_count_handles_file_codes_without_materialization() {
    let values = [100u32, 7, 100, 42, 7, 9];
    let bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let nulls = [0b0010_0000u8];
    let mut base = SelectionBitmap::all(values.len());
    base.clear(3);

    let (groups, stats) = group_count_u32_le_bytes(
        &bytes,
        values.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.counts.get(&100), Some(&2));
    assert_eq!(groups.counts.get(&7), Some(&2));
    assert_eq!(groups.counts.get(&42), None);
    assert_eq!(groups.counts.get(&9), None);
    assert_eq!(groups.null_count, 1);
    assert_eq!(groups.rows_grouped, 4);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn i64_le_group_count_handles_sparse_values_nulls_and_base_selection() {
    let values = [10i64, -2, 10, 7, -2, 99];
    let bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let nulls = [0b0010_0000u8];
    let mut base = SelectionBitmap::all(values.len());
    base.clear(3);

    let (groups, stats) = group_count_i64_le_bytes(
        &bytes,
        values.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(groups.counts.get(&10), Some(&2));
    assert_eq!(groups.counts.get(&-2), Some(&2));
    assert_eq!(groups.counts.get(&7), None);
    assert_eq!(groups.counts.get(&99), None);
    assert_eq!(groups.null_count, 1);
    assert_eq!(groups.rows_grouped, 4);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn distinct_u32_preserves_first_seen_order_and_validity() {
    let values = [3u32, 1, 3, 2, 1, 4];
    let nulls = [0b0000_1000u8];
    let mut base = SelectionBitmap::all(values.len());
    base.clear(5);

    let (distinct, stats) = distinct_u32(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(distinct, vec![3, 1]);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn i64_aggregates_preserve_wide_sum_and_nulls() {
    let values = [i64::MAX, -2, 5, 7];
    let nulls = [0b0000_0100u8];
    let (aggregates, stats) = aggregate_i64(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        None,
    );

    assert_eq!(aggregates.count, 3);
    assert_eq!(aggregates.null_count, 1);
    assert_eq!(aggregates.sum, i128::from(i64::MAX) - 2 + 7);
    assert_eq!(aggregates.min, Some(-2));
    assert_eq!(aggregates.max, Some(i64::MAX));
    assert_eq!(stats.rows_valid, 3);
}

#[test]
fn i64_le_byte_aggregates_match_typed_path_with_base_selection() {
    let values = [i64::MAX, -2, 5, 7];
    let bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect::<Vec<_>>();
    let nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(values.len());
    base.clear(3);

    let (aggregates, stats) = aggregate_i64_le_bytes(
        &bytes,
        values.len(),
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        Some(&base),
    )
    .unwrap();

    assert_eq!(aggregates.count, 2);
    assert_eq!(aggregates.null_count, 1);
    assert_eq!(aggregates.sum, i128::from(i64::MAX) - 2);
    assert_eq!(aggregates.min, Some(-2));
    assert_eq!(aggregates.max, Some(i64::MAX));
    assert_eq!(stats.rows_seen, values.len());
    assert_eq!(stats.rows_matched, 2);
}

#[test]
fn i64_sort_orders_row_ids_not_payloads() {
    let values = [5, 1, 5, 0];
    let nulls = [0b0000_0100u8];
    let (rows, stats) = sort_rows_i64_with_stats(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        NativeSortDirection::Ascending,
        NativeNullOrder::Last,
        None,
    )
    .unwrap();

    assert_eq!(rows.rows(), &[3, 1, 0, 2]);
    assert_eq!(stats.rows_seen, 4);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);
}

#[test]
fn i64_top_n_matches_full_row_id_sort_prefix() {
    let values = [5, -1, 5, 0, 9, -1, 4];
    let nulls = [0b0100_0000u8];
    let (full, _) = sort_rows_i64_with_stats(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        NativeSortDirection::Descending,
        NativeNullOrder::Last,
        None,
    )
    .unwrap();
    let (top, stats) = top_n_rows_i64_with_stats(
        &values,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: values.len(),
        },
        NativeSortDirection::Descending,
        NativeNullOrder::Last,
        None,
        3,
    )
    .unwrap();

    assert_eq!(top.rows(), &full.rows()[..3]);
    assert_eq!(top.rows(), &[4, 2, 0]);
    assert_eq!(stats.rows_seen, values.len());
    assert_eq!(stats.rows_valid, 6);
    assert_eq!(stats.rows_matched, values.len());
}

#[test]
fn u32_semi_join_requires_compatible_code_domain() {
    let left = [1u32, 2, 3, 4];
    let right = [2u32, 4, 8];
    let mut left_domain = NativeCodeDomain {
        semantic_domain_id: Some("customer-status".into()),
        dictionary_epoch: Some(7),
        ..NativeCodeDomain::default()
    };
    let right_domain = left_domain.clone();
    let (selected, stats) = semi_join_u32_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &left_domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &right_domain,
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[1, 3]);
    assert_eq!(stats.rows_matched, 2);

    left_domain.dictionary_epoch = Some(8);
    assert!(semi_join_u32_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &left_domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &right_domain,
        None,
    )
    .is_err());
}

#[test]
fn u32_anti_join_uses_same_domain_and_skips_nulls() {
    let left = [1u32, 2, 3, 4, 5];
    let right = [2u32, 4, 8];
    let nulls = [0b0000_0100u8];
    let domain = NativeCodeDomain {
        semantic_domain_id: Some("customer-status".into()),
        dictionary_epoch: Some(7),
        ..NativeCodeDomain::default()
    };
    let (selected, stats) = anti_join_u32_eq(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0, 4]);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 2);

    let other_domain = NativeCodeDomain {
        dictionary_epoch: Some(8),
        ..domain.clone()
    };
    assert!(anti_join_u32_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &other_domain,
        None,
    )
    .is_err());
}

#[test]
fn u32_inner_join_emits_row_pairs_and_rejects_domain_mismatch() {
    let left = [1u32, 2, 2, 3, 4];
    let right = [2u32, 2, 3, 8];
    let left_nulls = [0b0000_1000u8];
    let mut base = SelectionBitmap::all(left.len());
    base.clear(4);
    let domain = NativeCodeDomain {
        semantic_domain_id: Some("customer-status".into()),
        dictionary_epoch: Some(7),
        ..NativeCodeDomain::default()
    };

    let (pairs, stats) = inner_join_u32_eq(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        Some(&base),
    )
    .unwrap();

    assert_eq!(pairs.left_rows, vec![1, 1, 2, 2]);
    assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 4);

    let other_domain = NativeCodeDomain {
        dictionary_epoch: Some(8),
        ..domain.clone()
    };
    assert!(inner_join_u32_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &other_domain,
        None,
    )
    .is_err());
}

#[test]
fn i64_join_kernels_handle_negative_duplicates_and_left_null_policy() {
    let left = [-7i64, 2, -7, 9, 0];
    let right = [-7i64, -7, 4];
    let left_nulls = [0b0001_0000u8];
    let domain = NativeCodeDomain {
        semantic_domain_id: Some("cove.datafusion.native.i64".into()),
        null_policy: Some("validity-bitmap-nulls-never-match".into()),
        ..NativeCodeDomain::default()
    };

    let (pairs, inner_stats) = inner_join_i64_eq(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        None,
    )
    .unwrap();

    assert_eq!(pairs.left_rows, vec![0, 0, 2, 2]);
    assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
    assert_eq!(inner_stats.rows_valid, 4);
    assert_eq!(inner_stats.rows_matched, 4);

    let (semi, semi_stats) = semi_join_i64_eq(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        None,
    )
    .unwrap();
    assert_eq!(semi.to_selection_vector().rows(), &[0, 2]);
    assert_eq!(semi_stats.rows_matched, 2);

    let (anti, anti_stats) = anti_join_i64_eq_left_nulls_unmatched(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        None,
    )
    .unwrap();
    assert_eq!(anti.to_selection_vector().rows(), &[1, 3, 4]);
    assert_eq!(anti_stats.rows_matched, 3);
}

#[test]
fn u64_semi_join_uses_shared_domain_and_base_selection() {
    let left = [10u64, 20, 30, 40, 50];
    let right = [20u64, 50, 90];
    let mut base = SelectionBitmap::all(left.len());
    base.clear(4);
    let domain = NativeCodeDomain {
        dictionary_id: Some("file-dict".into()),
        dictionary_epoch: Some(3),
        collation_id: Some(1),
        ..NativeCodeDomain::default()
    };

    let (selected, stats) = semi_join_u64_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &domain,
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[1]);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 1);
    assert_eq!(
        stats.bytes_touched_estimate,
        (left.len() + right.len()) * std::mem::size_of::<u64>()
    );
}

#[test]
fn u64_anti_join_skips_nulls_and_rejects_epoch_mismatch() {
    let left = [10u64, 20, 30, 40, 50];
    let right = [20u64, 50, 90];
    let left_nulls = [0b0000_0100u8];
    let right_nulls = [0b0000_0010u8];
    let domain = NativeCodeDomain {
        dictionary_id: Some("file-dict".into()),
        dictionary_epoch: Some(3),
        collation_id: Some(1),
        ..NativeCodeDomain::default()
    };

    let (selected, stats) = anti_join_u64_eq(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::CoveNullBitmap {
            bytes: &right_nulls,
            row_count: right.len(),
        },
        &domain,
        None,
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[0, 3, 4]);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 3);

    let other_domain = NativeCodeDomain {
        dictionary_epoch: Some(4),
        ..domain.clone()
    };
    assert!(anti_join_u64_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::AllValid {
            row_count: right.len(),
        },
        &other_domain,
        None,
    )
    .is_err());
}

#[test]
fn u64_anti_join_can_treat_left_nulls_as_unmatched() {
    let left = [10u64, 20, 30, 40, 50];
    let right = [20u64, 50, 90];
    let left_nulls = [0b0000_0100u8];
    let right_nulls = [0b0000_0010u8];
    let mut base = SelectionBitmap::all(left.len());
    base.clear(0);
    let domain = NativeCodeDomain {
        dictionary_id: Some("file-dict".into()),
        dictionary_epoch: Some(3),
        collation_id: Some(1),
        ..NativeCodeDomain::default()
    };

    let (selected, stats) = anti_join_u64_eq_left_nulls_unmatched(
        &left,
        ValidityRef::CoveNullBitmap {
            bytes: &left_nulls,
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::CoveNullBitmap {
            bytes: &right_nulls,
            row_count: right.len(),
        },
        &domain,
        Some(&base),
    )
    .unwrap();

    assert_eq!(selected.to_selection_vector().rows(), &[2, 3, 4]);
    assert_eq!(stats.rows_valid, 3);
    assert_eq!(stats.rows_matched, 3);
}

#[test]
fn u64_inner_join_preserves_left_order_and_skips_right_nulls() {
    let left = [10u64, 20, 20, 30, 40];
    let right = [20u64, 20, 30, 40];
    let right_nulls = [0b0000_0100u8];
    let mut base = SelectionBitmap::all(left.len());
    base.clear(4);
    let domain = NativeCodeDomain {
        dictionary_id: Some("file-dict".into()),
        dictionary_epoch: Some(3),
        collation_id: Some(1),
        ..NativeCodeDomain::default()
    };

    let (pairs, stats) = inner_join_u64_eq(
        &left,
        ValidityRef::AllValid {
            row_count: left.len(),
        },
        &domain,
        &right,
        ValidityRef::CoveNullBitmap {
            bytes: &right_nulls,
            row_count: right.len(),
        },
        &domain,
        Some(&base),
    )
    .unwrap();

    assert_eq!(pairs.left_rows, vec![1, 1, 2, 2]);
    assert_eq!(pairs.right_rows, vec![0, 1, 0, 1]);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 4);
    assert!(stats.bytes_touched_estimate >= (left.len() + right.len()) * 8);
}

#[test]
fn mutable_bitmap_operations_reuse_dense_words() {
    let mut selected = SelectionBitmap::default();
    selected.fill_all(70);
    selected.clear_bit(1);
    selected.clear_bit(64);

    let mut filter = SelectionBitmap::default();
    filter.fill_none(70);
    filter.set(0);
    filter.set(65);
    filter.set(69);

    selected.and_inplace(&filter);

    let mut rows = Vec::new();
    selected.write_selected_rows(&mut rows).unwrap();
    assert_eq!(rows, vec![0, 65, 69]);
    assert!(!selected.all_zero());
    assert_eq!(selected.words().len(), 2);
}

#[test]
fn bitmap_intersection_dispatch_matches_scalar() {
    let mut scalar = SelectionBitmap::all(4097);
    let mut auto = scalar.clone();
    let mut filter = SelectionBitmap::none(4097);
    for row in (0..4097).step_by(3) {
        filter.set(row);
    }
    for row in (0..4097).step_by(11) {
        scalar.clear(row);
        auto.clear(row);
    }

    scalar.intersect_with(&filter);
    let dispatch = auto.intersect_with_dispatch(&filter, NativeKernelDispatch::Auto);

    assert_eq!(auto, scalar);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(dispatch, NativeKernelDispatch::Neon);
    #[cfg(not(target_arch = "aarch64"))]
    assert!(matches!(
        dispatch,
        NativeKernelDispatch::Scalar | NativeKernelDispatch::Avx2
    ));
}

#[test]
fn bitmap_intersection_scalar_policy_reports_scalar() {
    let mut left = SelectionBitmap::all(130);
    let mut right = SelectionBitmap::none(130);
    right.set(3);
    right.set(64);
    right.set(129);

    let dispatch = left.intersect_with_dispatch(&right, NativeKernelDispatch::Scalar);

    assert_eq!(dispatch, NativeKernelDispatch::Scalar);
    assert_eq!(left.to_selection_vector().rows(), &[3, 64, 129]);
}

#[test]
fn selection_bitmap_compaction_reports_stats_and_preserves_order() {
    let mut bitmap = SelectionBitmap::none(130);
    bitmap.set(0);
    bitmap.set(63);
    bitmap.set(64);
    bitmap.set(129);

    let (vector, stats) = compact_selection_bitmap(&bitmap, NativeKernelDispatch::Auto).unwrap();

    assert_eq!(vector.rows(), &[0, 63, 64, 129]);
    assert_eq!(stats.rows_seen, 130);
    assert_eq!(stats.rows_valid, 4);
    assert_eq!(stats.rows_matched, 4);
    assert_eq!(stats.bitmap_words_touched, 3);
    assert_eq!(stats.bytes_touched_estimate, 3 * std::mem::size_of::<u64>());
    assert_eq!(stats.dispatch, NativeKernelDispatch::Scalar);
}

#[test]
fn retain_set_bits_visits_only_selected_rows() {
    let mut bitmap = SelectionBitmap::none(130);
    bitmap.set(3);
    bitmap.set(64);
    bitmap.set(129);
    let mut visited = Vec::new();

    bitmap.retain_set_bits(|row| {
        visited.push(row);
        row != 64
    });

    assert_eq!(visited, vec![3, 64, 129]);
    assert_eq!(bitmap.to_selection_vector().rows(), &[3, 129]);
}

#[test]
fn scan_program_summary_reports_exactness_and_ordering() {
    let program = NativeScanProgram {
        ops: vec![NativeScanOp::Numeric {
            column_index: 0,
            column_id: 7,
            exactness: NativePredicateExactness::FullRowPredicateExact,
            kernel: NativeDecodeKernel::PreparedNumCode,
        }],
        exact_filters: 1,
        inexact_filters: 0,
        lookup_rowref_eligible: true,
        predicate_ordered: true,
    };

    assert_eq!(
            program.display_summary(),
            "ops=1, exact_filters=1, inexact_filters=0, lookup_rowref_eligible=true, predicate_ordered=true"
        );
}

#[test]
fn retained_object_temporal_batch_exposes_filecode_page_lane() {
    let codes = [11u32, 22, 33];
    let mut values = Vec::new();
    for code in codes {
        values.extend_from_slice(&code.to_le_bytes());
    }
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::FileCode,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        Some(vec![0b0000_0010]),
        values,
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let segment = retained_segment_with_one_property_page(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        payload,
        1,
    );

    let batch = native_object_temporal_batch_from_retained_segment(
        &segment,
        NativeCodeDomain {
            file_id: Some("file-a".into()),
            ..NativeCodeDomain::default()
        },
    )
    .unwrap();

    assert_eq!(batch.segment_id, 9);
    assert_eq!(batch.object_type_id, 7);
    assert_eq!(batch.row_count, 3);
    assert_eq!(batch.property_pages.len(), 1);
    let page = &batch.property_pages[0];
    assert_eq!(page.property_id, 42);
    assert_eq!(page.row_start, 0);
    match &page.lane {
        LaneRef::FileCodeU32LeBytes {
            bytes,
            row_count,
            validity,
            domain,
            ..
        } => {
            assert_eq!(*row_count, 3);
            assert_eq!(validity.valid_count(), 2);
            assert_eq!(domain.file_id.as_deref(), Some("file-a"));
            assert_eq!(domain.object_type_id, Some(7));
            assert_eq!(domain.property_id, Some(42));

            let (selected, stats) =
                filter_u32_le_in_sorted(bytes, *row_count, *validity, &[22, 33], None).unwrap();
            assert_eq!(selected.to_selection_vector().rows(), &[2]);
            assert_eq!(stats.rows_matched, 1);
        }
        other => panic!("expected FileCodeU32LeBytes lane, got {other:?}"),
    }
}

#[test]
fn owned_object_temporal_batch_exposes_numcode_page_lane() {
    let values = [10u64, 20, 30];
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::NumCode,
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        None,
        bytes,
    )
    .unwrap();
    let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
    let segment = temporal_segment_with_one_property_page(
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        payload,
        0,
    );

    let batch = native_object_temporal_batch_from_segment(
        &segment,
        NativeCodeDomain {
            semantic_domain_id: Some("domain-a".into()),
            ..NativeCodeDomain::default()
        },
    )
    .unwrap();

    match &batch.property_pages[0].lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            domain,
            ..
        } => {
            assert_eq!(*row_count, 3);
            assert_eq!(domain.semantic_domain_id.as_deref(), Some("domain-a"));
            let (selected, stats) =
                filter_u64_le_eq(bytes, *row_count, *validity, 20, None).unwrap();
            assert_eq!(selected.to_selection_vector().rows(), &[1]);
            assert_eq!(stats.rows_matched, 1);
        }
        other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
    }
}

#[test]
fn retained_object_temporal_batch_exposes_bool_page_lane() {
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        Some(vec![0b0000_0010]),
        vec![1, 0, 1],
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let segment = retained_segment_with_one_property_page(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        payload,
        1,
    );

    let batch =
        native_object_temporal_batch_from_retained_segment(&segment, NativeCodeDomain::default())
            .unwrap();

    match &batch.property_pages[0].lane {
        LaneRef::Bool {
            values,
            row_count,
            validity,
            ..
        } => {
            assert_eq!(*row_count, 3);
            let (selected, stats) =
                filter_bool_eq(values, *row_count, *validity, true, None).unwrap();
            assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
            assert_eq!(stats.rows_valid, 2);
        }
        other => panic!("expected Bool lane, got {other:?}"),
    }
}

#[test]
fn retained_object_temporal_batch_exposes_fixed_bytes_page_lane() {
    let mut values = Vec::new();
    values.extend_from_slice(&[1u8; 16]);
    values.extend_from_slice(&[2u8; 16]);
    values.extend_from_slice(&[1u8; 16]);
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        None,
        values,
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let segment = retained_segment_with_one_property_page(
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        payload,
        0,
    );

    let batch =
        native_object_temporal_batch_from_retained_segment(&segment, NativeCodeDomain::default())
            .unwrap();

    match &batch.property_pages[0].lane {
        LaneRef::FixedBytes {
            values,
            width,
            row_count,
            validity,
            ..
        } => {
            assert_eq!((*width, *row_count), (16, 3));
            let (selected, stats) =
                filter_fixed_bytes_eq(values, *row_count, *width, *validity, &[1u8; 16], None)
                    .unwrap();
            assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
            assert_eq!(stats.rows_matched, 2);
        }
        other => panic!("expected FixedBytes lane, got {other:?}"),
    }
}

#[test]
fn retained_object_temporal_batch_exposes_varbytes_page_lane() {
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::VarBytes,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        None,
        length_prefixed_rows([b"red".as_slice(), b"blue".as_slice(), b"rose".as_slice()]),
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let segment = retained_segment_with_one_property_page(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        payload,
        0,
    );

    let batch =
        native_object_temporal_batch_from_retained_segment(&segment, NativeCodeDomain::default())
            .unwrap();

    match &batch.property_pages[0].lane {
        LaneRef::VarBytes {
            row_offsets,
            values,
            validity,
            ..
        } => {
            assert_eq!(row_offsets.as_ref(), &[0, 7, 15]);
            let (selected, stats) =
                filter_varbytes_prefix(row_offsets, values, *validity, b"r", None).unwrap();
            assert_eq!(selected.to_selection_vector().rows(), &[0, 2]);
            assert_eq!(stats.rows_matched, 2);
        }
        other => panic!("expected VarBytes lane, got {other:?}"),
    }
}

#[test]
fn retained_object_temporal_batch_exposes_local_codebook_filecode_lane() {
    let local_codebook = LocalCodebookPayload {
        values: LocalCodebookValues::FileCode(vec![10, 20, 30]),
        indexes: LocalIndexPayload::BitPacked(BitPackedPayload::pack(&[0, 1, 2], 2).unwrap()),
    };
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::LocalCodebook,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        None,
        local_codebook.encode(),
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let segment = retained_segment_with_one_property_page(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        payload,
        0,
    );

    let batch =
        native_object_temporal_batch_from_retained_segment(&segment, NativeCodeDomain::default())
            .unwrap();

    match &batch.property_pages[0].lane {
        LaneRef::LocalCodeU8 {
            values,
            validity,
            local_to_global,
            ..
        } => {
            assert_eq!(values.as_ref(), &[0, 1, 2]);
            assert_eq!(local_to_global.as_ref(), &[10, 20, 30]);
            let membership = local_membership_u8(local_to_global, &[20]);
            let (selected, stats) =
                filter_local_u8_membership(values, *validity, &membership, None);
            assert_eq!(selected.to_selection_vector().rows(), &[1]);
            assert_eq!(stats.rows_matched, 1);
        }
        other => panic!("expected LocalCodeU8 lane, got {other:?}"),
    }
}

#[test]
fn table_batch_exposes_borrowed_numcode_page_lane() {
    let mut values = Vec::new();
    for value in [11u64, 22, 33] {
        values.extend_from_slice(&value.to_le_bytes());
    }
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::NumCode,
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        None,
        values,
    )
    .unwrap();
    let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
    let mut column = table_column_directory(CoveLogicalType::UInt64, CovePhysicalKind::NumCode);
    column.domain_ref = 17;
    let segment = table_segment_with_one_column(column);
    let page = table_page_index_entry(42, 5, 3, 0);
    let page_ref = NativeTablePageRef {
        column: &segment.columns[0],
        page: &page,
        payload: Some(&payload),
    };

    let batch =
        native_table_batch_from_page_refs(&segment, &[page_ref], NativeCodeDomain::default())
            .unwrap();

    assert_eq!(batch.table_id, 77);
    assert_eq!(batch.segment_id, 88);
    assert_eq!(batch.row_start, 100);
    assert_eq!(batch.row_count, 3);
    assert_eq!(batch.column_pages[0].row_start, 100);
    assert_eq!(batch.column_pages[0].row_start_in_segment, 0);
    match &batch.column_pages[0].lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            domain,
            ..
        } => {
            assert_eq!(*row_count, 3);
            assert_eq!(bytes.len(), 24);
            assert_eq!(validity.row_count(), 3);
            assert_eq!(domain.table_id, Some(77));
            assert_eq!(domain.column_id, Some(42));
            assert_eq!(
                domain.semantic_domain_id.as_deref(),
                Some("table-domain:17")
            );
        }
        other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
    }
}

#[test]
fn direct_column_page_helper_exposes_borrowed_numcode_lane() {
    let mut values = Vec::new();
    for value in [11u64, 22, 33] {
        values.extend_from_slice(&value.to_le_bytes());
    }
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::NumCode,
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        None,
        values,
    )
    .unwrap();
    let payload = ColumnPagePayloadV1::parse(&payload_bytes).unwrap();
    let mut column = table_column_directory(CoveLogicalType::UInt64, CovePhysicalKind::NumCode);
    column.domain_ref = 17;
    let segment = table_segment_with_one_column(column);
    let page = table_page_index_entry(42, 5, 3, 0);
    let domain = NativeCodeDomain {
        table_id: Some(segment.header.table_id),
        ..NativeCodeDomain::default()
    };

    let lane =
        native_lane_from_column_page_payload(&segment.columns[0], &page, &payload, domain).unwrap();

    match lane {
        LaneRef::NumCodeU64LeBytes {
            bytes,
            row_count,
            validity,
            domain,
            ..
        } => {
            assert_eq!(row_count, 3);
            assert_eq!(bytes.len(), 24);
            assert_eq!(validity.row_count(), 3);
            assert_eq!(domain.table_id, Some(77));
            assert_eq!(domain.column_id, Some(42));
            assert_eq!(
                domain.semantic_domain_id.as_deref(),
                Some("table-domain:17")
            );
        }
        other => panic!("expected NumCodeU64LeBytes lane, got {other:?}"),
    }
}

#[test]
fn table_batch_exposes_retained_local_codebook_filecode_lane() {
    let local_codebook = LocalCodebookPayload {
        values: LocalCodebookValues::FileCode(vec![7, 9, 11]),
        indexes: LocalIndexPayload::BitPacked(BitPackedPayload::pack(&[2, 0, 1], 2).unwrap()),
    };
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        3,
        CoveEncodingKind::LocalCodebook,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        None,
        local_codebook.encode(),
    )
    .unwrap();
    let payload =
        RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes)).unwrap();
    let column = table_column_directory(CoveLogicalType::Utf8, CovePhysicalKind::FileCode);
    let segment = table_segment_with_one_column(column);
    let page = table_page_index_entry(42, 5, 3, 0);
    let page_ref = NativeTablePageRef {
        column: &segment.columns[0],
        page: &page,
        payload: Some(&payload),
    };

    let batch =
        native_table_batch_from_page_refs(&segment, &[page_ref], NativeCodeDomain::default())
            .unwrap();

    match &batch.column_pages[0].lane {
        LaneRef::LocalCodeU8 {
            values,
            local_to_global,
            validity,
            ..
        } => {
            assert_eq!(values.as_ref(), &[2, 0, 1]);
            assert_eq!(local_to_global.as_ref(), &[7, 9, 11]);
            let membership = local_membership_u8(local_to_global, &[11]);
            let (selected, stats) =
                filter_local_u8_membership(values, *validity, &membership, None);
            assert_eq!(selected.to_selection_vector().rows(), &[0]);
            assert_eq!(stats.rows_matched, 1);
        }
        other => panic!("expected LocalCodeU8 lane, got {other:?}"),
    }
}

fn temporal_segment_with_one_property_page(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    payload: ColumnPagePayloadV1,
    null_count: u32,
) -> TemporalSegmentData {
    TemporalSegmentData {
        header: TemporalSegmentHeaderV1 {
            segment_id: 9,
            object_type_id: 7,
            time_range_start_us: 1,
            time_range_end_us: 3,
            csn_min: 1,
            csn_max: 3,
            row_count: 3,
            morsel_count: 1,
            morsel_row_count: 3,
            column_count: 1,
            row_directory_offset: 0,
            column_directory_offset: 0,
            page_index_offset: 0,
            data_offset: 0,
            flags: 0,
            checksum: 0,
        },
        rows: vec![temporal_row(1), temporal_row(2), temporal_row(3)],
        property_columns: vec![TemporalPropertyColumn {
            directory: table_column_directory(logical_type, physical_kind),
            page_index: crate::page::ColumnPageIndex {
                entries: Vec::new(),
            },
            pages: vec![TemporalPropertyPage {
                index_entry: ColumnPageIndexEntryV1 {
                    column_id: 42,
                    morsel_id: 0,
                    row_count: 3,
                    non_null_count: 3 - null_count,
                    null_count,
                    encoding_root: 0,
                    page_offset: 0,
                    page_length: 0,
                    uncompressed_length: 0,
                    stats_ref: 0,
                    flags: 0,
                    checksum: 0,
                },
                payload: Some(payload),
            }],
        }],
    }
}

fn retained_segment_with_one_property_page(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    payload: RetainedColumnPagePayloadV1,
    null_count: u32,
) -> RetainedTemporalSegmentData {
    RetainedTemporalSegmentData {
        header: TemporalSegmentHeaderV1 {
            segment_id: 9,
            object_type_id: 7,
            time_range_start_us: 1,
            time_range_end_us: 3,
            csn_min: 1,
            csn_max: 3,
            row_count: 3,
            morsel_count: 1,
            morsel_row_count: 3,
            column_count: 1,
            row_directory_offset: 0,
            column_directory_offset: 0,
            page_index_offset: 0,
            data_offset: 0,
            flags: 0,
            checksum: 0,
        },
        rows: vec![temporal_row(1), temporal_row(2), temporal_row(3)],
        property_columns: vec![RetainedTemporalPropertyColumn {
            directory: table_column_directory(logical_type, physical_kind),
            page_index: crate::page::ColumnPageIndex {
                entries: Vec::new(),
            },
            pages: vec![RetainedTemporalPropertyPage {
                index_entry: ColumnPageIndexEntryV1 {
                    column_id: 42,
                    morsel_id: 0,
                    row_count: 3,
                    non_null_count: 3 - null_count,
                    null_count,
                    encoding_root: 0,
                    page_offset: 0,
                    page_length: 0,
                    uncompressed_length: 0,
                    stats_ref: 0,
                    flags: 0,
                    checksum: 0,
                },
                payload: Some(payload),
            }],
        }],
    }
}

fn table_segment_with_one_column(column: TableColumnDirectoryEntryV1) -> TableSegmentPayloadV1 {
    TableSegmentPayloadV1 {
        header: TableSegmentHeaderV1 {
            table_id: 77,
            segment_id: 88,
            row_start: 100,
            row_count: 3,
            morsel_count: 1,
            morsel_row_count: 3,
            column_count: 1,
            morsel_directory_offset: 0,
            column_directory_offset: 0,
            page_index_offset: 0,
            data_offset: 0,
            flags: 0,
            checksum: 0,
        },
        morsels: RowMorselDirectory {
            entries: vec![RowMorselEntryV1 {
                morsel_id: 5,
                first_row_in_segment: 0,
                row_count: 3,
                flags: 0,
                stats_ref: 0,
                checksum: 0,
            }],
        },
        columns: vec![column],
    }
}

fn table_page_index_entry(
    column_id: u32,
    morsel_id: u32,
    row_count: u32,
    null_count: u32,
) -> ColumnPageIndexEntryV1 {
    ColumnPageIndexEntryV1 {
        column_id,
        morsel_id,
        row_count,
        non_null_count: row_count - null_count,
        null_count,
        encoding_root: 0,
        page_offset: 0,
        page_length: 0,
        uncompressed_length: 0,
        stats_ref: 0,
        flags: 0,
        checksum: 0,
    }
}

fn table_column_directory(
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> TableColumnDirectoryEntryV1 {
    let mut bytes = [0u8; crate::segment::TABLE_COLUMN_DIRECTORY_ENTRY_LEN];
    bytes[0..4].copy_from_slice(&42u32.to_le_bytes());
    bytes[4..6].copy_from_slice(&(logical_type as u16).to_le_bytes());
    bytes[6] = physical_kind as u8;
    let checksum_value = checksum::crc32c(&bytes);
    bytes[48..52].copy_from_slice(&checksum_value.to_le_bytes());
    TableColumnDirectoryEntryV1::parse(&bytes).unwrap()
}

fn temporal_row(value: u8) -> TemporalRowEntryV1 {
    TemporalRowEntryV1 {
        timestamp_us: i64::from(value),
        csn: u64::from(value),
        branch_key: 0,
        goid: [value; 16],
        record_id: [value; 16],
        record_kind: RecordKind::Delta,
        prev_ref: None,
    }
}

fn length_prefixed_rows<const N: usize>(rows: [&[u8]; N]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        out.extend_from_slice(&(row.len() as u32).to_le_bytes());
        out.extend_from_slice(row);
    }
    out
}
