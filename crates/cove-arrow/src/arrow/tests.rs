use super::*;
use arrow_array::Array;

use crate::{
    array::EncodedArray,
    constants::{CoveEncodingKind, CoveLogicalType, CovePhysicalKind, StorageClass, ValueTag},
    dictionary::{FileDictionary, FileDictionaryHeaderV1, FileDictionaryIndexEntryV1},
    encoding::local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
    encoding::nested::{
        ListLayout, ListLayoutPayload, MapLayout, MapLayoutPayload, StructLayout,
        StructLayoutPayload,
    },
    encoding::rle::RlePayload,
    validity::ValidityBitmapBuilder,
};

fn view_options() -> ArrowExportOptions {
    ArrowExportOptions {
        varbytes_policy: ArrowVarBytesExportPolicy::View,
        ..ArrowExportOptions::default()
    }
}

#[test]
fn round_trip_inversion_preserves_payload() {
    let cove = vec![0b0000_1010u8]; // rows 1 and 3 are null
    let arrow = cove_null_to_arrow_validity(&cove, 8).unwrap();
    // Arrow: bits 1 and 3 should be 0 (invalid), others 1 (valid).
    assert_eq!(arrow[0], !cove[0]);
    let back = arrow_validity_to_cove_null(&arrow, 8).unwrap();
    assert_eq!(back, cove);
}

#[test]
fn partial_byte_only_iterates_row_count() {
    let cove = vec![0b1111_0000u8];
    let arrow = cove_null_to_arrow_validity(&cove, 4).unwrap();
    // Only the lower 4 bits of byte 0 are touched; high bits stay 0.
    assert_eq!(arrow[0] & 0b0000_1111, 0b0000_1111);
    assert_eq!(arrow[0] & 0b1111_0000, 0);
}

#[test]
fn rejects_short_cove_null_bitmap() {
    assert!(matches!(
        cove_null_to_arrow_validity(&[], 1),
        Err(CoveError::BufferTooShort)
    ));
}

#[test]
fn rejects_short_arrow_validity_bitmap() {
    assert!(matches!(
        arrow_validity_to_cove_null(&[], 1),
        Err(CoveError::BufferTooShort)
    ));
}

#[test]
fn exports_plain_int32_array_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&10i32.to_le_bytes());
    values.extend_from_slice(&20i32.to_le_bytes());
    let mut validity = ValidityBitmapBuilder::new(2).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 2);
    let cove = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let ints = arrow.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(ints.len(), 2);
    assert_eq!(ints.value(0), 10);
    assert!(ints.is_null(1));
}

#[test]
fn exports_plain_float64_array_directly() {
    let mut values = Vec::new();
    values.extend_from_slice(&1.5f64.to_bits().to_le_bytes());
    values.extend_from_slice(&(-2.25f64).to_bits().to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Float64,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let floats = arrow.as_any().downcast_ref::<Float64Array>().unwrap();
    assert_eq!(floats.value(0), 1.5);
    assert_eq!(floats.value(1), -2.25);
}

#[test]
fn exports_constant_int64_without_row_value_fallback() {
    let payload = ConstantPayload {
        value: 42,
        row_count: 3,
    }
    .encode();
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::FixedBytes,
        3,
        CoveEncodingKind::Constant,
        None,
        &payload,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let ints = arrow.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.values(), &[42, 42, 42]);
}

#[test]
fn exports_constant_numcode_uint64_from_raw_bits() {
    let raw = i64::MAX as u64 + 1;
    let payload = ConstantPayload {
        value: i64::from_le_bytes(raw.to_le_bytes()),
        row_count: 2,
    }
    .encode();
    let cove = EncodedArray::new(
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::Constant,
        None,
        &payload,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let uints = arrow.as_any().downcast_ref::<UInt64Array>().unwrap();
    assert_eq!(uints.values(), &[raw, raw]);
}

#[test]
fn exports_constant_numcode_signed_from_raw_bits() {
    let raw = (-7i64) as u64;
    let payload = ConstantPayload {
        value: i64::from_le_bytes(raw.to_le_bytes()),
        row_count: 2,
    }
    .encode();
    let cove = EncodedArray::new(
        CoveLogicalType::TimestampNanos,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::Constant,
        None,
        &payload,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let timestamps = arrow
        .as_any()
        .downcast_ref::<TimestampNanosecondArray>()
        .unwrap();
    assert_eq!(timestamps.values(), &[-7, -7]);
}

#[test]
fn exports_rle_int64_without_row_value_fallback() {
    let payload = RlePayload {
        runs: vec![(7, 2), (9, 1)],
    }
    .encode();
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::FixedBytes,
        3,
        CoveEncodingKind::Rle,
        None,
        &payload,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let ints = arrow.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.values(), &[7, 7, 9]);
}

#[test]
fn exports_fixed_length_varbytes_fast_path_values() {
    let mut values = Vec::new();
    for value in [b"aa".as_slice(), b"bb".as_slice(), b"cc".as_slice()] {
        values.extend_from_slice(&(value.len() as u32).to_le_bytes());
        values.extend_from_slice(value);
    }
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let strings = arrow
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "aa");
    assert_eq!(strings.value(1), "bb");
    assert_eq!(strings.value(2), "cc");
}

#[test]
fn exports_numcode_int64_array() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&20u64.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::NumCode,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let ints = arrow.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.values(), &[10, 20]);
}

#[test]
fn retained_numcode_uint64_array_can_use_backing_owner() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&20u64.to_le_bytes());
    let owner = Arc::new(values);
    let cove = EncodedArray::new(
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::NumCode,
        None,
        owner.as_slice(),
        None,
    );
    let buffer_owner = arrow_buffer_owner(Arc::clone(&owner));

    let result = encoded_array_to_arrow_with_options_and_owner(
        &cove,
        ArrowExportOptions::default(),
        Some(&buffer_owner),
    )
    .unwrap();
    let uints = result.value.as_any().downcast_ref::<UInt64Array>().unwrap();
    assert_eq!(uints.values(), &[10, 20]);
    if (owner.as_ptr() as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        assert_eq!(uints.to_data().buffers()[0].as_ptr(), owner.as_ptr());
    }
}

#[test]
fn retained_numcode_int64_array_with_nulls_keeps_backing_owner() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&99u64.to_le_bytes());
    values.extend_from_slice(&30u64.to_le_bytes());
    let owner = Arc::new(values);
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        3,
        CoveEncodingKind::NumCode,
        Some(bitmap),
        owner.as_slice(),
        None,
    );
    let buffer_owner = arrow_buffer_owner(Arc::clone(&owner));

    let result = encoded_array_to_arrow_with_options_and_owner(
        &cove,
        ArrowExportOptions::default(),
        Some(&buffer_owner),
    )
    .unwrap();
    let ints = result.value.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.value(0), 10);
    assert!(ints.is_null(1));
    assert_eq!(ints.value(2), 30);
    if (owner.as_ptr() as usize).is_multiple_of(std::mem::align_of::<i64>()) {
        assert_eq!(ints.to_data().buffers()[0].as_ptr(), owner.as_ptr());
    }
}

#[test]
fn retained_plain_fixed_uuid_array_with_nulls_keeps_backing_owner() {
    let mut values = Vec::new();
    values.extend_from_slice(&[1u8; 16]);
    values.extend_from_slice(&[99u8; 16]);
    values.extend_from_slice(&[3u8; 16]);
    let owner = Arc::new(values);
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        3,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        owner.as_slice(),
        None,
    );
    let buffer_owner = arrow_buffer_owner(Arc::clone(&owner));

    let result = encoded_array_to_arrow_with_options_and_owner(
        &cove,
        ArrowExportOptions::default(),
        Some(&buffer_owner),
    )
    .unwrap();
    let uuids = result
        .value
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(uuids.value(0), &[1u8; 16]);
    assert!(uuids.is_null(1));
    assert_eq!(uuids.value(2), &[3u8; 16]);
    assert_eq!(uuids.to_data().buffers()[0].as_ptr(), owner.as_ptr());
}

#[test]
fn exports_numcode_int64_array_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&99u64.to_le_bytes());
    values.extend_from_slice(&30u64.to_le_bytes());
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        3,
        CoveEncodingKind::NumCode,
        Some(bitmap),
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let ints = arrow.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.value(0), 10);
    assert!(ints.is_null(1));
    assert_eq!(ints.value(2), 30);
}

#[test]
fn rejects_invalid_plain_bool_array() {
    let values = [1u8, 2u8];
    let cove = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn exports_plain_bool_array_with_nulls() {
    let values = [1u8, 0u8, 1u8];
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        3,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let bools = arrow.as_any().downcast_ref::<BooleanArray>().unwrap();
    assert!(bools.value(0));
    assert!(bools.is_null(1));
    assert!(bools.value(2));
}

#[test]
fn rejects_invalid_plain_bool_array_even_for_null_row() {
    let values = [1u8, 2u8];
    let mut validity = ValidityBitmapBuilder::new(2).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 2);
    let cove = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        2,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn bool_chunk_packer_matches_scalar_bits_and_rejects_invalid_bytes() {
    for pattern in 0u16..=255 {
        let mut chunk = [0u8; 8];
        let mut expected = 0u8;
        for (bit, slot) in chunk.iter_mut().enumerate() {
            let value = ((pattern >> bit) & 1) as u8;
            *slot = value;
            expected |= value << bit;
        }
        assert_eq!(pack_bool_chunk_8(&chunk).unwrap(), expected);
    }

    for pos in 0..8 {
        let mut chunk = [0u8; 8];
        chunk[pos] = 2;
        assert!(matches!(
            pack_bool_chunk_8(&chunk),
            Err(CoveError::PageCorrupt)
        ));
    }
}

#[test]
fn exports_utf8_varbytes_array() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let strings = arrow
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "hi");
    assert_eq!(strings.value(1), "there");
}

#[test]
fn trusted_utf8_validation_policy_preserves_valid_standard_output() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    let cove = EncodedArray::new(
        CoveLogicalType::Json,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let strict = encoded_array_to_arrow_with_options(&cove, ArrowExportOptions::default())
        .unwrap()
        .value;
    let trusted = encoded_array_to_arrow_with_options(
        &cove,
        ArrowExportOptions {
            string_validation_policy: ArrowStringValidationPolicy::TrustedPageProof,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap()
    .value;
    assert_eq!(trusted.data_type(), &DataType::Utf8);
    let strict = strict
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    let trusted = trusted
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(trusted.value(0), strict.value(0));
    assert_eq!(trusted.value(1), strict.value(1));
}

#[test]
fn exports_binary_varbytes_array_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(&[0, 1]);
    values.extend_from_slice(&3u32.to_le_bytes());
    values.extend_from_slice(&[2, 3, 4]);
    values.extend_from_slice(&1u32.to_le_bytes());
    values.extend_from_slice(&[5]);
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Binary,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        Some(bitmap),
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let binary = arrow.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(binary.value(0), &[0, 1]);
    assert!(binary.is_null(1));
    assert_eq!(binary.value(2), &[5]);
}

#[test]
fn view_export_maps_varbytes_to_arrow_view_arrays() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&19u32.to_le_bytes());
    values.extend_from_slice(b"long-string-payload");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let standard = encoded_array_to_arrow_with_options(&cove, ArrowExportOptions::default())
        .unwrap()
        .value;
    let view = encoded_array_to_arrow_with_options(&cove, view_options())
        .unwrap()
        .value;
    assert_eq!(view.data_type(), &DataType::Utf8View);
    let standard = standard
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    let view = view.as_any().downcast_ref::<StringViewArray>().unwrap();
    assert_eq!(view.value(0), standard.value(0));
    assert_eq!(view.value(1), standard.value(1));
}

#[test]
fn view_export_handles_binary_and_selected_rows() {
    let mut values = Vec::new();
    values.extend_from_slice(&1u32.to_le_bytes());
    values.extend_from_slice(&[0xaa]);
    values.extend_from_slice(&14u32.to_le_bytes());
    values.extend_from_slice(b"binary-payload");
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(&[0xbb, 0xcc]);
    let cove = EncodedArray::new(
        CoveLogicalType::Binary,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Rows(&[2, 1, 2]),
        view_options(),
    )
    .unwrap();
    assert_eq!(result.value.data_type(), &DataType::BinaryView);
    let binary = result
        .value
        .as_any()
        .downcast_ref::<BinaryViewArray>()
        .unwrap();
    assert_eq!(binary.value(0), &[0xbb, 0xcc]);
    assert_eq!(binary.value(1), b"binary-payload");
    assert_eq!(binary.value(2), &[0xbb, 0xcc]);
}

#[test]
fn selected_utf8_view_without_owner_uses_compact_owned_buffer() {
    let selected_a = "selected-visible-payload-alpha";
    let hidden = "hidden-unselected-payload-beta";
    let selected_b = "selected-visible-payload-gamma";
    let mut values = Vec::new();
    for value in [selected_a, hidden, selected_b] {
        values.extend_from_slice(&(value.len() as u32).to_le_bytes());
        values.extend_from_slice(value.as_bytes());
    }
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Rows(&[0, 2]),
        view_options(),
    )
    .unwrap();
    let view = result
        .value
        .as_any()
        .downcast_ref::<StringViewArray>()
        .unwrap();
    assert_eq!(view.value(0), selected_a);
    assert_eq!(view.value(1), selected_b);
    let backing = view_data_buffers(result.value.as_ref());
    assert!(contains_subslice(&backing, selected_a.as_bytes()));
    assert!(contains_subslice(&backing, selected_b.as_bytes()));
    assert!(!contains_subslice(&backing, hidden.as_bytes()));
}

#[test]
fn selected_binary_view_without_owner_uses_compact_owned_buffer() {
    let selected_a = b"selected-binary-payload-alpha".as_slice();
    let hidden = b"hidden-binary-payload-beta".as_slice();
    let selected_b = b"selected-binary-payload-gamma".as_slice();
    let mut values = Vec::new();
    for value in [selected_a, hidden, selected_b] {
        values.extend_from_slice(&(value.len() as u32).to_le_bytes());
        values.extend_from_slice(value);
    }
    let cove = EncodedArray::new(
        CoveLogicalType::Binary,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Rows(&[0, 2]),
        view_options(),
    )
    .unwrap();
    let view = result
        .value
        .as_any()
        .downcast_ref::<BinaryViewArray>()
        .unwrap();
    assert_eq!(view.value(0), selected_a);
    assert_eq!(view.value(1), selected_b);
    let backing = view_data_buffers(result.value.as_ref());
    assert!(contains_subslice(&backing, selected_a));
    assert!(contains_subslice(&backing, selected_b));
    assert!(!contains_subslice(&backing, hidden));
}

#[test]
fn all_row_view_without_owner_returns_owned_view_array() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    let long_value = b"long-all-row-view-payload";
    values.extend_from_slice(&(long_value.len() as u32).to_le_bytes());
    values.extend_from_slice(long_value);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_options(&cove, view_options())
        .unwrap()
        .value;
    let view = result.as_any().downcast_ref::<StringViewArray>().unwrap();
    assert_eq!(view.value(0), "hi");
    assert_eq!(view.value(1), "long-all-row-view-payload");
    assert!(contains_subslice(
        &view_data_buffers(result.as_ref()),
        b"long-all-row-view-payload"
    ));
}

#[test]
fn view_export_schema_policy_is_opt_in() {
    assert_eq!(
        arrow_data_type_for_export_options(CoveLogicalType::Utf8, ArrowExportOptions::default())
            .unwrap()
            .value,
        DataType::Utf8
    );
    assert_eq!(
        arrow_data_type_for_export_options(CoveLogicalType::Utf8, view_options())
            .unwrap()
            .value,
        DataType::Utf8View
    );
    assert_eq!(
        arrow_data_type_for_export_options(CoveLogicalType::Json, view_options())
            .unwrap()
            .value,
        DataType::Utf8View
    );
    assert_eq!(
        arrow_data_type_for_export_options(CoveLogicalType::Binary, view_options())
            .unwrap()
            .value,
        DataType::BinaryView
    );
}

fn view_data_buffers(array: &dyn Array) -> Vec<u8> {
    array
        .to_data()
        .buffers()
        .iter()
        .skip(1)
        .flat_map(|buffer| buffer.as_slice().iter().copied())
        .collect()
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn exports_utf8_varbytes_array_with_invalid_null_payload() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xff);
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        Some(bitmap),
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let strings = arrow
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "hi");
    assert!(strings.is_null(1));
    assert_eq!(strings.value(2), "there");

    let view = encoded_array_to_arrow_with_options(&cove, view_options())
        .unwrap()
        .value;
    let strings = view.as_any().downcast_ref::<StringViewArray>().unwrap();
    assert_eq!(strings.value(0), "hi");
    assert!(strings.is_null(1));
    assert_eq!(strings.value(2), "there");
}

#[test]
fn rejects_invalid_utf8_varbytes_non_null_payload() {
    let mut values = Vec::new();
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xff);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(encoded_array_to_arrow(&cove).is_err());
    assert!(encoded_array_to_arrow_with_options(&cove, view_options()).is_err());
}

#[test]
fn strict_utf8_rejects_invalid_row_boundaries_even_if_concat_valid() {
    let mut values = Vec::new();
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xc2);
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xa2);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(encoded_array_to_arrow(&cove).is_err());
}

#[test]
fn strict_utf8_accepts_valid_multibyte_rows() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(&[0xc2, 0xa2]);
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(&[0xc3, 0xa9]);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let strings = arrow
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "¢");
    assert_eq!(strings.value(1), "é");
}

#[test]
fn selected_utf8_rejects_invalid_row_boundaries_even_if_concat_valid() {
    let mut values = Vec::new();
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xc2);
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xa2);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Rows(&[0, 1]),
        ArrowExportOptions::default(),
    )
    .is_err());
}

#[test]
fn rejects_truncated_utf8_varbytes_payload() {
    let mut values = Vec::new();
    values.extend_from_slice(&4u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::OffsetRange)
    ));
}

#[test]
fn rejects_trailing_utf8_varbytes_payload() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.push(0);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn small_varbytes_copy_matches_slice_copy_for_short_and_large_values() {
    for len in 0usize..=128 {
        let src = (0..len).map(|i| i as u8).collect::<Vec<_>>();
        let mut dst = vec![0xaa; len];
        // SAFETY: source and destination are separate allocations and both
        // are valid for `len` bytes.
        unsafe {
            copy_varbytes_value(src.as_ptr(), dst.as_mut_ptr(), len);
        }
        assert_eq!(dst, src);
    }
}

#[test]
fn fused_varbytes_copy_reports_ascii_high_bits() {
    for len in 0usize..=128 {
        let src = vec![b'a'; len];
        let mut dst = vec![0xaa; len];
        // SAFETY: source and destination are separate allocations and both
        // are valid for `len` bytes.
        let mask = unsafe { copy_varbytes_value_ascii_mask(src.as_ptr(), dst.as_mut_ptr(), len) };
        assert_eq!(dst, src);
        assert_eq!(mask, 0, "len={len}");
    }

    for len in 1usize..=128 {
        let mut src = vec![b'a'; len];
        src[len - 1] = 0xc2;
        let mut dst = vec![0xaa; len];
        // SAFETY: source and destination are separate allocations and both
        // are valid for `len` bytes.
        let mask = unsafe { copy_varbytes_value_ascii_mask(src.as_ptr(), dst.as_mut_ptr(), len) };
        assert_eq!(dst, src);
        assert_ne!(mask, 0, "len={len}");
    }
}

#[test]
fn selected_export_filters_primitive_rows_and_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&10i32.to_le_bytes());
    values.extend_from_slice(&20i32.to_le_bytes());
    values.extend_from_slice(&30i32.to_le_bytes());
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        3,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    let result =
        encoded_array_to_arrow_selected_with_options(&cove, &[2, 1], ArrowExportOptions::default())
            .unwrap();
    let ints = result.value.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(ints.len(), 2);
    assert_eq!(ints.value(0), 30);
    assert!(ints.is_null(1));
}

#[test]
fn selected_record_batch_exports_varbytes_rows() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    values.extend_from_slice(&3u32.to_le_bytes());
    values.extend_from_slice(b"bye");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_columns_to_record_batch_selected_with_options(
        &[("word", &cove)],
        &[1],
        ArrowExportOptions::default(),
    )
    .unwrap();
    let strings = result
        .value
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "there");
}

#[test]
fn selected_export_reads_numcode_rows_directly_in_requested_order() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&20u64.to_le_bytes());
    values.extend_from_slice(&30u64.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        3,
        CoveEncodingKind::NumCode,
        None,
        &values,
        None,
    );

    let result =
        encoded_array_to_arrow_selected_with_options(&cove, &[2, 0], ArrowExportOptions::default())
            .unwrap();
    let ints = result.value.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.values(), &[30, 10]);
}

#[test]
fn selected_export_reads_numcode_rows_directly_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&20u64.to_le_bytes());
    values.extend_from_slice(&30u64.to_le_bytes());
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        3,
        CoveEncodingKind::NumCode,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_selected_with_options(
        &cove,
        &[2, 1, 0],
        ArrowExportOptions::default(),
    )
    .unwrap();
    let ints = result.value.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.value(0), 30);
    assert!(ints.is_null(1));
    assert_eq!(ints.value(2), 10);
}

#[test]
fn bitset_export_reads_numcode_rows_directly_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&20u64.to_le_bytes());
    values.extend_from_slice(&30u64.to_le_bytes());
    values.extend_from_slice(&40u64.to_le_bytes());
    let mut validity = ValidityBitmapBuilder::new(4).unwrap();
    validity.set_null(2).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 4);
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        4,
        CoveEncodingKind::NumCode,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Bitset {
            words: &[0b1101],
            len: 4,
        },
        ArrowExportOptions::default(),
    )
    .unwrap();
    let ints = result.value.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.value(0), 10);
    assert!(ints.is_null(1));
    assert_eq!(ints.value(2), 40);
}

#[test]
fn rejects_numcode_int64_overflow_all_rows() {
    let mut values = Vec::new();
    values.extend_from_slice(&10u64.to_le_bytes());
    values.extend_from_slice(&(i64::MAX as u64 + 1).to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Int64,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::NumCode,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn selected_export_reads_varbytes_rows_directly_with_nulls() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xff);
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_selected_with_options(
        &cove,
        &[2, 1, 0],
        ArrowExportOptions::default(),
    )
    .unwrap();
    let strings = result
        .value
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "there");
    assert!(strings.is_null(1));
    assert_eq!(strings.value(2), "hi");
}

#[test]
fn bitset_export_reads_varbytes_rows_directly() {
    let mut values = Vec::new();
    values.extend_from_slice(&1u32.to_le_bytes());
    values.extend_from_slice(b"a");
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"bb");
    values.extend_from_slice(&3u32.to_le_bytes());
    values.extend_from_slice(b"ccc");
    values.extend_from_slice(&4u32.to_le_bytes());
    values.extend_from_slice(b"dddd");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        4,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Bitset {
            words: &[0b1010],
            len: 4,
        },
        ArrowExportOptions::default(),
    )
    .unwrap();
    let strings = result
        .value
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.len(), 2);
    assert_eq!(strings.value(0), "bb");
    assert_eq!(strings.value(1), "dddd");
}

#[test]
fn bitset_export_ignores_invalid_utf8_for_null_rows() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&1u32.to_le_bytes());
    values.push(0xff);
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::VarBytes,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Bitset {
            words: &[0b011],
            len: 3,
        },
        ArrowExportOptions::default(),
    )
    .unwrap();
    let strings = result
        .value
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "hi");
    assert!(strings.is_null(1));
}

#[test]
fn bitset_export_rejects_wrong_page_length() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow_with_row_selection_options(
            &cove,
            ArrowRowSelection::Bitset {
                words: &[0b1],
                len: 2,
            },
            ArrowExportOptions::default()
        ),
        Err(CoveError::OffsetRange)
    ));
}

#[test]
fn selected_export_reads_binary_varbytes_rows_directly() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(&[0, 1]);
    values.extend_from_slice(&3u32.to_le_bytes());
    values.extend_from_slice(&[2, 3, 4]);
    let cove = EncodedArray::new(
        CoveLogicalType::Binary,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result =
        encoded_array_to_arrow_selected_with_options(&cove, &[1, 0], ArrowExportOptions::default())
            .unwrap();
    let binary = result
        .value
        .as_any()
        .downcast_ref::<arrow_array::BinaryArray>()
        .unwrap();
    assert_eq!(binary.value(0), &[2, 3, 4]);
    assert_eq!(binary.value(1), &[0, 1]);
}

#[test]
fn selected_export_rejects_trailing_varbytes_payload() {
    let mut values = Vec::new();
    values.extend_from_slice(&2u32.to_le_bytes());
    values.extend_from_slice(b"hi");
    values.extend_from_slice(&5u32.to_le_bytes());
    values.extend_from_slice(b"there");
    values.push(0);
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow_selected_with_options(&cove, &[1], ArrowExportOptions::default()),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn selected_export_reads_bool_rows_directly_with_nulls() {
    let values = [1u8, 0u8, 1u8];
    let mut validity = ValidityBitmapBuilder::new(3).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 3);
    let cove = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        3,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_selected_with_options(
        &cove,
        &[2, 1, 0],
        ArrowExportOptions::default(),
    )
    .unwrap();
    let bools = result
        .value
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(bools.value(0));
    assert!(bools.is_null(1));
    assert!(bools.value(2));
}

#[test]
fn bitset_export_reads_bool_rows_directly_with_nulls() {
    let values = [1u8, 0u8, 1u8, 1u8];
    let mut validity = ValidityBitmapBuilder::new(4).unwrap();
    validity.set_null(2).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 4);
    let cove = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        4,
        CoveEncodingKind::PlainFixed,
        Some(bitmap),
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_row_selection_options(
        &cove,
        ArrowRowSelection::Bitset {
            words: &[0b1101],
            len: 4,
        },
        ArrowExportOptions::default(),
    )
    .unwrap();
    let bools = result
        .value
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(bools.value(0));
    assert!(bools.is_null(1));
    assert!(bools.value(2));
}

#[test]
fn selected_export_full_row_selection_matches_bulk_transform_decode() {
    let payload = LocalCodebookPayload {
        values: LocalCodebookValues::VarBytes(vec![b"red".to_vec(), b"blue".to_vec()]),
        indexes: LocalIndexPayload::Rle(RlePayload {
            runs: vec![(0, 1), (1, 2)],
        }),
    };
    let data = payload.encode();
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        3,
        CoveEncodingKind::LocalCodebook,
        None,
        &data,
        None,
    );

    let full = encoded_array_to_arrow_with_options(&cove, ArrowExportOptions::default())
        .unwrap()
        .value;
    let selected = encoded_array_to_arrow_selected_with_options(
        &cove,
        &[0, 1, 2],
        ArrowExportOptions::default(),
    )
    .unwrap()
    .value;

    let full = full
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    let selected = selected
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.value(0), full.value(0));
    assert_eq!(selected.value(1), full.value(1));
    assert_eq!(selected.value(2), full.value(2));
}

#[test]
fn strict_export_rejects_json_without_extension_metadata() {
    let mut values = Vec::new();
    values.extend_from_slice(&7u32.to_le_bytes());
    values.extend_from_slice(br#"{"a":1}"#);
    let cove = EncodedArray::new(
        CoveLogicalType::Json,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::UnsupportedEncoding(_))
    ));
    let result = encoded_array_to_arrow_with_report(&cove).unwrap();
    assert!(result.report.has_lossy_or_unsupported());
    assert_eq!(result.value.data_type(), &DataType::Utf8);
}

#[test]
fn record_batch_emits_json_extension_metadata_when_requested() {
    let mut values = Vec::new();
    values.extend_from_slice(&7u32.to_le_bytes());
    values.extend_from_slice(br#"{"a":1}"#);
    let cove = EncodedArray::new(
        CoveLogicalType::Json,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        None,
        &values,
        None,
    );

    let result = encoded_columns_to_record_batch_with_options(
        &[("payload", &cove)],
        ArrowExportOptions {
            emit_json_extension_metadata: true,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    let schema = result.value.schema();
    let field = schema.field(0);
    assert_eq!(
        field.metadata().get("ARROW:extension:name"),
        Some(&"cove.json".to_string())
    );
    assert!(result.report.issues.is_empty());
}

#[test]
fn strict_export_rejects_decimal_without_precision_scale() {
    let mut values = Vec::new();
    values.extend_from_slice(&123u64.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Decimal64,
        CovePhysicalKind::NumCode,
        1,
        CoveEncodingKind::NumCode,
        None,
        &values,
        None,
    );

    assert!(matches!(
        encoded_array_to_arrow(&cove),
        Err(CoveError::UnsupportedEncoding(_))
    ));
    let result = encoded_array_to_arrow_with_report(&cove).unwrap();
    assert!(result.report.has_lossy_or_unsupported());
    assert_eq!(result.value.data_type(), &DataType::Int64);
}

#[test]
fn exports_decimal128_with_explicit_context() {
    let mut values = Vec::new();
    values.extend_from_slice(&12345i128.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Decimal128,
        CovePhysicalKind::FixedBytes,
        1,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );

    let result = encoded_array_to_arrow_with_options(
        &cove,
        ArrowExportOptions {
            decimal: Some(ArrowDecimalContext {
                precision: 10,
                scale: 2,
            }),
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    assert!(result.report.issues.is_empty());
    assert_eq!(result.value.data_type(), &DataType::Decimal128(10, 2));
}

#[test]
fn exports_uuid_as_fixed_size_binary() {
    let values = [7u8; 16];
    let cove = EncodedArray::new(
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        1,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    assert_eq!(arrow.data_type(), &DataType::FixedSizeBinary(16));
    let uuids = arrow
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(uuids.value(0), &[7u8; 16]);
}

#[test]
fn record_batch_emits_uuid_extension_metadata_when_requested() {
    let values = [7u8; 16];
    let cove = EncodedArray::new(
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        1,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );

    let result = encoded_columns_to_record_batch_with_options(
        &[("id", &cove)],
        ArrowExportOptions {
            emit_uuid_extension_metadata: true,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    let schema = result.value.schema();
    let field = schema.field(0);
    assert_eq!(
        field.metadata().get("ARROW:extension:name"),
        Some(&"cove.uuid".to_string())
    );
    assert!(result.report.issues.is_empty());
}

#[test]
fn exports_filecode_dictionary_values_as_logical_utf8() {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("red"), inline_utf8_entry("blue")],
        payload: Vec::new(),
    };
    let mut codes = Vec::new();
    codes.extend_from_slice(&1u32.to_le_bytes());
    codes.extend_from_slice(&0u32.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        2,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );

    let arrow = encoded_array_to_arrow(&cove).unwrap();
    let strings = arrow
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "blue");
    assert_eq!(strings.value(1), "red");
}

#[test]
fn exports_filecode_dictionary_keys_when_requested() {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("red"), inline_utf8_entry("blue")],
        payload: Vec::new(),
    };
    let mut codes = Vec::new();
    codes.extend_from_slice(&1u32.to_le_bytes());
    codes.extend_from_slice(&0u32.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        2,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );

    let arrow = arrow_export_node_to_array(&ArrowExportNode::scalar(&cove)).unwrap();
    let dictionary = arrow
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert_eq!(dictionary.keys().value(0), 1);
    assert_eq!(dictionary.keys().value(1), 0);
    let values = dictionary
        .values()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(values.value(0), "red");
    assert_eq!(values.value(1), "blue");
}

#[test]
fn selected_filecode_dictionary_output_preserves_keys() {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("red"), inline_utf8_entry("blue")],
        payload: Vec::new(),
    };
    let mut codes = Vec::new();
    codes.extend_from_slice(&1u32.to_le_bytes());
    codes.extend_from_slice(&0u32.to_le_bytes());
    codes.extend_from_slice(&1u32.to_le_bytes());
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        3,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );

    let result = encoded_array_to_arrow_selected_with_options(
        &cove,
        &[1, 2],
        ArrowExportOptions {
            dictionary_policy: ArrowDictionaryPolicy::DictionaryKeys,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    let dictionary = result
        .value
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert_eq!(dictionary.keys().value(0), 0);
    assert_eq!(dictionary.keys().value(1), 1);
}

#[test]
fn mixed_file_dictionary_remaps_only_used_logical_values() {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_binary_entry(&[0xff]), inline_utf8_entry("red")],
        payload: Vec::new(),
    };
    let codes = 1u32.to_le_bytes();
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        1,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );

    let result = encoded_array_to_arrow_with_options(
        &cove,
        ArrowExportOptions {
            dictionary_policy: ArrowDictionaryPolicy::DictionaryKeys,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    let dictionary = result
        .value
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert_eq!(dictionary.keys().value(0), 0);
    let values = dictionary
        .values()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values.value(0), "red");
}

#[test]
fn unrelated_redacted_dictionary_entry_does_not_block_remap() {
    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 2,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![redacted_binary_entry(), inline_utf8_entry("red")],
        payload: Vec::new(),
    };
    let codes = 1u32.to_le_bytes();
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        1,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );

    let result = encoded_array_to_arrow_with_options(
        &cove,
        ArrowExportOptions {
            dictionary_policy: ArrowDictionaryPolicy::DictionaryKeys,
            ..ArrowExportOptions::default()
        },
    )
    .unwrap();
    let dictionary = result
        .value
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert_eq!(dictionary.keys().value(0), 0);
}

#[test]
fn filecode_dictionary_output_uses_standard_values_even_with_view_option() {
    let options = ArrowExportOptions {
        dictionary_policy: ArrowDictionaryPolicy::DictionaryKeys,
        varbytes_policy: ArrowVarBytesExportPolicy::View,
        ..ArrowExportOptions::default()
    };
    let schema_type = arrow_data_type_for_column_export_options(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        true,
        options,
    )
    .unwrap()
    .value;
    assert_eq!(
        schema_type,
        DataType::Dictionary(Box::new(DataType::UInt32), Box::new(DataType::Utf8))
    );

    let dictionary = FileDictionary {
        header: FileDictionaryHeaderV1 {
            entry_count: 1,
            flags: 0,
            index_entry_len: FileDictionaryHeaderV1::INDEX_ENTRY_LEN,
            value_hash_algorithm: 0,
            payload_length: 0,
            reserved: [0; 24],
        },
        entries: vec![inline_utf8_entry("red")],
        payload: Vec::new(),
    };
    let codes = 0u32.to_le_bytes();
    let cove = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        1,
        CoveEncodingKind::FileCode,
        None,
        &codes,
        Some(&dictionary),
    );
    let result = encoded_array_to_arrow_with_options(&cove, options).unwrap();
    let dictionary = result
        .value
        .as_any()
        .downcast_ref::<DictionaryArray<UInt32Type>>()
        .unwrap();
    assert!(dictionary
        .values()
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .is_some());
    assert!(dictionary
        .values()
        .as_any()
        .downcast_ref::<StringViewArray>()
        .is_none());
}

#[test]
fn exports_list_of_int32() {
    let values = [1i32.to_le_bytes(), 2i32.to_le_bytes(), 3i32.to_le_bytes()].concat();
    let child = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        3,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );
    let layout = ListLayoutPayload {
        layout: ListLayout {
            offsets: vec![0, 2, 3],
        },
        child_row_count: 3,
    };
    let node = ArrowExportNode::List {
        layout: &layout,
        child: Box::new(ArrowExportNode::scalar(&child)),
        validity: None,
    };

    let arrow = arrow_export_node_to_array(&node).unwrap();
    let lists = arrow.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists.value(0).len(), 2);
    assert_eq!(lists.value(1).len(), 1);
}

#[test]
fn exports_nullable_list() {
    let values = [1i32.to_le_bytes(), 2i32.to_le_bytes()].concat();
    let child = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );
    let layout = ListLayoutPayload {
        layout: ListLayout {
            offsets: vec![0, 2, 2],
        },
        child_row_count: 2,
    };
    let mut validity = ValidityBitmapBuilder::new(2).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 2);
    let node = ArrowExportNode::List {
        layout: &layout,
        child: Box::new(ArrowExportNode::scalar(&child)),
        validity: Some(bitmap),
    };

    let arrow = arrow_export_node_to_array(&node).unwrap();
    let lists = arrow.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(lists.len(), 2);
    assert!(lists.is_null(1));
}

#[test]
fn exports_struct_with_nullable_parent() {
    let ids = [10i32.to_le_bytes(), 20i32.to_le_bytes()].concat();
    let id_array = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &ids,
        None,
    );
    let flags = [1u8, 0u8];
    let flag_array = EncodedArray::new(
        CoveLogicalType::Bool,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &flags,
        None,
    );
    let layout = StructLayoutPayload {
        layout: StructLayout {
            field_row_counts: vec![2, 2],
        },
        parent_null_handling_declared: true,
    };
    let mut validity = ValidityBitmapBuilder::new(2).unwrap();
    validity.set_null(1).unwrap();
    let validity_bytes = validity.into_bytes();
    let bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 2);
    let node = ArrowExportNode::Struct {
        layout: &layout,
        fields: vec![
            ArrowExportColumn::scalar("id", &id_array),
            ArrowExportColumn::scalar("flag", &flag_array),
        ],
        validity: Some(bitmap),
    };

    let arrow = arrow_export_node_to_array(&node).unwrap();
    let structs = arrow.as_any().downcast_ref::<StructArray>().unwrap();
    assert_eq!(structs.len(), 2);
    assert!(structs.is_null(1));
}

#[test]
fn exports_map_with_scalar_keys() {
    let mut keys_data = Vec::new();
    keys_data.extend_from_slice(&1u32.to_le_bytes());
    keys_data.extend_from_slice(b"a");
    keys_data.extend_from_slice(&1u32.to_le_bytes());
    keys_data.extend_from_slice(b"b");
    let keys = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &keys_data,
        None,
    );
    let values_data = [7i32.to_le_bytes(), 9i32.to_le_bytes()].concat();
    let values = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &values_data,
        None,
    );
    let layout = MapLayoutPayload {
        layout: MapLayout {
            offsets: vec![0, 2],
            key_row_count: 2,
            value_row_count: 2,
            keys_are_scalar: true,
            allow_duplicate_keys: false,
            canonical_keys: vec![b"a".to_vec(), b"b".to_vec()],
        },
    };
    let node = ArrowExportNode::Map {
        layout: &layout,
        keys: Box::new(ArrowExportNode::scalar(&keys)),
        values: Box::new(ArrowExportNode::scalar(&values)),
        validity: None,
        ordered: false,
    };

    let arrow = arrow_export_node_to_array(&node).unwrap();
    let map = arrow.as_any().downcast_ref::<MapArray>().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.entries().len(), 2);
}

#[test]
fn rejects_duplicate_map_keys() {
    let key_bytes = [0u8; 10];
    let keys = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &key_bytes,
        None,
    );
    let value_bytes = [0u8; 8];
    let values = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        2,
        CoveEncodingKind::PlainFixed,
        None,
        &value_bytes,
        None,
    );
    let layout = MapLayoutPayload {
        layout: MapLayout {
            offsets: vec![0, 2],
            key_row_count: 2,
            value_row_count: 2,
            keys_are_scalar: true,
            allow_duplicate_keys: false,
            canonical_keys: vec![b"a".to_vec(), b"a".to_vec()],
        },
    };
    let node = ArrowExportNode::Map {
        layout: &layout,
        keys: Box::new(ArrowExportNode::scalar(&keys)),
        values: Box::new(ArrowExportNode::scalar(&values)),
        validity: None,
        ordered: false,
    };

    assert!(matches!(
        arrow_export_node_to_array(&node),
        Err(CoveError::PageCorrupt)
    ));
}

#[test]
fn rejects_null_map_key_arrow_export() {
    let mut keys_data = Vec::new();
    keys_data.extend_from_slice(&1u32.to_le_bytes());
    keys_data.extend_from_slice(b"a");
    let mut validity = ValidityBitmapBuilder::new(1).unwrap();
    validity.set_null(0).unwrap();
    let validity_bytes = validity.into_bytes();
    let key_bitmap = crate::validity::ValidityBitmap::new(&validity_bytes, 1);
    let keys = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        1,
        CoveEncodingKind::VarBytes,
        Some(key_bitmap),
        &keys_data,
        None,
    );
    let values_data = [7i32.to_le_bytes()].concat();
    let values = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        1,
        CoveEncodingKind::PlainFixed,
        None,
        &values_data,
        None,
    );
    let layout = MapLayoutPayload {
        layout: MapLayout {
            offsets: vec![0, 1],
            key_row_count: 1,
            value_row_count: 1,
            keys_are_scalar: true,
            allow_duplicate_keys: false,
            canonical_keys: vec![b"a".to_vec()],
        },
    };
    let node = ArrowExportNode::Map {
        layout: &layout,
        keys: Box::new(ArrowExportNode::scalar(&keys)),
        values: Box::new(ArrowExportNode::scalar(&values)),
        validity: None,
        ordered: false,
    };

    assert!(matches!(
        arrow_export_node_to_array(&node),
        Err(CoveError::UnsupportedEncoding(_))
    ));
}

#[test]
fn rejects_oversized_arrow_offsets() {
    let values = [1i32.to_le_bytes()].concat();
    let child = EncodedArray::new(
        CoveLogicalType::Int32,
        CovePhysicalKind::FixedBytes,
        1,
        CoveEncodingKind::PlainFixed,
        None,
        &values,
        None,
    );
    let layout = ListLayoutPayload {
        layout: ListLayout {
            offsets: vec![0, i32::MAX as u32 + 1],
        },
        child_row_count: i32::MAX as u32 + 1,
    };
    let node = ArrowExportNode::List {
        layout: &layout,
        child: Box::new(ArrowExportNode::scalar(&child)),
        validity: None,
    };

    assert!(matches!(
        arrow_export_node_to_array(&node),
        Err(CoveError::UnsupportedEncoding(_))
    ));
}

#[test]
fn exports_record_batch_from_named_columns() {
    let ids = [1u64.to_le_bytes(), 2u64.to_le_bytes()].concat();
    let id_array = EncodedArray::new(
        CoveLogicalType::UInt64,
        CovePhysicalKind::NumCode,
        2,
        CoveEncodingKind::NumCode,
        None,
        &ids,
        None,
    );
    let mut names = Vec::new();
    names.extend_from_slice(&1u32.to_le_bytes());
    names.extend_from_slice(b"a");
    names.extend_from_slice(&1u32.to_le_bytes());
    names.extend_from_slice(b"b");
    let name_array = EncodedArray::new(
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        2,
        CoveEncodingKind::VarBytes,
        None,
        &names,
        None,
    );

    let batch =
        encoded_columns_to_record_batch(&[("id", &id_array), ("name", &name_array)]).unwrap();
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
}

fn inline_utf8_entry(value: &str) -> FileDictionaryIndexEntryV1 {
    let mut canonical = wire::encode_u64_leb128(value.len() as u64);
    canonical.extend_from_slice(value.as_bytes());
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

fn inline_binary_entry(value: &[u8]) -> FileDictionaryIndexEntryV1 {
    let mut canonical = wire::encode_u64_leb128(value.len() as u64);
    canonical.extend_from_slice(value);
    let mut inline_data = [0u8; 16];
    inline_data[..canonical.len()].copy_from_slice(&canonical);
    FileDictionaryIndexEntryV1 {
        value_tag: ValueTag::Binary as u16,
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

fn redacted_binary_entry() -> FileDictionaryIndexEntryV1 {
    FileDictionaryIndexEntryV1 {
        value_tag: ValueTag::Binary as u16,
        storage_class: StorageClass::Redacted as u8,
        flags: 0,
        inline_len: 0,
        reserved0: [0; 3],
        inline_data: [0; 16],
        payload_offset: 0,
        payload_length: 0,
        canonical_hash64: 0,
        reserved1: 0,
    }
}
